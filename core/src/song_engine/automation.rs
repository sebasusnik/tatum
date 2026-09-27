//! Automation lanes: `auto` targets resolved once when a scene starts, and
//! evaluated once per block while it plays.

use super::track::MAX_TRACK_LEVEL;
use super::SongEngine;

/// The longest name in the parameter registry is 13 bytes; this leaves room.
/// `inline_name_holds_every_registry_param` keeps that true as params are added.
pub const INLINE_NAME_CAP: usize = 24;

/// A parameter name stored inline. Automation targets are resolved when a scene
/// starts, which happens on the audio thread, where a `String` would allocate.
#[derive(Clone, Copy)]
pub(super) struct InlineName {
    bytes: [u8; INLINE_NAME_CAP],
    len: u8,
}

impl InlineName {
    /// `None` when the name does not fit. No registry name is that long.
    pub(super) fn new(s: &str) -> Option<Self> {
        let src = s.as_bytes();
        if src.len() > INLINE_NAME_CAP {
            return None;
        }
        let mut bytes = [0u8; INLINE_NAME_CAP];
        bytes[..src.len()].copy_from_slice(src);
        Some(Self { bytes, len: src.len() as u8 })
    }

    pub(super) fn as_str(&self) -> &str {
        // Always built from a &str, so the prefix is valid UTF-8.
        core::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or("")
    }
}

/// A running automation lane within a scene. `keyframes` points back into
/// `scenes[scene_idx].automations[auto_idx]` rather than copying them: applying a
/// scene happens on the audio thread.
pub(super) struct ActiveAutomation {
    pub(super) target: AutoTarget,
    pub(super) scene_idx: usize,
    pub(super) auto_idx: usize,
}

/// Resolved automation target.
pub(super) enum AutoTarget {
    InstrumentParam {
        instrument_idx: usize,
        param_name: InlineName,
    },
    MasterParam {
        param_name: InlineName,
    },
    TrackLevel {
        track_idx: usize,
    },
    /// Dry/wet of one named node in a track's insert chain, resolved to its
    /// position at compile time so the audio thread never looks up a name.
    TrackNodeWet {
        track_idx: usize,
        node_idx: usize,
    },
    ReverbMix,
    DelayMix,
    ReverbFreeze,
}

impl SongEngine {
    /// Resolve an automation target string to an AutoTarget.
    pub(super) fn resolve_auto_target(&self, target: &str) -> Option<AutoTarget> {
        match target {
            "reverb_mix" => Some(AutoTarget::ReverbMix),
            "reverb_freeze" => Some(AutoTarget::ReverbFreeze),
            "delay_mix" => Some(AutoTarget::DelayMix),
            _ => {
                // Check for "instrument.param" or "track.level"
                if let Some(dot_pos) = target.find('.') {
                    let name = &target[..dot_pos];
                    let param = &target[dot_pos + 1..];

                    if name == "master" {
                        return InlineName::new(param).map(|param_name| AutoTarget::MasterParam { param_name });
                    }

                    // `<track>.<node> wet`: two dots, because the node is
                    // named inside a track. Resolved to a position here, once,
                    // so the audio thread never compares a string.
                    if let Some(inner) = param.strip_suffix(".wet") {
                        if let Some(ti) = self.track_names.iter().position(|n| n == name) {
                            if let Some(ni) = self.tracks[ti]
                                .fx_labels
                                .iter()
                                .position(|l| l.as_ref().is_some_and(|l| l.as_str() == inner))
                            {
                                return Some(AutoTarget::TrackNodeWet { track_idx: ti, node_idx: ni });
                            }
                        }
                    }

                    if param == "level" {
                        // `<track> level` by track name, else by the instrument a track uses
                        let track_idx = self.track_names.iter().position(|n| n == name).or_else(|| {
                            self.instrument_names.iter().position(|n| n == name).and_then(|inst_idx| {
                                self.tracks.iter().position(|t| t.instrument_idx == inst_idx && t.active)
                            })
                        });
                        if let Some(ti) = track_idx {
                            return Some(AutoTarget::TrackLevel { track_idx: ti });
                        }
                    }

                    // Try as instrument.param
                    if let Some(inst_idx) = self.instrument_names.iter().position(|n| n == name) {
                        return InlineName::new(param)
                            .map(|param_name| AutoTarget::InstrumentParam { instrument_idx: inst_idx, param_name });
                    }
                }
                None
            }
        }
    }

    /// Automation, evaluated once per block. It used to run in `advance_step`,
    /// which meant a sweep moved in sixteenth-note stairs: eight jumps a second
    /// on a resonant filter, which is heard as steps rather than a sweep.
    pub(super) fn apply_automation(&mut self) {
        if self.scene_total_steps == 0 || self.active_automations.is_empty() {
            return;
        }
        let within_step = if self.current_step_duration > 0.0 {
            (self.sample_counter / self.current_step_duration).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let progress = ((self.scene_step as f32 + within_step) / self.scene_total_steps as f32).clamp(0.0, 1.0);
        let lanes = core::mem::take(&mut self.active_automations);
        for auto_lane in &lanes {
            if self.holds_auto(&auto_lane.target) {
                continue;
            }
            let keyframes = &self.scenes[auto_lane.scene_idx].automations[auto_lane.auto_idx].keyframes;
            let value = interpolate_automation(keyframes, progress);
            match &auto_lane.target {
                AutoTarget::InstrumentParam { instrument_idx, param_name } => {
                    // `auto cloud cutoff` means the module, so it reaches every
                    // copy of it — one per track that named it.
                    if *instrument_idx < self.instruments.len() {
                        let names = &self.instrument_names;
                        for (i, inst) in self.instruments.iter_mut().enumerate() {
                            let same = match (names.get(*instrument_idx), names.get(i)) {
                                (Some(a), Some(b)) => a == b,
                                _ => i == *instrument_idx,
                            };
                            if same {
                                inst.set_param_by_name(param_name.as_str(), value);
                            }
                        }
                    }
                }
                AutoTarget::MasterParam { param_name } => {
                    self.master_fx.set_param(param_name.as_str(), value);
                }
                AutoTarget::TrackNodeWet { track_idx, node_idx } => {
                    if let Some(t) = self.tracks.get_mut(*track_idx) {
                        t.insert_fx.set_wet(*node_idx, value);
                    }
                }
                AutoTarget::TrackLevel { track_idx } => {
                    if *track_idx < self.tracks.len() {
                        self.tracks[*track_idx].level = value.clamp(0.0, MAX_TRACK_LEVEL);
                    }
                }
                AutoTarget::ReverbMix => self.reverb_wet_level = value,
                AutoTarget::ReverbFreeze => {
                    self.freeze_set = value >= 0.5;
                    self.apply_freeze();
                }
                AutoTarget::DelayMix => self.delay_wet_level = value,
            }
        }
        self.active_automations = lanes;
    }
}

/// Interpolate automation keyframes at a given progress (0.0 - 1.0).
fn interpolate_automation(keyframes: &[f32], progress: f32) -> f32 {
    let p = progress.clamp(0.0, 1.0);
    match keyframes.len() {
        0 => 0.0,
        1 => keyframes[0],
        2 => {
            // Linear: start → end
            keyframes[0] + (keyframes[1] - keyframes[0]) * p
        }
        3 => {
            // Triangle: start → peak (at midpoint) → end
            if p < 0.5 {
                let t = p * 2.0;
                keyframes[0] + (keyframes[1] - keyframes[0]) * t
            } else {
                let t = (p - 0.5) * 2.0;
                keyframes[1] + (keyframes[2] - keyframes[1]) * t
            }
        }
        _ => keyframes[0], // shouldn't happen
    }
}
