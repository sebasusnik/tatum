use crate::math;

/// Biquad filter (Direct Form II Transposed) for shelving and peaking EQ.
/// Implements low shelf, high shelf, and peaking EQ using RBJ Audio EQ Cookbook formulas.
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    fn new() -> Self {
        Self { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0, z1: 0.0, z2: 0.0 }
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let out = self.b0 * input + self.z1;
        self.z1 = self.b1 * input - self.a1 * out + self.z2;
        self.z2 = self.b2 * input - self.a2 * out;
        out
    }

    fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }

    /// Low shelf filter coefficients (RBJ cookbook).
    fn set_low_shelf(&mut self, freq: f32, gain_db: f32, sample_rate: f32) {
        let a = math::pow(10.0, gain_db / 40.0);
        let w0 = math::TWO_PI * freq / sample_rate;
        let sin_w0 = math::sin(w0);
        let cos_w0 = math::cos(w0);
        let alpha = sin_w0 / (2.0 * 0.707);
        let two_sqrt_a_alpha = 2.0 * math::sqrt(a) * alpha;

        let a0 = (a + 1.0) + (a - 1.0) * cos_w0 + two_sqrt_a_alpha;
        let inv_a0 = 1.0 / a0;

        self.b0 = a * ((a + 1.0) - (a - 1.0) * cos_w0 + two_sqrt_a_alpha) * inv_a0;
        self.b1 = 2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w0) * inv_a0;
        self.b2 = a * ((a + 1.0) - (a - 1.0) * cos_w0 - two_sqrt_a_alpha) * inv_a0;
        self.a1 = -2.0 * ((a - 1.0) + (a + 1.0) * cos_w0) * inv_a0;
        self.a2 = ((a + 1.0) + (a - 1.0) * cos_w0 - two_sqrt_a_alpha) * inv_a0;
    }

    /// High shelf filter coefficients (RBJ cookbook).
    fn set_high_shelf(&mut self, freq: f32, gain_db: f32, sample_rate: f32) {
        let a = math::pow(10.0, gain_db / 40.0);
        let w0 = math::TWO_PI * freq / sample_rate;
        let sin_w0 = math::sin(w0);
        let cos_w0 = math::cos(w0);
        let alpha = sin_w0 / (2.0 * 0.707);
        let two_sqrt_a_alpha = 2.0 * math::sqrt(a) * alpha;

        let a0 = (a + 1.0) - (a - 1.0) * cos_w0 + two_sqrt_a_alpha;
        let inv_a0 = 1.0 / a0;

        self.b0 = a * ((a + 1.0) + (a - 1.0) * cos_w0 + two_sqrt_a_alpha) * inv_a0;
        self.b1 = -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w0) * inv_a0;
        self.b2 = a * ((a + 1.0) + (a - 1.0) * cos_w0 - two_sqrt_a_alpha) * inv_a0;
        self.a1 = 2.0 * ((a - 1.0) - (a + 1.0) * cos_w0) * inv_a0;
        self.a2 = ((a + 1.0) - (a - 1.0) * cos_w0 - two_sqrt_a_alpha) * inv_a0;
    }

    /// Peaking EQ filter coefficients (RBJ cookbook).
    fn set_peaking(&mut self, freq: f32, gain_db: f32, q: f32, sample_rate: f32) {
        let a = math::pow(10.0, gain_db / 40.0);
        let w0 = math::TWO_PI * freq / sample_rate;
        let sin_w0 = math::sin(w0);
        let cos_w0 = math::cos(w0);
        let alpha = sin_w0 / (2.0 * q);

        let a0 = 1.0 + alpha / a;
        let inv_a0 = 1.0 / a0;

        self.b0 = (1.0 + alpha * a) * inv_a0;
        self.b1 = -2.0 * cos_w0 * inv_a0;
        self.b2 = (1.0 - alpha * a) * inv_a0;
        self.a1 = -2.0 * cos_w0 * inv_a0;
        self.a2 = (1.0 - alpha / a) * inv_a0;
    }
}

/// Tilt EQ: single knob that tilts frequency balance between bright and dark.
/// Crossover at 800Hz. Tilt +1.0 = dark (bass up, treble down), -1.0 = bright.
pub struct TiltEq {
    low_l: Biquad,
    low_r: Biquad,
    high_l: Biquad,
    high_r: Biquad,
    sample_rate: f32,
}

impl TiltEq {
    pub fn new(sample_rate: f32) -> Self {
        Self { low_l: Biquad::new(), low_r: Biquad::new(), high_l: Biquad::new(), high_r: Biquad::new(), sample_rate }
    }

    /// Set tilt amount: -1.0 (bright) to +1.0 (dark).
    /// Low shelf gets +tilt*6dB, high shelf gets -tilt*6dB.
    pub fn set_tilt(&mut self, amount: f32) {
        let amount = math::clamp(amount, -1.0, 1.0);
        let low_db = amount * 6.0;
        let high_db = -amount * 6.0;
        self.low_l.set_low_shelf(800.0, low_db, self.sample_rate);
        self.low_r.set_low_shelf(800.0, low_db, self.sample_rate);
        self.high_l.set_high_shelf(800.0, high_db, self.sample_rate);
        self.high_r.set_high_shelf(800.0, high_db, self.sample_rate);
    }

    #[inline]
    pub fn process_stereo(&mut self, l: f32, r: f32) -> (f32, f32) {
        let l = self.high_l.process(self.low_l.process(l));
        let r = self.high_r.process(self.low_r.process(r));
        (l, r)
    }

    pub fn reset(&mut self) {
        self.low_l.reset();
        self.low_r.reset();
        self.high_l.reset();
        self.high_r.reset();
    }
}

/// 3-band parametric EQ: low shelf (200Hz) + mid peak (1kHz, Q=1.0) + high shelf (8kHz).
/// Each band has ±12dB range.
pub struct ThreeBandEq {
    low_l: Biquad,
    low_r: Biquad,
    mid_l: Biquad,
    mid_r: Biquad,
    high_l: Biquad,
    high_r: Biquad,
    sample_rate: f32,
}

impl ThreeBandEq {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            low_l: Biquad::new(),
            low_r: Biquad::new(),
            mid_l: Biquad::new(),
            mid_r: Biquad::new(),
            high_l: Biquad::new(),
            high_r: Biquad::new(),
            sample_rate,
        }
    }

    /// Set low shelf gain in dB (clamped to ±12dB). Frequency: 200Hz.
    pub fn set_low(&mut self, gain_db: f32) {
        let db = math::clamp(gain_db, -12.0, 12.0);
        self.low_l.set_low_shelf(200.0, db, self.sample_rate);
        self.low_r.set_low_shelf(200.0, db, self.sample_rate);
    }

    /// Set mid peak gain in dB (clamped to ±12dB). Frequency: 1kHz, Q=1.0.
    pub fn set_mid(&mut self, gain_db: f32) {
        let db = math::clamp(gain_db, -12.0, 12.0);
        self.mid_l.set_peaking(1000.0, db, 1.0, self.sample_rate);
        self.mid_r.set_peaking(1000.0, db, 1.0, self.sample_rate);
    }

    /// Set high shelf gain in dB (clamped to ±12dB). Frequency: 8kHz.
    pub fn set_high(&mut self, gain_db: f32) {
        let db = math::clamp(gain_db, -12.0, 12.0);
        self.high_l.set_high_shelf(8000.0, db, self.sample_rate);
        self.high_r.set_high_shelf(8000.0, db, self.sample_rate);
    }

    #[inline]
    pub fn process_stereo(&mut self, l: f32, r: f32) -> (f32, f32) {
        let l = self.high_l.process(self.mid_l.process(self.low_l.process(l)));
        let r = self.high_r.process(self.mid_r.process(self.low_r.process(r)));
        (l, r)
    }

    pub fn reset(&mut self) {
        self.low_l.reset();
        self.low_r.reset();
        self.mid_l.reset();
        self.mid_r.reset();
        self.high_l.reset();
        self.high_r.reset();
    }
}
