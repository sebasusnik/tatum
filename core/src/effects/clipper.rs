//! A hard clipper: the signal pushed into a ceiling and cut flat there.
//!
//! The density of a club master is largely clipping: the kick's peak, which
//! carries little loudness, is shaved off so everything else can come up.
//! `saturate` is a tanh, which rounds the peak over a wide range and
//! thickens what it touches; a clipper leaves everything under the ceiling
//! exactly alone and only cuts what goes over it.
//!
//! A flat cut has corners, and corners at 44.1 kHz alias: the harmonics they
//! make above Nyquist fold back down as tones that are not in the signal.
//! The clipper is antialiased with a first-order antiderivative (ADAA): it
//! outputs the average of the clip function over the step between two
//! samples instead of the clip of one sample, which takes the folded
//! harmonics down by tens of dB for half a sample of delay.

use crate::math;

pub struct Clipper {
    drive: f32,
    ceiling: f32,
    /// Previous input, scaled, per channel.
    last: [f32; 2],
}

impl Clipper {
    /// `drive` is the gain into the ceiling, `ceiling` its level (linear).
    pub fn new(drive: f32, ceiling: f32) -> Self {
        Self { drive: math::clamp(drive, 0.01, 100.0), ceiling: math::clamp(ceiling, 0.01, 1.0), last: [0.0; 2] }
    }

    pub fn set_drive(&mut self, drive: f32) {
        self.drive = math::clamp(drive, 0.01, 100.0);
    }

    /// Antiderivative of the clip at 1: x²/2 inside, |x| - 1/2 outside.
    #[inline]
    fn integral(x: f32) -> f32 {
        if math::abs(x) <= 1.0 {
            0.5 * x * x
        } else {
            math::abs(x) - 0.5
        }
    }

    #[inline]
    fn run(&mut self, ch: usize, input: f32) -> f32 {
        // Clipped at 1 and scaled back to the ceiling: x is the input as a
        // fraction of the ceiling.
        let x = input * self.drive / self.ceiling;
        let prev = self.last[ch];
        self.last[ch] = x;
        let dx = x - prev;
        // Where both samples sit on one piece of the clip, its average over
        // the step is known outright: the midpoint under the ceiling, the
        // ceiling over it. The difference quotient is only needed across a
        // corner; between two close samples it cancels in f32, and a slow
        // wave under the ceiling came out a percent off through it.
        let y = if math::abs(x) <= 1.0 && math::abs(prev) <= 1.0 {
            0.5 * (x + prev)
        } else if x > 1.0 && prev > 1.0 {
            1.0
        } else if x < -1.0 && prev < -1.0 {
            -1.0
        } else if math::abs(dx) < 1e-5 {
            math::clamp(0.5 * (x + prev), -1.0, 1.0)
        } else {
            (Self::integral(x) - Self::integral(prev)) / dx
        };
        y * self.ceiling
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        self.run(0, x)
    }

    #[inline]
    pub fn process_stereo(&mut self, l: f32, r: f32) -> (f32, f32) {
        (self.run(0, l), self.run(1, r))
    }

    pub fn reset(&mut self) {
        self.last = [0.0; 2];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    fn sine(hz: f32, amp: f32, n: usize) -> impl Iterator<Item = f32> {
        (0..n).map(move |i| amp * math::sin(math::TWO_PI * hz * i as f32 / 44100.0))
    }

    #[test]
    fn under_the_ceiling_nothing_changes_and_over_it_nothing_passes() {
        let mut c = Clipper::new(1.0, 0.5);
        let ys: Vec<f32> = sine(100.0, 0.3, 4410).map(|x| c.process(x)).collect();
        // Half a sample late, and otherwise the same: a 100 Hz sine moves
        // 0.2% of its peak in half a sample.
        let xs: Vec<f32> = sine(100.0, 0.3, 4410).collect();
        for i in 1..xs.len() {
            assert!((ys[i] - 0.5 * (xs[i] + xs[i - 1])).abs() < 1e-4, "at {i}");
        }
        let mut c = Clipper::new(4.0, 0.5);
        for y in sine(100.0, 0.9, 4410).map(|x| c.process(x)) {
            assert!(y.abs() <= 0.5 + 1e-6, "{y}");
        }
    }

    /// A slow wave, 0.5 Hz, driven 20 times into a ceiling at -24 dB,
    /// against the clip worked out in f64: from one that stays under the
    /// ceiling to one 50 dB over it. Two samples this close are a few
    /// hundred-thousandths apart, where the difference quotient of the
    /// antiderivative cancels in f32: under the ceiling it once came out
    /// about a percent off.
    #[test]
    fn a_slow_wave_comes_out_as_the_exact_clip() {
        let ceiling = math::db_to_linear(-24.0);
        let integral = |x: f64| if x.abs() <= 1.0 { 0.5 * x * x } else { x.abs() - 0.5 };
        // The peak of the driven wave, as a fraction of the ceiling.
        for peak in [0.5f32, 0.9, 0.999, 1.5, 316.0] {
            let amp = peak * ceiling / 20.0;
            let mut c = Clipper::new(20.0, ceiling);
            let (mut prev, mut worst) = (0.0f64, 0.0f64);
            for i in 0..88200 {
                let input = amp * math::sin(math::TWO_PI * 0.5 * i as f32 / 44100.0);
                let y = c.process(input) as f64;
                let x = (input * 20.0 / ceiling) as f64;
                let ideal = if x == prev { x.clamp(-1.0, 1.0) } else { (integral(x) - integral(prev)) / (x - prev) };
                prev = x;
                worst = worst.max((y / ceiling as f64 - ideal).abs());
            }
            std::println!("peak {peak}: worst deviation {worst:.3e} of the ceiling");
            assert!(worst < 1e-4, "at a peak of {peak}: {worst:.3e} of the ceiling");
        }
    }

    /// The energy a clipped 3.1 kHz sine puts where none of its harmonics
    /// are: what folded back from above Nyquist. The antialiased clipper
    /// leaves far less there than clipping each sample does.
    #[test]
    fn it_folds_back_far_less_than_a_naive_clip() {
        let n = 8192;
        let hz = 44100.0 * 577.0 / 8192.0; // on a bin: 3106 Hz
        let input: Vec<f32> = sine(hz, 1.0, n).collect();
        let mut c = Clipper::new(8.0, 1.0);
        let adaa: Vec<f32> = input.iter().map(|&x| c.process(x)).collect();
        let naive: Vec<f32> = input.iter().map(|&x| math::clamp(8.0 * x, -1.0, 1.0)).collect();
        // Odd harmonics of bin 577 that stay under Nyquist land on bins
        // 577 * k; everything else is aliasing.
        let alias = |x: &[f32]| -> f64 {
            let mut total = 0.0f64;
            // Every fifth bin is plenty to weigh the floor between harmonics.
            for bin in (1..n / 2).step_by(5) {
                if bin % 577 == 0 {
                    continue;
                }
                let (mut re, mut im) = (0.0f64, 0.0f64);
                for (i, v) in x.iter().enumerate() {
                    let a = -2.0 * core::f64::consts::PI * (bin * i) as f64 / n as f64;
                    re += *v as f64 * a.cos();
                    im += *v as f64 * a.sin();
                }
                total += re * re + im * im;
            }
            total
        };
        let (a, b) = (alias(&adaa), alias(&naive));
        assert!(a < b * 0.2, "adaa {a:.3e} against naive {b:.3e}");
    }
}
