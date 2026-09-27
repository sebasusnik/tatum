//! The engine's half of a live swap (see `live.rs`): finding the next bar
//! line, starting just before it, and taking over from the engine it replaces
//! whatever the edit did not change.

use crate::{math, SAMPLE_RATE};

use super::sequencer::{PendingTrigger, MAX_PENDING_TRIGGERS};
use super::SongEngine;

impl SongEngine {
    // ═══════════════════════════════════════════════════════
    //  Live session support (see live.rs)
    // ═══════════════════════════════════════════════════════

    /// Samples this engine will render before the next step fires: the trigger
    /// happens on the first sample where the step clock reaches the step
    /// duration. Zero means the next sample rendered is that step.
    fn samples_until_step(&self) -> f32 {
        let k = -math::floor(-(self.current_step_duration - self.sample_counter));
        (k - 1.0).max(0.0)
    }

    /// Samples this engine will render before the first step of the next bar
    /// fires. Exact within the current step; across the remaining steps it
    /// assumes no timing humanization, which a caller absorbs by asking again
    /// every block. `usize::MAX` when not running.
    pub fn samples_until_bar(&self) -> usize {
        if !self.running || self.steps_per_bar == 0 {
            return usize::MAX;
        }
        let mut total = self.samples_until_step();
        let mut step = self.global_step;
        while !step.is_multiple_of(self.steps_per_bar) {
            step += 1;
            total += self.effective_step_samples(step);
        }
        total as usize
    }

    /// Bar the next step to fire belongs to. On a bar line this is the bar
    /// about to start, while `current_bar` still reads the one that ended.
    pub fn bar_of_next_step(&self) -> usize {
        self.global_step.checked_div(self.steps_per_bar).unwrap_or(0)
    }

    /// Position the engine as if it had just played through bar `bar - 1`
    /// and were about to fire the downbeat of `bar`: the previous entry's
    /// scene is active, the bar line has not been crossed. A live swap starts
    /// the new engine here so that it crosses the line itself, on the same
    /// sample and with the same consequences (scene change, arrangement end,
    /// automation progress) as the engine it replaces would have.
    pub fn start_before_bar(&mut self, bar: usize) {
        if bar == 0 {
            self.start_from_bar(0);
            return;
        }
        self.start_from_bar(bar - 1);
        self.global_step = bar * self.steps_per_bar;
        if !self.arrangement.is_empty() {
            self.scene_step = (self.arrangement_bar_count as usize + 1) * self.steps_per_bar;
        }
        self.current_step_duration = self.effective_step_samples(self.global_step);
        self.sample_counter = self.current_step_duration;
    }

    /// Stop sequencing but keep rendering: what sounds decays, nothing new
    /// fires. The retiring half of a swap runs like this under a fade-out.
    pub fn coast(&mut self) {
        self.current_step_duration = f32::MAX;
        self.sample_counter = 0.0;
        self.pending_triggers.clear();
        self.active_automations.clear();
        self.scene_total_steps = 0;
        for t in self.tracks.iter_mut() {
            t.arp = None;
            t.gate_samples_remaining = 0.0;
        }
    }

    /// Take over state from `old`, the engine this one replaces at the start
    /// of the block in which the next bar line falls, `until` samples in.
    /// What the source text did not change keeps sounding: instruments keep
    /// their voices, sends and buses keep their tails, inherited tracks keep
    /// their place in the pattern and their held notes. Everything moves with
    /// `mem::swap`, so this allocates nothing. Call after `start_before_bar`.
    pub fn inherit_from(&mut self, old: &mut SongEngine, map: &crate::live::Inherit, until: usize) {
        // Old tracks nobody continues release their notes now, on the old
        // instruments, before an instrument moves over with a note that no
        // track in the new engine would ever release.
        for j in 0..old.tracks.len() {
            let continued = map.tracks.contains(&Some(j));
            if !continued {
                Self::release_track_notes(&mut old.tracks[j], &mut old.instruments);
                old.tracks[j].gate_samples_remaining = 0.0;
            }
        }
        for (i, m) in map.instruments.iter().enumerate() {
            if let Some(j) = *m {
                if i < self.instruments.len() && j < old.instruments.len() {
                    core::mem::swap(&mut self.instruments[i], &mut old.instruments[j]);
                    self.instruments[i].set_bpm(self.tempo);
                }
            }
        }
        for (i, m) in map.tracks.iter().enumerate() {
            let Some(j) = *m else { continue };
            if i >= self.tracks.len() || j >= old.tracks.len() { continue; }
            let n = &mut self.tracks[i];
            let o = &mut old.tracks[j];
            core::mem::swap(&mut n.current_step, &mut o.current_step);
            core::mem::swap(&mut n.current_notes, &mut o.current_notes);
            core::mem::swap(&mut n.current_notes_count, &mut o.current_notes_count);
            core::mem::swap(&mut n.gate_samples_remaining, &mut o.gate_samples_remaining);
            core::mem::swap(&mut n.arp, &mut o.arp);
            core::mem::swap(&mut n.insert_fx, &mut o.insert_fx);
            n.insert_fx.set_bpm(self.tempo);
            if let (Some(arp), Some(cfg)) = (n.arp.as_mut(), n.arp_cfg) {
                arp.set_bpm(self.tempo * cfg.rate_mult);
            }
        }
        // Nudged hits still in flight follow their instrument. The queue's
        // capacity is reserved, so this cannot allocate.
        for k in 0..old.pending_triggers.len() {
            let t = old.pending_triggers[k];
            if let Some(i) = map.instruments.iter().position(|m| *m == Some(t.inst_idx)) {
                if self.pending_triggers.len() < MAX_PENDING_TRIGGERS {
                    self.pending_triggers.push(PendingTrigger { inst_idx: i, ..t });
                }
            }
        }
        old.pending_triggers.clear();
        // The sidechain follower of a track is the envelope of whatever it
        // plays: it continues by name even where the track itself does not.
        for i in 0..self.tracks.len() {
            if let Some(j) = old.track_names.iter().position(|n| *n == self.track_names[i]) {
                self.tracks[i].sc_env = old.tracks[j].sc_env;
                // Same name, same voice: it keeps the point it had reached in
                // its own random stream, so a swap does not rewind its feel.
                core::mem::swap(&mut self.tracks[i].rng, &mut old.tracks[j].rng);
            }
        }
        for (i, m) in map.buses.iter().enumerate() {
            if let Some(j) = *m {
                if i < self.buses.len() && j < old.buses.len() {
                    core::mem::swap(&mut self.buses[i].fx_chain, &mut old.buses[j].fx_chain);
                    self.buses[i].fx_chain.set_bpm(self.tempo);
                }
            }
        }
        if map.sends {
            // Freeze belongs to the scene this engine is starting, not to the
            // reverb that carried the tail here.
            let frozen = self.send_reverb.is_frozen();
            core::mem::swap(&mut self.send_delay, &mut old.send_delay);
            core::mem::swap(&mut self.send_reverb, &mut old.send_reverb);
            core::mem::swap(&mut self.reverb_return, &mut old.reverb_return);
            core::mem::swap(&mut self.delay_return, &mut old.delay_return);
            self.send_reverb.set_freeze(frozen);
            self.send_delay.set_bpm(self.tempo, SAMPLE_RATE);
            self.reverb_return.set_bpm(self.tempo);
            self.delay_return.set_bpm(self.tempo);
        }
        if map.master {
            core::mem::swap(&mut self.master_fx, &mut old.master_fx);
            self.master_fx.set_bpm(self.tempo);
        }
        // The output stage always carries over: an edit does not re-measure
        // the song, so the level does not move while it is being played, and
        // the limiter's look-ahead holds the old engine's last 1.7 ms.
        core::mem::swap(&mut self.output, &mut old.output);
        // Smoothed and random state continues regardless of what changed: a
        // gain ramp restarting or the humanize sequence rewinding is audible.
        core::mem::swap(&mut self.timing_rng, &mut old.timing_rng);
        self.sc_envelope = old.sc_envelope;
        // The downbeat fires `until` samples into this block, on the old
        // clock; the new clock is set so it fires there too, carrying the
        // fractional residue (5512.5 samples per step at 120 BPM) with it.
        if old.running {
            let until = until as f32;
            let residue = (old.sample_counter + until + 1.0 - old.current_step_duration).clamp(0.0, 1.0);
            self.sample_counter = self.current_step_duration - until - 1.0 + residue;
        }
    }
}
