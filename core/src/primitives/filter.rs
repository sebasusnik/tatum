use crate::math;

#[derive(Clone, Copy, PartialEq)]
pub enum FilterType {
    LowPass,
    HighPass,
    BandPass,
}

// ─── Biquad Filter (Direct Form II Transposed) ───

pub struct BiquadFilter {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
    sample_rate: f32,
    filter_type: FilterType,
    cutoff: f32,
    resonance: f32,
}

impl BiquadFilter {
    pub fn new(sample_rate: f32) -> Self {
        let mut f = Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            z1: 0.0,
            z2: 0.0,
            sample_rate,
            filter_type: FilterType::LowPass,
            cutoff: 10000.0,
            resonance: 0.5,
        };
        f.recalc();
        f
    }

    pub fn set_params(&mut self, filter_type: FilterType, cutoff: f32, resonance: f32) {
        self.filter_type = filter_type;
        self.cutoff = math::clamp(cutoff, 20.0, self.sample_rate * 0.49);
        self.resonance = math::clamp(resonance, 0.01, 1.0);
        self.recalc();
    }

    fn recalc(&mut self) {
        let w0 = math::TWO_PI * self.cutoff / self.sample_rate;
        let sin_w0 = math::sin(w0);
        let cos_w0 = math::cos(w0);
        // Q from resonance: map 0-1 to 0.5-20
        let q = 0.5 + self.resonance * 19.5;
        let alpha = sin_w0 / (2.0 * q);

        let (b0, b1, b2, a0, a1, a2) = match self.filter_type {
            FilterType::LowPass => {
                let b1 = 1.0 - cos_w0;
                let b0 = b1 * 0.5;
                let b2 = b0;
                let a0 = 1.0 + alpha;
                let a1 = -2.0 * cos_w0;
                let a2 = 1.0 - alpha;
                (b0, b1, b2, a0, a1, a2)
            }
            FilterType::HighPass => {
                let b1c = 1.0 + cos_w0;
                let b0 = b1c * 0.5;
                let b1 = -(1.0 + cos_w0);
                let b2 = b0;
                let a0 = 1.0 + alpha;
                let a1 = -2.0 * cos_w0;
                let a2 = 1.0 - alpha;
                (b0, b1, b2, a0, a1, a2)
            }
            FilterType::BandPass => {
                let b0 = alpha;
                let b1 = 0.0;
                let b2 = -alpha;
                let a0 = 1.0 + alpha;
                let a1 = -2.0 * cos_w0;
                let a2 = 1.0 - alpha;
                (b0, b1, b2, a0, a1, a2)
            }
        };

        let inv_a0 = 1.0 / a0;
        self.b0 = b0 * inv_a0;
        self.b1 = b1 * inv_a0;
        self.b2 = b2 * inv_a0;
        self.a1 = a1 * inv_a0;
        self.a2 = a2 * inv_a0;
    }

    /// Update only the cutoff frequency (skips filter_type/resonance).
    /// Use this for per-sample cutoff modulation after calling set_params once.
    pub fn set_cutoff(&mut self, cutoff: f32) {
        self.cutoff = math::clamp(cutoff, 20.0, self.sample_rate * 0.49);
        self.recalc();
    }

    pub fn process(&mut self, input: f32) -> f32 {
        let out = self.b0 * input + self.z1;
        self.z1 = self.b1 * input - self.a1 * out + self.z2;
        self.z2 = self.b2 * input - self.a2 * out;
        out
    }

    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

// ─── Ladder Filter (Moog-style 4-pole) ───

pub struct LadderFilter {
    stage: [f32; 4],
    delay: [f32; 4],
    cutoff: f32,
    resonance: f32,
    sample_rate: f32,
    sample_rate_4x: f32,
    tune: f32,
    res_scale: f32,
    gain_comp: f32,
}

impl LadderFilter {
    pub fn new(sample_rate: f32) -> Self {
        let mut f = Self {
            stage: [0.0; 4],
            delay: [0.0; 4],
            cutoff: 10000.0,
            resonance: 0.0,
            sample_rate,
            sample_rate_4x: sample_rate * 4.0,
            tune: 0.0,
            res_scale: 0.0,
            gain_comp: 1.0,
        };
        f.recalc();
        f
    }

    pub fn set_params(&mut self, cutoff: f32, resonance: f32) {
        self.cutoff = math::clamp(cutoff, 20.0, self.sample_rate * 0.49);
        self.resonance = math::clamp(resonance, 0.0, 1.0);
        self.recalc();
    }

    /// Update only the cutoff frequency (skips resonance recalc).
    /// Use this for per-sample cutoff modulation after calling set_params once.
    pub fn set_cutoff(&mut self, cutoff: f32) {
        self.cutoff = math::clamp(cutoff, 20.0, self.sample_rate * 0.49);
        let fc = self.cutoff / self.sample_rate_4x;
        self.tune = (1.0 - math::exp(-math::TWO_PI * fc)) * 1.8730;
    }

    fn recalc(&mut self) {
        // Use 4x oversampled rate for tuning with Huovilainen correction
        let fc = self.cutoff / self.sample_rate_4x;
        self.tune = (1.0 - math::exp(-math::TWO_PI * fc)) * 1.8730;
        // Resonance scaling: 0-1 maps to 0-4 for self-oscillation
        self.res_scale = self.resonance * 4.0;
        // Gain compensation: restore volume lost at high resonance
        self.gain_comp = 1.0 + self.res_scale * 0.25;
    }

    /// Inner processing step (one oversampled tick).
    fn process_inner(&mut self, input: f32) -> f32 {
        let res_input = input - self.res_scale * self.delay[3];
        // Soft clip the feedback
        let clipped = math::tanh(res_input);

        // Four cascaded one-pole filters with per-stage saturation
        self.stage[0] = self.delay[0] + self.tune * (math::tanh(clipped) - self.delay[0]);
        self.stage[1] = self.delay[1] + self.tune * (math::tanh(self.stage[0]) - self.delay[1]);
        self.stage[2] = self.delay[2] + self.tune * (math::tanh(self.stage[1]) - self.delay[2]);
        self.stage[3] = self.delay[3] + self.tune * (math::tanh(self.stage[2]) - self.delay[3]);

        self.delay[0] = self.stage[0];
        self.delay[1] = self.stage[1];
        self.delay[2] = self.stage[2];
        self.delay[3] = self.stage[3];

        self.stage[3]
    }

    /// Process with 4x oversampling: run four inner steps, return the last result.
    pub fn process(&mut self, input: f32) -> f32 {
        self.process_inner(input);
        self.process_inner(input);
        self.process_inner(input);
        let out = self.process_inner(input);
        out * self.gain_comp
    }

    pub fn reset(&mut self) {
        self.stage = [0.0; 4];
        self.delay = [0.0; 4];
    }
}
