use crate::math;

#[derive(Clone, Copy, PartialEq, Debug)]
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
        // The cosine as 1 - 2 sin²(w0/2): at a low cutoff it sits within a
        // few millionths of 1, the size of the sine table's own error, and
        // read from the table it moved the filter. A `highpass(20hz, 0.05)`,
        // meant as +3.7 dB at 25 Hz, played +6.9 dB at 30 Hz at 44.1 kHz and
        // something else again at 32 kHz. The sine of a small angle reads
        // true off the table, so this lands on the designed filter.
        let half = math::sin(w0 * 0.5);
        let cos_w0 = 1.0 - 2.0 * half * half;
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

    /// Update cutoff and resonance without changing the filter type.
    pub fn set_resonance(&mut self, cutoff: f32, resonance: f32) {
        self.cutoff = math::clamp(cutoff, 20.0, self.sample_rate * 0.49);
        self.resonance = math::clamp(resonance, 0.01, 1.0);
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
    /// The resonance feedback path leaves a DC offset behind: measured at 1.5%
    /// of peak on a bass at the default resonance, against 0.3% with resonance
    /// off. It is inaudible on its own and eats headroom once tracks sum, so a
    /// 5 Hz blocker takes it out below anything anyone can hear.
    dc_x1: f32,
    dc_y1: f32,
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
            dc_x1: 0.0,
            dc_y1: 0.0,
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
        let out = self.process_inner(input) * self.gain_comp;
        // One-pole DC blocker at about 5 Hz.
        const R: f32 = crate::per_sample(0.99929);
        let y = out - self.dc_x1 + R * self.dc_y1;
        self.dc_x1 = out;
        self.dc_y1 = y;
        y
    }

    pub fn reset(&mut self) {
        self.stage = [0.0; 4];
        self.delay = [0.0; 4];
        self.dc_x1 = 0.0;
        self.dc_y1 = 0.0;
    }
}

// ─── State-variable filter (SEM-style multimode, 12 dB) ───

/// A two-pole state-variable filter in the trapezoidal (zero-delay feedback)
/// form, with its lowpass, bandpass and highpass taken from one pair of
/// integrators. The Oberheim SEM is this circuit: one knob crossfades the
/// lowpass into the highpass and passes through a notch on the way, and a
/// switch picks the bandpass instead.
///
/// The trapezoidal form is what lets the cutoff move at audio rate. A biquad
/// whose coefficients change every sample is not the filter its coefficients
/// describe, and pushed hard by an oscillator it rings or blows up; this one
/// keeps its state in the integrators and stays the same filter at any speed.
pub struct SvfFilter {
    ic1eq: f32,
    ic2eq: f32,
    a1: f32,
    a2: f32,
    a3: f32,
    k: f32,
    cutoff: f32,
    resonance: f32,
    sample_rate: f32,
}

/// The three outputs of one step.
#[derive(Clone, Copy, Debug)]
pub struct SvfOut {
    pub low: f32,
    pub band: f32,
    pub high: f32,
}

impl SvfFilter {
    pub fn new(sample_rate: f32) -> Self {
        let mut f = Self {
            ic1eq: 0.0,
            ic2eq: 0.0,
            a1: 1.0,
            a2: 0.0,
            a3: 0.0,
            k: 2.0,
            cutoff: 10000.0,
            resonance: 0.0,
            sample_rate,
        };
        f.recalc();
        f
    }

    /// Resonance reads as the biquads' does, Q = 0.5 + resonance * 19.5, so a
    /// patch moved from `lowpass` to `svf` keeps its peak.
    pub fn set_params(&mut self, cutoff: f32, resonance: f32) {
        self.cutoff = math::clamp(cutoff, 20.0, self.sample_rate * 0.49);
        self.resonance = math::clamp(resonance, 0.0, 1.0);
        self.recalc();
    }

    pub fn set_cutoff(&mut self, cutoff: f32) {
        self.cutoff = math::clamp(cutoff, 20.0, self.sample_rate * 0.49);
        self.recalc();
    }

    fn recalc(&mut self) {
        // g = tan(pi * fc / fs), with the cosine written as the biquad writes
        // it, 1 - 2 sin²(w/2), so a low cutoff reads true off the sine table.
        let w = math::PI * self.cutoff / self.sample_rate;
        let half = math::sin(w * 0.5);
        let g = math::sin(w) / (1.0 - 2.0 * half * half);
        self.k = 1.0 / (0.5 + self.resonance * 19.5);
        self.a1 = 1.0 / (1.0 + g * (g + self.k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }

    #[inline]
    pub fn process(&mut self, input: f32) -> SvfOut {
        let v3 = input - self.ic2eq;
        let v1 = self.a1 * self.ic1eq + self.a2 * v3;
        let v2 = self.ic2eq + self.a2 * self.ic1eq + self.a3 * v3;
        self.ic1eq = 2.0 * v1 - self.ic1eq;
        self.ic2eq = 2.0 * v2 - self.ic2eq;
        SvfOut { low: v2, band: v1, high: input - self.k * v1 - v2 }
    }

    /// The SEM's mode knob: 0 lowpass, 0.5 notch, 1 highpass, crossfaded in
    /// between. Halfway the two halves sum to a notch at half the level, so
    /// the crossfade is scaled back up by the louder side: the passband holds
    /// at unity all the way across.
    #[inline]
    pub fn morph(out: SvfOut, morph: f32) -> f32 {
        let m = math::clamp(morph, 0.0, 1.0);
        let lp = 1.0 - m;
        (lp * out.low + m * out.high) / lp.max(m)
    }

    /// The bandpass at unity on its peak, as the biquad bandpass is.
    #[inline]
    pub fn band(&self, out: SvfOut) -> f32 {
        out.band * self.k
    }

    pub fn reset(&mut self) {
        self.ic1eq = 0.0;
        self.ic2eq = 0.0;
    }
}
