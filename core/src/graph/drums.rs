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
    // Where the hit is in each period, found in f64 and kept to one turn: as
    // an f32 the count outgrew it, 4% off by hit 1,000,000 and anywhere at
    // all by hit 100,000,000.
    let turn = |period: f32| {
        let turns = hit as f64 / period as f64;
        (turns - (turns as u64) as f64) as f32
    };
    0.6 * math::sin(math::TWO_PI * turn(a) + pa) + 0.4 * math::sin(math::TWO_PI * turn(b) + pb)
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
/// to the same loudness, and a hit to a peak near half of full scale. Only
/// the balance between the two is left to them: `hat_gain` sets the level.
const HAT_METAL_GAIN: f32 = 6.0;
const HAT_NOISE_GAIN: f32 = 0.95;
/// The level every hat is brought to: the root mean square of `hat()` over
/// its 60 ms at 44.1 kHz, where it peaks near half of full scale.
const HAT_LEVEL: f32 = 0.045_277_47;
/// No hit of any hat goes over this peak, so that none clips its track: the
/// level gives way to it on a hat that is all edge.
const HAT_PEAK: f32 = 0.95;
/// Where each partial is in its cycle when a hit starts. Six squares that all
/// step up on the same sample are one loud click before they are a hat; a
/// hit starts with them already apart, three high and three low.
const HAT_PHASES: [f32; 6] = [0.0, 0.37, 0.71, 0.13, 0.55, 0.89];
/// How many places the metal can start a hit from when it sways. They come
/// round by the hit's number, so hit 40,000 is as different from its
/// neighbours as hit 4, and not in step with a bar of any length; and they
/// are few enough for every one to be played when the line is compiled.
const HAT_STARTS: u32 = 29;

/// Where the six partials are when a hit starts from place `start`: where
/// oscillators that never stop would be `start` sixteenths at 125 BPM on,
/// each kept to the half of its cycle `HAT_PHASES` puts it in, so the six
/// steps of the first sample cancel from every place as they do from the
/// first. Left free they all step the same way from one place in thirty.
fn hat_phases(start: u32, pitch: f32) -> [f32; 6] {
    core::array::from_fn(|i| {
        let half = if HAT_PHASES[i] < 0.5 { 0.0 } else { 0.5 };
        let turns = 2.0 * (HAT_PHASES[i] - half) + start as f32 * 0.12 * pitch * HAT_RATIOS[i];
        half + (0.5 * (turns - math::floor(turns))).min(0.499_999)
    })
}

/// What brings the hat these arguments make to the level `hat()` has. The
/// band, the floor, the pitch and the share of noise each decide how much of
/// the six squares and of the noise comes through, by tens of dB from one end
/// of their ranges to the other, and so does the rate the engine runs at;
/// nothing short of playing the hit says by how much. So it is played here,
/// once, when the line is compiled, and measured: its mean square over its
/// length, and its peak. With `vary` it is played from every place the metal
/// can start, at the longest its decay sways to: the level is their mean and
/// the peak the highest of them, which no hit can then pass. A hit costs
/// what it did, and nothing is allocated.
pub fn hat_gain(decay_ms: f32, band: f32, floor: f32, noise: f32, pitch: f32, vary: f32) -> f32 {
    let decay_ms = decay_ms * (1.0 + vary);
    let spec = NodeSpec::Hat { decay_ms, band, floor, noise, pitch, gain: 1.0, vary: 0.0 };
    let len = samples(decay_ms);
    let starts = if vary > 0.0 { HAT_STARTS } else { 1 };
    let (mut peak, mut power) = (0.0f32, 0.0f64);
    for start in 0..starts {
        let mut hat = HatState::new(&spec);
        hat.phase = hat_phases(start, pitch);
        // As far as 30 dB down: a thousandth of it is left after that.
        for _ in 0..(len * 0.5) as usize + 1 {
            let s = hat.next_sample();
            peak = peak.max(s.abs());
            power += (s * s) as f64;
        }
    }
    let level = math::sqrt((power / (len as f64 * starts as f64)) as f32);
    if level < 1.0e-9 {
        // Nothing comes through at all: there is nothing to bring up.
        return 1.0;
    }
    (HAT_LEVEL / level).min(HAT_PEAK / peak)
}

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
    /// decay sways by that share, slowly, and the metal starts from another
    /// of its places on every hit, so no two hits are the same samples; the
    /// same hit number is always the same hat.
    pub fn new_hit(spec: &NodeSpec, hit: u32) -> Self {
        Self::build(spec, Some(hit))
    }

    fn build(spec: &NodeSpec, hit: Option<u32>) -> Self {
        let NodeSpec::Hat { decay_ms, band, floor, noise, pitch, gain, vary } = *spec else {
            unreachable!("HatState takes a hat")
        };
        let top = TOP * SAMPLE_RATE;
        let mut phase = HAT_PHASES;
        let mut decay_ms = decay_ms;
        if let (Some(hit), true) = (hit, vary > 0.0) {
            decay_ms *= 1.0 + vary * sway(hit, 31.0, 0.7, 12.3, 2.1);
            phase = hat_phases(hit % HAT_STARTS, pitch);
        }
        Self {
            phase,
            inc: core::array::from_fn(|i| (pitch * HAT_RATIOS[i]).min(top) / SAMPLE_RATE),
            metal: gain * (1.0 - noise) * HAT_METAL_GAIN / 6.0,
            noise: gain * noise * HAT_NOISE_GAIN * noise_gain(band, 8000.0, true),
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
        let spec = NodeSpec::Hat {
            decay_ms: 60.0,
            band: 16000.0,
            floor: 12000.0,
            noise: 0.3,
            pitch: 4000.0,
            gain: 1.0,
            vary: 0.0,
        };
        let hat = HatState::new(&spec);
        for inc in hat.inc {
            assert!(inc <= TOP + 1e-6, "a partial at {} of the rate", inc);
        }
        // A square's phase step over a half would be a wave running backwards.
        assert!(hat.inc.iter().all(|i| *i < 0.5));
    }

    fn hat(decay_ms: f32, band: f32, floor: f32, noise: f32, pitch: f32, vary: f32) -> NodeSpec {
        let gain = hat_gain(decay_ms, band, floor, noise, pitch, vary);
        NodeSpec::Hat { decay_ms, band, floor, noise, pitch, gain, vary }
    }

    /// Where the six partials start on hit `hit`, as bits.
    fn start_of(spec: &NodeSpec, hit: u32) -> [u32; 6] {
        HatState::new_hit(spec, hit).phase.map(f32::to_bits)
    }

    /// The hit count as a float outgrew the fraction it was multiplied for:
    /// at `pitch=1khz` 64 hits in a row started 54 ways from hit 0, 11 ways
    /// from hit 5,000 and one way, all six partials at zero, from hit 40,000.
    #[test]
    fn the_metal_starts_as_many_ways_at_hit_forty_thousand_as_at_hit_four() {
        for pitch in [205.0, 1000.0] {
            let spec = hat(60.0, 8000.0, 6000.0, 0.0, pitch, 0.1);
            let ways = |from: u32| {
                let starts: [[u32; 6]; 64] = core::array::from_fn(|i| start_of(&spec, from.wrapping_add(i as u32)));
                (0..64).filter(|&i| !starts[..i].contains(&starts[i])).count()
            };
            let first = ways(0);
            assert_eq!(first, HAT_STARTS as usize, "pitch={pitch}: every start is another");
            for from in [4, 5_000, 40_000, 700_000, 16_777_216, 4_000_000_000, u32::MAX - 20] {
                assert_eq!(ways(from), first, "pitch={pitch}: 64 hits from hit {from}");
            }
            // And a hit never starts the way the one before it did.
            for hit in [4, 40_000, 4_000_000_000] {
                assert_ne!(start_of(&spec, hit), start_of(&spec, hit + 1), "pitch={pitch}: hit {hit} and the next");
            }
        }
    }

    /// The slow drift of a hat's, a snare's and a clap's lengths took the hit
    /// count as an f32: right to 0.001 at hit 40,000, 0.04 off at hit
    /// 1,000,000, 0.5 off at 16,000,000 and anywhere at all after that. It is
    /// where two exact sines would be at any count.
    #[test]
    fn the_sway_is_as_slow_at_hit_four_thousand_million_as_at_hit_four() {
        let tau = core::f64::consts::TAU;
        for from in [4u32, 40_000, 1_000_000, 16_000_000, 100_000_000, 4_000_000_000, u32::MAX - 64] {
            for hit in from..from + 64 {
                for (a, pa, b, pb) in
                    [(31.0f32, 0.7f32, 12.3f32, 2.1f32), (41.0, 1.3, 14.9, 0.2), (23.0, 2.4, 9.7, 0.9)]
                {
                    let n = hit as f64;
                    let exact =
                        0.6 * (tau * n / a as f64 + pa as f64).sin() + 0.4 * (tau * n / b as f64 + pb as f64).sin();
                    let got = sway(hit, a, pa, b, pb) as f64;
                    assert!((got - exact).abs() < 2e-3, "hit {hit}: {got} for {exact}");
                }
            }
        }
    }

    /// From every place the metal starts, three partials are high and three
    /// low, so the first sample's six steps cancel: no hit opens on a click.
    #[test]
    fn every_start_has_three_partials_high_and_three_low() {
        for pitch in [100.0, 205.0, 733.0, 1000.0] {
            for start in 0..HAT_STARTS {
                let phases = hat_phases(start, pitch);
                assert!(phases.iter().all(|p| (0.0..1.0).contains(p)), "{phases:?}");
                let high = phases.iter().filter(|p| **p < 0.5).count();
                assert_eq!(high, 3, "pitch={pitch}, start {start}: {phases:?}");
            }
        }
        assert_eq!(hat_phases(0, 205.0), HAT_PHASES, "the first place is where a plain hit starts");
    }

    /// The peak of hit `hit` and its mean square over the length it is
    /// written with, at the node, before velocity and the track's level.
    fn measure(spec: &NodeSpec, hit: Option<u32>) -> (f32, f32) {
        let NodeSpec::Hat { decay_ms, .. } = *spec else { unreachable!() };
        let mut hat = match hit {
            Some(hit) => HatState::new_hit(spec, hit),
            None => HatState::new(spec),
        };
        let (mut peak, mut power, mut n) = (0.0f32, 0.0f64, 0usize);
        while !hat.is_idle() {
            let s = hat.next_sample();
            assert!(s.is_finite());
            peak = peak.max(s.abs());
            power += (s * s) as f64;
            n += 1;
        }
        assert!(n > 0);
        (peak, math::sqrt((power / samples(decay_ms) as f64) as f32))
    }

    fn db(ratio: f32) -> f32 {
        20.0 * math::log10(ratio)
    }

    /// The metal went through with one gain whatever the band: at `noise=0`
    /// a band of 1 to 4 kHz over a floor of 200 Hz peaked at 1.3 to 2.4, and
    /// a floor over the band left 26 dB less than `hat()`. Every hat is now at
    /// the level of `hat()`, at any rate the engine is built for, and under
    /// full scale.
    #[test]
    fn a_hat_is_as_loud_whatever_its_band_floor_noise_and_pitch() {
        let (_, level) = measure(&hat(60.0, 8000.0, 6000.0, 0.3, 205.0, 0.0), None);
        assert!(db(level / HAT_LEVEL).abs() < 0.1, "hat() is at {level}");
        let (mut low, mut high, mut top) = (0.0f32, 0.0f32, 0.0f32);
        for decay in [5.0, 60.0, 400.0] {
            for band in [1000.0, 2000.0, 4000.0, 8000.0, 16000.0] {
                for floor in [200.0, 1000.0, 6000.0, 12000.0] {
                    for noise in [0.0, 0.3, 1.0] {
                        for pitch in [100.0, 205.0, 1000.0] {
                            let (peak, level) = measure(&hat(decay, band, floor, noise, pitch, 0.0), None);
                            let off = db(level / HAT_LEVEL);
                            let line = (decay, band, floor, noise, pitch);
                            assert!(off.abs() < 2.0, "{line:?}: {off:+.1} dB from hat()");
                            assert!(peak <= HAT_PEAK + 1e-4, "{line:?}: peaks at {peak}");
                            (low, high, top) = (low.min(off), high.max(off), top.max(peak));
                        }
                    }
                }
            }
        }
        // The level holds much closer than the 2 dB asked of it; the peak is
        // let move, a hat of clicks being more peak for its level than one
        // of noise.
        assert!(low > -0.5 && high < 0.5, "{low:+.2}..{high:+.2} dB");
        assert!(top < 1.0, "{top}");
        // Where the level would take the peak over 0.95 it is the level that
        // gives: the bare metal ringing for seconds, the most peak a hat has
        // for its level, comes out up to 2.7 dB under, and nothing over.
        for (band, floor, noise, pitch) in
            [(8000.0, 200.0, 0.0, 100.0), (16000.0, 200.0, 0.05, 100.0), (1000.0, 12000.0, 0.0, 150.0)]
        {
            let (peak, level) = measure(&hat(5000.0, band, floor, noise, pitch, 0.0), None);
            let off = db(level / HAT_LEVEL);
            assert!(off > -3.0 && off < 0.5, "decay=5s, {band} {floor} {noise} {pitch}: {off:+.1} dB from hat()");
            assert!(peak <= HAT_PEAK + 1e-4, "decay=5s, {band} {floor} {noise} {pitch}: peaks at {peak}");
        }
    }

    /// With `vary` every hit is another: the level the sweep holds is the
    /// mean of the hits, and no hit goes over full scale, whatever place the
    /// metal starts from and however long its decay has swayed.
    #[test]
    fn no_hit_of_a_swaying_hat_goes_over_full_scale() {
        for decay in [5.0, 60.0, 400.0] {
            for (band, floor, noise, pitch) in [
                (8000.0, 6000.0, 0.3, 205.0),
                (9000.0, 7000.0, 0.2, 205.0),
                (16000.0, 200.0, 0.0, 100.0),
                (8000.0, 4000.0, 0.1, 100.0),
                (2000.0, 200.0, 0.0, 1000.0),
                (1000.0, 12000.0, 0.0, 100.0),
                (1500.0, 12000.0, 0.1, 205.0),
                (16000.0, 12000.0, 1.0, 1000.0),
            ] {
                let spec = hat(decay, band, floor, noise, pitch, 0.3);
                let (mut top, mut power) = (0.0f32, 0.0f32);
                let hits = 4 * HAT_STARTS;
                for hit in (0..hits).chain([40_000, 55_434, 71_272, 87_110, u32::MAX]) {
                    let (peak, level) = measure(&spec, Some(hit));
                    top = top.max(peak);
                    if hit < hits {
                        power += level * level / hits as f32;
                    }
                }
                let line = (decay, band, floor, noise, pitch);
                assert!(top <= HAT_PEAK + 1e-4, "{line:?}: a hit peaks at {top}");
                // Its hits are around the level of `hat()`, under it where
                // the peak is what holds them.
                let off = db(math::sqrt(power) / HAT_LEVEL);
                assert!(off < 1.0 && off > -3.0, "{line:?}: {off:+.1} dB from hat()");
            }
        }
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
