use crate::math;
use crate::primitives::envelope::EnvStage;
use crate::primitives::fm_operator::{FmOperator, FmWaveform};
use crate::primitives::lfo::{LfoWaveform, LfoSyncMode, LfoTarget, ModulationRouter};
use crate::effects::chorus::Chorus;
use crate::{Module, MAX_VOICES, SAMPLE_RATE};

/// FM Voice: 4 operators with configurable algorithm.
struct FmVoice {
    ops: [FmOperator; 4],
    algorithm: u8,
    note: u8,
    active: bool,
    mod_index: f32,
    age: u32,
    velocity_mod: f32,
}

impl FmVoice {
    fn new() -> Self {
        Self {
            ops: core::array::from_fn(|_| FmOperator::new(SAMPLE_RATE)),
            algorithm: 0,
            note: 0,
            active: false,
            mod_index: 1.0,
            age: 0,
            velocity_mod: 1.0,
        }
    }

    fn note_on(&mut self, note: u8, velocity: f32) {
        let freq = math::midi_to_freq(note);
        self.note = note;
        self.active = true;
        self.velocity_mod = 0.3 + velocity * 0.7; // range 0.3-1.0
        for op in &mut self.ops {
            op.note_on(freq);
            op.set_amplitude(velocity);
        }
    }

    fn note_off(&mut self) {
        for op in &mut self.ops {
            op.note_off();
        }
    }

    fn next_sample(&mut self) -> f32 {
        if !self.active {
            return 0.0;
        }

        // Check if all operators are idle
        let all_idle = self.ops.iter().all(|op| op.is_idle());
        if all_idle {
            self.active = false;
            return 0.0;
        }

        let mi = self.mod_index * self.velocity_mod;

        match self.algorithm {
            // Algorithm 0: Op3 -> Op2 -> Op1 -> Out (+ Op4 -> Out)
            // Classic FM: serial modulation chain
            0 => {
                let op3 = self.ops[2].next_sample(0.0);
                let op2 = self.ops[1].next_sample(op3 * mi * math::TWO_PI);
                let op1 = self.ops[0].next_sample(op2 * mi * math::TWO_PI);
                let op4 = self.ops[3].next_sample(0.0);
                (op1 + op4) * 0.5
            }
            // Algorithm 1: (Op2 -> Op1) -> Out, (Op4 -> Op3) -> Out
            // Two parallel 2-op stacks
            1 => {
                let op2 = self.ops[1].next_sample(0.0);
                let op1 = self.ops[0].next_sample(op2 * mi * math::TWO_PI);
                let op4 = self.ops[3].next_sample(0.0);
                let op3 = self.ops[2].next_sample(op4 * mi * math::TWO_PI);
                (op1 + op3) * 0.5
            }
            // Algorithm 2: Op2 -> Op1 -> Out (single carrier, single modulator)
            2 => {
                let op2 = self.ops[1].next_sample(0.0);
                let op1 = self.ops[0].next_sample(op2 * mi * math::TWO_PI);
                // Advance other ops to keep envelopes in sync
                let _ = self.ops[2].next_sample(0.0);
                let _ = self.ops[3].next_sample(0.0);
                op1
            }
            // Algorithm 3: All carriers (additive)
            3 => {
                let mut sum = 0.0;
                for op in &mut self.ops {
                    sum += op.next_sample(0.0);
                }
                sum * 0.25
            }
            // Algorithm 4: Op4 -> Op3 -> Op2 -> Op1 -> Out (full serial chain)
            // Brassy/metallic
            4 => {
                let op4 = self.ops[3].next_sample(0.0);
                let op3 = self.ops[2].next_sample(op4 * mi * math::TWO_PI);
                let op2 = self.ops[1].next_sample(op3 * mi * math::TWO_PI);
                let op1 = self.ops[0].next_sample(op2 * mi * math::TWO_PI);
                op1
            }
            // Algorithm 5: (Op3 + Op4) -> Op2 -> Op1 -> Out
            // Dual modulators, complex textures
            5 => {
                let op3 = self.ops[2].next_sample(0.0);
                let op4 = self.ops[3].next_sample(0.0);
                let mod_sum = (op3 + op4) * mi * math::TWO_PI;
                let op2 = self.ops[1].next_sample(mod_sum);
                let op1 = self.ops[0].next_sample(op2 * mi * math::TWO_PI);
                op1
            }
            // Algorithm 6: (Op2 + Op3 + Op4) -> Op1 -> Out
            // Three modulators, one carrier — rich harmonics
            6 => {
                let op2 = self.ops[1].next_sample(0.0);
                let op3 = self.ops[2].next_sample(0.0);
                let op4 = self.ops[3].next_sample(0.0);
                let mod_sum = (op2 + op3 + op4) * mi * math::TWO_PI;
                let op1 = self.ops[0].next_sample(mod_sum);
                op1
            }
            // Algorithm 7: (Op2 -> Op1) -> Out + Op3 -> Out + Op4 -> Out
            // One stack + two carriers — DX7-like layered
            _ => {
                let op2 = self.ops[1].next_sample(0.0);
                let op1 = self.ops[0].next_sample(op2 * mi * math::TWO_PI);
                let op3 = self.ops[2].next_sample(0.0);
                let op4 = self.ops[3].next_sample(0.0);
                (op1 + op3 + op4) / 3.0
            }
        }
    }

    fn reset(&mut self) {
        for op in &mut self.ops {
            op.reset();
        }
        self.active = false;
        self.velocity_mod = 1.0;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FmParam {
    Algorithm,
    ModIndex,
    LfoRate,
    LfoDepth,
    LfoWaveform,
    LfoTarget,
    LfoSync,
    Feedback,       // Global feedback (all ops)
    Waveform,
    ChorusMix,
    VibratoRate,
    VibratoDepth,
    Attack,         // Carrier (op 0) attack
    Decay,          // Carrier (op 0) decay
    Sustain,        // Carrier (op 0) sustain
    Release,        // Carrier (op 0) release
    // Per-operator ADSR (op 0-3) — IDs 80-95
    Op0Attack, Op0Decay, Op0Sustain, Op0Release,
    Op1Attack, Op1Decay, Op1Sustain, Op1Release,
    Op2Attack, Op2Decay, Op2Sustain, Op2Release,
    Op3Attack, Op3Decay, Op3Sustain, Op3Release,
    // Per-operator feedback — IDs 96 would collide with beats, use after ops
    Op0Feedback, Op1Feedback, Op2Feedback, Op3Feedback,
    // Per-operator ratio
    Op0Ratio, Op1Ratio, Op2Ratio, Op3Ratio,
    Level,
}

pub struct FmModule {
    pub harmony: Option<crate::harmony::HarmonyContext>, // always None for FM
    voices: [FmVoice; MAX_VOICES],
    voice_counter: u32,
    algorithm: u8,
    mod_index: f32,
    /// Output gain. FM voices sum lower than the other modules; `level 2.0`
    /// puts a single voice in the same range as keys. Default 1.0 keeps
    /// existing songs unchanged.
    level: f32,
    // LFO
    lfo_router: ModulationRouter,
    // Chorus
    chorus: Chorus,
    chorus_mix: f32,
    // Pitch bend (set by engine)
    pub pitch_bend_ratio: f32,
    /// One-pole DC blocker state; see the comment in `process_block`.
    dc_x1: f32,
    dc_y1: f32,
    // Vibrato
    vibrato_phase: f32,
    vibrato_rate: f32,
    vibrato_depth: f32,
    vibrato_delay: f32,
    vibrato_onset: f32,
}

impl FmModule {
    pub fn new() -> Self {
        Self {
            harmony: None,
            voices: core::array::from_fn(|_| FmVoice::new()),
            voice_counter: 0,
            algorithm: 0,
            mod_index: 1.0,
            level: 1.0,
            lfo_router: ModulationRouter::new(SAMPLE_RATE, 5003),
            chorus: Chorus::new(),
            chorus_mix: 0.0,
            pitch_bend_ratio: 1.0,
            dc_x1: 0.0,
            dc_y1: 0.0,
            vibrato_phase: 0.0,
            vibrato_rate: 5.0,
            vibrato_depth: 0.0,
            vibrato_delay: 0.3,
            vibrato_onset: 0.0,
        }
    }

    /// Set a single operator's ratio across all voices.
    fn set_ratios_single(&mut self, op_index: usize, ratio: f32) {
        for voice in &mut self.voices {
            voice.ops[op_index].set_ratio(ratio);
        }
    }

    /// Set operator ratios. Ratios are frequency multipliers relative to the note.
    pub fn set_ratios(&mut self, ratios: [f32; 4]) {
        for voice in &mut self.voices {
            for (i, &r) in ratios.iter().enumerate() {
                voice.ops[i].set_ratio(r);
            }
        }
    }

    /// Set individual operator envelope.
    pub fn set_op_envelope(&mut self, op_index: usize, a: f32, d: f32, s: f32, r: f32) {
        if op_index >= 4 {
            return;
        }
        for voice in &mut self.voices {
            voice.ops[op_index].env.set_adsr(a, d, s, r);
        }
    }

    /// Set feedback for a specific operator across all voices.
    pub fn set_op_feedback(&mut self, op_index: usize, feedback: f32) {
        if op_index >= 4 {
            return;
        }
        for voice in &mut self.voices {
            voice.ops[op_index].set_feedback(feedback);
        }
    }

    pub fn set_param(&mut self, param: FmParam, value: f32) {
        match param {
            FmParam::Algorithm => {
                self.algorithm = (value * 7.0) as u8;
                for v in &mut self.voices {
                    v.algorithm = self.algorithm;
                }
            }
            FmParam::ModIndex => {
                self.mod_index = 0.1 * math::pow(40.0, value); // 0.1-4.0 exponential
                for v in &mut self.voices {
                    v.mod_index = self.mod_index;
                }
            }
            FmParam::LfoRate => {
                self.lfo_router.lfo.set_rate(0.1 + value * 19.9);
            }
            FmParam::LfoDepth => {
                self.lfo_router.lfo.set_depth(value);
                self.lfo_router.enabled = value > 0.001;
            }
            FmParam::LfoWaveform => {
                let idx = (value * 4.0) as u8;
                let wf = match idx {
                    0 => LfoWaveform::Sine,
                    1 => LfoWaveform::Triangle,
                    2 => LfoWaveform::Saw,
                    3 => LfoWaveform::Square,
                    _ => LfoWaveform::SampleHold,
                };
                self.lfo_router.lfo.set_waveform(wf);
            }
            FmParam::LfoTarget => {
                let idx = (value * 2.0) as u8;
                let target = match idx {
                    0 => LfoTarget::Pitch,
                    1 => LfoTarget::Amplitude,
                    _ => LfoTarget::ModIndex,
                };
                self.lfo_router.set_target(target);
            }
            FmParam::LfoSync => {
                let idx = (value * 11.0) as u8;
                let mode = match idx {
                    0 => LfoSyncMode::FreeHz,
                    1 => LfoSyncMode::Quarter,
                    2 => LfoSyncMode::Eighth,
                    3 => LfoSyncMode::Sixteenth,
                    4 => LfoSyncMode::DottedEighth,
                    5 => LfoSyncMode::TripletEighth,
                    6 => LfoSyncMode::Bar,
                    7 => LfoSyncMode::Bars2,
                    8 => LfoSyncMode::Bars4,
                    9 => LfoSyncMode::Bars8,
                    10 => LfoSyncMode::Bars12,
                    _ => LfoSyncMode::Bars16,
                };
                self.lfo_router.lfo.set_sync_mode(mode);
            }
            FmParam::Feedback => {
                let clamped = value * 0.7;
                for v in &mut self.voices {
                    for op in &mut v.ops {
                        op.set_feedback(clamped);
                    }
                }
            }
            FmParam::Waveform => {
                let wf = match (value * 3.0) as u8 {
                    0 => FmWaveform::Sine,
                    1 => FmWaveform::HalfSine,
                    2 => FmWaveform::AbsSine,
                    _ => FmWaveform::QuarterSine,
                };
                for v in &mut self.voices {
                    for op in &mut v.ops {
                        op.set_waveform(wf);
                    }
                }
            }
            FmParam::Level => self.level = value,
            FmParam::ChorusMix => {
                self.chorus_mix = value;
                self.chorus.set_mix(value);
            }
            FmParam::VibratoRate => self.vibrato_rate = crate::params::VIBRATO_RATE.to_real(value),
            FmParam::VibratoDepth => self.vibrato_depth = value * 0.5,
            FmParam::Attack => {
                // Carrier (op 0) only — modulators keep their own envelopes
                let a = crate::params::ENV_TIME.to_real(value) * 0.001;
                for v in &mut self.voices { v.ops[0].env.set_attack(a); }
            }
            FmParam::Decay => {
                let d = crate::params::ENV_TIME.to_real(value) * 0.001;
                for v in &mut self.voices { v.ops[0].env.set_decay(d); }
            }
            FmParam::Sustain => {
                for v in &mut self.voices { v.ops[0].env.set_sustain(value); }
            }
            FmParam::Release => {
                let r = crate::params::ENV_TIME.to_real(value) * 0.001;
                for v in &mut self.voices { v.ops[0].env.set_release(r); }
            }
            // Per-operator ADSR (logarithmic for A/D/R, raw for S)
            FmParam::Op0Attack => { let a = 0.001 * math::pow(2000.0, value); for v in &mut self.voices { v.ops[0].env.set_attack(a); } }
            FmParam::Op0Decay  => { let d = 0.001 * math::pow(2000.0, value); for v in &mut self.voices { v.ops[0].env.set_decay(d); } }
            FmParam::Op0Sustain => { for v in &mut self.voices { v.ops[0].env.set_sustain(value); } }
            FmParam::Op0Release => { let r = 0.001 * math::pow(2000.0, value); for v in &mut self.voices { v.ops[0].env.set_release(r); } }
            FmParam::Op1Attack => { let a = 0.001 * math::pow(2000.0, value); for v in &mut self.voices { v.ops[1].env.set_attack(a); } }
            FmParam::Op1Decay  => { let d = 0.001 * math::pow(2000.0, value); for v in &mut self.voices { v.ops[1].env.set_decay(d); } }
            FmParam::Op1Sustain => { for v in &mut self.voices { v.ops[1].env.set_sustain(value); } }
            FmParam::Op1Release => { let r = 0.001 * math::pow(2000.0, value); for v in &mut self.voices { v.ops[1].env.set_release(r); } }
            FmParam::Op2Attack => { let a = 0.001 * math::pow(2000.0, value); for v in &mut self.voices { v.ops[2].env.set_attack(a); } }
            FmParam::Op2Decay  => { let d = 0.001 * math::pow(2000.0, value); for v in &mut self.voices { v.ops[2].env.set_decay(d); } }
            FmParam::Op2Sustain => { for v in &mut self.voices { v.ops[2].env.set_sustain(value); } }
            FmParam::Op2Release => { let r = 0.001 * math::pow(2000.0, value); for v in &mut self.voices { v.ops[2].env.set_release(r); } }
            FmParam::Op3Attack => { let a = 0.001 * math::pow(2000.0, value); for v in &mut self.voices { v.ops[3].env.set_attack(a); } }
            FmParam::Op3Decay  => { let d = 0.001 * math::pow(2000.0, value); for v in &mut self.voices { v.ops[3].env.set_decay(d); } }
            FmParam::Op3Sustain => { for v in &mut self.voices { v.ops[3].env.set_sustain(value); } }
            FmParam::Op3Release => { let r = 0.001 * math::pow(2000.0, value); for v in &mut self.voices { v.ops[3].env.set_release(r); } }
            // Per-operator feedback
            FmParam::Op0Feedback => { for v in &mut self.voices { v.ops[0].set_feedback(value); } }
            FmParam::Op1Feedback => { for v in &mut self.voices { v.ops[1].set_feedback(value); } }
            FmParam::Op2Feedback => { for v in &mut self.voices { v.ops[2].set_feedback(value); } }
            FmParam::Op3Feedback => { for v in &mut self.voices { v.ops[3].set_feedback(value); } }
            // Per-operator ratio (0.0-1.0 → 0.5-16.0 range)
            FmParam::Op0Ratio => { let r = 0.5 + value * 15.5; self.set_ratios_single(0, r); }
            FmParam::Op1Ratio => { let r = 0.5 + value * 15.5; self.set_ratios_single(1, r); }
            FmParam::Op2Ratio => { let r = 0.5 + value * 15.5; self.set_ratios_single(2, r); }
            FmParam::Op3Ratio => { let r = 0.5 + value * 15.5; self.set_ratios_single(3, r); }
        }
    }

    pub fn set_bpm(&mut self, bpm: f32) {
        self.lfo_router.set_bpm(bpm);
    }

    fn find_voice(&self) -> usize {
        // 1. Prefer idle voice
        for (i, v) in self.voices.iter().enumerate() {
            if !v.active {
                return i;
            }
        }
        // 2. Prefer voice in release stage (oldest first)
        let mut oldest_release: Option<(usize, u32)> = None;
        for (i, v) in self.voices.iter().enumerate() {
            if v.ops[0].env.stage() == EnvStage::Release {
                match oldest_release {
                    None => oldest_release = Some((i, v.age)),
                    Some((_, age)) if v.age < age => oldest_release = Some((i, v.age)),
                    _ => {}
                }
            }
        }
        if let Some((i, _)) = oldest_release {
            return i;
        }
        // 3. Steal the oldest active voice
        let mut oldest_idx = 0;
        let mut oldest_age = u32::MAX;
        for (i, v) in self.voices.iter().enumerate() {
            if v.age < oldest_age {
                oldest_age = v.age;
                oldest_idx = i;
            }
        }
        oldest_idx
    }
}

impl Module for FmModule {
    fn process_block(&mut self, output: &mut [f32]) {
        for sample in output.iter_mut() {
            let lfo_val = self.lfo_router.next_sample();

            // Pitch LFO modulation
            let mut pitch_mult = 1.0;
            if self.lfo_router.target == LfoTarget::Pitch && self.lfo_router.enabled {
                pitch_mult = math::pow2(lfo_val);
            }

            // Vibrato with delayed onset
            let delay_samples = self.vibrato_delay * SAMPLE_RATE;
            if self.vibrato_onset < delay_samples {
                self.vibrato_onset += 1.0;
            }
            let onset_amt = if delay_samples > 0.0 {
                (self.vibrato_onset / delay_samples).min(1.0)
            } else {
                1.0
            };
            let vibrato_mult = if self.vibrato_depth > 0.0 {
                let vib = math::sin(self.vibrato_phase * math::TWO_PI)
                    * self.vibrato_depth * onset_amt;
                self.vibrato_phase += self.vibrato_rate / SAMPLE_RATE;
                if self.vibrato_phase >= 1.0 { self.vibrato_phase -= 1.0; }
                math::pow2(vib / 12.0)
            } else {
                1.0
            };

            // Apply pitch to all active voices (always — ensures pitch_bend + vibrato apply)
            let combined_pitch = pitch_mult * vibrato_mult * self.pitch_bend_ratio;
            for voice in &mut self.voices {
                if voice.active {
                    for op in &mut voice.ops {
                        op.osc.set_frequency(op.base_freq() * op.ratio * combined_pitch);
                    }
                }
            }

            // Apply LFO mod_index modulation
            if self.lfo_router.target == LfoTarget::ModIndex && self.lfo_router.enabled {
                let mod_idx = self.mod_index + lfo_val * 4.0;
                for voice in &mut self.voices {
                    if voice.active {
                        voice.mod_index = mod_idx;
                    }
                }
            }

            let mut sum = 0.0;
            for voice in &mut self.voices {
                sum += voice.next_sample();
            }

            // Apply LFO amplitude modulation
            let amp_mod = if self.lfo_router.target == LfoTarget::Amplitude && self.lfo_router.enabled {
                1.0 + lfo_val
            } else {
                1.0
            };

            sum *= amp_mod * self.level;

            // Apply chorus post-process
            if self.chorus_mix > 0.001 {
                sum = self.chorus.process(sum);
            }

            // The rectified operator waveforms are rectifiers, so they carry a
            // DC offset by construction: measured at 32% of peak on half_sine,
            // 64% on abs_sine and 15% on quarter_sine. It eats headroom, it
            // sums across every track that uses one, and it made a C4 note read
            // as 74% sub in the band report -- there was no sub, it was the
            // offset. A 5 Hz blocker takes it out below anything audible.
            const DC_R: f32 = 0.99929;
            let blocked = sum - self.dc_x1 + DC_R * self.dc_y1;
            self.dc_x1 = sum;
            self.dc_y1 = blocked;

            *sample = blocked;
        }
    }

    fn note_on(&mut self, note: u8, velocity: f32) {
        self.vibrato_onset = 0.0;
        let idx = self.find_voice();
        self.voice_counter += 1;
        self.voices[idx].algorithm = self.algorithm;
        self.voices[idx].mod_index = self.mod_index;
        self.voices[idx].age = self.voice_counter;
        self.voices[idx].note_on(note, velocity);
    }

    fn note_off(&mut self, note: u8) {
        for voice in &mut self.voices {
            if voice.active && voice.note == note {
                voice.note_off();
                break;
            }
        }
    }

    fn reset(&mut self) {
        for voice in &mut self.voices {
            voice.reset();
        }
        self.voice_counter = 0;
        self.lfo_router.reset();
        self.chorus.reset();
        self.dc_x1 = 0.0;
        self.dc_y1 = 0.0;
        self.pitch_bend_ratio = 1.0;
        self.vibrato_phase = 0.0;
        self.vibrato_onset = 0.0;
    }
}
