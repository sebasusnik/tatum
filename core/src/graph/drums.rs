//! The rest of the kit as nodes: `hat`, `snare` and `clap`. Like the `kick`
//! node (whose state `tom` shares), each is a whole piece: it carries its own
//! level, plays whole whatever the note's length, and ends its voice when it
//! is 60 dB down. Every time is given in ms and turned into samples at the
//! engine's rate here, and everything is set when the hit starts, the noise's
//! seed included, so the same hit is always the same samples.

use super::node::{fall_per_sample, NodeSpec};
use crate::primitives::filter::{BiquadFilter, FilterType};
use crate::rng::Rng;
use crate::{math, SAMPLE_RATE};

/// Below this a level has reached -60 dB and the piece is over.
const FLOOR: f32 = 1.0e-3;
/// Nothing is asked to sound over this share of the rate: a partial or a band
/// written higher is held here, so a build at 22050 Hz plays it as high as it
/// can and does not fold it back down.
const TOP: f32 = 0.45;

/// What keeps noise through a band at one loudness. White noise carries the
/// same power whatever the rate, spread up to half of it, so a band of it is
/// louder on a slower engine; and a band of one shape is wider in Hz the
/// higher it sits, so it lets more through. This brings a band at `band` Hz
/// to the level one at `reference` Hz has at 44.1 kHz. A band of one shape
/// lets through power in proportion to its centre; a `wide` one (a Q under
/// 1.5) runs into half the rate on its way up and gains about half of that
/// in dB, as measured from 2 to 8 kHz.
fn noise_gain(band: f32, reference: f32, wide: bool) -> f32 {
    let centre = math::sqrt(reference / band.min(TOP * SAMPLE_RATE));
    math::sqrt(SAMPLE_RATE / 44100.0) * if wide { math::sqrt(centre) } else { centre }
}

/// Where hit number `hit` is in its slow drift, -1..1: two sines of periods
/// that do not divide, as the kick sways, so it breathes over bars and does
/// not repeat soon. The same hit number is always the same value.
fn sway(hit: u32, a: f32, pa: f32, b: f32, pb: f32) -> f32 {
    let n = hit as f32;
    0.6 * math::sin(math::TWO_PI * n / a + pa) + 0.4 * math::sin(math::TWO_PI * n / b + pb)
}

fn samples(ms: f32) -> f32 {
    (ms * 0.001 * SAMPLE_RATE).max(1.0)
}

fn bandpass(hz: f32, resonance: f32) -> BiquadFilter {
    let mut f = BiquadFilter::new(SAMPLE_RATE);
    f.set_params(FilterType::BandPass, hz.min(TOP * SAMPLE_RATE), resonance);
    f
}

fn highpass(hz: f32) -> BiquadFilter {
    let mut f = BiquadFilter::new(SAMPLE_RATE);
    f.set_params(FilterType::HighPass, hz.min(TOP * SAMPLE_RATE), 0.01);
    f
}

// ── Hat ──

/// The six partials of the metal, as multiples of the lowest: the spacing of
/// the six oscillators of an 808's cymbal voice (205.3, 304.4, 369.6, 522.7,
/// 540 and 800 Hz), which share no harmonic.
const HAT_RATIOS: [f32; 6] = [1.0, 1.4827, 1.8003, 2.5460, 2.6303, 3.8967];
const HAT_SEED: u32 = 0x4841_5421;
/// The band's width: a Q of about 1.5, a band and not a whistle.
const HAT_BAND_RES: f32 = 0.05;
/// What brings the metal and the noise, each alone through the default band,
/// to the same loudness, and a hit to a peak near half of full scale.
const HAT_METAL_GAIN: f32 = 6.0;
const HAT_NOISE_GAIN: f32 = 0.95;
/// Where each partial is in its cycle when a hit starts. Six squares that all
/// step up on the same sample are one loud click before they are a hat; a
/// hit starts with them already apart.
const HAT_PHASES: [f32; 6] = [0.0, 0.37, 0.71, 0.13, 0.55, 0.89];

/// A band-limited square: the step at each edge smoothed over one sample
/// (polyBLEP), so the partials' harmonics above half the rate stay quiet.
#[inline]
fn square(phase: f32, dt: f32) -> f32 {
    let blep = |t: f32| {
        if t < dt {
            let x = t / dt;
            2.0 * x - x * x - 1.0
        } else if t > 1.0 - dt {
            let x = (t - 1.0) / dt;
            x * x + 2.0 * x + 1.0
        } else {
            0.0
        }
    };
    let half = if phase < 0.5 { phase + 0.5 } else { phase - 0.5 };
    (if phase < 0.5 { 1.0 } else { -1.0 }) + blep(phase) - blep(half)
}

/// The `hat` node at run time: six square partials and noise, through a band
/// and a highpass, under a level that falls 60 dB in `decay` ms from the hit.
pub struct HatState {
    phase: [f32; 6],
    inc: [f32; 6],
    metal: f32,
    noise: f32,
    band: BiquadFilter,
    floor: BiquadFilter,
    level: f32,
    mul: f32,
    rng: Rng,
    pub(super) done: bool,
}

impl HatState {
    pub fn new(spec: &NodeSpec) -> Self {
        Self::build(spec, None)
    }

    /// The hat for hit number `hit` of its instrument. With `vary=`, its
    /// decay sways by that share, slowly, and the metal starts where six
    /// oscillators that never stop would be by then, so no two hits are the
    /// same samples; the same hit number is always the same hat.
    pub fn new_hit(spec: &NodeSpec, hit: u32) -> Self {
        Self::build(spec, Some(hit))
    }

    fn build(spec: &NodeSpec, hit: Option<u32>) -> Self {
        let NodeSpec::Hat { decay_ms, band, floor, noise, pitch, vary } = *spec else {
            unreachable!("HatState takes a hat")
        };
        let top = TOP * SAMPLE_RATE;
        let mut phase = HAT_PHASES;
        let mut decay_ms = decay_ms;
        if let (Some(hit), true) = (hit, vary > 0.0) {
            decay_ms *= 1.0 + vary * sway(hit, 31.0, 0.7, 12.3, 2.1);
            for (i, p) in phase.iter_mut().enumerate() {
                // A sixteenth at 125 BPM between hits: far enough for every
                // partial to have turned many times, each by its own amount.
                let turns = HAT_PHASES[i] + hit as f32 * 0.12 * pitch * HAT_RATIOS[i];
                *p = turns - math::floor(turns);
            }
        }
        Self {
            phase,
            inc: core::array::from_fn(|i| (pitch * HAT_RATIOS[i]).min(top) / SAMPLE_RATE),
            metal: (1.0 - noise) * HAT_METAL_GAIN / 6.0,
            noise: noise * HAT_NOISE_GAIN * noise_gain(band, 8000.0, true),
            band: bandpass(band, HAT_BAND_RES),
            floor: highpass(floor),
            level: 1.0,
            mul: fall_per_sample(-60.0, decay_ms),
            rng: Rng::new(HAT_SEED),
            done: false,
        }
    }

    #[inline]
    pub fn next_sample(&mut self) -> f32 {
        if self.done {
            return 0.0;
        }
        let mut metal = 0.0;
        for (p, &dt) in self.phase.iter_mut().zip(self.inc.iter()) {
            metal += square(*p, dt);
            *p += dt;
            if *p >= 1.0 {
                *p -= 1.0;
            }
        }
        let src = metal * self.metal + self.rng.next_bipolar() * self.noise;
        let out = self.floor.process(self.band.process(src)) * self.level;
        self.level *= self.mul;
        if self.level < FLOOR {
            self.done = true;
        }
        out
    }

    pub fn is_idle(&self) -> bool {
        self.done
    }
}

// ── Snare ──

const SNARE_SEED: u32 = 0x534e_4152;
/// The drop of the hit: the body starts this share over its note.
const SNARE_DROP: f32 = 0.7;
/// The second mode of the head against the first: a struck membrane's (1,1)
/// mode sits 1.593 times over its (0,1).
const SNARE_MODE: f32 = 1.593;
/// Its level against the first.
const SNARE_MODE_LEVEL: f32 = 0.5;
/// The wires' band is wide: a Q of about 0.9.
const SNARE_BAND_RES: f32 = 0.02;
/// What brings the wires alone to the peak the body alone has.
const SNARE_WIRES_GAIN: f32 = 1.6;

/// The `snare` node at run time: a body of two sines a membrane's interval
/// apart, dropping onto the note, and noise through a band for the wires,
/// each under its own level falling 60 dB in its own time.
pub struct SnareState {
    phase: [f32; 2],
    note: f32,
    /// The share over the note the pitch still is, and its fall per sample.
    drop: f32,
    drop_mul: f32,
    body: f32,
    body_mul: f32,
    body_gain: f32,
    wires: f32,
    wires_mul: f32,
    wires_gain: f32,
    band: BiquadFilter,
    drive_gain: f32,
    drive_norm: f32,
    rng: Rng,
    pub(super) done: bool,
}

impl SnareState {
    pub fn new(spec: &NodeSpec) -> Self {
        Self::build(spec, None)
    }

    /// The snare for hit number `hit`: with `vary=`, the decays of its body
    /// and its wires sway by that share, slowly, and the same on every play.
    pub fn new_hit(spec: &NodeSpec, hit: u32) -> Self {
        Self::build(spec, Some(hit))
    }

    fn build(spec: &NodeSpec, hit: Option<u32>) -> Self {
        let NodeSpec::Snare { note, snap_ms, body_ms, wires_ms, band, balance, drive, vary } = *spec else {
            unreachable!("SnareState takes a snare")
        };
        let (mut body_ms, mut wires_ms) = (body_ms, wires_ms);
        if let (Some(hit), true) = (hit, vary > 0.0) {
            body_ms *= 1.0 + vary * sway(hit, 41.0, 1.3, 14.9, 0.2);
            wires_ms *= 1.0 + vary * sway(hit, 33.0, 0.4, 11.3, 1.7);
        }
        // The pitch is within 10% of the note when the share over it is down
        // from SNARE_DROP to 0.1, in `snap_ms`.
        let snap_tau = samples(snap_ms) / math::ln(SNARE_DROP / 0.1);
        let drive_gain = 1.0 + 5.0 * drive;
        Self {
            phase: [0.0; 2],
            note,
            drop: SNARE_DROP,
            drop_mul: math::exp(-1.0 / snap_tau),
            body: 1.0,
            body_mul: fall_per_sample(-60.0, body_ms),
            body_gain: (1.0 - balance) / (1.0 + SNARE_MODE_LEVEL),
            wires: 1.0,
            wires_mul: fall_per_sample(-60.0, wires_ms),
            wires_gain: balance * SNARE_WIRES_GAIN * noise_gain(band, 4500.0, true),
            band: bandpass(band, SNARE_BAND_RES),
            drive_gain,
            drive_norm: 1.0 / math::tanh(drive_gain),
            rng: Rng::new(SNARE_SEED),
            done: false,
        }
    }

    #[inline]
    pub fn next_sample(&mut self) -> f32 {
        if self.done {
            return 0.0;
        }
        // The second mode dies twice as fast as the first, in dB.
        let body = (math::sin(self.phase[0] * math::TWO_PI) * self.body
            + SNARE_MODE_LEVEL * math::sin(self.phase[1] * math::TWO_PI) * self.body * self.body)
            * self.body_gain;
        let wires = if self.wires_gain > 0.0 {
            self.band.process(self.rng.next_bipolar()) * self.wires * self.wires_gain
        } else {
            0.0
        };
        let mut out = body + wires;
        if self.drive_gain > 1.0 {
            out = math::tanh(out * self.drive_gain) * self.drive_norm;
        }

        let freq = self.note * (1.0 + self.drop);
        self.drop *= self.drop_mul;
        for (p, ratio) in self.phase.iter_mut().zip([1.0, SNARE_MODE]) {
            *p += (freq * ratio).min(TOP * SAMPLE_RATE) / SAMPLE_RATE;
            if *p >= 1.0 {
                *p -= 1.0;
            }
        }
        self.body *= self.body_mul;
        self.wires *= self.wires_mul;
        let body_over = self.body < FLOOR || self.body_gain == 0.0;
        let wires_over = self.wires < FLOOR || self.wires_gain == 0.0;
        if body_over && wires_over {
            self.done = true;
        }
        out
    }

    pub fn is_idle(&self) -> bool {
        self.done
    }
}

// ── Clap ──

const CLAP_SEED: u32 = 0x434c_4150;
/// How far each burst has fallen when the next one starts.
const CLAP_BURST_DB: f32 = -20.0;
/// The band's width: a Q of about 2.
const CLAP_BAND_RES: f32 = 0.08;
/// What brings a clap in its default band to a peak near full scale.
const CLAP_GAIN: f32 = 2.8;

/// The `clap` node at run time: noise through a band, under a level that
/// starts again at full for every burst, falls fast between them and, after
/// the last, falls 60 dB in `tail` ms.
pub struct ClapState {
    /// Samples since the hit, and where the next burst starts.
    at: u32,
    next: u32,
    /// Bursts still to start, the first not counted.
    left: u32,
    /// Samples from one burst to the next, kept whole and in fractions so
    /// the spacing does not round off burst after burst.
    spacing: f32,
    started: u32,
    level: f32,
    burst_mul: f32,
    tail_mul: f32,
    gain: f32,
    band: BiquadFilter,
    rng: Rng,
    pub(super) done: bool,
}

impl ClapState {
    pub fn new(spec: &NodeSpec) -> Self {
        Self::build(spec, None)
    }

    /// The clap for hit number `hit`: with `vary=`, the spacing of its bursts
    /// and its tail sway by that share, each on its own slow drift, the way
    /// hands do not meet twice alike; the same on every play.
    pub fn new_hit(spec: &NodeSpec, hit: u32) -> Self {
        Self::build(spec, Some(hit))
    }

    fn build(spec: &NodeSpec, hit: Option<u32>) -> Self {
        let NodeSpec::Clap { bursts, spacing_ms, tail_ms, band, vary } = *spec else {
            unreachable!("ClapState takes a clap")
        };
        let (mut spacing_ms, mut tail_ms) = (spacing_ms, tail_ms);
        if let (Some(hit), true) = (hit, vary > 0.0) {
            spacing_ms *= 1.0 + vary * sway(hit, 23.0, 2.4, 9.7, 0.9);
            tail_ms *= 1.0 + vary * sway(hit, 35.0, 0.1, 13.1, 1.5);
        }
        let spacing = samples(spacing_ms);
        let left = (bursts as u32).max(1) - 1;
        Self {
            at: 0,
            next: spacing as u32,
            left,
            spacing,
            started: 1,
            level: 1.0,
            burst_mul: fall_per_sample(CLAP_BURST_DB, spacing_ms),
            tail_mul: fall_per_sample(-60.0, tail_ms),
            gain: CLAP_GAIN * noise_gain(band, 1200.0, false),
            band: bandpass(band, CLAP_BAND_RES),
            rng: Rng::new(CLAP_SEED),
            done: false,
        }
    }

    #[inline]
    pub fn next_sample(&mut self) -> f32 {
        if self.done {
            return 0.0;
        }
        if self.left > 0 && self.at >= self.next {
            self.left -= 1;
            self.started += 1;
            self.next = (self.started as f32 * self.spacing) as u32;
            self.level = 1.0;
        }
        let out = self.band.process(self.rng.next_bipolar()) * self.level * self.gain;
        self.level *= if self.left > 0 { self.burst_mul } else { self.tail_mul };
        self.at += 1;
        if self.left == 0 && self.level < FLOOR {
            self.done = true;
        }
        out
    }

    pub fn is_idle(&self) -> bool {
        self.done
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whatever is asked for, no partial and no band is set over 0.45 of the
    /// rate the engine is built at (the CI renders at 22050 and 32000 Hz too).
    #[test]
    fn nothing_is_set_over_half_the_rate() {
        let spec =
            NodeSpec::Hat { decay_ms: 60.0, band: 16000.0, floor: 12000.0, noise: 0.3, pitch: 4000.0, vary: 0.0 };
        let hat = HatState::new(&spec);
        for inc in hat.inc {
            assert!(inc <= TOP + 1e-6, "a partial at {} of the rate", inc);
        }
        // A square's phase step over a half would be a wave running backwards.
        assert!(hat.inc.iter().all(|i| *i < 0.5));
    }

    #[test]
    fn a_square_is_one_up_and_one_down() {
        let dt = 500.0 / SAMPLE_RATE;
        let (mut p, mut sum, mut top) = (0.0f32, 0.0f32, 0.0f32);
        let n = (SAMPLE_RATE / 5.0) as usize;
        for _ in 0..n {
            let s = square(p, dt);
            sum += s;
            top = top.max(s.abs());
            p += dt;
            if p >= 1.0 {
                p -= 1.0;
            }
        }
        assert!((sum / n as f32).abs() < 0.02, "no DC: {}", sum / n as f32);
        assert!(top <= 1.0 + 1e-3, "within full scale: {top}");
    }
}
