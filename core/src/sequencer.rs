use crate::rng::Rng;
use crate::{math, SAMPLE_RATE};

pub const MAX_PATTERNS: usize = 4;
pub const MAX_CHAIN_LENGTH: usize = 16;

// -- Motion Sequencing: ParamLock types --

pub type ParamLock = (u8, f32);
pub const MAX_LOCKS_PER_STEP: usize = 4;

// -- ParamId constants (ranges of 32 per module) --

// Bass 0..31
pub const PARAM_BASS_CUTOFF: u8 = 0;
pub const PARAM_BASS_CUTOFF_ENV: u8 = 1;
pub const PARAM_BASS_RESONANCE: u8 = 2;
pub const PARAM_BASS_GLIDE: u8 = 3;
pub const PARAM_BASS_ATTACK: u8 = 4;
pub const PARAM_BASS_LFO_RATE: u8 = 5;
pub const PARAM_BASS_LFO_DEPTH: u8 = 6;
pub const PARAM_BASS_LFO_WAVEFORM: u8 = 7;
pub const PARAM_BASS_LFO_TARGET: u8 = 8;
pub const PARAM_BASS_LFO_SYNC: u8 = 9;
pub const PARAM_BASS_OSC2_PITCH: u8 = 10;
pub const PARAM_BASS_OSC3_PITCH: u8 = 11;
pub const PARAM_BASS_OSC1_WAVE: u8 = 12;
pub const PARAM_BASS_OSC2_WAVE: u8 = 13;
pub const PARAM_BASS_OSC3_WAVE: u8 = 14;
pub const PARAM_BASS_KEYTRACK: u8 = 15;

// Keys 32..63
pub const PARAM_KEYS_CUTOFF: u8 = 32;
pub const PARAM_KEYS_DETUNE: u8 = 33;
pub const PARAM_KEYS_CHORUS_MIX: u8 = 34;
pub const PARAM_KEYS_LEVEL: u8 = 35;
pub const PARAM_KEYS_LFO_RATE: u8 = 36;
pub const PARAM_KEYS_LFO_DEPTH: u8 = 37;
pub const PARAM_KEYS_LFO_WAVEFORM: u8 = 38;
pub const PARAM_KEYS_LFO_TARGET: u8 = 39;
pub const PARAM_KEYS_LFO_SYNC: u8 = 40;
pub const PARAM_KEYS_VOICE_MODE: u8 = 41;

// FM 64..95
pub const PARAM_FM_ALGORITHM: u8 = 64;
pub const PARAM_FM_MOD_INDEX: u8 = 65;
pub const PARAM_FM_LFO_RATE: u8 = 66;
pub const PARAM_FM_LFO_DEPTH: u8 = 67;
pub const PARAM_FM_LFO_WAVEFORM: u8 = 68;
pub const PARAM_FM_LFO_TARGET: u8 = 69;
pub const PARAM_FM_LFO_SYNC: u8 = 70;
pub const PARAM_FM_FEEDBACK: u8 = 71;
pub const PARAM_FM_WAVEFORM: u8 = 72;
pub const PARAM_FM_CHORUS_MIX: u8 = 73;

// Beats 96..127
pub const PARAM_BEATS_LEVEL: u8 = 96;
pub const PARAM_BEATS_KICK_DECAY: u8 = 97;
pub const PARAM_BEATS_SNARE_DECAY: u8 = 98;
pub const PARAM_BEATS_KICK_PAN: u8 = 99;
pub const PARAM_BEATS_SNARE_PAN: u8 = 100;
pub const PARAM_BEATS_HIHAT_PAN: u8 = 101;
pub const PARAM_BEATS_CLAP_PAN: u8 = 102;
pub const PARAM_BEATS_KICK_CLICK: u8 = 103;
pub const PARAM_BEATS_KICK_LEVEL: u8 = 104;
pub const PARAM_BEATS_SNARE_LEVEL: u8 = 105;
pub const PARAM_BEATS_HIHAT_LEVEL: u8 = 106;
pub const PARAM_BEATS_CLAP_LEVEL: u8 = 107;
pub const PARAM_BEATS_KICK_PITCH: u8 = 108;
pub const PARAM_BEATS_SNARE_PITCH: u8 = 109;
pub const PARAM_BEATS_HIHAT_PITCH: u8 = 110;
pub const PARAM_BEATS_STUTTER_RATE: u8 = 111;
pub const PARAM_BEATS_TOM_PAN: u8 = 112;
pub const PARAM_BEATS_CRASH_PAN: u8 = 113;

// Arp 128..159
pub const PARAM_ARP_RATE: u8 = 128;
pub const PARAM_ARP_GATE: u8 = 129;
pub const PARAM_ARP_PATTERN: u8 = 130;
pub const PARAM_ARP_LEVEL: u8 = 131;

// Reverb 160..191
pub const PARAM_REVERB_ROOM_SIZE: u8 = 160;
pub const PARAM_REVERB_DAMPING: u8 = 161;
pub const PARAM_REVERB_MIX: u8 = 162;
pub const PARAM_REVERB_PRE_DELAY: u8 = 163;

// EQ 192..223
pub const PARAM_EQ_TILT: u8 = 192;
pub const PARAM_EQ_LOW: u8 = 193;
pub const PARAM_EQ_MID: u8 = 194;
pub const PARAM_EQ_HIGH: u8 = 195;

// Compressor/Sidechain 196..200 (within EQ/Master FX range)
pub const PARAM_COMP_THRESHOLD: u8 = 196;
pub const PARAM_COMP_RATIO: u8 = 197;
pub const PARAM_COMP_ATTACK: u8 = 198;
pub const PARAM_COMP_RELEASE: u8 = 199;
pub const PARAM_COMP_SIDECHAIN: u8 = 200;
pub const PARAM_PITCH_BEND: u8 = 201;

// Per-module vibrato
pub const PARAM_BASS_VIBRATO_RATE: u8 = 16;
pub const PARAM_BASS_VIBRATO_DEPTH: u8 = 17;
pub const PARAM_KEYS_VIBRATO_RATE: u8 = 42;
pub const PARAM_KEYS_VIBRATO_DEPTH: u8 = 43;
pub const PARAM_FM_VIBRATO_RATE: u8 = 74;
pub const PARAM_FM_VIBRATO_DEPTH: u8 = 75;

// Engine delay 224..255
pub const PARAM_DELAY_TIME: u8 = 224;
pub const PARAM_DELAY_LFO_RATE: u8 = 225;
pub const PARAM_DELAY_LFO_DEPTH: u8 = 226;
pub const PARAM_DELAY_FEEDBACK: u8 = 227;
pub const PARAM_DELAY_FILTER: u8 = 228;
pub const PARAM_DELAY_SYNC: u8 = 229;

#[derive(Clone, Copy)]
pub struct Step {
    pub note: Option<u8>,
    pub velocity: f32,
    pub gate: f32, // fraction of step length 0.0-1.0
    pub slide: bool,
    pub lock_slide: bool,
    pub active: bool,
    pub probability: f32,
    pub locks: [Option<ParamLock>; MAX_LOCKS_PER_STEP],
}

impl Step {
    pub fn empty() -> Self {
        Self {
            note: None,
            velocity: 0.0,
            gate: 0.0,
            slide: false,
            lock_slide: false,
            active: true,
            probability: 1.0,
            locks: [None; MAX_LOCKS_PER_STEP],
        }
    }

    pub fn new(note: u8, velocity: f32, gate: f32) -> Self {
        Self {
            note: Some(note),
            velocity,
            gate,
            slide: false,
            lock_slide: false,
            active: true,
            probability: 1.0,
            locks: [None; MAX_LOCKS_PER_STEP],
        }
    }

    pub fn with_slide(mut self) -> Self {
        self.slide = true;
        self
    }

    pub fn with_lock(mut self, param_id: u8, value: f32) -> Self {
        for slot in self.locks.iter_mut() {
            if slot.is_none() {
                *slot = Some((param_id, value));
                return self;
            }
        }
        self // all slots full, silently ignore
    }

    pub fn with_lock_slide(mut self) -> Self {
        self.lock_slide = true;
        self
    }

    pub fn with_probability(mut self, p: f32) -> Self {
        self.probability = p;
        self
    }

    pub fn inactive(mut self) -> Self {
        self.active = false;
        self
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum SequencerEvent {
    NoteOn {
        note: u8,
        velocity: f32,
        slide: bool,
        locks: [Option<ParamLock>; MAX_LOCKS_PER_STEP],
        lock_slide: bool,
        step_samples: f32,
    },
    NoteOff { note: u8 },
    Tick {
        locks: [Option<ParamLock>; MAX_LOCKS_PER_STEP],
        lock_slide: bool,
        step_samples: f32,
    },
}

pub struct Sequencer {
    pub steps: [Step; 16],
    pub num_steps: usize,
    current_step: usize,
    sample_counter: f32,
    samples_per_step: f32,
    gate_off_sent: bool,
    bpm: f32,
    running: bool,
    current_note: Option<u8>,

    // Pattern storage (patterns 1-3; pattern 0 is `steps`)
    patterns_extra: [[Step; 16]; 3],

    // Pattern chaining
    chain: [u8; MAX_CHAIN_LENGTH],
    chain_length: usize, // 0 = no chaining, use pattern 0
    chain_position: usize,

    // Current pattern index (0-3)
    current_pattern: usize,

    // Swing: 0.5 = straight, 0.67 = triplet feel
    swing: f32,

    // Velocity humanization: 0.0 = none, 1.0 = max
    humanize: f32,

    // RNG for probability + humanization
    rng: Rng,
}

impl Sequencer {
    pub fn new(bpm: f32) -> Self {
        // 16th notes: 4 steps per beat
        let samples_per_step = SAMPLE_RATE * 60.0 / bpm / 4.0;
        Self {
            steps: [Step::empty(); 16],
            num_steps: 16,
            current_step: 0,
            sample_counter: 0.0,
            samples_per_step,
            gate_off_sent: true,
            bpm,
            running: false,
            current_note: None,
            patterns_extra: [[Step::empty(); 16]; 3],
            chain: [0; MAX_CHAIN_LENGTH],
            chain_length: 0,
            chain_position: 0,
            current_pattern: 0,
            swing: 0.5,
            humanize: 0.0,
            rng: Rng::new(7919),
        }
    }

    pub fn set_bpm(&mut self, bpm: f32) {
        self.bpm = bpm;
        self.samples_per_step = SAMPLE_RATE * 60.0 / bpm / 4.0;
    }

    pub fn start(&mut self) {
        self.running = true;
        self.current_step = 0;
        self.gate_off_sent = true;
        self.current_note = None;
        self.chain_position = 0;
        if self.chain_length > 0 {
            self.current_pattern = (self.chain[0] as usize).min(MAX_PATTERNS - 1);
        } else {
            self.current_pattern = 0;
        }
        // Trigger immediately using swing-aware duration
        self.sample_counter = self.effective_step_samples(0);
    }

    pub fn stop(&mut self) {
        self.running = false;
    }

    // -- Pattern accessors --

    pub fn pattern(&self, index: usize) -> &[Step; 16] {
        if index == 0 {
            &self.steps
        } else {
            &self.patterns_extra[index.min(MAX_PATTERNS - 1) - 1]
        }
    }

    pub fn pattern_mut(&mut self, index: usize) -> &mut [Step; 16] {
        if index == 0 {
            &mut self.steps
        } else {
            &mut self.patterns_extra[index.min(MAX_PATTERNS - 1) - 1]
        }
    }

    fn active_step(&self) -> &Step {
        &self.pattern(self.current_pattern)[self.current_step]
    }

    // -- Swing timing --

    fn effective_step_samples(&self, step_index: usize) -> f32 {
        // Fast path: no swing
        if math::abs(self.swing - 0.5) < 0.001 {
            return self.samples_per_step;
        }
        let pair_duration = self.samples_per_step * 2.0;
        if step_index % 2 == 0 {
            pair_duration * self.swing
        } else {
            pair_duration * (1.0 - self.swing)
        }
    }

    // -- Step advancement with chaining --

    fn advance_step(&mut self) {
        self.current_step = (self.current_step + 1) % self.num_steps;
        // Chain advance on wrap
        if self.current_step == 0 && self.chain_length > 0 {
            self.chain_position = (self.chain_position + 1) % self.chain_length;
            self.current_pattern = (self.chain[self.chain_position] as usize).min(MAX_PATTERNS - 1);
        }
    }

    /// Advance by one sample. Returns an event if one occurs.
    pub fn tick(&mut self) -> Option<SequencerEvent> {
        if !self.running {
            return None;
        }

        self.sample_counter += 1.0;

        let eff_samples = self.effective_step_samples(self.current_step);

        // Check gate off
        if !self.gate_off_sent {
            let step = self.active_step();
            let gate_off_point = eff_samples * step.gate;
            if self.sample_counter >= gate_off_point {
                self.gate_off_sent = true;
                if let Some(note) = self.current_note {
                    self.current_note = None;
                    return Some(SequencerEvent::NoteOff { note });
                }
            }
        }

        // Check step advance
        if self.sample_counter >= eff_samples {
            self.sample_counter = 0.0;
            self.advance_step();

            let step = *self.active_step();
            let new_step_samples = self.effective_step_samples(self.current_step);

            // Inactive step: emit Tick with locks, no note
            if !step.active {
                self.gate_off_sent = true;
                return Some(SequencerEvent::Tick { locks: step.locks, lock_slide: step.lock_slide, step_samples: new_step_samples });
            }

            if let Some(note) = step.note {
                // Probability check
                if step.probability < 1.0 && self.rng.next_f32() >= step.probability {
                    // Skipped by probability — still emit Tick with locks
                    self.gate_off_sent = true;
                    return Some(SequencerEvent::Tick { locks: step.locks, lock_slide: step.lock_slide, step_samples: new_step_samples });
                }

                // Apply velocity humanization
                let vel = if self.humanize > 0.0 {
                    let offset = self.rng.next_bipolar() * self.humanize * 0.3;
                    math::clamp(step.velocity * (1.0 + offset), 0.0, 1.0)
                } else {
                    step.velocity
                };

                self.gate_off_sent = false;
                self.current_note = Some(note);
                return Some(SequencerEvent::NoteOn {
                    note,
                    velocity: vel,
                    slide: step.slide,
                    locks: step.locks,
                    lock_slide: step.lock_slide,
                    step_samples: new_step_samples,
                });
            } else {
                self.gate_off_sent = true;
                return Some(SequencerEvent::Tick { locks: step.locks, lock_slide: step.lock_slide, step_samples: new_step_samples });
            }
        }

        None
    }

    pub fn bpm(&self) -> f32 {
        self.bpm
    }

    pub fn current_step(&self) -> usize {
        self.current_step
    }

    pub fn reset(&mut self) {
        self.current_step = 0;
        self.sample_counter = 0.0;
        self.gate_off_sent = true;
        self.current_note = None;
        self.chain_position = 0;
        if self.chain_length > 0 {
            self.current_pattern = (self.chain[0] as usize).min(MAX_PATTERNS - 1);
        } else {
            self.current_pattern = 0;
        }
    }

    // -- Public API: Swing --

    pub fn set_swing(&mut self, swing: f32) {
        self.swing = math::clamp(swing, 0.5, 0.75);
    }

    pub fn swing(&self) -> f32 {
        self.swing
    }

    // -- Public API: Humanize --

    pub fn set_humanize(&mut self, amount: f32) {
        self.humanize = math::clamp(amount, 0.0, 1.0);
    }

    pub fn humanize(&self) -> f32 {
        self.humanize
    }

    // -- Public API: Pattern chaining --

    pub fn set_chain(&mut self, chain: &[u8], length: usize) {
        let len = length.min(MAX_CHAIN_LENGTH).min(chain.len());
        for i in 0..len {
            self.chain[i] = chain[i];
        }
        self.chain_length = len;
    }

    pub fn clear_chain(&mut self) {
        self.chain_length = 0;
        self.chain_position = 0;
        self.current_pattern = 0;
    }

    pub fn current_pattern(&self) -> usize {
        self.current_pattern
    }

    pub fn chain_position(&self) -> usize {
        self.chain_position
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;

    /// Helper: run sequencer until it emits the next event (or give up after max_samples)
    fn next_event(seq: &mut Sequencer, max_samples: usize) -> Option<SequencerEvent> {
        for _ in 0..max_samples {
            if let Some(ev) = seq.tick() {
                return Some(ev);
            }
        }
        None
    }

    /// Helper: count samples between two consecutive step-advance events
    fn measure_step_duration(seq: &mut Sequencer) -> usize {
        // Skip until we get the first event (step boundary)
        let limit = (SAMPLE_RATE * 2.0) as usize;
        for _ in 0..limit {
            if seq.tick().is_some() {
                break;
            }
        }
        // Now count samples until the next event
        let mut count = 0;
        for _ in 0..limit {
            count += 1;
            if seq.tick().is_some() {
                return count;
            }
        }
        0
    }

    #[test]
    fn test_swing_timing() {
        let mut seq = Sequencer::new(120.0);
        seq.set_swing(0.67);
        // Fill all steps with notes so every step produces an event
        for i in 0..4 {
            seq.steps[i] = Step::new(60, 0.8, 0.1); // short gate to avoid NoteOff interference
        }
        seq.num_steps = 4;
        seq.start();

        // start() triggers immediate advance to step 1 (odd).
        // First measured duration: step 1 (odd) → step 2 (even) = pair*(1-swing)
        // Second measured duration: step 2 (even) → step 3 (odd) = pair*swing
        let d_odd = measure_step_duration(&mut seq);  // step 1 duration (odd = short)
        let d_even = measure_step_duration(&mut seq); // step 2 duration (even = long)

        // With swing=0.67, even steps get 67% of pair, odd get 33%
        // Ratio should be approximately 0.67/0.33 ≈ 2.03
        let ratio = d_even as f32 / d_odd as f32;
        assert!(
            ratio > 1.8 && ratio < 2.3,
            "Swing ratio should be ~2.0, got {} (even={}, odd={})",
            ratio, d_even, d_odd
        );
    }

    #[test]
    fn test_active_step_mute() {
        let mut seq = Sequencer::new(120.0);
        // start() immediately advances to step 1, so:
        // Step 1: active note, Step 0: inactive note
        seq.steps[1] = Step::new(60, 0.8, 0.5);
        seq.steps[0] = Step::new(64, 0.8, 0.5).inactive();
        seq.num_steps = 2;
        seq.start();

        let limit = (SAMPLE_RATE * 2.0) as usize;

        // First event should be NoteOn for step 1 (active)
        let ev1 = next_event(&mut seq, limit).unwrap();
        assert!(matches!(ev1, SequencerEvent::NoteOn { note: 60, .. }));

        // Skip past NoteOff, then step 0 (inactive) should emit Tick
        loop {
            if let Some(ev) = next_event(&mut seq, limit) {
                match ev {
                    SequencerEvent::NoteOff { .. } => continue,
                    SequencerEvent::Tick { .. } => {
                        // Step 0 was inactive — should emit Tick, not NoteOn
                        break;
                    }
                    SequencerEvent::NoteOn { .. } => {
                        panic!("Inactive step should not emit NoteOn");
                    }
                }
            } else {
                panic!("Expected Tick event for inactive step");
            }
        }
    }

    #[test]
    fn test_probability_zero_never_plays() {
        let mut seq = Sequencer::new(120.0);
        // All steps have note but probability=0.0
        for i in 0..4 {
            seq.steps[i] = Step::new(60, 0.8, 0.5).with_probability(0.0);
        }
        seq.num_steps = 4;
        seq.start();

        let limit = (SAMPLE_RATE * 4.0) as usize;
        let mut note_on_count = 0;
        for _ in 0..limit {
            if let Some(ev) = seq.tick() {
                if matches!(ev, SequencerEvent::NoteOn { .. }) {
                    note_on_count += 1;
                }
            }
        }
        assert_eq!(note_on_count, 0, "Probability 0.0 should never produce NoteOn");
    }

    #[test]
    fn test_probability_emits_tick_with_locks() {
        let mut seq = Sequencer::new(120.0);
        seq.steps[0] = Step::new(60, 0.8, 0.5) // short gate
            .with_probability(0.0) // never plays
            .with_lock(PARAM_BASS_CUTOFF, 0.9);
        seq.num_steps = 1;
        seq.start();

        let limit = (SAMPLE_RATE * 2.0) as usize;
        let ev = next_event(&mut seq, limit).unwrap();

        match ev {
            SequencerEvent::Tick { locks, .. } => {
                assert!(locks[0].is_some(), "Tick should carry param locks");
                let (id, val) = locks[0].unwrap();
                assert_eq!(id, PARAM_BASS_CUTOFF);
                assert!((val - 0.9).abs() < 0.001);
            }
            other => panic!("Expected Tick with locks, got NoteOn/NoteOff: {:?}", matches!(other, SequencerEvent::NoteOn { .. })),
        }
    }

    #[test]
    fn test_pattern_chaining() {
        let mut seq = Sequencer::new(120.0);
        // Pattern 0: note 60 on step 0+1, Pattern 1: note 72 on step 0+1
        // Use num_steps=2 so chain advances every 2 steps
        seq.steps[0] = Step::new(60, 0.8, 0.1);
        seq.steps[1] = Step::new(60, 0.8, 0.1);
        seq.pattern_mut(1)[0] = Step::new(72, 0.8, 0.1);
        seq.pattern_mut(1)[1] = Step::new(72, 0.8, 0.1);
        seq.num_steps = 2;
        seq.set_chain(&[0, 1, 0, 1], 4);
        seq.start();

        let limit = (SAMPLE_RATE * 2.0) as usize;
        let mut notes = Vec::new();

        for _ in 0..8 {
            loop {
                if let Some(ev) = next_event(&mut seq, limit) {
                    match ev {
                        SequencerEvent::NoteOn { note, .. } => {
                            notes.push(note);
                            break;
                        }
                        _ => continue,
                    }
                } else {
                    break;
                }
            }
        }

        // start() fires step 1 of pattern 0 (note 60), then wraps to step 0 advancing chain
        // Sequence: pat0-step1(60), pat1-step0(72), pat1-step1(72), pat0-step0(60), ...
        assert!(notes.len() >= 4, "Should have collected at least 4 notes, got {}", notes.len());
        assert_eq!(notes[0], 60, "First note from pattern 0");
        assert_eq!(notes[1], 72, "Second note from pattern 1");
        assert_eq!(notes[2], 72, "Third note still pattern 1");
        assert_eq!(notes[3], 60, "Fourth note back to pattern 0");
    }

    #[test]
    fn test_default_values_unchanged() {
        // Verify that default settings produce identical behavior to pre-change code
        let mut seq = Sequencer::new(120.0);
        assert!((seq.swing() - 0.5).abs() < 0.001, "Default swing should be 0.5");
        assert!((seq.humanize() - 0.0).abs() < 0.001, "Default humanize should be 0.0");

        // Fill a pattern: start() advances to step 1 first
        seq.steps[1] = Step::new(60, 0.8, 0.5);
        seq.steps[0] = Step::new(64, 0.9, 0.5);
        seq.num_steps = 2;
        seq.start();

        let limit = (SAMPLE_RATE * 2.0) as usize;

        // Default: active=true, probability=1.0 → all notes play
        // First event is step 1 (note 60)
        let ev1 = next_event(&mut seq, limit).unwrap();
        match ev1 {
            SequencerEvent::NoteOn { note, velocity, .. } => {
                assert_eq!(note, 60);
                assert!((velocity - 0.8).abs() < 0.001, "Velocity should be exact with humanize=0");
            }
            _ => panic!("Expected NoteOn"),
        }

        // Verify default Step field values
        let step = Step::empty();
        assert!(step.active, "Default step should be active");
        assert!((step.probability - 1.0).abs() < 0.001, "Default probability should be 1.0");
    }
}
