extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;
use crate::math;
use crate::rng::Rng;

/// How far `wow = 1` swings the delay time either way: 3.5 ms. At the wow's
/// ~0.55 Hz, sped up by a fifth at the top of its wander, that bends the
/// repeats by up to about ±1.5%, some 25 cents, at the steepest point.
pub const WOW_SECONDS: f32 = 0.0035;
/// How far `flutter = 1` swings it: 0.15 ms, a twentieth of the wow's swing
/// at thirteen times its rate. With its faster partials and its noise that
/// bends the repeats by up to about ±1% (some 18 cents), fast enough to read
/// as a tremble on the note rather than a drift.
pub const FLUTTER_SECONDS: f32 = 0.00015;
/// The wow's rate, left and right. Each wanders ±20% around it over a
/// twelve-second cycle or so, so the drift never settles into a vibrato.
pub const WOW_HZ: [f32; 2] = [0.53, 0.61];
/// The flutter's main rate, left and right. Two more partials at 1.47 and
/// 2.31 times it, and a little smoothed noise, make it irregular.
pub const FLUTTER_HZ: [f32; 2] = [6.7, 7.4];
const WOW_DRIFT_HZ: [f32; 2] = [0.071, 0.083];
const WOW_DRIFT_AMOUNT: f32 = 0.2;
/// The flutter's partials: rate against `FLUTTER_HZ`, and level.
const FLUTTER_PARTIALS: [(f32, f32); 3] = [(1.0, 0.6), (1.47, 0.28), (2.31, 0.12)];
/// Corner of the two one-pole lowpasses that smooth the flutter's noise.
const FLUTTER_NOISE_HZ: f32 = 9.0;
/// How much of the flutter the noise is, against the partials.
const FLUTTER_NOISE_AMOUNT: f32 = 0.25;
const NOISE_SEEDS: [u32; 2] = [0x5EED_D1A7, 0x0B0E_0B0E];
/// The greatest swing, in seconds, the line makes room for past its longest
/// time: the wow and the flutter at their furthest, with a margin for the
/// flutter's noise.
const MOD_HEADROOM_SECONDS: f32 = WOW_SECONDS + FLUTTER_SECONDS * 3.0 + 0.0005;
/// The shortest delay a modulated read may reach, in samples: the cubic read
/// takes a tap one sample newer than its own, and the newest sample in the
/// line is the one written a step ago. A time shorter than the swing (under
/// some 3.7 ms at full wow) is held here for the part of each cycle that
/// would take it lower: the bend rests there, without a click.
const MIN_MOD_DELAY: usize = 2;
const TWO_PI: f32 = core::f32::consts::TAU;

#[derive(Clone, Copy, PartialEq)]
pub enum DelaySync {
    Free,          // manual time in seconds
    Quarter,       // 1/4 note
    DottedEighth,  // 3/16 note
    Eighth,        // 1/8 note
    Sixteenth,     // 1/16 note
    TripletEighth, // 1/8 triplet
}

impl DelaySync {
    /// Convert a normalized 0.0..1.0 value to a sync mode (for param locks).
    pub fn from_normalized(v: f32) -> Self {
        if v < 0.17 {
            DelaySync::Free
        } else if v < 0.33 {
            DelaySync::Quarter
        } else if v < 0.50 {
            DelaySync::DottedEighth
        } else if v < 0.67 {
            DelaySync::Eighth
        } else if v < 0.83 {
            DelaySync::Sixteenth
        } else {
            DelaySync::TripletEighth
        }
    }

    fn beat_multiplier(self) -> Option<f32> {
        match self {
            DelaySync::Free => None,
            DelaySync::Quarter => Some(1.0),
            DelaySync::DottedEighth => Some(0.75),
            DelaySync::Eighth => Some(0.5),
            DelaySync::Sixteenth => Some(0.25),
            DelaySync::TripletEighth => Some(1.0 / 3.0),
        }
    }
}

/// What moves one channel's read point: its wow and its flutter.
///
/// The wow is a slow sine whose own rate wanders; the flutter is three sines
/// at rates that never line up, plus a little lowpassed noise, so it trembles
/// without repeating. All of it is seeded: a render is the same every time.
struct Drift {
    wow_phase: f32,
    wow_drift_phase: f32,
    flutter_phase: [f32; 3],
    noise: Rng,
    noise_lp: [f32; 2],
    channel: usize,
}

impl Drift {
    fn new(channel: usize) -> Self {
        let right = channel == 1;
        Self {
            // The right channel starts elsewhere in every cycle, so the two
            // sides do not begin by bending together.
            wow_phase: if right { TWO_PI * 0.25 } else { 0.0 },
            wow_drift_phase: if right { TWO_PI * 0.5 } else { 0.0 },
            flutter_phase: if right { [1.6, 0.4, 3.9] } else { [0.0, 1.1, 2.3] },
            noise: Rng::new(NOISE_SEEDS[channel]),
            noise_lp: [0.0; 2],
            channel,
        }
    }

    #[inline]
    fn advance(phase: &mut f32, inc: f32) {
        *phase += inc;
        if *phase >= TWO_PI {
            *phase -= TWO_PI;
        }
    }

    /// One step: the wow and the flutter, each about -1..1 (the flutter's
    /// noise can, rarely, take it a little past).
    fn next(&mut self, sample_rate: f32, flutter_on: bool) -> (f32, f32) {
        let c = self.channel;
        let rate = 1.0 + WOW_DRIFT_AMOUNT * math::sin(self.wow_drift_phase);
        Self::advance(&mut self.wow_drift_phase, TWO_PI * WOW_DRIFT_HZ[c] / sample_rate);
        let wow = math::sin(self.wow_phase);
        Self::advance(&mut self.wow_phase, TWO_PI * WOW_HZ[c] * rate / sample_rate);
        if !flutter_on {
            return (wow, 0.0);
        }

        let mut partials = 0.0;
        for (phase, (ratio, level)) in self.flutter_phase.iter_mut().zip(FLUTTER_PARTIALS) {
            partials += level * math::sin(*phase);
            Self::advance(phase, TWO_PI * FLUTTER_HZ[c] * ratio / sample_rate);
        }
        // White noise through two one-poles: smooth enough that its slope,
        // which is what the ear hears as pitch, carries no hiss. Its spread
        // after them is about sqrt(a / 12) for a uniform -1..1 input, which
        // brings it back to unit spread at any sample rate.
        let a = (TWO_PI * FLUTTER_NOISE_HZ / sample_rate).min(1.0);
        let white = self.noise.next_bipolar();
        self.noise_lp[0] += a * (white - self.noise_lp[0]);
        self.noise_lp[1] += a * (self.noise_lp[0] - self.noise_lp[1]);
        let noise = self.noise_lp[1] / math::sqrt(a / 12.0);
        (wow, partials * (1.0 - FLUTTER_NOISE_AMOUNT) + noise * FLUTTER_NOISE_AMOUNT)
    }
}

pub struct Delay {
    buffer_l: Vec<f32>,
    buffer_r: Vec<f32>,
    write_pos: usize,
    delay_samples: usize,
    /// The longest time `set_time` gives, in samples: what the line held
    /// before it had room for the wow and flutter to swing into, so a time is
    /// clamped where it always was.
    max_delay_samples: usize,
    sample_rate: f32,
    feedback: f32,
    mix: f32,
    // Feedback filter (one-pole lowpass)
    fb_filter_l: f32,
    fb_filter_r: f32,
    fb_filter_coeff: f32, // 0.0 = no filtering, up to 0.95 = very dark
    // Tempo sync
    sync: DelaySync,
    bpm: f32,
    /// Wow and flutter depth, 0..1 each. With both at 0 the read is the
    /// whole-sample one it always was, sample for sample.
    wow: f32,
    flutter: f32,
    drift: [Drift; 2],
}

impl Delay {
    pub fn new(sample_rate: f32, max_time: f32) -> Self {
        let max_samples = (sample_rate * max_time) as usize + 1;
        // Room past the longest time for the wow and flutter to swing into,
        // and for the cubic read's taps either side of it.
        let headroom = (sample_rate * MOD_HEADROOM_SECONDS) as usize + 4;
        Self {
            buffer_l: vec![0.0; max_samples + headroom],
            buffer_r: vec![0.0; max_samples + headroom],
            write_pos: 0,
            delay_samples: ((sample_rate * 0.3) as usize).min(max_samples - 1), // default 300ms
            max_delay_samples: max_samples - 1,
            sample_rate,
            feedback: 0.35,
            mix: 0.2,
            fb_filter_l: 0.0,
            fb_filter_r: 0.0,
            fb_filter_coeff: 0.0,
            sync: DelaySync::Free,
            bpm: 120.0,
            wow: 0.0,
            flutter: 0.0,
            drift: [Drift::new(0), Drift::new(1)],
        }
    }

    pub fn set_time(&mut self, time: f32, sample_rate: f32) {
        let samples = (sample_rate * time) as usize;
        self.delay_samples = samples.min(self.max_delay_samples);
    }

    pub fn set_feedback(&mut self, feedback: f32) {
        self.feedback = math::clamp(feedback, 0.0, 0.95);
    }

    pub fn set_mix(&mut self, mix: f32) {
        self.mix = math::clamp(mix, 0.0, 1.0);
    }

    pub fn set_filter(&mut self, amount: f32) {
        self.fb_filter_coeff = math::clamp(amount, 0.0, 0.95);
    }

    /// Wow, 0..1: a slow (~0.5 Hz), wandering drift of the delay time, the
    /// repeats sagging and rising in pitch like a tired bucket brigade. At 1
    /// the time swings ±3.5 ms, up to about ±25 cents.
    pub fn set_wow(&mut self, amount: f32) {
        self.wow = math::clamp(amount, 0.0, 1.0);
    }

    /// Flutter, 0..1: a fast (~7 Hz), smaller, irregular tremble of the delay
    /// time. At 1 it swings about ±0.15 ms, up to about ±18 cents.
    pub fn set_flutter(&mut self, amount: f32) {
        self.flutter = math::clamp(amount, 0.0, 1.0);
    }

    pub fn set_sync(&mut self, sync: DelaySync, bpm: f32, sample_rate: f32) {
        self.sync = sync;
        self.bpm = bpm;
        if let Some(mult) = sync.beat_multiplier() {
            let time = 60.0 / bpm * mult;
            self.set_time(time, sample_rate);
        }
    }

    pub fn set_bpm(&mut self, bpm: f32, sample_rate: f32) {
        self.bpm = bpm;
        if let Some(mult) = self.sync.beat_multiplier() {
            let time = 60.0 / bpm * mult;
            self.set_time(time, sample_rate);
        }
    }

    /// The sample `offset` steps behind the write head (which is written
    /// after the read, so offset 1 is the newest sample in the line).
    #[inline]
    fn tap(buf: &[f32], write_pos: usize, offset: usize) -> f32 {
        if write_pos >= offset {
            buf[write_pos - offset]
        } else {
            buf[buf.len() + write_pos - offset]
        }
    }

    /// Cubic Hermite (Catmull-Rom) read at a fractional delay, the points
    /// running towards the older sample as the delay grows, as the chorus
    /// reads. The delay is `d` whole samples and `t` (0..=1) of one more, `d`
    /// within `MIN_MOD_DELAY..=len - 3`.
    #[inline]
    fn read_cubic(buf: &[f32], write_pos: usize, (d, t): (usize, f32)) -> f32 {
        let s0 = Self::tap(buf, write_pos, d - 1);
        let s1 = Self::tap(buf, write_pos, d);
        let s2 = Self::tap(buf, write_pos, d + 1);
        let s3 = Self::tap(buf, write_pos, d + 2);
        let t2 = t * t;
        let t3 = t2 * t;
        s1 + 0.5 * t * (s2 - s0)
            + t2 * (s0 - 2.5 * s1 + 2.0 * s2 - 0.5 * s3)
            + t3 * (-0.5 * s0 + 1.5 * s1 - 1.5 * s2 + 0.5 * s3)
    }

    /// Both channels' delays this step, in samples: the set time moved by the
    /// wow and flutter, kept where the cubic read can reach. Each is whole
    /// samples and the fraction of one more, kept apart: the swing is a few
    /// hundred samples at most, and added to a long time as one f32 it would
    /// move in steps of 1/128 of a sample rather than glide.
    #[inline]
    fn modulated_delays(&mut self) -> [(usize, f32); 2] {
        let wow_samples = self.wow * WOW_SECONDS * self.sample_rate;
        let flutter_samples = self.flutter * FLUTTER_SECONDS * self.sample_rate;
        let flutter_on = self.flutter > 0.0;
        let top = (self.buffer_l.len() - 3) as isize;
        let base = self.delay_samples as isize;
        let mut out = [(0, 0.0); 2];
        for (d, drift) in out.iter_mut().zip(self.drift.iter_mut()) {
            let (wow, flutter) = drift.next(self.sample_rate, flutter_on);
            let swing = wow * wow_samples + flutter * flutter_samples;
            let whole = math::floor(swing);
            let at = base + whole as isize;
            *d = if at < MIN_MOD_DELAY as isize {
                (MIN_MOD_DELAY, 0.0)
            } else if at >= top {
                (top as usize, 0.0)
            } else {
                (at as usize, swing - whole)
            };
        }
        out
    }

    /// Core processing: advances state and returns raw delayed samples.
    fn process_core(&mut self, input_l: f32, input_r: f32) -> (f32, f32) {
        let buf_len = self.buffer_l.len();
        let (delayed_l, delayed_r) = if self.wow > 0.0 || self.flutter > 0.0 {
            let [dl, dr] = self.modulated_delays();
            (Self::read_cubic(&self.buffer_l, self.write_pos, dl), Self::read_cubic(&self.buffer_r, self.write_pos, dr))
        } else {
            // A zero time reads the slot about to be written, which holds the
            // sample from one whole line ago: the line as long as it was
            // before it had headroom, so a zero time plays as it always did.
            let delay = if self.delay_samples == 0 { self.max_delay_samples + 1 } else { self.delay_samples };
            let read_pos =
                if self.write_pos >= delay { self.write_pos - delay } else { buf_len - (delay - self.write_pos) };
            (self.buffer_l[read_pos % buf_len], self.buffer_r[read_pos % buf_len])
        };

        // Cross-feedback with one-pole lowpass filter (same pattern as CombFilter in reverb)
        let fb_l = delayed_r * self.feedback;
        let fb_r = delayed_l * self.feedback;
        self.fb_filter_l = fb_l + self.fb_filter_coeff * (self.fb_filter_l - fb_l);
        self.fb_filter_r = fb_r + self.fb_filter_coeff * (self.fb_filter_r - fb_r);

        self.buffer_l[self.write_pos % buf_len] = input_l + self.fb_filter_l;
        self.buffer_r[self.write_pos % buf_len] = input_r + self.fb_filter_r;

        self.write_pos += 1;
        if self.write_pos >= buf_len {
            self.write_pos = 0;
        }

        (delayed_l, delayed_r)
    }

    /// Stereo ping-pong delay: R feeds back into L, L feeds back into R.
    /// Feedback path includes a one-pole lowpass for analog-style darkening.
    pub fn process_stereo(&mut self, input_l: f32, input_r: f32) -> (f32, f32) {
        let (delayed_l, delayed_r) = self.process_core(input_l, input_r);
        let dry = 1.0 - self.mix;
        let wet = self.mix;
        (input_l * dry + delayed_l * wet, input_r * dry + delayed_r * wet)
    }

    /// Wet-only stereo delay for send/return routing.
    /// Returns only the delayed signal without dry blend.
    pub fn process_stereo_wet(&mut self, input_l: f32, input_r: f32) -> (f32, f32) {
        self.process_core(input_l, input_r)
    }

    /// Backward-compatible mono process: delegates to stereo and sums.
    pub fn process(&mut self, input: f32) -> f32 {
        let (l, r) = self.process_stereo(input, input);
        (l + r) * 0.5
    }

    pub fn reset(&mut self) {
        for s in self.buffer_l.iter_mut() {
            *s = 0.0;
        }
        for s in self.buffer_r.iter_mut() {
            *s = 0.0;
        }
        self.write_pos = 0;
        self.fb_filter_l = 0.0;
        self.fb_filter_r = 0.0;
        self.drift = [Drift::new(0), Drift::new(1)];
    }
}
