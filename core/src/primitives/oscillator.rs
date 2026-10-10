use crate::math;
use crate::rng::Rng;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Waveform {
    Sine,
    Saw,
    Square,
    Triangle,
    Pulse(f32), // duty cycle 0.0-1.0
}

pub struct Oscillator {
    phase: f32,
    freq: f32,
    waveform: Waveform,
    sample_rate: f32,
    phase_inc: f32,
    // Drift state
    drift_value: f32,
    drift_counter: u32,
    drift_rng: Rng,
    drift_enabled: bool,
}

const DRIFT_UPDATE_INTERVAL: u32 = 512;
// ±3 cents = 2^(3/1200) - 1 ≈ 0.001731
const DRIFT_AMOUNT: f32 = 0.001731;

/// Wrap value to [0, 1).
#[inline]
fn fmod_1(x: f32) -> f32 {
    let r = x - math::floor(x);
    if r < 0.0 {
        r + 1.0
    } else {
        r
    }
}

/// PolyBLEP residual for anti-aliased waveforms.
/// `t` is the phase position relative to a discontinuity (0.0 at the edge),
/// `dt` is the phase increment per sample.
#[inline]
fn polyblep(t: f32, dt: f32) -> f32 {
    if t < dt {
        // t/dt is in [0, 1)
        let t = t / dt;
        t + t - t * t - 1.0
    } else if t > 1.0 - dt {
        // (t-1)/dt is in (-1, 0]
        let t = (t - 1.0) / dt;
        t * t + t + t + 1.0
    } else {
        0.0
    }
}

impl Oscillator {
    pub fn new(waveform: Waveform, sample_rate: f32) -> Self {
        Self {
            phase: 0.0,
            freq: 440.0,
            waveform,
            sample_rate,
            phase_inc: 440.0 / sample_rate,
            drift_value: 0.0,
            drift_counter: 0,
            drift_rng: Rng::new(42),
            drift_enabled: true,
        }
    }

    pub fn set_frequency(&mut self, freq: f32) {
        self.freq = freq;
        self.update_phase_inc();
    }

    pub fn set_waveform(&mut self, waveform: Waveform) {
        self.waveform = waveform;
    }

    pub fn set_drift_enabled(&mut self, enabled: bool) {
        self.drift_enabled = enabled;
    }

    pub fn set_drift_seed(&mut self, seed: u32) {
        self.drift_rng = Rng::new(seed);
    }

    fn update_phase_inc(&mut self) {
        let drift_mult = if self.drift_enabled { 1.0 + self.drift_value * DRIFT_AMOUNT } else { 1.0 };
        self.phase_inc = (self.freq * drift_mult) / self.sample_rate;
    }

    fn update_drift(&mut self) {
        self.drift_counter += 1;
        if self.drift_counter >= DRIFT_UPDATE_INTERVAL {
            self.drift_counter = 0;
            // Smooth drift: blend towards new random target
            let target = self.drift_rng.next_bipolar();
            self.drift_value += (target - self.drift_value) * 0.3;
            self.update_phase_inc();
        }
    }

    pub fn next_sample(&mut self) -> f32 {
        let out = self.compute_sample(0.0);
        self.advance_phase();
        if self.drift_enabled {
            self.update_drift();
        }
        out
    }

    /// Generate sample with phase modulation (for FM synthesis).
    pub fn next_sample_with_pm(&mut self, phase_mod: f32) -> f32 {
        let out = self.compute_sample(phase_mod);
        self.advance_phase();
        if self.drift_enabled {
            self.update_drift();
        }
        out
    }

    fn advance_phase(&mut self) {
        self.phase += self.phase_inc;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        if self.phase < 0.0 {
            self.phase += 1.0;
        }
    }

    fn compute_sample(&self, phase_mod: f32) -> f32 {
        // phase_mod is in radians; convert to normalized phase offset
        let pm_offset = phase_mod / math::TWO_PI;
        let mut p = self.phase + pm_offset;
        p = p - math::floor(p); // wrap to [0, 1)
        if p < 0.0 {
            p += 1.0;
        }

        let dt = self.phase_inc;

        match self.waveform {
            Waveform::Sine => math::sin(p * math::TWO_PI),
            Waveform::Saw => {
                let naive = 2.0 * p - 1.0;
                naive - polyblep(p, dt)
            }
            Waveform::Square => {
                let naive = if p < 0.5 { 1.0 } else { -1.0 };
                // Rising edge at p=0, falling edge at p=0.5
                naive + polyblep(p, dt) - polyblep(fmod_1(p + 0.5), dt)
            }
            Waveform::Triangle => {
                if p < 0.25 {
                    p * 4.0
                } else if p < 0.75 {
                    2.0 - p * 4.0
                } else {
                    p * 4.0 - 4.0
                }
            }
            Waveform::Pulse(duty) => {
                let naive = if p < duty { 1.0 } else { -1.0 };
                // Rising edge at p=0, falling edge at p=duty. A duty other
                // than a half spends longer on one side, which averages to
                // 2*duty - 1 rather than zero; that offset is taken back out.
                naive + polyblep(p, dt) - polyblep(fmod_1(p + 1.0 - duty), dt) - (2.0 * duty - 1.0)
            }
        }
    }

    pub fn phase(&self) -> f32 {
        self.phase
    }

    pub fn reset(&mut self) {
        self.phase = 0.0;
        self.drift_value = 0.0;
        self.drift_counter = 0;
    }

    /// Back to the start of the cycle, keeping the drift where it is: the
    /// pitch carries on moving as it was, only the phase lines up.
    pub fn reset_phase(&mut self) {
        self.phase = 0.0;
    }
}
