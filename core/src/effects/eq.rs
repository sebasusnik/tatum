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

/// Sine and cosine of `w` in 0..pi, from their series in f64. `math::cos`
/// reads a 1024-entry table, which is off by a few millionths near zero; a
/// low band's poles sit that close to 1, so through it a bell asked for at
/// 20 Hz cut deepest at 29. Thirty terms are exact to the last bit up to pi.
fn exact_sin_cos(w: f64) -> (f64, f64) {
    let (mut sin, mut cos) = (0.0, 0.0);
    let (mut s_term, mut c_term) = (w, 1.0);
    for n in 0..30 {
        sin += s_term;
        cos += c_term;
        let k = (2 * n + 2) as f64;
        c_term *= -w * w / (k * (k - 1.0));
        s_term *= -w * w / (k * (k + 1.0));
    }
    (sin, cos)
}

impl Biquad {
    /// One band's coefficients (RBJ cookbook) worked out in f64 with exact
    /// trigonometry, for bands that may sit anywhere down to 20 Hz.
    fn set_band(&mut self, band: Band, freq: f32, gain_db: f32, q: f32, sample_rate: f32) {
        let gain_db = gain_db as f64;
        let a = math::exp((gain_db / 40.0 * core::f64::consts::LN_10) as f32) as f64;
        let sqrt_a = math::exp((gain_db / 80.0 * core::f64::consts::LN_10) as f32) as f64;
        let (sin_w0, cos_w0) = exact_sin_cos(core::f64::consts::TAU * freq as f64 / sample_rate as f64);
        let (b0, b1, b2, a0, a1, a2) = match band {
            Band::Bell => {
                let alpha = sin_w0 / (2.0 * q as f64);
                (1.0 + alpha * a, -2.0 * cos_w0, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * cos_w0, 1.0 - alpha / a)
            }
            Band::LowShelf => {
                let k = 2.0 * sqrt_a * sin_w0 / (2.0 * 0.707);
                (
                    a * ((a + 1.0) - (a - 1.0) * cos_w0 + k),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w0),
                    a * ((a + 1.0) - (a - 1.0) * cos_w0 - k),
                    (a + 1.0) + (a - 1.0) * cos_w0 + k,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cos_w0),
                    (a + 1.0) + (a - 1.0) * cos_w0 - k,
                )
            }
            Band::HighShelf => {
                let k = 2.0 * sqrt_a * sin_w0 / (2.0 * 0.707);
                (
                    a * ((a + 1.0) + (a - 1.0) * cos_w0 + k),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w0),
                    a * ((a + 1.0) + (a - 1.0) * cos_w0 - k),
                    (a + 1.0) - (a - 1.0) * cos_w0 + k,
                    2.0 * ((a - 1.0) - (a + 1.0) * cos_w0),
                    (a + 1.0) - (a - 1.0) * cos_w0 - k,
                )
            }
        };
        self.b0 = (b0 / a0) as f32;
        self.b1 = (b1 / a0) as f32;
        self.b2 = (b2 / a0) as f32;
        self.a1 = (a1 / a0) as f32;
        self.a2 = (a2 / a0) as f32;
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

/// Which curve a [`ParamEq`] band draws.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Band {
    /// A bell at `freq`, `q` wide: the cut at 300 Hz that takes the box out
    /// of a kick, or the lift at its fundamental.
    Bell,
    LowShelf,
    HighShelf,
}

/// One band of a parametric EQ, anywhere in the spectrum. The fixed bands of
/// [`ThreeBandEq`] sit at 200 Hz, 1 kHz and 8 kHz, and nothing a kick or a
/// bass needs is at any of them: its fundamental is at 50, its box at 300,
/// its click at 4 kHz.
pub struct ParamEq {
    l: Biquad,
    r: Biquad,
    band: Band,
    freq: f32,
    gain_db: f32,
    q: f32,
    sample_rate: f32,
}

impl ParamEq {
    pub fn new(band: Band, freq: f32, gain_db: f32, q: f32, sample_rate: f32) -> Self {
        let mut eq = Self { l: Biquad::new(), r: Biquad::new(), band, freq, gain_db, q, sample_rate };
        eq.update();
        eq
    }

    pub fn set_gain(&mut self, gain_db: f32) {
        self.gain_db = gain_db;
        self.update();
    }

    pub fn set_freq(&mut self, freq: f32) {
        self.freq = freq;
        self.update();
    }

    fn update(&mut self) {
        // Under Nyquist with room to spare: the cookbook's bilinear warp
        // folds a band asked for at 21 kHz back down into the audible range.
        let freq = math::clamp(self.freq, 10.0, self.sample_rate * 0.45);
        let q = math::clamp(self.q, 0.1, 18.0);
        for f in [&mut self.l, &mut self.r] {
            f.set_band(self.band, freq, self.gain_db, q, self.sample_rate);
        }
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        self.l.process(x)
    }

    #[inline]
    pub fn process_stereo(&mut self, l: f32, r: f32) -> (f32, f32) {
        (self.l.process(l), self.r.process(r))
    }

    pub fn reset(&mut self) {
        self.l.reset();
        self.r.reset();
    }
}

#[cfg(test)]
mod param_eq_tests {
    use super::*;

    /// Gain of a steady sine at `hz` through `eq`, in dB, once it settles.
    fn gain_at(mut eq: ParamEq, hz: f32) -> f32 {
        let sr = 44100.0;
        let (mut peak_in, mut peak_out) = (0.0f32, 0.0f32);
        for i in 0..44100 {
            let x = math::sin(math::TWO_PI * hz * i as f32 / sr);
            let y = eq.process(x);
            if i > 22050 {
                peak_in = peak_in.max(math::abs(x));
                peak_out = peak_out.max(math::abs(y));
            }
        }
        20.0 * math::log10(peak_out / peak_in)
    }

    /// The filter's own response at `hz` in dB, from its coefficients as
    /// they are stored: what a sine there settles to, without the wait.
    fn response_db(eq: &ParamEq, hz: f64) -> f64 {
        let f = &eq.l;
        let w = core::f64::consts::TAU * hz / 44100.0;
        let (c1, s1, c2, s2) = (w.cos(), w.sin(), (2.0 * w).cos(), (2.0 * w).sin());
        let mag2 = |k0: f32, k1: f32, k2: f32| {
            let (k0, k1, k2) = (k0 as f64, k1 as f64, k2 as f64);
            let re = k0 + k1 * c1 + k2 * c2;
            let im = -(k1 * s1 + k2 * s2);
            re * re + im * im
        };
        10.0 * (mag2(f.b0, f.b1, f.b2) / mag2(1.0, f.a1, f.a2)).log10()
    }

    /// Where a cut is deepest, scanning an octave either side of `hz`.
    fn deepest(eq: &ParamEq, hz: f64) -> f64 {
        let (mut at, mut low) = (hz, f64::MAX);
        for i in 0..=4000 {
            let f = hz * 0.5 + hz * 1.5 * i as f64 / 4000.0;
            let r = response_db(eq, f);
            if r < low {
                (at, low) = (f, r);
            }
        }
        at
    }

    /// A cut at 20 Hz lands at 20 Hz: the table `math::cos` is read from is
    /// off by a few millionths near zero, and a bell's poles there sit that
    /// close to 1, so through it a 20 Hz cut was deepest at 29.
    #[test]
    fn a_low_bell_cuts_where_it_is_asked() {
        for (hz, q) in [(20.0, 2.0), (30.0, 2.0), (50.0, 1.0), (60.0, 4.0)] {
            let eq = ParamEq::new(Band::Bell, hz, -12.0, q, 44100.0);
            let at = deepest(&eq, hz as f64);
            let gain = response_db(&eq, hz as f64);
            std::println!("bell({hz}hz, -12db, q{q}): deepest at {at:.2} Hz, {gain:.3} dB at {hz} Hz");
            assert!((at / hz as f64 - 1.0).abs() < 0.01, "bell at {hz} Hz is deepest at {at:.2}");
            assert!((gain + 12.0).abs() < 0.2, "bell at {hz} Hz cuts {gain:.3} dB there");
        }
    }

    #[test]
    fn a_low_shelf_at_20hz_lifts_under_it() {
        let eq = ParamEq::new(Band::LowShelf, 20.0, 6.0, 0.707, 44100.0);
        let (corner, under, over) = (response_db(&eq, 20.0), response_db(&eq, 2.0), response_db(&eq, 400.0));
        std::println!("lowshelf(20hz, +6db): {under:.3} dB at 2 Hz, {corner:.3} at 20, {over:.3} at 400");
        assert!((corner - 3.0).abs() < 0.2, "{corner}");
        assert!((under - 6.0).abs() < 0.2, "{under}");
        assert!(over.abs() < 0.1, "{over}");
    }

    #[test]
    fn a_bell_moves_its_frequency_and_leaves_the_rest() {
        let bell = || ParamEq::new(Band::Bell, 300.0, -6.0, 1.5, 44100.0);
        assert!((gain_at(bell(), 300.0) + 6.0).abs() < 0.2);
        assert!(gain_at(bell(), 50.0).abs() < 0.5);
        assert!(gain_at(bell(), 4000.0).abs() < 0.5);
    }

    #[test]
    fn shelves_lift_their_side() {
        let low = || ParamEq::new(Band::LowShelf, 100.0, 4.0, 0.707, 44100.0);
        assert!((gain_at(low(), 30.0) - 4.0).abs() < 0.4);
        assert!(gain_at(low(), 2000.0).abs() < 0.3);
        let high = || ParamEq::new(Band::HighShelf, 6000.0, -5.0, 0.707, 44100.0);
        assert!((gain_at(high(), 16000.0) + 5.0).abs() < 0.6);
        assert!(gain_at(high(), 200.0).abs() < 0.3);
    }
}
