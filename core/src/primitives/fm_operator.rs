use crate::math;
use crate::primitives::oscillator::{Oscillator, Waveform};
use crate::primitives::envelope::Envelope;

#[derive(Clone, Copy, PartialEq)]
pub enum FmWaveform {
    Sine,        // sin(x)
    HalfSine,    // max(sin(x), 0)
    AbsSine,     // |sin(x)|
    QuarterSine, // sin(x) for first quarter of cycle, 0 otherwise
}

pub struct FmOperator {
    pub osc: Oscillator,
    pub env: Envelope,
    pub ratio: f32,
    pub amplitude: f32,
    base_freq: f32,
    waveform: FmWaveform,
    feedback: f32,
    prev_output: f32,
}

impl FmOperator {
    pub fn new(sample_rate: f32) -> Self {
        let mut osc = Oscillator::new(Waveform::Sine, sample_rate);
        osc.set_drift_enabled(false); // FM operators need clean pitch
        Self {
            osc,
            env: Envelope::new(sample_rate),
            ratio: 1.0,
            amplitude: 1.0,
            base_freq: 440.0,
            waveform: FmWaveform::Sine,
            feedback: 0.0,
            prev_output: 0.0,
        }
    }

    pub fn base_freq(&self) -> f32 {
        self.base_freq
    }

    pub fn set_ratio(&mut self, ratio: f32) {
        self.ratio = ratio;
        self.osc.set_frequency(self.base_freq * self.ratio);
    }

    pub fn set_amplitude(&mut self, amplitude: f32) {
        self.amplitude = amplitude;
    }

    pub fn set_waveform(&mut self, wf: FmWaveform) {
        self.waveform = wf;
    }

    pub fn set_feedback(&mut self, fb: f32) {
        self.feedback = fb;
    }

    pub fn note_on(&mut self, freq: f32) {
        self.base_freq = freq;
        self.osc.set_frequency(freq * self.ratio);
        // From silence only, as in `keys`: resetting the phase of an operator
        // that is still sounding is a step in the wave, heard as a click.
        if self.env.is_idle() {
            self.osc.reset();
            self.prev_output = 0.0;
        }
        self.env.gate_on();
    }

    pub fn note_off(&mut self) {
        self.env.gate_off();
    }

    /// Generate one sample, with optional phase modulation input (radians).
    pub fn next_sample(&mut self, phase_mod: f32) -> f32 {
        let fb_mod = self.prev_output * self.feedback * math::TWO_PI;
        let env = self.env.next_sample();
        let raw_osc = self.osc.next_sample_with_pm(phase_mod + fb_mod);
        let shaped = self.apply_waveform(raw_osc);
        let out = shaped * env * self.amplitude;
        self.prev_output = out;
        out
    }

    fn apply_waveform(&self, raw: f32) -> f32 {
        match self.waveform {
            FmWaveform::Sine => raw,
            FmWaveform::HalfSine => {
                if raw > 0.0 {
                    raw
                } else {
                    0.0
                }
            }
            FmWaveform::AbsSine => math::abs(raw),
            FmWaveform::QuarterSine => {
                let p = self.osc.phase(); // 0.0..1.0 normalized (after advance)
                                          // We want the first quarter: phase was just advanced, so check
                                          // if we're in the first quarter of the cycle
                if p < 0.25 {
                    raw
                } else {
                    0.0
                }
            }
        }
    }

    pub fn is_idle(&self) -> bool {
        self.env.is_idle()
    }

    pub fn reset(&mut self) {
        self.osc.reset();
        self.env.reset();
        self.prev_output = 0.0;
    }
}
