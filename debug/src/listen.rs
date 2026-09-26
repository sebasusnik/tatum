//! What the report listens for, on each part on its own.
//!
//! Everything here works on a coarse summary of the audio -- one set of
//! numbers per millisecond -- kept for the whole render, because the
//! questions are about what happens around a moment ("is this jump louder
//! than anything near it?") and the audio itself is too big to keep.

/// Samples per summary frame: one millisecond.
pub const FRAME: usize = 44;

/// Per millisecond of one part: the biggest sample-to-sample jump in it, its
/// power, and its power below 25 Hz and above 16 kHz, the two ends of the
/// range where nothing an instrument means to play lives.
pub struct Frames {
    pub jump: Vec<f32>,
    pub power: Vec<f32>,
    pub low: Vec<f32>,
    pub high: Vec<f32>,
    low_pass: [Biquad; 2],
    high_pass: [Biquad; 2],
    cur_jump: f32,
    cur_sq: f64,
    cur_low: f64,
    cur_high: f64,
    cur_n: usize,
    p1: f32,
    p2: f32,
}

impl Default for Frames {
    fn default() -> Frames {
        let lp = Biquad::lowpass(RUMBLE_HZ);
        let hp = Biquad::highpass(FIZZ_HZ);
        Frames {
            jump: Vec::new(), power: Vec::new(), low: Vec::new(), high: Vec::new(),
            low_pass: [lp.clone(), lp], high_pass: [hp.clone(), hp],
            cur_jump: 0.0, cur_sq: 0.0, cur_low: 0.0, cur_high: 0.0, cur_n: 0, p1: 0.0, p2: 0.0,
        }
    }
}

impl Frames {
    pub fn push(&mut self, x: f32) {
        let lo = self.low_pass[0].run(x);
        let lo = self.low_pass[1].run(lo);
        let hi = self.high_pass[0].run(x);
        let hi = self.high_pass[1].run(hi);
        self.cur_low += (lo * lo) as f64;
        self.cur_high += (hi * hi) as f64;
        // How far this sample is from where the last two were heading. A
        // smooth wave, however loud, barely moves this; a click is a corner,
        // and a corner is what it measures. High notes and noise are corners
        // everywhere, which is why a jump only counts against its neighbours.
        let bend = (x - 2.0 * self.p1 + self.p2).abs();
        self.p2 = self.p1;
        self.p1 = x;
        self.cur_jump = self.cur_jump.max(bend);
        self.cur_sq += (x * x) as f64;
        self.cur_n += 1;
        if self.cur_n == FRAME {
            self.jump.push(self.cur_jump);
            self.power.push((self.cur_sq / FRAME as f64) as f32);
            self.low.push((self.cur_low / FRAME as f64) as f32);
            self.high.push((self.cur_high / FRAME as f64) as f32);
            self.cur_low = 0.0;
            self.cur_high = 0.0;
            self.cur_jump = 0.0;
            self.cur_sq = 0.0;
            self.cur_n = 0;
        }
    }

    pub fn len(&self) -> usize { self.jump.len() }

    pub fn is_empty(&self) -> bool { self.jump.is_empty() }

    /// Power in dB over frames `a..b`.
    pub fn db(&self, a: usize, b: usize) -> f32 {
        let (a, b) = (a.min(self.len()), b.min(self.len()));
        if b <= a { return -200.0 }
        let p: f64 = self.power[a..b].iter().map(|&v| v as f64).sum::<f64>() / (b - a) as f64;
        10.0 * p.max(1e-20).log10() as f32
    }
}

pub fn db(x: f32) -> f32 { 20.0 * x.max(1e-10).log10() }

/// Below this is felt as a push on the speaker cone, not heard as a note: the
/// lowest E on a bass is 41 Hz, and a sub rarely goes under 30.
pub const RUMBLE_HZ: f32 = 25.0;
/// Above this is air at most. A synth that puts a lot of energy up here is
/// usually folding its own overtones back down -- aliasing -- or clipping.
pub const FIZZ_HZ: f32 = 16_000.0;

/// A second-order Butterworth section; two in a row make the band edges
/// steep enough that a 40 Hz sub barely counts as rumble.
#[derive(Clone)]
struct Biquad { b: [f32; 3], a: [f32; 2], x: [f32; 2], y: [f32; 2] }

impl Biquad {
    fn new(hz: f32, high: bool) -> Biquad {
        let w = 2.0 * std::f32::consts::PI * hz / tatum_core::SAMPLE_RATE;
        let (sin, cos) = w.sin_cos();
        let alpha = sin / std::f32::consts::SQRT_2;
        let a0 = 1.0 + alpha;
        let b = if high {
            [(1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0]
        } else {
            [(1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0]
        };
        Biquad {
            b: [b[0] / a0, b[1] / a0, b[2] / a0],
            a: [-2.0 * cos / a0, (1.0 - alpha) / a0],
            x: [0.0; 2],
            y: [0.0; 2],
        }
    }
    fn lowpass(hz: f32) -> Biquad { Biquad::new(hz, false) }
    fn highpass(hz: f32) -> Biquad { Biquad::new(hz, true) }

    fn run(&mut self, x: f32) -> f32 {
        let y = self.b[0] * x + self.b[1] * self.x[0] + self.b[2] * self.x[1]
            - self.a[0] * self.y[0] - self.a[1] * self.y[1];
        self.x = [x, self.x[0]];
        self.y = [y, self.y[0]];
        y
    }
}

/// How much of a part's energy sits in one of the edge bands, in dB against
/// the whole part (0 dB = all of it).
pub fn share_db(f: &Frames, band: &[f32]) -> f32 {
    let total: f64 = f.power.iter().map(|&v| v as f64).sum();
    let part: f64 = band.iter().map(|&v| v as f64).sum();
    if total <= 0.0 { return -200.0 }
    10.0 * (part / total).max(1e-20).log10() as f32
}

/// A jump smaller than this is not heard on its own, whatever surrounds it.
const MIN_JUMP_DB: f32 = -48.0;
/// How much a jump has to stand above the biggest one near it.
const STANDS_OUT: f32 = 4.0; // 12 dB
/// "Near", in frames each side: long enough to hold several cycles of the
/// lowest bass note, so a saw's own corner every cycle is its own neighbour.
const NEIGHBOURHOOD: usize = 50;
/// Frames right next to the jump that do not count as neighbours: a click
/// smears over a sample or two into the frame beside it.
const GUARD: usize = 2;
/// If the part is this much louder just after the jump than just before, the
/// jump is a note starting -- a drum hit, a pluck -- not a click.
const ONSET_RISE: f32 = 2.0; // 3 dB in power
/// How loud a corner has to be against the level around it: a note cut
/// off dead has a corner as big as the note itself.
const UNDER_SOUND: f32 = 0.25; // -12 dB
/// Clicks closer than this are one click.
const MERGE: usize = 30;

pub struct Click {
    pub frame: usize,
    pub db: f32,
}

/// Sudden corners in the wave that are not the start of a note: a note cut
/// off mid-cycle, a voice stolen, a value jumping instead of gliding.
pub fn clicks(f: &Frames) -> Vec<Click> {
    let n = f.len();
    let floor = 10f32.powf(MIN_JUMP_DB / 20.0);
    let mut out: Vec<Click> = Vec::new();
    for i in 0..n {
        let j = f.jump[i];
        if j < floor { continue }
        let near = (i.saturating_sub(NEIGHBOURHOOD)..i.saturating_sub(GUARD))
            .chain((i + GUARD + 1).min(n)..(i + NEIGHBOURHOOD + 1).min(n))
            .map(|k| f.jump[k])
            .fold(0.0f32, f32::max);
        if j < near * STANDS_OUT { continue }
        let before = f.power[i.saturating_sub(12)..i.saturating_sub(1)].iter().sum::<f32>();
        let after = f.power[(i + 1).min(n)..(i + 12).min(n)].iter().sum::<f32>();
        if after > before * ONSET_RISE { continue }
        // Against the sound it sits in. A corner well under the level of the
        // note around it is part of that note's shape -- an envelope turning,
        // a compressor grabbing -- and is not heard as anything separate.
        let around = (before.max(after) / 11.0).sqrt();
        if j < around * UNDER_SOUND { continue }
        match out.last_mut() {
            Some(last) if i - last.frame < MERGE => {
                if db(j) > last.db { *last = Click { frame: i, db: db(j) } }
            }
            _ => out.push(Click { frame: i, db: db(j) }),
        }
    }
    out
}

/// A stretch between notes where the part should have gone quiet and did not.
pub struct Floor {
    /// The frames measured: the end of the gap.
    pub start: usize,
    pub end: usize,
    pub db: f32,
    /// How much the level fell across the gap. A tail dies away; a floor of
    /// noise or hum sits still.
    pub fall_db: f32,
}

/// Only gaps this long are judged: a release and most tails are over by then.
const MIN_GAP: usize = 1500;
/// Below this nothing is worth reporting.
const QUIET_DB: f32 = -80.0;
/// A tail falls at least this much across a second; a floor does not.
const STILL_DB: f32 = 3.0;

/// Gaps where no note is held that end still sounding, at a steady level.
pub fn floors(f: &Frames, held: &dyn Fn(usize) -> bool) -> Vec<Floor> {
    let mut out = Vec::new();
    let n = f.len();
    let mut i = 0;
    while i < n {
        if held(i) { i += 1; continue }
        let start = i;
        while i < n && !held(i) { i += 1; }
        let end = i;
        // A gap cut off by the end of the render did not get to finish.
        if end - start < MIN_GAP || end == n { continue }
        // Stop short of the end: `held` is read once a block, so the next
        // note can be a frame or two into the gap before it says so.
        let late = (end - 210, end - 10);
        let early = (start + 300, start + 500);
        let late_db = f.db(late.0, late.1);
        let fall = f.db(early.0, early.1) - late_db;
        if late_db > QUIET_DB && fall < STILL_DB {
            out.push(Floor { start: late.0, end: late.1, db: late_db, fall_db: fall });
        }
    }
    out
}

/// How loud a part is while it plays: the level it reaches in its loudest
/// tenth, which is what a floor is heard against.
pub fn playing_db(f: &Frames) -> f32 {
    let mut p: Vec<f32> = f.power.iter().copied().filter(|&v| v > 1e-12).collect();
    if p.is_empty() { return -200.0 }
    p.sort_by(|a, b| a.partial_cmp(b).unwrap());
    10.0 * p[p.len() * 9 / 10].log10()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(xs: impl Iterator<Item = f32>) -> Frames {
        let mut f = Frames::default();
        for x in xs { f.push(x) }
        f
    }

    fn sine(hz: f32, i: usize) -> f32 {
        (2.0 * std::f32::consts::PI * hz * i as f32 / 44100.0).sin() * 0.5
    }

    #[test]
    fn a_sine_cut_off_mid_cycle_clicks_and_a_smooth_one_does_not() {
        let smooth = frames((0..44100).map(|i| sine(110.0, i)));
        assert!(clicks(&smooth).is_empty());
        // the note stops dead a quarter of the way into a cycle
        let cut = frames((0..44100).map(|i| if i < 22050 + 100 { sine(110.0, i) } else { 0.0 }));
        let found = clicks(&cut);
        assert_eq!(found.len(), 1, "{:?}", found.iter().map(|c| c.frame).collect::<Vec<_>>());
        assert!((found[0].frame as i64 - (22150 / FRAME) as i64).abs() <= 1);
    }

    #[test]
    fn a_saw_is_not_a_click_every_cycle() {
        let saw = frames((0..88200).map(|i| ((i as f32 * 41.2 / 44100.0).fract() - 0.5) * 0.8));
        assert!(clicks(&saw).is_empty());
    }

    #[test]
    fn a_note_starting_is_an_onset_not_a_click() {
        let hit = frames((0..44100).map(|i| if i >= 22000 { sine(200.0, i - 22000 + 50) } else { 0.0 }));
        assert!(clicks(&hit).is_empty());
    }

    #[test]
    fn hiss_that_never_stops_is_a_floor_and_a_tail_is_not() {
        let mut rng = 1u32;
        let mut noise = || { rng = rng.wrapping_mul(1664525).wrapping_add(1013904223); (rng >> 8) as f32 / (1 << 24) as f32 - 0.5 };
        // a note for the first second, three seconds of gap, a note again
        let held = |i: usize| !(1000..3900).contains(&i);
        let hiss = frames((0..4 * 44100).map(|i| if i < 44100 { sine(220.0, i) } else { 0.01 * noise() }));
        assert_eq!(floors(&hiss, &held).len(), 1);
        let tail = frames((0..4 * 44100).map(|i| sine(220.0, i) * (-(i as f32) / 20000.0).exp()));
        assert!(floors(&tail, &held).is_empty());
    }

    #[test]
    fn the_edge_bands_catch_only_what_is_in_them() {
        let tone = |hz: f32| frames((0..44100).map(move |i| sine(hz, i)));
        let (sub, bass, top) = (tone(18.0), tone(110.0), tone(18_000.0));
        assert!(share_db(&sub, &sub.low) > -3.0, "18 Hz: {}", share_db(&sub, &sub.low));
        assert!(share_db(&bass, &bass.low) < -30.0, "110 Hz: {}", share_db(&bass, &bass.low));
        assert!(share_db(&bass, &bass.high) < -60.0);
        assert!(share_db(&top, &top.high) > -2.0, "18 kHz: {}", share_db(&top, &top.high));
    }
}
