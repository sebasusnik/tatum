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
/// are few enough for the opening of every one to be played when the line is
/// compiled.
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

/// How many samples of a hat are played to find its level when its line is
/// compiled, at the most and at any rate: as far as `hat()` runs at 44.1 kHz
/// before it is 30 dB down, and 48 from each of the 29 places a hat that
/// sways starts from. What compiling a hat costs is this and no more,
/// however long it rings.
const HAT_PLAYED: usize = 48 * HAT_STARTS as usize;
/// At how many pitches the filters are asked what noise they let through.
const HAT_NOISE_STEPS: usize = 128;
/// What ln(1000) is: a level falls by this, per its length, to be 60 dB down.
const SIXTY_DB: f64 = 6.907_755_278_982_137;

/// What brings the hat these arguments make to the level `hat()` has. The
/// band, the floor, the pitch and the share of noise each decide how much of
/// the six squares and of the noise comes through, by tens of dB from one end
/// of their ranges to the other, and so does the rate the engine runs at. So
/// its level is found here, once, when the line is compiled: its mean square
/// over its length, and its peak.
///
/// The hit is played, to 30 dB down or for `HAT_PLAYED` samples, whichever is
/// less; with `vary`, at the longest its decay sways to and that many
/// samples shared between the 29 places the metal starts from, the level
/// being their mean. What is left of a hit after that is not played but
/// worked out (`hat_power`): by then the filters have run in, and what comes
/// through is the same power whatever the start. So a closed hat that does
/// not sway is measured whole, sample for sample, and one that rings five
/// seconds costs no more than `hat()` does.
///
/// The level gives way to the peak of what was played, so that it stays
/// under `HAT_PEAK`. A peak that was not played, late in a long hit or from
/// another start further in than its 48 samples, is not known here: the hit
/// itself holds it at `HAT_PEAK` (`HatState::next_sample`). Nothing is
/// allocated.
pub fn hat_gain(decay_ms: f32, band: f32, floor: f32, noise: f32, pitch: f32, vary: f32) -> f32 {
    hat_measure(decay_ms, band, floor, noise, pitch, vary).0
}

/// `hat_gain`, and the work it took: the samples it played plus the pitches
/// it asked the filters about.
fn hat_measure(decay_ms: f32, band: f32, floor: f32, noise: f32, pitch: f32, vary: f32) -> (f32, usize) {
    let decay_ms = decay_ms * (1.0 + vary);
    let spec = NodeSpec::Hat { decay_ms, band, floor, noise, pitch, gain: 1.0, vary: 0.0 };
    let len = samples(decay_ms);
    // As far as 30 dB down: a thousandth of it is left after that.
    let whole = (len * 0.5) as usize + 1;
    let starts = if vary > 0.0 { HAT_STARTS as usize } else { 1 };
    let played = whole.min(HAT_PLAYED / starts);
    let (mut peak, mut power) = (0.0f32, 0.0f64);
    let mut hat = HatState::new(&spec);
    for start in 0..starts {
        // The same hat from the top, from this start: built once.
        hat.phase = hat_phases(start as u32, pitch);
        hat.band.reset();
        hat.floor.reset();
        hat.rng = Rng::new(HAT_SEED);
        (hat.level, hat.done) = (1.0, false);
        for _ in 0..played {
            let s = hat.sound();
            peak = peak.max(s.abs());
            power += (s * s) as f64;
        }
    }
    let (mut power, mut work) = (power / starts as f64, starts * played);
    if played < whole {
        // The rest of it, from where the playing stopped to 30 dB down: the
        // power it has run in, under the square of the level, which falls by
        // `fall` a sample. Summed as the series it is.
        let (steady, asked) = hat_power(&hat);
        let fall = 2.0 * SIXTY_DB / len as f64;
        let at = |n: usize| math::exp(-(fall * n as f64) as f32) as f64;
        // 1 - exp(-fall), which an f32 exponential loses for a long hat.
        let step = if fall < 1.0e-3 { fall - 0.5 * fall * fall } else { 1.0 - at(1) };
        power += steady as f64 * (at(played) - at(whole)) / step;
        work += asked;
    }
    let level = math::sqrt((power / len as f64) as f32);
    if level < 1.0e-9 {
        // Nothing comes through at all: there is nothing to bring up.
        return (1.0, work);
    }
    ((HAT_LEVEL / level).min(HAT_PEAK / peak), work)
}

/// The power of a hat before its level falls, once its filters have run in,
/// and at how many pitches they were asked for it. Nothing is played: the
/// noise is a third (what numbers drawn evenly from -1 to 1 carry) of the
/// mean of what the two filters let through from nothing to half the rate;
/// each square is its odd harmonics, the k-th at 4/(pi k), each through the
/// filters at its own pitch. The one-sample smoothing of the squares' edges
/// rolls a harmonic at the share x of the rate off by sinc^2(x), and one
/// over half the rate comes back down folded, where the filters meet it:
/// they are followed to three quarters of the rate, 21 dB down. Where each
/// partial is in its cycle does not come into it, so this is the level from
/// any start.
fn hat_power(hat: &HatState) -> (f32, usize) {
    use core::f32::consts::PI;
    let through = |cos: f32, sin: f32| hat.band.power_at(cos, sin) * hat.floor.power_at(cos, sin);
    let (mut noise, mut metal, mut asked) = (0.0f32, 0.0f32, 0usize);
    if hat.noise > 0.0 {
        // The angle is turned from one pitch to the next, not looked up.
        let step = PI / HAT_NOISE_STEPS as f32;
        let (turn_cos, turn_sin) = (math::cos(step), math::sin(step));
        let (mut cos, mut sin) = (math::cos(0.5 * step), math::sin(0.5 * step));
        for _ in 0..HAT_NOISE_STEPS {
            noise += through(cos, sin);
            (cos, sin) = (cos * turn_cos - sin * turn_sin, sin * turn_cos + cos * turn_sin);
        }
        noise *= hat.noise * hat.noise / (3.0 * HAT_NOISE_STEPS as f32);
        asked += HAT_NOISE_STEPS;
    }
    if hat.metal > 0.0 {
        for &dt in hat.inc.iter() {
            let angle = math::TWO_PI * dt;
            let (mut cos, mut sin) = (math::cos(angle), math::sin(angle));
            // From one odd harmonic to the next is twice the angle.
            let (turn_cos, turn_sin) = (cos * cos - sin * sin, 2.0 * sin * cos);
            let mut k = 1.0f32;
            while k * dt < 0.75 {
                let x = k * dt;
                // sinc^2(x), from the cosine of 2 pi x that is at hand.
                let roll = (1.0 - cos) / (2.0 * PI * PI * x * x);
                let amp = 4.0 / (PI * k) * roll;
                metal += 0.5 * amp * amp * through(cos, sin);
                (cos, sin) = (cos * turn_cos - sin * turn_sin, sin * turn_cos + cos * turn_sin);
                k += 2.0;
                asked += 1;
            }
        }
        metal *= hat.metal * hat.metal;
    }
    (noise + metal, asked)
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
    ///
    /// The share scales the sway of the decay and nothing else. Another
    /// start is another hit whatever the share: the partials' edges land
    /// against each other by more than a cycle of the band, so there is no
    /// moving them a little. Measured over 116 hits at 44.1 kHz, each hit's
    /// rms over its own length against `hat()`: -1.3..+0.8 dB at `vary=1%`,
    /// -1.4..+1.1 dB at 10%, -2.5..+1.7 dB at 30% (the decay's sway added),
    /// with peaks from 0.30 to 0.71; -2.0..+1.4 dB at 22.05 kHz at 10%; and
    /// -6..+4 dB on a hat of 5 ms in a band at 1 kHz, which is a few cycles
    /// long. The mean of the hits is what is brought to the level.
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

    /// The next sample, held within `HAT_PEAK`. The level a hat is compiled
    /// with keeps every peak that was played for it under that (`hat_gain`);
    /// one that was not, late in a long hit or from another of a swaying
    /// hat's starts, stops here, so no hit of any hat goes over. It is the
    /// top of a peak that stops: in 19,000 hats at each of 44.1, 32 and
    /// 22.05 kHz the highest was 3.9 dB over before the hold (bare metal of
    /// 228 ms that sways), with at the most 5 samples of one hit held, 2 in
    /// a row, and 0.43 dB of one hit's level taken (a hit of 100 ms; 0.1 dB
    /// on those of 0.3 s and up). The tests hold every hat they play to
    /// 4.5 dB, 8 samples, 3 in a row and 0.5 dB.
    #[inline]
    pub fn next_sample(&mut self) -> f32 {
        self.sound().clamp(-HAT_PEAK, HAT_PEAK)
    }

    /// The next sample as the hat makes it, before the hold.
    #[inline]
    fn sound(&mut self) -> f32 {
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

    /// One hit of a hat, as it is before the hold and as it comes out.
    struct Hit {
        /// The highest sample before the hold: what the compiled gain left.
        top: f32,
        /// The root mean square of what comes out, over the length written.
        level: f32,
        /// How many samples the hold touched, and how many the hit has.
        held: usize,
        of: usize,
        /// The most samples held in a row.
        run: usize,
        /// dB of the hit's level the hold took.
        lost: f32,
    }

    /// Hit `hit` at the node, before velocity and the track's level, read
    /// before the hold (`sound`) so that a gain set too high shows as what
    /// it is and not as a hit that sits on the hold. What comes out is that
    /// held within `HAT_PEAK`, which `next_sample` is tested to be below.
    fn play(spec: &NodeSpec, hit: Option<u32>) -> Hit {
        let NodeSpec::Hat { decay_ms, .. } = *spec else { unreachable!() };
        let mut hat = match hit {
            Some(hit) => HatState::new_hit(spec, hit),
            None => HatState::new(spec),
        };
        let (mut top, mut held, mut of, mut run, mut row) = (0.0f32, 0usize, 0usize, 0usize, 0usize);
        let (mut power, mut power_free) = (0.0f64, 0.0f64);
        while !hat.is_idle() {
            let free = hat.sound();
            assert!(free.is_finite());
            let out = free.clamp(-HAT_PEAK, HAT_PEAK);
            row = if out != free { row + 1 } else { 0 };
            held += (out != free) as usize;
            run = run.max(row);
            top = top.max(free.abs());
            power += (out * out) as f64;
            power_free += (free * free) as f64;
            of += 1;
        }
        assert!(of > 0);
        let level = math::sqrt((power / samples(decay_ms) as f64) as f32);
        let lost = if held > 0 { 10.0 * math::log10((power_free / power) as f32) } else { 0.0 };
        Hit { top, level, held, of, run, lost }
    }

    /// How far over `HAT_PEAK` a peak may be before the hold: 4.5 dB. The
    /// most found is 3.9 dB, in 19,000 hats at each of three rates (the hat is
    /// in `the_hold_takes_the_top_of_a_few_peaks_of_any_hat`).
    const OVER_MOST: f32 = 1.68;

    /// The hold is the top of a few peaks: before it no peak is 4.5 dB over
    /// `HAT_PEAK`, and it touches at the most eight samples of a hit, three
    /// in a row, one in a thousand, and half a dB of that hit's level. A
    /// gain that read a hat's level low would leave it sitting on the hold,
    /// and fail every one of these.
    fn held_little(h: &Hit, line: &dyn core::fmt::Debug) {
        assert!(h.top < OVER_MOST * HAT_PEAK, "{line:?}: {} before the hold", h.top);
        assert!(h.held <= 8 && h.run <= 3, "{line:?}: {} samples held, {} in a row", h.held, h.run);
        assert!(h.held * 1000 < h.of, "{line:?}: {} samples of {} held", h.held, h.of);
        assert!(h.lost < 0.5, "{line:?}: the hold takes {} dB", h.lost);
    }

    /// Whether a hat that does not sway is played whole when its line is
    /// compiled: then its gain alone keeps every peak under `HAT_PEAK`.
    fn played_whole(decay_ms: f32) -> bool {
        ((samples(decay_ms) * 0.5) as usize) < HAT_PLAYED
    }

    /// A number from `low` to `high`, evenly by ratio (by difference from
    /// zero), and one of the two ends four times in ten.
    fn drawn(rng: &mut Rng, low: f32, high: f32) -> f32 {
        let end = rng.next_bipolar();
        let at = 0.5 * (rng.next_bipolar() + 1.0);
        if end < -0.6 {
            low
        } else if end > 0.6 {
            high
        } else if low > 0.0 {
            low * math::exp(at * math::ln(high / low))
        } else {
            low + at * (high - low)
        }
    }

    /// The level (root mean square over the length written) and the peak of
    /// the first `most` samples of a hit, before the hold.
    fn hat_level_and_peak(spec: &NodeSpec, most: usize) -> (f32, f32) {
        let NodeSpec::Hat { decay_ms, .. } = *spec else { unreachable!() };
        let mut hat = HatState::new(spec);
        let (mut peak, mut power, mut n) = (0.0f32, 0.0f64, 0usize);
        while !hat.is_idle() && n < most {
            let s = hat.sound();
            peak = peak.max(s.abs());
            power += (s * s) as f64;
            n += 1;
        }
        (math::sqrt((power / samples(decay_ms) as f64) as f32), peak)
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
        let level = play(&hat(60.0, 8000.0, 6000.0, 0.3, 205.0, 0.0), None).level;
        assert!(db(level / HAT_LEVEL).abs() < 0.1, "hat() is at {level}");
        let (mut low, mut high) = (0.0f32, 0.0f32);
        let (mut long_low, mut long_high) = (0.0f32, 0.0f32);
        for decay in [5.0, 60.0, 400.0, 2000.0] {
            for band in [1000.0, 2000.0, 4000.0, 8000.0, 16000.0] {
                for floor in [200.0, 1000.0, 6000.0, 12000.0] {
                    for noise in [0.0, 0.3, 1.0] {
                        for pitch in [100.0, 205.0, 1000.0] {
                            let h = play(&hat(decay, band, floor, noise, pitch, 0.0), None);
                            let off = db(h.level / HAT_LEVEL);
                            let line = (decay, band, floor, noise, pitch);
                            assert!(off.abs() < 2.0, "{line:?}: {off:+.1} dB from hat()");
                            // The peak before the hold: under `HAT_PEAK` by
                            // the gain alone where the whole hit was played
                            // for it, and held only by its top where not.
                            if played_whole(decay) {
                                assert!(h.top <= HAT_PEAK + 1e-4, "{line:?}: peaks at {}", h.top);
                            }
                            held_little(&h, &line);
                            if decay <= 400.0 {
                                (low, high) = (low.min(off), high.max(off));
                            } else {
                                (long_low, long_high) = (long_low.min(off), long_high.max(off));
                            }
                        }
                    }
                }
            }
        }
        // The level holds much closer than the 2 dB asked of it; the peak is
        // let move, a hat of clicks being more peak for its level than one
        // of noise.
        assert!(low > -0.5 && high < 0.5, "{low:+.2}..{high:+.2} dB");
        // One that rings two seconds, most of it worked out and not played,
        // holds as close where its peak does not hold it lower.
        assert!(long_low > -1.0 && long_high < 0.5, "decay=2s: {long_low:+.2}..{long_high:+.2} dB");
        // The bare metal ringing for seconds is the most peak a hat has for
        // its level. Its level gives way to the peaks played when its line
        // was compiled and no further (it came out up to 2.7 dB under when
        // the whole of it was played); a later peak is held, and takes
        // nothing a meter of its level shows.
        for (band, floor, noise, pitch) in
            [(8000.0, 200.0, 0.0, 100.0), (16000.0, 200.0, 0.05, 100.0), (1000.0, 12000.0, 0.0, 150.0)]
        {
            let h = play(&hat(5000.0, band, floor, noise, pitch, 0.0), None);
            let off = db(h.level / HAT_LEVEL);
            assert!(off > -1.0 && off < 0.5, "decay=5s, {band} {floor} {noise} {pitch}: {off:+.1} dB from hat()");
            held_little(&h, &(5000.0, band, floor, noise, pitch));
        }
    }

    /// The gain as it was first found: the whole hit played to 30 dB down,
    /// however long, which is what made compiling a long hat slow.
    fn gain_played_whole(decay_ms: f32, band: f32, floor: f32, noise: f32, pitch: f32) -> f32 {
        let spec = NodeSpec::Hat { decay_ms, band, floor, noise, pitch, gain: 1.0, vary: 0.0 };
        let len = samples(decay_ms);
        let mut hat = HatState::new(&spec);
        let (mut peak, mut power) = (0.0f32, 0.0f64);
        for _ in 0..(len * 0.5) as usize + 1 {
            let s = hat.sound();
            peak = peak.max(s.abs());
            power += (s * s) as f64;
        }
        (HAT_LEVEL / math::sqrt((power / len as f64) as f32)).min(HAT_PEAK / peak)
    }

    /// Compiling `hat(decay=5000ms, vary=30%)` played 29 hits of it, 4 million
    /// samples and 49 ms, on every compile of its song; an open hat that
    /// sways took 3 ms, thirty times the rest of the song. The work is now
    /// bounded whatever the arguments: the samples played and the pitches the
    /// filters are asked about are counted here, 1324 for `hat()` at 44.1 kHz
    /// and never over 2059 there. The bound is worked out for the rate the
    /// engine is built at: `HAT_PLAYED` samples, the pitches the noise is
    /// asked at, and the odd harmonics of the six partials, at the lowest
    /// pitch a hat takes, up to three quarters of the rate.
    #[test]
    fn compiling_a_hat_costs_no_more_however_long_it_rings() {
        let harmonics: usize =
            HAT_RATIOS.iter().map(|ratio| (0.375 * SAMPLE_RATE / (100.0 * ratio)) as usize + 1).sum();
        let bound = HAT_PLAYED + HAT_NOISE_STEPS + harmonics;
        let mut most = 0;
        for decay in [5.0, 45.0, 60.0, 61.0, 320.0, 1000.0, 5000.0] {
            for vary in [0.0, 0.001, 0.1, 0.3] {
                for band in [1000.0, 8000.0, 16000.0] {
                    for floor in [200.0, 6000.0, 12000.0] {
                        for noise in [0.0, 0.3, 1.0] {
                            for pitch in [100.0, 205.0, 1000.0] {
                                let (gain, work) = hat_measure(decay, band, floor, noise, pitch, vary);
                                let line = (decay, band, floor, noise, pitch, vary);
                                assert!(gain.is_finite() && gain > 0.0, "{line:?}: gain {gain}");
                                assert!(work <= bound, "{line:?}: {work} samples and pitches, over {bound}");
                                most = most.max(work);
                            }
                        }
                    }
                }
            }
        }
        // And the bound is what the work is, to a harmonic a partial.
        assert!(most + 6 >= bound, "{most} of {bound}");
        if SAMPLE_RATE == 44100.0 {
            assert_eq!(most, 2059);
        }
        // `hat()` is played whole, and nothing of it worked out, at any rate
        // where it is 30 dB down within what is played: up to 46.4 kHz.
        let (_, plain) = hat_measure(60.0, 8000.0, 6000.0, 0.3, 205.0, 0.0);
        if played_whole(60.0) {
            assert!(plain <= HAT_PLAYED, "hat() is played whole within the bound: {plain}");
        }
    }

    /// A hat that is 30 dB down within what is played, and does not sway, is
    /// measured as it always was, to the bit: `hat()`, `tick` and anything
    /// as short are where they were.
    #[test]
    fn a_closed_hat_is_measured_whole() {
        for (decay, band, floor, noise, pitch) in [
            (60.0, 8000.0, 6000.0, 0.3, 205.0),
            (45.0, 9000.0, 7000.0, 0.2, 205.0),
            (5.0, 1000.0, 200.0, 0.0, 1000.0),
            (30.0, 2000.0, 200.0, 0.0, 1000.0),
        ] {
            let whole = gain_played_whole(decay, band, floor, noise, pitch);
            let now = hat_gain(decay, band, floor, noise, pitch, 0.0);
            assert_eq!(now.to_bits(), whole.to_bits(), "{decay} {band} {floor} {noise} {pitch}: {now} for {whole}");
        }
    }

    /// The part of a long hat that is not played is worked out. It has to
    /// come to what playing it gives: the gain of a hat ten times too long
    /// to be played whole is within 0.5 dB of the one found by playing all
    /// of it, where the peak is not what holds either.
    #[test]
    fn a_long_hat_worked_out_is_as_loud_as_one_played_whole() {
        let (mut low, mut high) = (0.0f32, 0.0f32);
        for decay in [70.0, 150.0, 700.0, 5000.0] {
            for band in [1000.0, 3000.0, 8000.0, 16000.0] {
                for floor in [200.0, 2000.0, 12000.0] {
                    for noise in [0.0, 0.3, 1.0] {
                        for pitch in [100.0, 205.0, 380.0, 1000.0] {
                            let spec =
                                NodeSpec::Hat { decay_ms: decay, band, floor, noise, pitch, gain: 1.0, vary: 0.0 };
                            let (level, _) = hat_level_and_peak(&spec, usize::MAX);
                            let (_, played) = hat_level_and_peak(&spec, HAT_PLAYED);
                            let now = hat_gain(decay, band, floor, noise, pitch, 0.0);
                            if now * played > 0.999 * HAT_PEAK {
                                // Its level gave way to a peak that was played.
                                continue;
                            }
                            let off = db(now * level / HAT_LEVEL);
                            let line = (decay, band, floor, noise, pitch);
                            assert!(off.abs() < 0.5, "{line:?}: {off:+.2} dB from hat()");
                            (low, high) = (low.min(off), high.max(off));
                        }
                    }
                }
            }
        }
        assert!(low < 0.0 && high > 0.0, "{low:+.2}..{high:+.2} dB");
    }

    /// What `hat_power` works out is what the filters do: the hat played
    /// with its level undone, once it has run in, carries that power, for
    /// the metal alone, the noise alone and both, in any band.
    #[test]
    fn the_power_worked_out_is_the_power_played() {
        for (band, floor, noise, pitch) in [
            (8000.0, 6000.0, 0.3, 205.0),
            (1000.0, 200.0, 0.0, 100.0),
            (16000.0, 12000.0, 0.0, 1000.0),
            (2000.0, 200.0, 1.0, 205.0),
            (16000.0, 200.0, 1.0, 205.0),
            (4000.0, 9000.0, 0.5, 333.0),
        ] {
            let spec = NodeSpec::Hat { decay_ms: 5000.0, band, floor, noise, pitch, gain: 1.0, vary: 0.0 };
            let (worked, _) = hat_power(&HatState::new(&spec));
            let mut hat = HatState::new(&spec);
            let (mut power, n) = (0.0f64, SAMPLE_RATE as usize);
            for i in 0..n + 2000 {
                let level = hat.level;
                let s = hat.sound() / level;
                if i >= 2000 {
                    power += (s * s) as f64 / n as f64;
                }
            }
            let off = 10.0 * math::log10(worked / power as f32);
            assert!(off.abs() < 0.4, "{band} {floor} {noise} {pitch}: worked out {off:+.2} dB from played");
        }
    }

    /// The level of a hat that sways or rings long is set by the peaks played
    /// for it; a higher one further in is held at `HAT_PEAK` by the hit. It
    /// is the top of a few peaks that goes. These are hats found to need it
    /// most, bare metal that rings, the last three the highest before the
    /// hold in what was measured (1.27, 1.46 and 1.49 at 44.1 kHz, the last
    /// 3.9 dB over): what the node gives is the hit held within `HAT_PEAK`,
    /// sample for sample and nothing else, and how little that is, is
    /// `held_little`.
    #[test]
    fn a_peak_that_was_not_played_is_held_and_little_else() {
        let mut over = 0.0f32;
        for (decay, band, floor, noise, pitch, vary) in [
            (4959.0, 6829.0, 254.0, 0.02, 100.0, 0.0),
            (5000.0, 8000.0, 200.0, 0.0, 100.0, 0.0),
            (5000.0, 16000.0, 200.0, 0.0, 100.0, 0.3),
            (5000.0, 7355.0, 12000.0, 0.0, 124.0, 0.3),
            (636.8, 1332.0, 12000.0, 0.0, 199.0, 0.3),
            (810.1, 1000.0, 12000.0, 0.0, 102.0, 0.001),
            (2000.0, 1000.0, 12000.0, 0.0, 150.0, 0.1),
            (5000.0, 16000.0, 200.0, 0.02, 157.5, 0.1575),
            (3486.3213, 1054.6442, 12000.0, 0.05, 111.364_494, 0.002_112_755_6),
            (227.979_03, 16000.0, 200.0, 0.029_376_943, 108.810_03, 0.001_652_524_6),
        ] {
            let spec = hat(decay, band, floor, noise, pitch, vary);
            let line = (decay, band, floor, noise, pitch, vary);
            for hit in 0..2 * HAT_STARTS {
                let hit = (vary > 0.0).then_some(hit);
                let build = || match hit {
                    Some(hit) => HatState::new_hit(&spec, hit),
                    None => HatState::new(&spec),
                };
                let (mut held, mut free) = (build(), build());
                while !held.is_idle() {
                    let (s, f) = (held.next_sample(), free.sound());
                    assert!(s.abs() <= HAT_PEAK, "{line:?}, hit {hit:?}: a sample at {s}");
                    assert_eq!(s, f.clamp(-HAT_PEAK, HAT_PEAK), "{line:?}, hit {hit:?}: the hold and nothing else");
                }
                assert!(free.is_idle());
                let h = play(&spec, hit);
                held_little(&h, &(line, hit));
                over = over.max(h.top);
                if vary == 0.0 {
                    break;
                }
            }
        }
        // At 44.1 kHz these go further over than the 3 dB this was once
        // taken to be, so the hold at its hardest is what this tests; a
        // slower engine has less metal up there and needs it less.
        if SAMPLE_RATE == 44100.0 {
            assert!(over > 1.42 * HAT_PEAK, "none of these needed the hold as much: {over}");
        }
    }

    /// What a sweep of hats found, over all of them.
    #[derive(Default)]
    struct Found {
        /// The highest peak before the hold.
        top: f32,
        /// The most samples held in one hit, in a row, and as a share of it.
        held: usize,
        run: usize,
        share: f32,
        /// The most dB the hold took of one hit's level.
        lost: f32,
        /// The most hits of one hat that reached the hold, and how many
        /// hats did.
        hits: usize,
        hats: usize,
        /// dB from `hat()`, lowest and highest: hats that do not sway and
        /// whose level no peak held, those a played peak did hold, and the
        /// mean of the hits of those that sway.
        free: (f32, f32),
        gave: f32,
        swaying: (f32, f32),
    }

    impl Found {
        /// One hat, compiled and played: every hit within `held_little`, and
        /// its level where the docs say it is.
        fn hat(&mut self, line: (f32, f32, f32, f32, f32, f32), hits: u32) {
            let (decay, band, floor, noise, pitch, vary) = line;
            let spec = hat(decay, band, floor, noise, pitch, vary);
            let hits = if vary > 0.0 { hits } else { 1 };
            let (mut power, mut reached) = (0.0f32, 0usize);
            for hit in 0..hits {
                let h = play(&spec, (vary > 0.0).then_some(hit));
                held_little(&h, &(line, hit));
                if vary == 0.0 && played_whole(decay) {
                    assert!(h.top <= HAT_PEAK + 1e-4, "{line:?}: peaks at {}", h.top);
                }
                self.top = self.top.max(h.top);
                (self.held, self.run, self.lost) = (self.held.max(h.held), self.run.max(h.run), self.lost.max(h.lost));
                self.share = self.share.max(h.held as f32 / h.of as f32);
                reached += (h.held > 0) as usize;
                power += h.level * h.level / hits as f32;
            }
            self.hits = self.hits.max(reached);
            self.hats += (reached > 0) as usize;
            let off = db(math::sqrt(power) / HAT_LEVEL);
            if vary > 0.0 {
                // The mean of its hits, around the level of `hat()`: under
                // it where a peak that was played holds it lower.
                assert!(off > -3.5 && off < 1.4, "{line:?}: {off:+.2} dB from hat()");
                self.swaying = (self.swaying.0.min(off), self.swaying.1.max(off));
                return;
            }
            let unit = NodeSpec::Hat { decay_ms: decay, band, floor, noise, pitch, gain: 1.0, vary: 0.0 };
            let (_, played) = hat_level_and_peak(&unit, HAT_PLAYED);
            let NodeSpec::Hat { gain, .. } = spec else { unreachable!() };
            if gain * played > 0.999 * HAT_PEAK {
                // Its level gave way to a peak that was played.
                assert!(off > -1.5 && off < 0.4, "{line:?}: {off:+.2} dB from hat()");
                self.gave = self.gave.min(off);
            } else {
                assert!(off.abs() < 0.4, "{line:?}: {off:+.2} dB from hat()");
                self.free = (self.free.0.min(off), self.free.1.max(off));
            }
        }
    }

    /// How hard the hold works on any hat, and how well the level holds, at
    /// whatever rate the engine is built for: 380 hats drawn the same way on
    /// every run, the ends of every range four times in ten. 160 from the
    /// whole of every range, 120 of bare metal ringing 0.3 to 5 s, 100 of
    /// noise in a low band (where the level of a hat that sways is furthest
    /// over); a share of each sways, and those are played from all 29
    /// starts. Then the hats found furthest out in 19,000 drawn like these
    /// and aimed at each corner, at each of 44.1, 32 and 22.05 kHz: the highest
    /// peaks before the hold (1.27, 1.46 and 1.49, the last 3.9 dB over
    /// `HAT_PEAK`), the most of one hit's level the hold took (0.43 dB, of
    /// a hit of 100 ms), the largest share of a hit held (one sample of a
    /// hit of 32 ms, 7 in 10,000), and the levels furthest from `hat()`.
    ///
    /// What these numbers are at 44.1 kHz is asserted at the end, so that
    /// the docs that quote them fail here when they stop being true.
    #[test]
    fn the_hold_takes_the_top_of_a_few_peaks_of_any_hat() {
        let mut rng = Rng::new(77);
        let mut found = Found::default();
        for (kind, count, sways) in [(0, 160, 4), (1, 120, 3), (2, 100, 2)] {
            for i in 0..count {
                let (decay, band, noise) = match kind {
                    0 => (drawn(&mut rng, 5.0, 5000.0), drawn(&mut rng, 1000.0, 16000.0), drawn(&mut rng, 0.0, 1.0)),
                    1 => (drawn(&mut rng, 300.0, 5000.0), drawn(&mut rng, 1000.0, 16000.0), drawn(&mut rng, 0.0, 0.05)),
                    _ => (drawn(&mut rng, 8.0, 250.0), drawn(&mut rng, 1000.0, 3000.0), drawn(&mut rng, 0.6, 1.0)),
                };
                let (floor, pitch) = (drawn(&mut rng, 200.0, 12000.0), drawn(&mut rng, 100.0, 1000.0));
                let vary = drawn(&mut rng, 0.001, 0.3);
                let vary = if i % sways == 0 { vary } else { 0.0 };
                found.hat((decay, band, floor, noise, pitch, vary), HAT_STARTS);
            }
        }
        let drawn_hats = found.hats;
        for line in [
            (5000.0, 16000.0, 200.0, 0.02, 157.5, 0.1575),
            (3486.3213, 1054.6442, 12000.0, 0.05, 111.364_494, 0.002_112_755_6),
            (227.979_03, 16000.0, 200.0, 0.029_376_943, 108.810_03, 0.001_652_524_6),
            (100.0, 1000.0, 12000.0, 0.068_740_22, 111.116_516, 0.3),
            (31.978_407, 7154.8076, 7361.8354, 0.031_408_64, 111.159_58, 0.3),
            (647.063_84, 16000.0, 200.0, 0.068_675_51, 113.029_526, 0.162_032_78),
            (3747.0, 16000.0, 371.0, 0.02, 100.0, 0.0),
            (5000.0, 16000.0, 203.861_79, 0.009_069_609, 100.0, 0.0),
            (12.5, 1241.0, 1034.0, 0.714, 192.5, 0.001),
            (52.817_528, 1000.0, 917.522_46, 0.999_698_8, 119.459_06, 0.267_285_5),
        ] {
            found.hat(line, 2 * HAT_STARTS);
        }
        if SAMPLE_RATE == 44100.0 {
            let over = db(found.top / HAT_PEAK);
            assert!(over > 3.8 && over < 4.0, "the highest peak is {over:+.2} dB over before the hold");
            assert!(found.held == 2 && found.run == 2, "{} samples of a hit held, {} in a row", found.held, found.run);
            assert!(found.lost > 0.4, "the hold takes {} dB at the most", found.lost);
            assert!(found.gave < -0.9, "a played peak holds the level {:+.2} dB under", found.gave);
            assert!(found.swaying.0 < -2.9, "and {:+.2} dB under on one that sways", found.swaying.0);
            assert_eq!(drawn_hats, 18, "of the 380 drawn, the hats that reach the hold");
        }
    }

    /// With `vary` every hit is another: the level the sweep holds is the
    /// mean of the hits, and no hit goes over full scale, whatever place the
    /// metal starts from and however long its decay has swayed: before the
    /// hold a peak is no further over than `held_little` lets it be. The
    /// last three colours are noise in a low band, where the mean of the
    /// hits is furthest over the level: every start plays the same noise
    /// when the line is compiled, and the rest is worked out for noise at
    /// large. These are played at the least share too.
    #[test]
    fn no_hit_of_a_swaying_hat_goes_over_full_scale() {
        for decay in [5.0, 12.5, 60.0, 400.0, 2000.0] {
            for (i, (band, floor, noise, pitch)) in [
                (8000.0, 6000.0, 0.3, 205.0),
                (9000.0, 7000.0, 0.2, 205.0),
                (16000.0, 200.0, 0.0, 100.0),
                (8000.0, 4000.0, 0.1, 100.0),
                (2000.0, 200.0, 0.0, 1000.0),
                (1000.0, 12000.0, 0.0, 100.0),
                (1500.0, 12000.0, 0.1, 205.0),
                (16000.0, 12000.0, 1.0, 1000.0),
                (1241.0, 1034.0, 0.714, 192.5),
                (1000.0, 917.5, 1.0, 119.5),
                (1508.0, 1723.0, 1.0, 856.0),
            ]
            .into_iter()
            .enumerate()
            {
                for vary in [0.3, 0.001] {
                    if (vary < 0.3 && i < 8) || (decay == 12.5 && i < 8) || (decay == 2000.0 && i >= 8) {
                        continue;
                    }
                    let spec = hat(decay, band, floor, noise, pitch, vary);
                    let line = (decay, band, floor, noise, pitch, vary);
                    let mut power = 0.0f32;
                    let hits = 4 * HAT_STARTS;
                    for hit in (0..hits).chain([40_000, 55_434, 71_272, 87_110, u32::MAX]) {
                        let h = play(&spec, Some(hit));
                        held_little(&h, &(line, hit));
                        if hit < hits {
                            power += h.level * h.level / hits as f32;
                        }
                    }
                    // Its hits are around the level of `hat()`, under it
                    // where the peak is what holds them.
                    let off = db(math::sqrt(power) / HAT_LEVEL);
                    assert!(off < 1.4 && off > -3.0, "{line:?}: {off:+.1} dB from hat()");
                }
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
