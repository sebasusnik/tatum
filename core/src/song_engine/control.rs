//! Changes made while the song plays, without recompiling it: levels, pan,
//! tempo, patterns, sends, module parameters, notes from outside, groove.

use crate::params::ParamId;
use crate::rng::Rng;
use crate::SAMPLE_RATE;

use super::instrument::apply_param;
use super::track::{pan_gains, seed_from_name, MAX_TRACK_LEVEL};
use super::SongEngine;

impl SongEngine {
    // ═══════════════════════════════════════════════════════
    //  Real-time track control (no recompile)
    // ═══════════════════════════════════════════════════════

    pub fn track_count(&self) -> usize {
        self.tracks.len()
    }

    pub fn track_name(&self, idx: usize) -> &str {
        if idx < self.track_names.len() {
            &self.track_names[idx]
        } else {
            ""
        }
    }

    pub fn track_kind(&self, idx: usize) -> &str {
        self.tracks
            .get(idx)
            .and_then(|t| self.instruments.get(t.instrument_idx))
            .map_or("unknown", |inst| inst.kind_str())
    }

    pub fn track_level(&self, idx: usize) -> f32 {
        self.tracks.get(idx).map_or(0.0, |t| t.level)
    }

    pub fn track_pan(&self, idx: usize) -> f32 {
        self.tracks.get(idx).map_or(0.0, |t| t.pan)
    }

    // Every runtime setter ignores a value that is not a number. They are fed
    // from outside -- a knob, the browser, a MIDI map -- and `clamp` passes a
    // NaN through, which would sit in a filter's state and silence the track
    // until it is rebuilt.
    pub fn set_track_level(&mut self, idx: usize, level: f32) {
        if !level.is_finite() {
            return;
        }
        if let Some(track) = self.tracks.get_mut(idx) {
            track.level = level.clamp(0.0, MAX_TRACK_LEVEL);
        }
    }

    pub fn set_track_pan(&mut self, idx: usize, pan: f32) {
        if !pan.is_finite() {
            return;
        }
        if let Some(track) = self.tracks.get_mut(idx) {
            let p = pan.clamp(-1.0, 1.0);
            track.pan = p;
            let (l, r) = pan_gains(p);
            track.pan_l = l;
            track.pan_r = r;
        }
    }

    // ═══════════════════════════════════════════════════════
    //  Phase 2: Runtime mutations (no recompile)
    // ═══════════════════════════════════════════════════════

    pub fn set_tempo(&mut self, bpm: f32) {
        if !bpm.is_finite() {
            return;
        }
        let bpm = bpm.clamp(20.0, 999.0);
        self.tempo = bpm;
        self.samples_per_step = SAMPLE_RATE * 60.0 / bpm / 4.0;
        self.current_step_duration = self.effective_step_samples(self.global_step);
        for inst in self.instruments.iter_mut() {
            inst.set_bpm(bpm);
        }
        for track in self.tracks.iter_mut() {
            if let (Some(arp), Some(cfg)) = (track.arp.as_mut(), track.arp_cfg) {
                arp.set_bpm(bpm * cfg.rate_mult);
            }
        }
        self.send_delay.set_bpm(bpm, SAMPLE_RATE);
        self.retune_fx(bpm);
    }

    /// Pattern index a track is currently playing (for tests and UI).
    pub fn track_pattern(&self, idx: usize) -> usize {
        self.tracks.get(idx).map_or(0, |t| t.pattern_idx)
    }

    pub fn set_track_pattern(&mut self, track_idx: usize, pattern_idx: usize) {
        if let Some(track) = self.tracks.get_mut(track_idx) {
            if pattern_idx < self.patterns.len() {
                track.pattern_idx = pattern_idx;
                track.current_step = 0;
            }
        }
    }

    pub fn set_track_velocity(&mut self, track_idx: usize, velocity: f32) {
        if !velocity.is_finite() {
            return;
        }
        if let Some(track) = self.tracks.get_mut(track_idx) {
            track.velocity = velocity.clamp(0.0, 1.0);
        }
    }

    /// Switch one node of a track's insert chain in or out. `wet` 0 bypasses
    /// it and stops it being processed, 1 is the node alone. Returns false if
    /// there is no such track or node, so a live edit can tell a no-op from a
    /// real one. This is a fast-path edit: the node is already built, so it
    /// applies inside the bar with no swap and no voice restart.
    pub fn set_node_wet(&mut self, track_idx: usize, node_idx: usize, wet: f32) -> bool {
        if !wet.is_finite() {
            return false;
        }
        self.tracks.get_mut(track_idx).is_some_and(|t| t.insert_fx.set_wet(node_idx, wet))
    }

    /// The `as` name of each node in a track's insert chain, in chain order;
    /// `None` for a node nobody named.
    pub fn track_node_labels(&self, track_idx: usize) -> impl Iterator<Item = Option<&str>> {
        self.tracks.get(track_idx).into_iter().flat_map(|t| t.fx_labels.iter().map(|l| l.as_ref().map(|l| l.as_str())))
    }

    /// A note played from outside the pattern -- a key, a pad -- on the
    /// instrument track `track_idx` plays. A muted track does not sound it,
    /// the same as its own pattern: bring the fader up to hear what you play.
    pub fn live_note_on(&mut self, track_idx: usize, note: u8, velocity: f32) {
        if track_idx >= self.tracks.len() {
            return;
        }
        let inst = self.trigger_instrument(track_idx);
        if let Some(i) = self.instruments.get_mut(inst) {
            i.note_on(note, velocity.clamp(0.0, 1.0));
        }
    }

    /// Release a note played with [`Self::live_note_on`]. Reaches the
    /// instrument even on a muted track, so a note held while the fader
    /// came down still lets go.
    pub fn live_note_off(&mut self, track_idx: usize, note: u8) {
        let Some(inst) = self.tracks.get(track_idx).map(|t| t.instrument_idx) else { return };
        if let Some(i) = self.instruments.get_mut(inst) {
            i.note_off(note);
        }
    }

    /// Bend instrument `inst_idx` by `ratio` of its frequency (1.0 is none).
    pub fn set_pitch_bend(&mut self, inst_idx: usize, ratio: f32) {
        if !ratio.is_finite() {
            return;
        }
        if let Some(i) = self.instruments.get_mut(inst_idx) {
            i.set_pitch_bend(ratio);
        }
    }

    /// Wet level of the global reverb return, 0..1 of what `reverb` sets up.
    /// The same value `reverb_mix =` and `auto reverb_mix` write, so a scene
    /// that sets it takes over again when it starts.
    pub fn set_reverb_mix(&mut self, mix: f32) {
        if !mix.is_finite() {
            return;
        }
        self.reverb_wet_level = mix.clamp(0.0, 1.0);
    }

    /// Wet level of the global delay return, like [`Self::set_reverb_mix`].
    pub fn set_delay_mix(&mut self, mix: f32) {
        if !mix.is_finite() {
            return;
        }
        self.delay_wet_level = mix.clamp(0.0, 1.0);
    }

    /// Hold the reverb tail: no decay and no new input while frozen.
    pub fn set_reverb_freeze(&mut self, frozen: bool) {
        self.send_reverb.set_freeze(frozen);
    }

    pub fn set_track_gate(&mut self, track_idx: usize, gate: f32) {
        if !gate.is_finite() {
            return;
        }
        if let Some(track) = self.tracks.get_mut(track_idx) {
            track.gate = gate.clamp(0.0, 1.0);
        }
    }

    pub fn pattern_count(&self) -> usize {
        self.patterns.len()
    }

    pub fn pattern_name(&self, idx: usize) -> &str {
        self.patterns.get(idx).map_or("", |p| &p.name)
    }

    /// Set a named module parameter at runtime. Returns false if the
    /// instrument does not exist or the name is not in the registry.
    pub fn set_module_param(&mut self, inst_idx: usize, name: &str, value: f32) -> bool {
        if !value.is_finite() {
            return false;
        }
        match self.instruments.get_mut(inst_idx) {
            Some(inst) => inst.set_param_by_name(name, value),
            None => false,
        }
    }

    pub fn instrument_count(&self) -> usize {
        self.instruments.len()
    }

    pub fn instrument_name(&self, idx: usize) -> &str {
        self.instrument_names.get(idx).map_or("", |n| n.as_str())
    }

    pub fn instrument_index(&self, name: &str) -> Option<usize> {
        self.instrument_names.iter().position(|n| n == name)
    }

    /// The live instrument a track plays right now. With one copy per track
    /// that names a module this is not the compiled index.
    pub fn track_instrument(&self, idx: usize) -> Option<usize> {
        self.tracks.get(idx).map(|t| t.instrument_idx)
    }

    /// Set a parameter already resolved to a registry id. False if the
    /// instrument does not exist or the id belongs to another module kind.
    pub fn set_module_param_id(&mut self, inst_idx: usize, id: ParamId, value: f32) -> bool {
        if !value.is_finite() {
            return false;
        }
        match self.instruments.get_mut(inst_idx) {
            Some(inst) => apply_param(inst, id, value),
            None => false,
        }
    }

    /// Swing 0.5 (straight) ..= 0.75 (hard shuffle). Takes effect on the next step.
    pub fn set_swing(&mut self, swing: f32) {
        if !swing.is_finite() {
            return;
        }
        self.swing = swing.clamp(0.5, 0.75);
    }

    /// Put every track's random stream back to its seed. Called wherever the
    /// clock stream is reset, so a render is reproducible start to start.
    pub(super) fn reseed_tracks(&mut self) {
        for (i, t) in self.tracks.iter_mut().enumerate() {
            let name = self.track_names.get(i).map(|s| s.as_str()).unwrap_or("");
            t.rng = Rng::new(seed_from_name(name));
        }
    }

    pub fn set_humanize(&mut self, velocity: f32, timing: f32) {
        if !(velocity.is_finite() && timing.is_finite()) {
            return;
        }
        self.humanize_velocity = velocity.clamp(0.0, 1.0);
        self.humanize_timing = timing.clamp(0.0, 1.0);
    }

    /// A track's send into the reverb (`reverb`) or the delay, 0..1.
    pub fn set_track_send(&mut self, track: usize, reverb: bool, amount: f32) {
        if !amount.is_finite() {
            return;
        }
        if let Some(t) = self.tracks.get_mut(track) {
            let amount = amount.clamp(0.0, 1.0);
            if reverb {
                t.reverb_send = amount;
            } else {
                t.delay_send = amount;
            }
        }
    }

    /// A named parameter (`cutoff`, `tilt`, `drive`...) of one node of a
    /// track's insert chain, or of the master chain when `track` is `None`.
    pub fn set_node_param(&mut self, track: Option<usize>, node: usize, param: &str, value: f32) -> bool {
        if !value.is_finite() {
            return false;
        }
        match track {
            None => self.master_fx.set_node_param(node, param, value),
            Some(t) => self.tracks.get_mut(t).is_some_and(|t| t.insert_fx.set_node_param(node, param, value)),
        }
    }

    /// How much of the reverb return is heard, 0..1: a scene's
    /// `reverb_mix`, or where a knob left it.
    pub fn reverb_mix(&self) -> f32 {
        self.reverb_wet_level
    }

    pub fn delay_mix(&self) -> f32 {
        self.delay_wet_level
    }

    pub fn swing(&self) -> f32 {
        self.swing
    }

    pub fn humanize(&self) -> (f32, f32) {
        (self.humanize_velocity, self.humanize_timing)
    }
}
