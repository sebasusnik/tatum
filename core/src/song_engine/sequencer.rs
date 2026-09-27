//! The step sequencer: what fires on each step, for patterns, drum lanes and
//! arpeggiated tracks, and how long a step lasts once swing is in it.

use alloc::vec::Vec;

use crate::dsl::compiler::{self, CompiledStep};
use crate::primitives::arp_processor::ArpEvent;
use crate::rng::Rng;
use crate::math;

use super::instrument::SongInstrument;
use super::track::TrackPlayback;
use super::SongEngine;

/// What the current step is played with. Copied out of the engine so the track,
/// instrument and rng borrows can all stay live across the call.
#[derive(Clone, Copy)]
struct StepTiming {
    humanize_velocity: f32,
    samples_per_step: f32,
    step_duration: f32,
}

/// The arpeggiator holds at most this many notes (chord notes x octaves).
const MAX_ARP_NOTES: usize = 16;

/// How many delayed drum hits can be in flight at once. A nudge resolves within a
/// fraction of a step, so this is far above anything a pattern can produce.
pub(super) const MAX_PENDING_TRIGGERS: usize = 128;

/// A note the sequencer owes the future: a nudged drum hit, or one note of a
/// subdivided step. `release` is the note this one replaces (255 = none), so a
/// run inside one step does not pile eight voices up on a poly instrument.
#[derive(Clone, Copy, Debug)]
pub(super) struct PendingTrigger {
    pub(super) samples: f32,
    pub(super) inst_idx: usize,
    pub(super) midi_note: u8,
    pub(super) velocity: f32,
    pub(super) release: u8,
    pub(super) slide: bool,
}
pub(super) const NO_RELEASE: u8 = 255;

impl SongEngine {
    // ── Swing & groove timing ──

    /// Compute effective step duration in samples, applying swing.
    pub(super) fn effective_step_samples(&self, step_index: usize) -> f32 {
        if math::abs(self.swing - 0.5) < 0.001 {
            return self.samples_per_step;
        }
        // Swing: alternate long/short pairs (like classic drum machines)
        let pair_duration = self.samples_per_step * 2.0;
        if step_index.is_multiple_of(2) {
            pair_duration * self.swing
        } else {
            pair_duration * (1.0 - self.swing)
        }
    }

    /// Release all active notes on a track.
    #[inline]
    pub(super) fn release_track_notes(track: &mut TrackPlayback, instruments: &mut [SongInstrument]) {
        if let Some(arp) = track.arp.as_mut() {
            if let Some(ArpEvent::NoteOff(n)) = arp.stop() {
                if track.instrument_idx < instruments.len() {
                    instruments[track.instrument_idx].note_off(n);
                }
            }
            track.current_notes_count = 0;
            return;
        }
        let count = track.current_notes_count as usize;
        if count > 0 {
            let inst_idx = track.instrument_idx;
            if inst_idx < instruments.len() {
                for ni in 0..count {
                    instruments[inst_idx].note_off(track.current_notes[ni]);
                }
            }
            track.current_notes_count = 0;
        }
    }

    /// Apply velocity humanization (random jitter).
    #[inline]
    fn humanize_vel(velocity: f32, amount: f32, rng: &mut Rng) -> f32 {
        if amount <= 0.0 {
            return velocity;
        }
        let jitter = rng.next_bipolar() * amount * 0.15;
        (velocity * (1.0 + jitter)).clamp(0.01, 1.0)
    }

    /// Schedule a nudged hit. If the queue is full the hit fires now rather than
    /// growing the queue: the audio thread must not allocate, and losing a few
    /// milliseconds of swing is better than losing the hit.
    fn push_trigger(
        pending: &mut Vec<PendingTrigger>,
        instruments: &mut [SongInstrument],
        samples: f32,
        inst_idx: usize,
        midi_note: u8,
        vel: f32,
    ) {
        Self::push_pending(
            pending,
            instruments,
            PendingTrigger { samples, inst_idx, midi_note, velocity: vel, release: NO_RELEASE, slide: false },
        );
    }

    /// Queue a trigger. If the queue is full it fires now rather than growing:
    /// the audio thread must not allocate, and losing a few milliseconds of
    /// swing is better than losing the note.
    fn push_pending(pending: &mut Vec<PendingTrigger>, instruments: &mut [SongInstrument], t: PendingTrigger) {
        if pending.len() < MAX_PENDING_TRIGGERS {
            pending.push(t);
        } else if t.inst_idx < instruments.len() {
            if t.release != NO_RELEASE {
                instruments[t.inst_idx].note_off(t.release);
            }
            instruments[t.inst_idx].note_on(t.midi_note, t.velocity);
        }
    }

    /// The instrument a track triggers into, or `usize::MAX` when the track is
    /// muted. Every trigger site is already guarded by `inst_idx < len`, so a
    /// muted track fires nothing while its pattern keeps advancing: bringing it
    /// back in drops it onto the grid rather than where it left off. Firing
    /// nothing is what lets its voices run out, and an idle muted track is
    /// skipped whole by the render loop.
    pub(super) fn trigger_instrument(&self, ti: usize) -> usize {
        if self.tracks[ti].level <= 0.0 && !self.tracks[ti].is_sc_source {
            return usize::MAX;
        }
        self.tracks[ti].instrument_idx
    }

    pub(super) fn advance_step(&mut self) {
        // Cross the bar line before triggering, when the step about to fire is
        // the first of a bar. It used to be counted after the last step of the
        // previous bar fired, so `current_bar` read one sixteenth early: scene
        // changes killed the note the last step had just started, every
        // arranged render stopped a sixteenth short, and a hot-swap keyed on
        // the bar counter landed a sixteenth before the downbeat.
        if let Some(bar_of_step) = self.global_step.checked_div(self.steps_per_bar) {
            if bar_of_step > self.current_bar {
                self.current_bar = bar_of_step;
                self.check_arrangement_advance();
                if !self.running {
                    return;
                }
            }
        }
        self.scene_step += 1;

        let track_count = self.tracks.len();

        for ti in 0..track_count {
            if !self.tracks[ti].active {
                continue;
            }

            let pat_idx = self.tracks[ti].pattern_idx;
            if pat_idx >= self.patterns.len() {
                continue;
            }

            let pattern = &self.patterns[pat_idx];

            // Multi-lane drum pattern
            if !pattern.lanes.is_empty() {
                let lane_len = pattern.lanes[0].steps.len();
                if lane_len == 0 {
                    continue;
                }
                let step_idx = self.tracks[ti].current_step % lane_len;
                let inst_idx = self.trigger_instrument(ti);
                if inst_idx < self.instruments.len() {
                    for lane in &pattern.lanes {
                        if step_idx < lane.steps.len() {
                            if let CompiledStep::DrumHit { velocity, probability, roll, .. } = lane.steps[step_idx] {
                                // Probability gate: skip hit if random exceeds probability
                                if probability < 1.0 {
                                    let chance = self.tracks[ti].rng.next_f32();
                                    if chance > probability {
                                        continue; // skip this hit
                                    }
                                }
                                let raw_vel = velocity * self.tracks[ti].velocity;
                                let vel = Self::humanize_vel(raw_vel, self.humanize_velocity, &mut self.tracks[ti].rng);

                                // Per-lane nudge: delay trigger by nudge * step_samples
                                let nudge_samples = lane.nudge * self.samples_per_step;
                                if nudge_samples.abs() > 0.5 && nudge_samples > 0.0 {
                                    // Positive nudge = delay trigger
                                    Self::push_trigger(
                                        &mut self.pending_triggers,
                                        &mut self.instruments,
                                        nudge_samples,
                                        inst_idx,
                                        lane.midi_note,
                                        vel,
                                    );
                                } else {
                                    // No nudge or negative nudge (trigger immediately, can't go back in time)
                                    self.instruments[inst_idx].note_on(lane.midi_note, vel);
                                }

                                // Roll: schedule additional retriggers within this step
                                if roll > 1 {
                                    let step_samples = self.effective_step_samples(self.global_step) as u32;
                                    let interval = step_samples / (roll as u32);
                                    if let SongInstrument::Beats(ref mut beats) = self.instruments[inst_idx] {
                                        beats.stutter_drum = Some(lane.midi_note);
                                        beats.stutter_velocity = vel * 0.9;
                                        beats.stutter_interval = interval;
                                        beats.stutter_counter = 0;
                                        beats.stutter_remaining = (roll - 1) as u32;
                                    }
                                }
                            }
                        }
                    }
                }
                // No gate management — drums self-decay
                self.tracks[ti].current_step += 1;
                continue;
            }

            // Sequential pattern (existing behavior)
            if pattern.steps.is_empty() {
                continue;
            }

            let step_idx = self.tracks[ti].current_step % pattern.steps.len();
            let step = pattern.steps[step_idx];

            let gate = self.tracks[ti].gate;

            // Arp tracks: the step only updates the held notes; the arp plays them.
            if self.tracks[ti].arp.is_some() && !matches!(step, CompiledStep::Tie) {
                let next_step_idx = (step_idx + 1) % pattern.steps.len();
                let next_is_tie = matches!(pattern.steps[next_step_idx], CompiledStep::Tie);
                let timing = StepTiming {
                    humanize_velocity: self.humanize_velocity,
                    samples_per_step: self.samples_per_step,
                    step_duration: self.current_step_duration,
                };
                Self::advance_arp_track(&mut self.tracks[ti], &mut self.instruments, timing, step, next_is_tie);
                self.tracks[ti].current_step += 1;
                continue;
            }

            match step {
                CompiledStep::Tie => {
                    // Keep previous note alive — extend gate for another step.
                    // Use generous duration to survive swing/humanization timing variance.
                    let next_step_idx = (step_idx + 1) % pattern.steps.len();
                    let next_continues = match pattern.steps[next_step_idx] {
                        CompiledStep::Tie => true,
                        CompiledStep::NoteOn { midi_note, .. } => {
                            self.tracks[ti].current_notes_count == 1 && self.tracks[ti].current_notes[0] == midi_note
                        }
                        CompiledStep::Chord { notes, count, .. } => {
                            let c = count as usize;
                            let pc = self.tracks[ti].current_notes_count as usize;
                            c == pc && (0..c).all(|i| self.tracks[ti].current_notes[i] == notes[i].midi_note)
                        }
                        _ => false,
                    };
                    if next_continues {
                        // Use 2x step duration to guarantee survival across swing variance.
                        // The next tie/note will reset this anyway.
                        self.tracks[ti].gate_samples_remaining = self.samples_per_step * 2.0;
                    } else {
                        // Last tie in chain — apply track gate for natural release
                        self.tracks[ti].gate_samples_remaining = self.current_step_duration * gate;
                    }
                }
                _ => {
                    // Peek at next step: if it's a Tie, force gate=1.0 so note
                    // survives until the Tie can extend it.
                    let next_step_idx = (step_idx + 1) % pattern.steps.len();
                    let next_is_tie = matches!(pattern.steps[next_step_idx], CompiledStep::Tie);
                    // A `~note` next step needs this note still gated to glide from.
                    let next_slides = matches!(pattern.steps[next_step_idx], CompiledStep::NoteOn { slide: true, .. });

                    match step {
                        // A run inside one step. The first note fires now and the
                        // rest go on the pending queue, spaced across this step's
                        // REAL duration -- `effective_step_samples` already has the
                        // swing in it, so a subdivided step swings with everything
                        // else instead of quietly opting out.
                        CompiledStep::Subdiv { notes: subs, count, plock } => {
                            let n = (count as usize).clamp(1, compiler::MAX_SUBDIV);
                            let tv = self.tracks[ti].velocity;
                            let mut vels = [0.0f32; compiler::MAX_SUBDIV];
                            for k in 0..n {
                                vels[k] = Self::humanize_vel(
                                    subs[k].velocity * tv,
                                    self.humanize_velocity,
                                    &mut self.tracks[ti].rng,
                                );
                            }
                            let step_samples = self.effective_step_samples(self.global_step);
                            let interval = step_samples / n as f32;
                            Self::release_track_notes(&mut self.tracks[ti], &mut self.instruments);
                            let inst_idx = self.trigger_instrument(ti);
                            if inst_idx < self.instruments.len() {
                                self.instruments[inst_idx].stage_plock(plock.cutoff, plock.env_depth, plock.resonance);
                                self.instruments[inst_idx].note_on(subs[0].midi_note, vels[0]);
                                for k in 1..n {
                                    Self::push_pending(
                                        &mut self.pending_triggers,
                                        &mut self.instruments,
                                        PendingTrigger {
                                            samples: interval * k as f32,
                                            inst_idx,
                                            midi_note: subs[k].midi_note,
                                            velocity: vels[k],
                                            // each note takes the previous one's place,
                                            // so eight of them do not stack up on a poly
                                            release: if subs[k].slide { NO_RELEASE } else { subs[k - 1].midi_note },
                                            slide: subs[k].slide,
                                        },
                                    );
                                }
                            }
                            // The step ends holding its LAST note, so a tie or a
                            // slide after it continues from where the run landed.
                            self.tracks[ti].current_notes[0] = subs[n - 1].midi_note;
                            self.tracks[ti].current_notes_count = 1;
                            let step_gate = plock.gate.unwrap_or(gate);
                            self.tracks[ti].gate_samples_remaining = if next_is_tie {
                                self.samples_per_step * 2.0
                            } else if next_slides {
                                self.samples_per_step * 1.5
                            } else {
                                // hold until the last note has started, then gate
                                // that one: a short gate must not cut the run off
                                interval * (n - 1) as f32 + interval * step_gate
                            };
                        }
                        CompiledStep::NoteOn { midi_note, velocity, plock, slide } => {
                            // If the same single note is already playing (pattern loop),
                            // just extend gate — don't re-trigger (avoids click/re-attack).
                            let same_note = self.tracks[ti].current_notes_count == 1
                                && self.tracks[ti].current_notes[0] == midi_note;
                            let held = self.tracks[ti].current_notes_count > 0;
                            if same_note && next_is_tie {
                                // Sustain continuation — treat as tie
                                self.tracks[ti].gate_samples_remaining = self.samples_per_step * 2.0;
                            } else if slide && held {
                                // Slide: glide pitch without retriggering (303-style)
                                let inst_idx = self.trigger_instrument(ti);
                                let raw_vel = velocity * self.tracks[ti].velocity;
                                let vel = Self::humanize_vel(raw_vel, self.humanize_velocity, &mut self.tracks[ti].rng);
                                let handled = inst_idx < self.instruments.len() && {
                                    self.instruments[inst_idx].stage_plock(
                                        plock.cutoff,
                                        plock.env_depth,
                                        plock.resonance,
                                    );
                                    self.instruments[inst_idx].slide_to(midi_note, vel)
                                };
                                if !handled {
                                    Self::release_track_notes(&mut self.tracks[ti], &mut self.instruments);
                                    if inst_idx < self.instruments.len() {
                                        self.instruments[inst_idx].note_on(midi_note, vel);
                                    }
                                }
                                self.tracks[ti].current_notes[0] = midi_note;
                                self.tracks[ti].current_notes_count = 1;
                                if next_is_tie {
                                    self.tracks[ti].gate_samples_remaining = self.samples_per_step * 2.0;
                                } else if next_slides {
                                    self.tracks[ti].gate_samples_remaining = self.samples_per_step * 1.5;
                                } else {
                                    let step_gate = plock.gate.unwrap_or(gate);
                                    self.tracks[ti].gate_samples_remaining = self.current_step_duration * step_gate;
                                }
                            } else {
                                // Release previous notes
                                Self::release_track_notes(&mut self.tracks[ti], &mut self.instruments);
                                let inst_idx = self.trigger_instrument(ti);
                                let raw_vel = velocity * self.tracks[ti].velocity;
                                let vel = Self::humanize_vel(raw_vel, self.humanize_velocity, &mut self.tracks[ti].rng);
                                if inst_idx < self.instruments.len() {
                                    self.instruments[inst_idx].stage_plock(
                                        plock.cutoff,
                                        plock.env_depth,
                                        plock.resonance,
                                    );
                                    self.instruments[inst_idx].note_on(midi_note, vel);
                                }
                                self.tracks[ti].current_notes[0] = midi_note;
                                self.tracks[ti].current_notes_count = 1;
                                if next_is_tie {
                                    self.tracks[ti].gate_samples_remaining = self.samples_per_step * 2.0;
                                } else if next_slides {
                                    self.tracks[ti].gate_samples_remaining = self.samples_per_step * 1.5;
                                } else {
                                    let step_gate = plock.gate.unwrap_or(gate);
                                    self.tracks[ti].gate_samples_remaining = self.current_step_duration * step_gate;
                                }
                            }
                        }
                        CompiledStep::Chord { notes, count, plock } => {
                            // Check if the exact same chord is already playing (pattern loop).
                            let c = count as usize;
                            let prev_c = self.tracks[ti].current_notes_count as usize;
                            let same_chord =
                                c == prev_c && (0..c).all(|i| self.tracks[ti].current_notes[i] == notes[i].midi_note);
                            if same_chord && next_is_tie {
                                // Sustain continuation — treat as tie
                                self.tracks[ti].gate_samples_remaining = self.samples_per_step * 2.0;
                            } else {
                                // Release previous notes
                                Self::release_track_notes(&mut self.tracks[ti], &mut self.instruments);
                                let inst_idx = self.trigger_instrument(ti);
                                if inst_idx < self.instruments.len() {
                                    self.instruments[inst_idx].stage_plock(
                                        plock.cutoff,
                                        plock.env_depth,
                                        plock.resonance,
                                    );
                                    for (ni, n) in notes[..c].iter().enumerate() {
                                        let raw_vel = n.velocity * self.tracks[ti].velocity;
                                        let vel = Self::humanize_vel(
                                            raw_vel,
                                            self.humanize_velocity,
                                            &mut self.tracks[ti].rng,
                                        );
                                        self.instruments[inst_idx].note_on(n.midi_note, vel);
                                        self.tracks[ti].current_notes[ni] = n.midi_note;
                                    }
                                    self.tracks[ti].current_notes_count = count;
                                }
                                if next_is_tie {
                                    self.tracks[ti].gate_samples_remaining = self.samples_per_step * 2.0;
                                } else if next_slides {
                                    self.tracks[ti].gate_samples_remaining = self.samples_per_step * 1.5;
                                } else {
                                    let step_gate = plock.gate.unwrap_or(gate);
                                    self.tracks[ti].gate_samples_remaining = self.current_step_duration * step_gate;
                                }
                            }
                        }
                        CompiledStep::DrumHit { velocity, plock, .. } => {
                            Self::release_track_notes(&mut self.tracks[ti], &mut self.instruments);
                            let inst_idx = self.trigger_instrument(ti);
                            let raw_vel = velocity * self.tracks[ti].velocity;
                            let vel = Self::humanize_vel(raw_vel, self.humanize_velocity, &mut self.tracks[ti].rng);
                            if inst_idx < self.instruments.len() {
                                self.instruments[inst_idx].stage_plock(plock.cutoff, plock.env_depth, plock.resonance);
                                self.instruments[inst_idx].note_on(36, vel);
                            }
                            self.tracks[ti].current_notes[0] = 36;
                            self.tracks[ti].current_notes_count = 1;
                            if next_is_tie {
                                self.tracks[ti].gate_samples_remaining = self.samples_per_step * 2.0;
                            } else {
                                let step_gate = plock.gate.unwrap_or(0.5);
                                self.tracks[ti].gate_samples_remaining = self.current_step_duration * step_gate;
                            }
                        }
                        CompiledStep::Rest => {
                            Self::release_track_notes(&mut self.tracks[ti], &mut self.instruments);
                        }
                        CompiledStep::Tie => unreachable!(),
                    }
                }
            }

            self.tracks[ti].current_step += 1;
        }

        self.global_step += 1;
    }

    /// Step handler for arp tracks: update the held notes, (re)start the arp,
    /// and set the track gate that will eventually stop it.
    fn advance_arp_track(
        track: &mut TrackPlayback,
        instruments: &mut [SongInstrument],
        timing: StepTiming,
        step: CompiledStep,
        next_is_tie: bool,
    ) {
        let inst_idx = track.instrument_idx;
        let mut notes = [0u8; compiler::MAX_CHORD_NOTES];
        let (count, step_vel, step_gate): (usize, f32, Option<f32>) = match step {
            CompiledStep::NoteOn { midi_note, velocity, plock, .. } => {
                notes[0] = midi_note;
                if inst_idx < instruments.len() {
                    instruments[inst_idx].stage_plock(plock.cutoff, plock.env_depth, plock.resonance);
                }
                (1, velocity, plock.gate)
            }
            // On an arp track a group is just the note set to cycle: the arp
            // owns the timing, so its own rate wins over the subdivision.
            CompiledStep::Subdiv { notes: sn, count: c, plock } => {
                let count = (c as usize).min(compiler::MAX_CHORD_NOTES);
                for i in 0..count {
                    notes[i] = sn[i].midi_note;
                }
                if inst_idx < instruments.len() {
                    instruments[inst_idx].stage_plock(plock.cutoff, plock.env_depth, plock.resonance);
                }
                (count, sn[0].velocity, plock.gate)
            }
            CompiledStep::Chord { notes: cn, count: c, plock } => {
                let count = (c as usize).min(compiler::MAX_CHORD_NOTES);
                for i in 0..count {
                    notes[i] = cn[i].midi_note;
                }
                if inst_idx < instruments.len() {
                    instruments[inst_idx].stage_plock(plock.cutoff, plock.env_depth, plock.resonance);
                }
                (count, cn[0].velocity, plock.gate)
            }
            CompiledStep::DrumHit { velocity, plock, .. } => {
                notes[0] = 36;
                (1, velocity, plock.gate)
            }
            CompiledStep::Rest => {
                Self::release_track_notes(track, instruments);
                return;
            }
            CompiledStep::Tie => return,
        };

        let same =
            count == track.current_notes_count as usize && (0..count).all(|i| track.current_notes[i] == notes[i]);
        track.current_notes[..count].copy_from_slice(&notes[..count]);
        track.current_notes_count = count as u8;

        let raw_vel = step_vel * track.velocity;
        let vel = Self::humanize_vel(raw_vel, timing.humanize_velocity, &mut track.rng);

        // Sorted ascending and expanded across octaves so "up" really goes up.
        // Fixed buffers: this runs on the audio thread, once per step.
        let mut sorted = [0u8; compiler::MAX_CHORD_NOTES];
        sorted[..count].copy_from_slice(&notes[..count]);
        sorted[..count].sort_unstable();
        let octaves = track.arp_cfg.map_or(1, |c| c.octaves).max(1);
        let mut list = [0u8; MAX_ARP_NOTES];
        let mut list_len = 0usize;
        for o in 0..octaves {
            for &n in &sorted[..count] {
                let v = n as u16 + 12 * o as u16;
                if v <= 127 && list_len < MAX_ARP_NOTES {
                    list[list_len] = v as u8;
                    list_len += 1;
                }
            }
        }
        let list = &list[..list_len];

        let mut pending_off = None;
        if let Some(arp) = track.arp.as_mut() {
            arp.set_notes(list);
            arp.set_velocity(vel);
            if !same || !arp.is_active() {
                // New material: close the open arp note and restart from the first note.
                if let Some(ArpEvent::NoteOff(n)) = arp.stop() {
                    pending_off = Some(n);
                }
                arp.start(vel);
            }
        }
        if let Some(n) = pending_off {
            if inst_idx < instruments.len() {
                instruments[inst_idx].note_off(n);
            }
        }

        track.gate_samples_remaining = if next_is_tie {
            timing.samples_per_step * 2.0
        } else {
            timing.step_duration * step_gate.unwrap_or(track.gate)
        };
    }

    /// True while a track's arpeggiator is running (for UI activity LEDs).
    pub fn track_arp_active(&self, idx: usize) -> bool {
        self.tracks.get(idx).and_then(|t| t.arp.as_ref()).is_some_and(|a| a.is_active())
    }
}
