//! The last thing every song goes through, and the one part of the mix a song
//! does not set: a gain that brings it to the same loudness as every other
//! song, and a limiter that keeps its true peak under -1 dB.
//!
//! Each song used to end in its own master chain, and its own idea of how
//! loud to be. Measured the same way the streaming services measure, the
//! examples spread over 8 dB, from -21.6 to -13.4 LUFS, and seven of them
//! peaked over full scale between samples, where a sample-peak limiter cannot
//! see: that is distortion the moment the file is converted or resampled.
//! Loudness and the ceiling are not a musical decision, so they are not a
//! control. A song's master chain still colours the mix; this sets its level.

use alloc::vec::Vec;

use crate::math;
use crate::SAMPLE_RATE;

/// Where every song lands, in LUFS. Close to where most of the examples
/// already were, so bringing them together takes little limiting and leaves
/// their dynamics alone. Streaming plays at -14; getting a song made at -21
/// there would flatten its drums.
pub const TARGET_LUFS: f32 = -18.0;

/// The true-peak ceiling, in dB. A song never goes over it.
pub const CEILING_DB: f32 = -1.0;

/// How far the stage delays the audio, in samples (1.7 ms): the limiter has
/// to see a peak coming to turn down smoothly before it.
pub const OUTPUT_DELAY: usize = INTERP_AHEAD + LOOKAHEAD - 1;

/// Samples of look-ahead for the limiter: 1.5 ms, long enough that turning
/// down over it is not heard as a click on the kick.
const LOOKAHEAD: usize = 66;

/// Samples past a point the interpolator needs to estimate the peak after it.
const INTERP_AHEAD: usize = 8;
const INTERP_TAPS: usize = 16;

/// Hann-windowed sinc taps that estimate the wave at each eighth of the way
/// from sample `n` to `n + 1`, from `x[n - 7] ..= x[n + 8]`. The ITU meter
/// oversamples four times, which can miss a peak between its points by a
/// few tenths of a dB; eight times misses less than the margin under the
/// ceiling. Checked against a 32-times interpolation of the rendered
/// examples, the loudest lands at -1.03 dB. (ffmpeg's `ebur128` reads the
/// same files up to 0.3 dB higher: its resampler rings.)
// Printed to nine places by the script that designed them; f32 keeps what it can.
#[allow(clippy::excessive_precision)]
const INTERP: [[f32; INTERP_TAPS]; 7] = [
    [
        -0.001080374,
        0.003591485,
        -0.008107968,
        0.015448013,
        -0.027364113,
        0.048931720,
        -0.103671226,
        0.974040346,
        0.135614286,
        -0.057477421,
        0.031495341,
        -0.017890355,
        0.009633771,
        -0.004508519,
        0.001550459,
        -0.000205446,
    ],
    [
        -0.001627413,
        0.005875778,
        -0.013693037,
        0.026480950,
        -0.047138350,
        0.083717542,
        -0.170631466,
        0.898431944,
        0.294389044,
        -0.115632070,
        0.062469397,
        -0.035526387,
        0.019339816,
        -0.009269085,
        0.003367697,
        -0.000554361,
    ],
    [
        -0.001698786,
        0.006755523,
        -0.016306292,
        0.032056542,
        -0.057412013,
        0.101462281,
        -0.200359181,
        0.780443106,
        0.464272926,
        -0.165134867,
        0.087670355,
        -0.049848980,
        0.027399751,
        -0.013420214,
        0.005119501,
        -0.000999654,
    ],
    [
        -0.001432930,
        0.006390225,
        -0.016038214,
        0.032103237,
        -0.057914905,
        0.102023068,
        -0.196306783,
        0.631176301,
        0.631176301,
        -0.196306783,
        0.102023068,
        -0.057914905,
        0.032103237,
        -0.016038214,
        0.006390225,
        -0.001432930,
    ],
    [
        -0.000999654,
        0.005119501,
        -0.013420214,
        0.027399751,
        -0.049848980,
        0.087670355,
        -0.165134867,
        0.464272926,
        0.780443106,
        -0.200359181,
        0.101462281,
        -0.057412013,
        0.032056542,
        -0.016306292,
        0.006755523,
        -0.001698786,
    ],
    [
        -0.000554361,
        0.003367697,
        -0.009269085,
        0.019339816,
        -0.035526387,
        0.062469397,
        -0.115632070,
        0.294389044,
        0.898431944,
        -0.170631466,
        0.083717542,
        -0.047138350,
        0.026480950,
        -0.013693037,
        0.005875778,
        -0.001627413,
    ],
    [
        -0.000205446,
        0.001550459,
        -0.004508519,
        0.009633771,
        -0.017890355,
        0.031495341,
        -0.057477421,
        0.135614286,
        0.974040346,
        -0.103671226,
        0.048931720,
        -0.027364113,
        0.015448013,
        -0.008107968,
        0.003591485,
        -0.001080374,
    ],
];

/// Ring sizes, powers of two so an index wraps with a mask.
const RING: usize = 128;
const MASK: usize = RING - 1;

/// The gain and the limiter at the end of the engine.
pub struct OutputStage {
    /// Linear gain that brings the song to `TARGET_LUFS`.
    pub gain: f32,
    /// Skip the stage: for measuring what goes into it.
    pub bypass: bool,
    limiter: TruePeakLimiter,
}

impl Default for OutputStage {
    fn default() -> Self {
        Self::new()
    }
}

impl OutputStage {
    pub fn new() -> Self {
        OutputStage { gain: 1.0, bypass: false, limiter: TruePeakLimiter::new() }
    }

    #[inline]
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        if self.bypass {
            return (l, r);
        }
        self.limiter.process(l * self.gain, r * self.gain)
    }

    pub fn reset(&mut self) {
        self.limiter.reset();
    }
}

/// The gain that takes a song measured at `lufs` to the target. A song that
/// measured silent keeps unity: there is nothing to level.
pub fn gain_for(lufs: Option<f32>) -> f32 {
    match lufs {
        Some(l) => math::db_to_linear((TARGET_LUFS - l).clamp(-30.0, 30.0)),
        None => 1.0,
    }
}

/// A look-ahead limiter on the true peak: the wave between the samples as
/// well as on them. The gain it needs for each point is held over the
/// look-ahead and then averaged over it, so it is already down, smoothly,
/// by the time the peak arrives, and never ducks in one step.
struct TruePeakLimiter {
    ceiling: f32,
    release: f32,
    /// Input, both channels, for the interpolator and the delay.
    in_l: [f32; RING],
    in_r: [f32; RING],
    /// Gain each recent point needs, and the released hold of it.
    need: [f32; RING],
    held: [f32; RING],
    /// True peak of the interval after the previous point.
    last_peak: f32,
    release_state: f32,
    pos: usize,
}

impl TruePeakLimiter {
    fn new() -> Self {
        TruePeakLimiter {
            // A fifth of a dB under the ceiling, for what eight times
            // oversampling can still miss.
            ceiling: math::db_to_linear(CEILING_DB - 0.2),
            release: 1.0 / (0.06 * SAMPLE_RATE),
            in_l: [0.0; RING],
            in_r: [0.0; RING],
            need: [1.0; RING],
            held: [1.0; RING],
            last_peak: 0.0,
            release_state: 1.0,
            pos: 0,
        }
    }

    fn reset(&mut self) {
        *self = Self::new();
    }

    fn at(&self, back: usize) -> (f32, f32) {
        let i = self.pos.wrapping_sub(back) & MASK;
        (self.in_l[i], self.in_r[i])
    }

    #[inline]
    fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        self.pos = (self.pos + 1) & MASK;
        self.in_l[self.pos] = l;
        self.in_r[self.pos] = r;

        // The point INTERP_AHEAD samples back, and the wave after it.
        let (pl, pr) = self.at(INTERP_AHEAD);
        let mut peak = math::abs(pl).max(math::abs(pr));
        for taps in &INTERP {
            let (mut yl, mut yr) = (0.0f32, 0.0f32);
            for (k, &t) in taps.iter().enumerate() {
                // tap k weighs x[n + k - 7], which is 15 - k samples back from now
                let (xl, xr) = self.at(INTERP_TAPS - 1 - k);
                yl += t * xl;
                yr += t * xr;
            }
            peak = peak.max(math::abs(yl)).max(math::abs(yr));
        }
        // A peak between two samples is heard through both of them.
        let around = peak.max(self.last_peak);
        self.last_peak = peak;
        let need = if around > self.ceiling { self.ceiling / around } else { 1.0 };
        self.need[self.pos] = need;

        // Hold the lowest need over the look-ahead, let go of it slowly ...
        let mut low = 1.0f32;
        for k in 0..LOOKAHEAD {
            low = low.min(self.need[self.pos.wrapping_sub(k) & MASK]);
        }
        self.release_state =
            if low < self.release_state { low } else { self.release_state + (low - self.release_state) * self.release };
        self.held[self.pos] = self.release_state;
        // ... and average the hold over the look-ahead, so the gain slopes
        // down into the peak instead of stepping.
        let mut sum = 0.0f32;
        for k in 0..LOOKAHEAD {
            sum += self.held[self.pos.wrapping_sub(k) & MASK];
        }
        let gain = sum / LOOKAHEAD as f32;

        let (ol, or) = self.at(OUTPUT_DELAY);
        (ol * gain, or * gain)
    }
}

/// Integrated loudness, as ITU-R BS.1770 and EBU R 128 define it: the
/// K-weighted power in 400 ms blocks, a quarter overlapping, with the silence
/// (under -70 LUFS) and the quiet passages (10 LU under the rest) left out.
pub struct Loudness {
    shelf: [Biquad; 2],
    highpass: [Biquad; 2],
    sum: f64,
    count: usize,
    /// Mean K-weighted power per 100 ms.
    hops: Vec<f64>,
}

impl Default for Loudness {
    fn default() -> Self {
        Self::new()
    }
}

const HOP: usize = (SAMPLE_RATE as usize) / 10;

impl Loudness {
    pub fn new() -> Self {
        // BS.1770's two stages, taken from their analog design to 44.1 kHz the
        // way libebur128 does. Its 48 kHz coefficients, or the same design
        // without the frequency warping, read a 1 kHz tone 0.26 dB quiet.
        let shelf = Biquad::new(
            [1.530_841_230_050_347_8, -2.650_979_995_154_729_7, 1.169_079_079_921_587],
            [-1.663_655_113_256_020_4, 0.712_595_428_073_225_4],
        );
        let highpass = Biquad::new([1.0, -2.0, 1.0], [-1.989_169_673_629_796, 0.989_199_035_787_039_3]);
        Loudness { shelf: [shelf; 2], highpass: [highpass; 2], sum: 0.0, count: 0, hops: Vec::new() }
    }

    pub fn push(&mut self, l: f32, r: f32) {
        let l = self.highpass[0].run(self.shelf[0].run(l as f64));
        let r = self.highpass[1].run(self.shelf[1].run(r as f64));
        self.sum += l * l + r * r;
        self.count += 1;
        if self.count == HOP {
            self.hops.push(self.sum / HOP as f64);
            self.sum = 0.0;
            self.count = 0;
        }
    }

    /// Integrated loudness in LUFS, or `None` when nothing was loud enough
    /// to count.
    pub fn lufs(&self) -> Option<f32> {
        let blocks: Vec<f64> = self.hops.windows(4).map(|w| w.iter().sum::<f64>() / 4.0).collect();
        let lufs = |p: f64| -0.691 + 10.0 * math::log10(p as f32);
        let over = |gate: f32| {
            let kept: Vec<f64> = blocks.iter().copied().filter(|&p| p > 0.0 && lufs(p) > gate).collect();
            (!kept.is_empty()).then(|| kept.iter().sum::<f64>() / kept.len() as f64)
        };
        let loud = over(-70.0)?;
        over(lufs(loud) - 10.0).map(lufs)
    }
}

#[derive(Clone, Copy)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    z: [f64; 2],
}

impl Biquad {
    fn new(b: [f64; 3], a: [f64; 2]) -> Self {
        Biquad { b, a, z: [0.0; 2] }
    }

    fn run(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f32, amp: f32, seconds: f32) -> Vec<f32> {
        let n = (seconds * SAMPLE_RATE) as usize;
        (0..n).map(|i| amp * libm_sin(2.0 * core::f32::consts::PI * hz * i as f32 / SAMPLE_RATE)).collect()
    }

    fn libm_sin(x: f32) -> f32 {
        std::primitive::f32::sin(x)
    }

    #[test]
    fn a_full_scale_1k_sine_in_both_channels_reads_about_zero_lufs() {
        // BS.1770's reference: a 0 dBFS 997 Hz sine in one channel is -3.01 LUFS,
        // so in both it is 0.
        let mut m = Loudness::new();
        for x in sine(997.0, 1.0, 3.0) {
            m.push(x, x);
        }
        let l = m.lufs().unwrap();
        assert!((l - 0.0).abs() < 0.2, "{l} LUFS");
    }

    #[test]
    fn silence_has_no_loudness() {
        let mut m = Loudness::new();
        for _ in 0..SAMPLE_RATE as usize {
            m.push(0.0, 0.0);
        }
        assert!(m.lufs().is_none());
    }

    #[test]
    fn the_limiter_keeps_the_peak_between_samples_under_the_ceiling() {
        // A sine at a quarter of the rate, phased so every sample lands at
        // 0.707 of the peak between them: a sample-peak limiter sees 3 dB less
        // than there is.
        let mut st = OutputStage::new();
        st.gain = math::db_to_linear(6.0);
        let n = SAMPLE_RATE as usize;
        let out: Vec<(f32, f32)> = (0..n)
            .map(|i| {
                let x = libm_sin(2.0 * core::f32::consts::PI * (i as f32 * 0.25 + 0.125));
                st.process(x, x)
            })
            .collect();
        // Reconstruct the true peak of the output the same way, by
        // interpolating it eight times over with a long sinc.
        let l: Vec<f32> = out.iter().map(|o| o.0).collect();
        let mut tp = 0.0f32;
        for i in n / 2..n - 64 {
            for f in 0..8 {
                let t = f as f32 / 8.0;
                let mut y = 0.0;
                for m in -31i32..=32 {
                    let x = t - m as f32;
                    let s =
                        if x == 0.0 { 1.0 } else { libm_sin(core::f32::consts::PI * x) / (core::f32::consts::PI * x) };
                    y += l[(i as i32 + m) as usize] * s;
                }
                tp = tp.max(y.abs());
            }
        }
        let db = 20.0 * std::primitive::f32::log10(tp);
        assert!(db <= CEILING_DB + 0.05 && db > CEILING_DB - 0.6, "true peak {db:.2} dB");
    }

    #[test]
    fn under_the_ceiling_the_stage_is_a_gain_and_a_delay() {
        let mut st = OutputStage::new();
        st.gain = 0.5;
        let input = sine(220.0, 0.5, 0.2);
        let out: Vec<f32> = input.iter().map(|&x| st.process(x, x).0).collect();
        for i in OUTPUT_DELAY..input.len() {
            assert!((out[i] - input[i - OUTPUT_DELAY] * 0.5).abs() < 1e-6, "sample {i}");
        }
    }
}
