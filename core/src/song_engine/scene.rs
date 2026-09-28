//! The arrangement: which scene plays, what it changes when it starts, and
//! which track each sidechain listens to while it plays.

use alloc::vec::Vec;

use crate::SAMPLE_RATE;

use super::automation::ActiveAutomation;
use super::instrument::SongInstrument;
use super::track::{make_arp, pan_gains};
use super::SongEngine;

/// How long a track the new scene drops takes to fade out: 10 ms.
pub(super) const SCENE_FADE: u32 = 441;

impl SongEngine {
    /// Apply a scene's track configuration.
    pub(super) fn apply_scene(&mut self, scene_idx: usize) {
        if scene_idx >= self.scenes.len() {
            return;
        }

        // Update tempo if scene overrides it
        if let Some(t) = self.scenes[scene_idx].tempo {
            let t = t.clamp(20.0, 999.0);
            self.tempo = t;
            self.samples_per_step = SAMPLE_RATE * 60.0 / t / 4.0;
            // Update BPM on module instruments
            for inst in self.instruments.iter_mut() {
                inst.set_bpm(t);
            }
            self.send_delay.set_bpm(t, SAMPLE_RATE);
            self.retune_fx(t);
        }
        let tempo = self.tempo;
        let scene = &self.scenes[scene_idx];

        // Apply effect overrides
        if let Some(rmix) = scene.reverb_mix {
            self.reverb_wet_level = rmix;
        }
        if let Some(dmix) = scene.delay_mix {
            self.delay_wet_level = dmix;
        }
        // Freeze is per scene: it holds only where asked for.
        self.freeze_set = scene.reverb_freeze.unwrap_or(false);
        self.send_reverb.set_freeze(self.freeze_set || self.freeze_pad);

        // Automation lanes are set up after the scene's tracks are activated
        // (targets resolve against the new layout, not the previous scene's).
        self.active_automations.clear();
        self.scene_step = 0;
        if self.arrangement_idx < self.arrangement.len() {
            let (_, repeat) = self.arrangement[self.arrangement_idx];
            self.scene_total_steps = repeat as usize * self.steps_per_bar;
        }

        // Update tracks from scene
        // Release all currently-playing notes before deactivating tracks.
        // This prevents stale notes ringing across scene transitions.
        for ti in 0..self.tracks.len() {
            if self.tracks[ti].active && self.tracks[ti].current_notes_count > 0 {
                Self::release_track_notes(&mut self.tracks[ti], &mut self.instruments);
                self.tracks[ti].gate_samples_remaining = 0.0;
            }
        }

        // Reset to make scene tracks the active set. A track the new scene
        // does not name fades out rather than stopping dead; one it does name
        // is simply active again below.
        for track in self.tracks.iter_mut() {
            if track.active {
                track.leaving = SCENE_FADE;
            }
            track.active = false;
        }

        // Activate scene tracks: each scene track reconfigures the top-level
        // track slot with the same name (never by position).
        let global_duck = self.sidechain_amount;
        for st in scene.tracks.iter() {
            if let Some(i) = self.track_names.iter().position(|n| n == &st.name) {
                // The scene names a module; this track's own copy of it plays.
                let inst_idx =
                    self.inst_for.get(i * self.n_instruments + st.instrument_idx).copied().unwrap_or(st.instrument_idx);
                let tp = &mut self.tracks[i];
                // Only a track that was already sounding glides to its new
                // settings; one coming in from silence starts at them.
                let arriving = !tp.sounding();
                tp.instrument_idx = inst_idx;
                tp.pattern_idx = st.pattern_idx;
                tp.play = st.play;
                tp.velocity = st.velocity;
                tp.level = st.level;
                tp.gate = st.gate;
                let (pan_l, pan_r) = pan_gains(st.pan);
                tp.pan_l = pan_l;
                tp.pan_r = pan_r;
                tp.delay_send = st.delay_send;
                tp.reverb_send = st.reverb_send;
                tp.sidechain_amount = st.sidechain;
                tp.stereo_src =
                    inst_idx < self.instruments.len() && matches!(self.instruments[inst_idx], SongInstrument::Beats(_));
                tp.active = true;
                tp.leaving = 0;
                tp.current_step = 0;
                tp.current_notes_count = 0;
                tp.gate_samples_remaining = 0.0;
                tp.arp_cfg = st.arp;
                tp.arp = st.arp.map(|c| make_arp(&c, tempo));
                if arriving {
                    tp.heard = tp.target();
                    tp.heard_duck = tp.sidechain_amount.unwrap_or(global_duck);
                }
            }
        }

        // Recompute kick track index after scene reassignment
        self.kick_track_idx = self.tracks.iter().position(|t| {
            t.active
                && t.instrument_idx < self.instruments.len()
                && matches!(self.instruments[t.instrument_idx], SongInstrument::Beats(_))
        });
        self.resolve_sidechain_sources(scene_idx);

        // Reuses the existing capacity: applying a scene runs on the audio thread.
        let mut lanes = core::mem::take(&mut self.active_automations);
        lanes.clear();
        for auto_idx in 0..self.scenes[scene_idx].automations.len() {
            let target_name = core::mem::take(&mut self.scenes[scene_idx].automations[auto_idx].target);
            let resolved = self.resolve_auto_target(&target_name);
            self.scenes[scene_idx].automations[auto_idx].target = target_name;
            if let Some(target) = resolved {
                lanes.push(ActiveAutomation { target, scene_idx, auto_idx });
            }
        }
        self.active_automations = lanes;
    }

    /// Bind every track's `sidechain from=` to a track index, and mark which
    /// tracks have to maintain an envelope. Names are resolved here rather than
    /// at compile time because a scene can rebind which track plays what.
    pub(super) fn resolve_sidechain_sources(&mut self, scene_idx: usize) {
        for t in self.tracks.iter_mut() {
            t.sc_source = None;
            t.is_sc_source = false;
        }
        // Names are borrowed, never cloned: this runs on the audio thread at
        // every scene change and at every live swap.
        let global_idx = match self.global_sc_source.as_deref() {
            Some(name) => self.find_source_track(name),
            None => self.kick_track_idx,
        };
        for ti in 0..self.tracks.len() {
            if !self.tracks[ti].active {
                continue;
            }
            let named = self
                .scenes
                .get(scene_idx)
                .and_then(|s| s.tracks.iter().find(|st| st.name == self.track_names[ti]))
                .and_then(|st| st.sidechain_source.as_deref());
            let source = match named {
                Some(name) => self.find_source_track(name),
                None => global_idx,
            };
            if let Some(si) = source {
                if si != ti {
                    self.tracks[ti].sc_source = Some(si);
                    self.tracks[si].is_sc_source = true;
                }
            }
        }
        // The global sends duck against the same source the song names.
        self.global_sc_idx = global_idx;
        if let Some(si) = self.global_sc_idx {
            self.tracks[si].is_sc_source = true;
        }
        if let Some(ki) = self.kick_track_idx {
            self.tracks[ki].is_sc_source = true;
        }
    }

    /// A sidechain source names a track, or the module a track plays.
    fn find_source_track(&self, name: &str) -> Option<usize> {
        self.track_names.iter().position(|n| n == name).filter(|i| self.tracks[*i].active).or_else(|| {
            let inst = self.instrument_names.iter().position(|n| n == name)?;
            self.tracks.iter().position(|t| t.active && t.instrument_idx == inst)
        })
    }

    pub(super) fn check_arrangement_advance(&mut self) {
        if self.arrangement.is_empty() {
            return;
        }
        if self.arrangement_idx >= self.arrangement.len() {
            return;
        }

        self.arrangement_bar_count += 1;
        let (_, repeat) = self.arrangement[self.arrangement_idx];

        // Each repeat = one bar's worth of the pattern
        if self.arrangement_bar_count >= repeat {
            self.arrangement_bar_count = 0;
            self.arrangement_idx += 1;

            if self.arrangement_idx < self.arrangement.len() {
                let (scene_idx, _) = self.arrangement[self.arrangement_idx];
                self.apply_scene(scene_idx);
                self.reassert_held();
            } else {
                // Arrangement finished
                self.running = false;
            }
        }
    }

    pub fn arrangement_bars(&self) -> u32 {
        self.arrangement.iter().map(|(_, r)| *r).sum()
    }

    /// The arrangement as `(scene name, bars, BPM)`, in order. A mix report
    /// can only talk about a song's shape if it knows where the sections are,
    /// and the tempo has to come along because a scene can change it -- put
    /// every section on the song's opening tempo and the later ones land in
    /// the wrong place.
    pub fn sections(&self) -> Vec<(&str, u32, f32)> {
        let mut bpm = self.tempo;
        let mut out = Vec::new();
        for (i, repeat) in &self.arrangement {
            let Some(scene) = self.scenes.get(*i) else { continue };
            if let Some(t) = scene.tempo {
                bpm = t.clamp(20.0, 999.0);
            }
            out.push((scene.name.as_str(), *repeat, bpm));
        }
        out
    }
}
