use crate::math;
use crate::primitives::oscillator::{Oscillator, Waveform};
use crate::primitives::envelope::Envelope;
use crate::primitives::filter::LadderFilter;
use crate::primitives::lfo::{LfoWaveform, LfoSyncMode, LfoTarget, ModulationRouter};
use crate::{Module, SAMPLE_RATE};

#[inline]
fn semitone_ratio(semitones: i8) -> f32 {
    math::pow2(semitones as f32 / 12.0)
}

/// `lfo_depth 1.0` sweeps the cutoff this many octaves either way.
pub const LFO_CUTOFF_OCTAVES: f32 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BassParam {
    Cutoff,
    CutoffEnv,
    Resonance,
    Glide,
    Attack,
    LfoRate,
    LfoDepth,
    LfoWaveform,
    LfoTarget,
    LfoSync,
    Osc2Pitch,
    Osc3Pitch,
    Osc1Wave,
    Osc2Wave,
    Osc3Wave,
    Decay,
    Sustain,
    Release,
    Keytrack,
    VibratoRate,
    VibratoDepth,
    VelEnv,
}

pub struct BassModule {
    oscs: [Oscillator; 3],
    osc_pitch_offsets: [i8; 3],
    amp_env: Envelope,
    filter_env: Envelope,
    filter: LadderFilter,
    // Glide
    current_freq: f32,
    target_freq: f32,
    glide_rate: f32, // 0-1, higher = faster glide
    // Filter params
    cutoff_base: f32,
    cutoff_env_amount: f32,
    resonance: f32,
    keytrack: f32,
    vel_env: f32,  // velocity-to-filter-envelope scaling (0=none, 1=full)
    // State
    velocity: f32,
    // LFO
    lfo_router: ModulationRouter,
    // Pitch bend (set by engine)
    pub pitch_bend_ratio: f32,
    // Vibrato
    vibrato_phase: f32,
    vibrato_rate: f32,
    vibrato_depth: f32,
    vibrato_delay: f32,
    vibrato_onset: f32,
}

impl Default for BassModule {
    fn default() -> Self {
        Self::new()
    }
}

impl BassModule {

    /// True when nothing is sounding and nothing is still releasing. A track
    /// whose level is 0 and whose instrument is idle is skipped whole by the
    /// song engine: no voices, no insert chain, no mix. That is what makes a
    /// rig of muted voices waiting to be brought in affordable, since `level`
    /// is a fast-path edit while scene membership is a structural one.
    pub fn is_idle(&self) -> bool {
        self.amp_env.is_idle()
    }
    pub fn new() -> Self {
        let mut osc0 = Oscillator::new(Waveform::Saw, SAMPLE_RATE);
        osc0.set_drift_seed(1337);
        let mut osc1 = Oscillator::new(Waveform::Saw, SAMPLE_RATE);
        osc1.set_drift_seed(1338);
        let mut osc2 = Oscillator::new(Waveform::Saw, SAMPLE_RATE);
        osc2.set_drift_seed(1339);

        let mut amp_env = Envelope::new(SAMPLE_RATE);
        amp_env.set_adsr(0.005, 0.2, 0.8, 0.15);

        let mut filter_env = Envelope::new(SAMPLE_RATE);
        filter_env.set_adsr(0.005, 0.3, 0.0, 0.1);

        Self {
            oscs: [osc0, osc1, osc2],
            osc_pitch_offsets: [0, 0, 0],
            amp_env,
            filter_env,
            filter: LadderFilter::new(SAMPLE_RATE),
            current_freq: 110.0,
            target_freq: 110.0,
            glide_rate: 0.003,
            cutoff_base: 400.0,
            cutoff_env_amount: 4000.0,
            resonance: 0.3,
            keytrack: 0.0,
            vel_env: 0.0,
            velocity: 1.0,
            lfo_router: ModulationRouter::new(SAMPLE_RATE, 5001),
            pitch_bend_ratio: 1.0,
            vibrato_phase: 0.0,
            vibrato_rate: 5.0,
            vibrato_depth: 0.0,
            vibrato_delay: 0.3,
            vibrato_onset: 0.0,
        }
    }

    pub fn set_param(&mut self, param: BassParam, value: f32) {
        match param {
            BassParam::Cutoff => {
                // Exponential scaling: 20Hz at 0.0, ~800Hz at 0.5, ~20kHz at 1.0
                // This gives more resolution in the bass/mid range where it matters
                self.cutoff_base = crate::params::BASS_CUTOFF.to_real(value);
            }
            BassParam::CutoffEnv => {
                // Envelope amount also exponential for more musical sweep
                self.cutoff_env_amount = crate::params::BASS_CUTOFF_ENV.to_real(value);
            }
            BassParam::Resonance => self.resonance = value,
            BassParam::Glide => self.glide_rate = 0.0001 + value * 0.05,
            BassParam::Attack => self.amp_env.set_attack(crate::params::ENV_TIME.to_real(value) * 0.001),
            BassParam::Decay => self.amp_env.set_decay(crate::params::ENV_TIME.to_real(value) * 0.001),
            BassParam::Sustain => self.amp_env.set_sustain(value),
            BassParam::Release => self.amp_env.set_release(crate::params::ENV_TIME.to_real(value) * 0.001),
            BassParam::LfoRate => {
                self.lfo_router.lfo.set_rate(crate::params::LFO_RATE.to_real(value));
            }
            BassParam::LfoDepth => {
                self.lfo_router.lfo.set_depth(value);
                self.lfo_router.enabled = value > 0.001;
            }
            BassParam::LfoWaveform => {
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
            BassParam::LfoTarget => {
                let idx = (value * 2.0) as u8;
                let target = match idx {
                    0 => LfoTarget::Cutoff,
                    1 => LfoTarget::Pitch,
                    _ => LfoTarget::Amplitude,
                };
                self.lfo_router.set_target(target);
            }
            BassParam::LfoSync => {
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
            BassParam::Osc2Pitch => {
                self.osc_pitch_offsets[1] = ((value * 48.0) - 24.0) as i8;
                self.oscs[1].set_frequency(self.current_freq * semitone_ratio(self.osc_pitch_offsets[1]));
            }
            BassParam::Osc3Pitch => {
                self.osc_pitch_offsets[2] = ((value * 48.0) - 24.0) as i8;
                self.oscs[2].set_frequency(self.current_freq * semitone_ratio(self.osc_pitch_offsets[2]));
            }
            BassParam::Osc1Wave => {
                self.oscs[0].set_waveform(if value < 0.5 { Waveform::Saw } else { Waveform::Square });
            }
            BassParam::Osc2Wave => {
                self.oscs[1].set_waveform(if value < 0.5 { Waveform::Saw } else { Waveform::Square });
            }
            BassParam::Osc3Wave => {
                self.oscs[2].set_waveform(if value < 0.5 { Waveform::Saw } else { Waveform::Square });
            }
            BassParam::Keytrack => self.keytrack = math::clamp(value, 0.0, 1.0),
            BassParam::VibratoRate => self.vibrato_rate = crate::params::VIBRATO_RATE.to_real(value),
            BassParam::VibratoDepth => self.vibrato_depth = value * 0.5,
            BassParam::VelEnv => self.vel_env = math::clamp(value, 0.0, 1.0),
        }
    }

    /// Slide to a new note without retriggering envelopes (portamento / legato).
    /// The existing glide_rate controls transition speed.
    pub fn slide_to(&mut self, note: u8, velocity: f32) {
        self.velocity = velocity;
        self.target_freq = math::midi_to_freq(note);
        // NO gate_on() — envelopes continue, glide_rate handles the transition
    }

    pub fn set_bpm(&mut self, bpm: f32) {
        self.lfo_router.set_bpm(bpm);
    }
}

impl Module for BassModule {
    fn process_block(&mut self, output: &mut [f32]) {
        // Set resonance once per block (avoids full recalc per sample)
        self.filter.set_params(self.cutoff_base, self.resonance);

        // Pre-compute semitone ratios once per block
        let ratio0 = semitone_ratio(self.osc_pitch_offsets[0]);
        let ratio1 = semitone_ratio(self.osc_pitch_offsets[1]);
        let ratio2 = semitone_ratio(self.osc_pitch_offsets[2]);

        for sample in output.iter_mut() {
            // Glide
            if math::abs(self.current_freq - self.target_freq) > 0.01 {
                self.current_freq += (self.target_freq - self.current_freq) * self.glide_rate;
            }

            let env_val = self.filter_env.next_sample();
            // Scale filter envelope by velocity when vel_env > 0
            let env_scale = 1.0 - self.vel_env + self.vel_env * self.velocity;
            let cutoff = self.cutoff_base + self.cutoff_env_amount * env_val * env_scale;

            // LFO modulation
            let lfo_val = self.lfo_router.next_sample();
            // Cutoff modulation is relative (in octaves), not an absolute Hz
            // offset: an offset large enough to hear on a bright sound slams a
            // dark one into the 20 Hz floor and then sweeps back up across the
            // whole spectrum, which rasps.
            let mut cutoff_mult = 1.0;
            let mut pitch_mult = 1.0;
            let mut amp_mod = 1.0;

            match self.lfo_router.target {
                LfoTarget::Cutoff => cutoff_mult = math::pow2(lfo_val * LFO_CUTOFF_OCTAVES),
                LfoTarget::Pitch => pitch_mult = math::pow2(lfo_val),
                LfoTarget::Amplitude => amp_mod = 1.0 + lfo_val,
                _ => {}
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
            if self.vibrato_depth > 0.0 {
                let vib = math::sin(self.vibrato_phase * math::TWO_PI)
                    * self.vibrato_depth * onset_amt;
                self.vibrato_phase += self.vibrato_rate / SAMPLE_RATE;
                if self.vibrato_phase >= 1.0 { self.vibrato_phase -= 1.0; }
                pitch_mult *= math::pow2(vib / 12.0);
            }

            self.filter.set_cutoff((cutoff + self.current_freq * self.keytrack) * cutoff_mult);

            let base = self.current_freq * pitch_mult * self.pitch_bend_ratio;
            self.oscs[0].set_frequency(base * ratio0);
            self.oscs[1].set_frequency(base * ratio1);
            self.oscs[2].set_frequency(base * ratio2);

            let raw = (self.oscs[0].next_sample()
                + self.oscs[1].next_sample()
                + self.oscs[2].next_sample()) / 3.0;
            let filtered = self.filter.process(raw);
            let amp = self.amp_env.next_sample();

            *sample = filtered * amp * self.velocity * amp_mod * 1.8;
        }
    }

    fn note_on(&mut self, note: u8, velocity: f32) {
        use crate::primitives::envelope::EnvStage;
        self.velocity = velocity;
        self.target_freq = math::midi_to_freq(note);

        let is_legato = !self.amp_env.is_idle()
            && self.amp_env.stage() != EnvStage::Release;

        if is_legato {
            // Legato: glide to new note without retriggering amp envelope.
            // Filter envelope always retriggered for expression (classic acid behavior).
            self.filter_env.gate_on();
        } else {
            // First note or note after release: snap frequency, retrigger both envelopes.
            self.current_freq = self.target_freq;
            for i in 0..3 {
                self.oscs[i].set_frequency(self.current_freq * semitone_ratio(self.osc_pitch_offsets[i]));
            }
            self.amp_env.gate_on();
            self.filter_env.gate_on();
        }
        self.vibrato_onset = 0.0;
    }

    fn note_off(&mut self, _note: u8) {
        self.amp_env.gate_off();
        self.filter_env.gate_off();
    }

    fn reset(&mut self) {
        for osc in self.oscs.iter_mut() {
            osc.reset();
        }
        self.amp_env.reset();
        self.filter_env.reset();
        self.filter.reset();
        self.current_freq = 110.0;
        self.target_freq = 110.0;
        self.velocity = 1.0;
        self.lfo_router.reset();
        self.pitch_bend_ratio = 1.0;
        self.vibrato_phase = 0.0;
        self.vibrato_onset = 0.0;
    }
}
