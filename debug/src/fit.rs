//! Fitting the `kick` node to a record's kick: the arguments whose hit has
//! the pitch and the level over time of the record's averaged hit.
//!
//! The target is a hit as [`crate::hit::average_hit`] gives it. A candidate
//! is built from `kick(...)`'s own arguments by the compiler's own rules
//! ([`tatum_core::dsl::compiler::kick_spec`]), rendered alone, cut where it
//! first reaches 5% of its peak the way the average is, and read with
//! [`crate::hit::curves`]. What is minimised is the pitch error in semitones
//! on an even grid from 0.6 to 215 ms, the same over the attack (to 10 ms)
//! on its own, since the attack is a few points of that grid and the ear
//! hears it first, and the level error in dB.
//!
//! The search is Nelder-Mead over the arguments mapped to 0..1 (Hz and ms on
//! a log scale), in three stages: the pitch alone, the level alone, then all
//! of it, each from the best so far with a few restarts. The click is set
//! last, on its own, to the record's click as `tatum analyze` measures it.

use tatum_core::dsl::ast::Param;
use tatum_core::dsl::compiler::kick_spec;
use tatum_core::graph::node::MAX_NODE_INPUTS;
use tatum_core::SAMPLE_RATE;

use crate::hit::{curves, Curves};
use crate::reference::Biquad;

/// The pitch is compared from here...
pub const PITCH_FROM_MS: f32 = 0.6;
/// ...to here, on a grid this fine.
pub const PITCH_TO_MS: f32 = 215.0;
const GRID_MS: f32 = 0.6;
/// The attack: the first this many ms, scored on their own as well.
pub const ATTACK_MS: f32 = 10.0;
/// Levels under this are the same level: the noise floor of an average.
const LEVEL_FLOOR_DB: f32 = -40.0;
/// No chirp under this many times the start.
const NO_CHIRP: f32 = 1.02;

/// The arguments of a `kick(...)`, in the units it is written in.
#[derive(Clone, Debug, PartialEq)]
pub struct KickArgs {
    pub note: f32,
    pub start: f32,
    pub knee: f32,
    pub snap: f32,
    pub body: f32,
    /// Hz of the chirp; under [`NO_CHIRP`] times the start, none.
    pub chirp: f32,
    pub chirp_time: f32,
    pub hold: f32,
    pub sag: f32,
    pub release: f32,
    pub phase: f32,
    /// dB; -80 is none.
    pub click: f32,
    pub tone: f32,
}

impl Default for KickArgs {
    fn default() -> Self {
        KickArgs {
            note: 50.0,
            start: 800.0,
            knee: 150.0,
            snap: 6.0,
            body: 60.0,
            chirp: 0.0,
            chirp_time: 1.0,
            hold: 60.0,
            sag: 0.0,
            release: 180.0,
            phase: 0.0,
            click: -80.0,
            tone: 3000.0,
        }
    }
}

impl KickArgs {
    fn has_chirp(&self) -> bool {
        self.chirp >= NO_CHIRP * self.start
    }

    /// The arguments as the parser hands them to the compiler.
    pub fn params(&self) -> Vec<Param> {
        let mut p = vec![
            Param::Float(self.note),
            Param::Named("start".into(), self.start),
            Param::Named("knee".into(), self.knee),
            Param::Named("snap".into(), self.snap),
            Param::Named("body".into(), self.body),
            Param::Named("hold".into(), self.hold),
            Param::Named("sag".into(), self.sag),
            Param::Named("release".into(), self.release),
            Param::Named("phase".into(), self.phase),
            Param::Named("click".into(), self.click),
            Param::Named("tone".into(), self.tone),
        ];
        if self.has_chirp() {
            p.push(Param::Named("chirp".into(), self.chirp));
            p.push(Param::Named("chirp_time".into(), self.chirp_time));
        }
        p
    }

    /// The `kick(...)` line, ready to paste: every value rounded to what an
    /// ear could tell apart, and only what differs from the defaults.
    pub fn line(&self) -> String {
        let hz = |v: f32| {
            if v >= 1000.0 {
                format!("{}khz", round_to(v / 1000.0, 2))
            } else {
                format!("{}hz", round_to(v, 1))
            }
        };
        let ms = |v: f32| format!("{}ms", round_to(v, if v < 10.0 { 2 } else { 1 }));
        let mut parts = vec![note_or_hz(self.note)];
        parts.push(format!("start={}", hz(self.start)));
        parts.push(format!("knee={}", hz(self.knee)));
        parts.push(format!("snap={}", ms(self.snap)));
        if self.has_chirp() {
            parts.push(format!("chirp={}", hz(self.chirp)));
            parts.push(format!("chirp_time={}", ms(self.chirp_time)));
        }
        parts.push(format!("body={}", ms(self.body)));
        parts.push(format!("hold={}", ms(self.hold)));
        if self.sag < -0.05 {
            parts.push(format!("sag={}db", round_to(self.sag, 1)));
        }
        parts.push(format!("release={}", ms(self.release)));
        if self.phase >= 0.5 {
            parts.push(format!("phase={}", round_to(self.phase, 0)));
        }
        if self.click > -79.5 {
            parts.push(format!("click={}db", round_to(self.click, 1)));
            if (self.tone - 3000.0).abs() > 1.0 {
                parts.push(format!("tone={}", hz(self.tone)));
            }
        }
        format!("kick({})", parts.join(", "))
    }

    /// The same arguments, rounded as [`Self::line`] writes them, so what is
    /// scored is what is pasted.
    pub fn rounded(&self) -> KickArgs {
        let hz = |v: f32| {
            if v >= 1000.0 {
                round_to(v / 1000.0, 2).parse::<f32>().unwrap_or(v / 1000.0) * 1000.0
            } else {
                r(v, 1)
            }
        };
        let ms = |v: f32| r(v, if v < 10.0 { 2 } else { 1 });
        let note = note_or_hz(self.note);
        let note = if tatum_core::dsl::compiler::note_in_midi_range(&note) {
            tatum_core::math::midi_to_freq(tatum_core::dsl::compiler::note_name_to_midi(&note))
        } else {
            r(self.note, 1)
        };
        KickArgs {
            note,
            start: hz(self.start),
            knee: hz(self.knee),
            snap: ms(self.snap),
            body: ms(self.body),
            chirp: if self.has_chirp() { hz(self.chirp) } else { 0.0 },
            chirp_time: ms(self.chirp_time),
            hold: ms(self.hold),
            sag: if self.sag < -0.05 { r(self.sag, 1) } else { 0.0 },
            release: ms(self.release),
            phase: if self.phase >= 0.5 { r(self.phase, 0) } else { 0.0 },
            click: if self.click > -79.5 { r(self.click, 1) } else { -80.0 },
            tone: hz(self.tone),
        }
    }

    /// One hit, alone, `ms` long from where it starts, as the average of a
    /// record's hits is cut. `None` when the
    /// compiler refuses the arguments.
    pub fn render(&self, ms: f32) -> Option<Vec<f32>> {
        let spec = kick_spec(&self.params()).ok()?;
        let mut node = spec.instantiate();
        let n = (((ms + 30.0) * 0.001) * SAMPLE_RATE) as usize;
        let inputs = [0.0f32; MAX_NODE_INPUTS];
        let x: Vec<f32> = (0..n).map(|_| node.process(&inputs, 0)).collect();
        Some(cut_like_an_average(&x, ms))
    }
}

/// From where the hit starts in `x`, `ms` long: how
/// [`crate::hit::average_hit`] cuts the hit it returns.
pub fn cut_like_an_average(x: &[f32], ms: f32) -> Vec<f32> {
    let at = crate::hit::hit_start(x).unwrap_or(0);
    let n = (ms * 0.001 * SAMPLE_RATE) as usize;
    x[at..(at + n).min(x.len())].to_vec()
}

fn r(v: f32, decimals: i32) -> f32 {
    let k = 10f32.powi(decimals);
    (v * k).round() / k
}

fn round_to(v: f32, decimals: usize) -> String {
    let s = format!("{v:.decimals$}");
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
    if s == "-0" {
        "0".into()
    } else {
        s
    }
}

/// A note's name when `hz` is within 3 cents of it, else Hz to a tenth.
fn note_or_hz(hz: f32) -> String {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    let m = 69.0 + 12.0 * (hz / 440.0).log2();
    let n = m.round();
    if (m - n).abs() * 100.0 <= 3.0 && (0.0..=127.0).contains(&n) {
        format!("{}{}", NAMES[n as usize % 12], n as i32 / 12 - 1)
    } else {
        format!("{}hz", round_to(hz, 1))
    }
}

// ── What is compared ──

/// Where the attack ends and the body is read through a low-pass.
const BODY_FROM_MS: f32 = 10.0;
/// The body's pitch and level are read under this.
const BODY_LOWPASS_HZ: f64 = 800.0;
/// The pitch is compared only while the target is within this of its peak.
const PITCH_WHILE_DB: f32 = -30.0;

/// A hit's pitch and level as the fit compares them: the attack full band,
/// the body through a low-pass. On a record the body's last hundred
/// milliseconds share the beat with hats and claps, and their crossings
/// read as a kick at 13 kHz; a kick under 800 Hz after its first 10 ms
/// loses nothing by the filter, and the hats are gone.
pub fn read(y: &[f32]) -> Curves {
    let full = curves(y);
    let body = curves(&Biquad::lowpass(BODY_LOWPASS_HZ).run_twice(y));
    let join = |early: &[(f32, f32)], late: &[(f32, f32)]| -> Vec<(f32, f32)> {
        early
            .iter()
            .filter(|p| p.0 <= BODY_FROM_MS)
            .chain(late.iter().filter(|p| p.0 > BODY_FROM_MS))
            .copied()
            .collect()
    };
    let (pitch, level) = (join(&full.pitch, &body.pitch), join(&full.level, &body.level));
    Curves { pitch, level }
}

/// What a fit is scored against: the record's hit, read once.
pub struct Target {
    pub hit: Vec<f32>,
    pub curves: Curves,
    /// ms of the hit.
    pub ms: f32,
    pitch: Vec<(f32, f32)>,
    level: Vec<(f32, f32)>,
    pub click_db: f32,
}

/// Linear between readings; before the first, the first; after the last,
/// nothing.
fn at(points: &[(f32, f32)], t: f32) -> Option<f32> {
    let first = points.first()?;
    if t <= first.0 {
        return Some(first.1);
    }
    let i = points.partition_point(|p| p.0 < t);
    let b = points.get(i)?;
    let a = points[i - 1];
    let k = if b.0 > a.0 { (t - a.0) / (b.0 - a.0) } else { 0.0 };
    Some(a.1 + k * (b.1 - a.1))
}

impl Target {
    pub fn new(hit: Vec<f32>) -> Target {
        let c = read(&hit);
        let ms = hit.len() as f32 * 1000.0 / SAMPLE_RATE;
        let last_pitch = c.pitch.last().map_or(0.0, |p| p.0);
        let last_level = c.level.last().map_or(0.0, |p| p.0);
        let mut pitch = Vec::new();
        let mut t = PITCH_FROM_MS;
        while t <= PITCH_TO_MS.min(last_pitch) {
            let loud = at(&c.level, t).is_some_and(|db| db >= PITCH_WHILE_DB);
            if let (Some(hz), true) = (at(&c.pitch, t), loud) {
                pitch.push((t, hz));
            }
            t += GRID_MS;
        }
        let mut level = Vec::new();
        let mut t = PITCH_FROM_MS;
        while t <= ms.min(last_level) {
            if let Some(db) = at(&c.level, t) {
                level.push((t, db.max(LEVEL_FLOOR_DB)));
            }
            t += 1.0;
        }
        let click_db = click_db(&hit);
        Target { hit, curves: c, ms, pitch, level, click_db }
    }
}

/// The click as `tatum analyze` measures it on a hit: the energy over 2 kHz
/// in the first 10 ms against the energy under 400 Hz in the first 100 ms.
pub fn click_db(y: &[f32]) -> f32 {
    let n = |ms: f32| ((ms * 0.001 * SAMPLE_RATE) as usize).min(y.len());
    let top = Biquad::highpass(2000.0).run_twice(y);
    let body = Biquad::lowpass(400.0).run_twice(y);
    let e_top: f32 = top[..n(10.0)].iter().map(|v| v * v).sum();
    let e_body: f32 = body[..n(100.0)].iter().map(|v| v * v).sum();
    if e_body <= 0.0 {
        return -60.0;
    }
    10.0 * (e_top.max(1e-12) / e_body).log10()
}

/// How far a hit is from the target.
#[derive(Clone, Copy, Debug, Default)]
pub struct Score {
    /// RMS pitch error over the whole grid, semitones.
    pub pitch_st: f32,
    /// RMS pitch error over the attack.
    pub attack_st: f32,
    /// RMS level error, dB.
    pub level_db: f32,
    /// The click, theirs minus ours, dB.
    pub click_db: f32,
}

impl Score {
    /// One number to minimise: a semitone weighs as 3 dB, and the attack
    /// counts again on its own.
    pub fn cost(&self) -> f32 {
        self.pitch_st + 0.3 * self.attack_st + self.level_db / 3.0
    }
}

/// A semitone error when the candidate has stopped before the target has.
const GONE_ST: f32 = 6.0;

/// Where two hits are cut can differ by a little: an average's start is
/// found under whatever rang before it. The score is the best over a shift
/// of up to this many ms either way.
pub const SHIFT_MS: f32 = 1.5;

pub fn score(target: &Target, y: &[f32]) -> Score {
    let c = read(y);
    let click = target.click_db - click_db(y);
    // Every 0.1 ms, then every sample around the best of those.
    let sample = 1000.0 / SAMPLE_RATE;
    let mut best = (0.0f32, Score { pitch_st: f32::MAX, attack_st: 0.0, level_db: 0.0, click_db: click });
    let try_at = |shift: f32, best: &mut (f32, Score)| {
        let s = score_at(target, &c, shift, click);
        if s.cost() < best.1.cost() {
            *best = (shift, s);
        }
    };
    for i in 0..=30 {
        try_at(-SHIFT_MS + 0.1 * i as f32, &mut best);
    }
    let around = best.0;
    for i in -5..=5 {
        try_at(around + sample * i as f32, &mut best);
    }
    best.1
}

/// The score with ours read `shift` ms later than theirs.
fn score_at(target: &Target, c: &Curves, shift: f32, click: f32) -> Score {
    let (mut sp, mut np, mut sa, mut na) = (0.0f32, 0, 0.0f32, 0);
    for &(t, want) in &target.pitch {
        let e = match at(&c.pitch, t + shift) {
            Some(hz) if hz > 0.0 => 12.0 * (hz / want).log2(),
            _ => GONE_ST,
        };
        sp += e * e;
        np += 1;
        if t <= ATTACK_MS {
            sa += e * e;
            na += 1;
        }
    }
    let mut sl = 0.0f32;
    for &(t, want) in &target.level {
        let got = at(&c.level, t + shift).unwrap_or(LEVEL_FLOOR_DB).max(LEVEL_FLOOR_DB);
        sl += (got - want) * (got - want);
    }
    let rms = |s: f32, n: usize| if n > 0 { (s / n as f32).sqrt() } else { 0.0 };
    Score { pitch_st: rms(sp, np), attack_st: rms(sa, na), level_db: rms(sl, target.level.len()), click_db: click }
}

// ── The search ──

/// The arguments the search moves, each mapped to 0..1.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Dim {
    Note,
    Start,
    Knee,
    Snap,
    Body,
    Chirp,
    ChirpTime,
    Hold,
    Sag,
    Release,
    Phase,
}

const PITCH_DIMS: [Dim; 8] =
    [Dim::Note, Dim::Start, Dim::Knee, Dim::Snap, Dim::Body, Dim::Chirp, Dim::ChirpTime, Dim::Phase];
const LEVEL_DIMS: [Dim; 4] = [Dim::Hold, Dim::Sag, Dim::Release, Dim::Phase];
const ALL_DIMS: [Dim; 11] = [
    Dim::Note,
    Dim::Start,
    Dim::Knee,
    Dim::Snap,
    Dim::Body,
    Dim::Chirp,
    Dim::ChirpTime,
    Dim::Hold,
    Dim::Sag,
    Dim::Release,
    Dim::Phase,
];

fn log_to(lo: f32, hi: f32, u: f32) -> f32 {
    lo * (hi / lo).powf(u.clamp(0.0, 1.0))
}
fn log_from(lo: f32, hi: f32, v: f32) -> f32 {
    ((v.max(lo) / lo).ln() / (hi / lo).ln()).clamp(0.0, 1.0)
}

/// The lowest a start or a knee may be over the note, with room for the
/// compiler's "more than 10%".
fn over_note(note: f32) -> f32 {
    1.12 * note
}

/// Set `d` of `k` from `u`. Start, knee and chirp are placed between what
/// the arguments before them allow, so every point of the cube is a kick
/// the compiler takes.
fn set(k: &mut KickArgs, d: Dim, u: f32) {
    let u = u.clamp(0.0, 1.0);
    match d {
        Dim::Note => k.note = log_to(20.0, 200.0, u),
        Dim::Start => k.start = log_to(over_note(k.note) * 1.02, 5000.0, u),
        Dim::Knee => k.knee = log_to(over_note(k.note), k.start, u),
        Dim::Snap => k.snap = log_to(0.1, 100.0, u),
        Dim::Body => k.body = log_to(1.0, 1000.0, u),
        Dim::Chirp => k.chirp = if u < 0.02 { 0.0 } else { log_to(k.start * NO_CHIRP, 12000.0, (u - 0.02) / 0.98) },
        Dim::ChirpTime => k.chirp_time = log_to(0.1, 20.0, u),
        Dim::Hold => k.hold = log_to(1.0, 2001.0, u) - 1.0,
        Dim::Sag => k.sag = -24.0 * u,
        Dim::Release => k.release = log_to(5.0, 5000.0, u),
        Dim::Phase => k.phase = 90.0 * u,
    }
}

fn get(k: &KickArgs, d: Dim) -> f32 {
    match d {
        Dim::Note => log_from(20.0, 200.0, k.note),
        Dim::Start => log_from(over_note(k.note) * 1.02, 5000.0, k.start),
        Dim::Knee => log_from(over_note(k.note), k.start.max(over_note(k.note) * 1.0001), k.knee),
        Dim::Snap => log_from(0.1, 100.0, k.snap),
        Dim::Body => log_from(1.0, 1000.0, k.body),
        Dim::Chirp => {
            if k.has_chirp() {
                0.02 + 0.98 * log_from(k.start * NO_CHIRP, 12000.0, k.chirp)
            } else {
                0.0
            }
        }
        Dim::ChirpTime => log_from(0.1, 20.0, k.chirp_time),
        Dim::Hold => log_from(1.0, 2001.0, k.hold + 1.0),
        Dim::Sag => (-k.sag / 24.0).clamp(0.0, 1.0),
        Dim::Release => log_from(5.0, 5000.0, k.release),
        Dim::Phase => (k.phase / 90.0).clamp(0.0, 1.0),
    }
}

/// `k` with `dims` taken from `u`, in the order of [`ALL_DIMS`] so a start
/// is placed after its note and a knee after its start.
fn with(k: &KickArgs, dims: &[Dim], u: &[f32]) -> KickArgs {
    let mut out = k.clone();
    // Keep the unmoved arguments where they were in 0..1, so a moved note
    // carries its start and knee with it rather than out of their range.
    let fixed: Vec<(Dim, f32)> = ALL_DIMS.iter().map(|&d| (d, get(k, d))).collect();
    for &d in &ALL_DIMS {
        let v = match dims.iter().position(|&x| x == d) {
            Some(i) => u[i],
            None => fixed.iter().find(|f| f.0 == d).map_or(0.0, |f| f.1),
        };
        set(&mut out, d, v);
    }
    out
}

/// What the search minimises for one kick.
pub trait Judge {
    fn judge(&mut self, k: &KickArgs) -> Score;
}

/// The node rendered on its own: what the search runs on.
pub struct Direct<'a> {
    pub target: &'a Target,
    pub evals: usize,
}

impl Judge for Direct<'_> {
    fn judge(&mut self, k: &KickArgs) -> Score {
        self.evals += 1;
        match k.render(self.target.ms) {
            Some(y) => score(self.target, &y),
            None => Score { pitch_st: 99.0, attack_st: 99.0, level_db: 99.0, click_db: 0.0 },
        }
    }
}

/// Which part of the score a stage minimises.
#[derive(Clone, Copy)]
enum Part {
    Pitch,
    Level,
    All,
}

fn part_cost(s: &Score, part: Part) -> f32 {
    match part {
        Part::Pitch => s.pitch_st + 0.3 * s.attack_st,
        Part::Level => s.level_db,
        Part::All => s.cost(),
    }
}

/// Nelder-Mead in the cube, from `start`, `evals` at most.
fn nelder_mead(f: &mut dyn FnMut(&[f32]) -> f32, start: &[f32], step: f32, evals: usize) -> (Vec<f32>, f32) {
    let n = start.len();
    let clamp = |v: Vec<f32>| v.into_iter().map(|x| x.clamp(0.0, 1.0)).collect::<Vec<f32>>();
    let mut pts: Vec<(Vec<f32>, f32)> = Vec::with_capacity(n + 1);
    let p0 = clamp(start.to_vec());
    let f0 = f(&p0);
    pts.push((p0.clone(), f0));
    for i in 0..n {
        let mut p = p0.clone();
        p[i] = if p[i] + step <= 1.0 { p[i] + step } else { p[i] - step };
        let fp = f(&p);
        pts.push((p, fp));
    }
    let mut used = n + 1;
    while used < evals {
        pts.sort_by(|a, b| a.1.total_cmp(&b.1));
        if (pts[n].1 - pts[0].1).abs() < 1e-5 {
            break;
        }
        let centroid: Vec<f32> = (0..n).map(|j| pts[..n].iter().map(|p| p.0[j]).sum::<f32>() / n as f32).collect();
        let towards = |k: f32| clamp((0..n).map(|j| centroid[j] + k * (pts[n].0[j] - centroid[j])).collect());
        let r = towards(-1.0);
        let fr = f(&r);
        used += 1;
        if fr < pts[0].1 {
            let e = towards(-2.0);
            let fe = f(&e);
            used += 1;
            pts[n] = if fe < fr { (e, fe) } else { (r, fr) };
        } else if fr < pts[n - 1].1 {
            pts[n] = (r, fr);
        } else {
            let c = if fr < pts[n].1 { towards(-0.5) } else { towards(0.5) };
            let fc = f(&c);
            used += 1;
            if fc < pts[n].1.min(fr) {
                pts[n] = (c, fc);
            } else {
                // Shrink towards the best.
                let best = pts[0].0.clone();
                for p in pts.iter_mut().skip(1) {
                    p.0 = clamp((0..n).map(|j| best[j] + 0.5 * (p.0[j] - best[j])).collect());
                    p.1 = f(&p.0);
                    used += 1;
                }
            }
        }
    }
    pts.sort_by(|a, b| a.1.total_cmp(&b.1));
    pts.swap_remove(0)
}

/// One stage: `dims` moved from `k` to minimise `part`, with restarts.
fn stage(judge: &mut dyn Judge, k: &KickArgs, dims: &[Dim], part: Part, evals: usize) -> KickArgs {
    let mut best = k.clone();
    let mut best_cost = part_cost(&judge.judge(&best), part);
    for &step in &[0.25f32, 0.1, 0.04] {
        let base = best.clone();
        let u0: Vec<f32> = dims.iter().map(|&d| get(&base, d)).collect();
        let mut f = |u: &[f32]| part_cost(&judge.judge(&with(&base, dims, u)), part);
        let (u, c) = nelder_mead(&mut f, &u0, step, evals / 3);
        if c < best_cost {
            best_cost = c;
            best = with(&base, dims, &u);
        }
    }
    best
}

/// A first guess read off the target's curves: the tail, where the pitch
/// starts, how long it takes to land, how long the level holds and falls.
pub fn first_guess(target: &Target) -> KickArgs {
    let c = &target.curves;
    let late: Vec<f32> = target.pitch.iter().filter(|p| p.0 > target.ms * 0.4).map(|p| p.1).collect();
    let note = if late.is_empty() { 50.0 } else { median(late) }.clamp(20.0, 200.0);
    let start = c.pitch.first().map_or(800.0, |p| p.1).clamp(over_note(note) * 1.05, 5000.0);
    let body = c.pitch.iter().find(|p| p.1 <= note * 1.1).map_or(60.0, |p| p.0).clamp(1.0, 1000.0);
    let top = c.level.iter().position(|l| l.1 >= -1.0).unwrap_or(0);
    let fall = |db: f32| c.level[top..].iter().find(|l| l.1 < db).map(|l| l.0);
    let hold = fall(-1.0).unwrap_or(60.0).clamp(0.0, 2000.0);
    let release = match fall(-24.0).or(fall(-12.0).map(|t| t * 1.6)) {
        Some(t) => ((t - hold).max(5.0) * 60.0 / 24.0).clamp(5.0, 5000.0),
        None => 300.0,
    };
    KickArgs {
        note,
        start,
        knee: (3.0 * note).clamp(over_note(note), start),
        snap: 4.0,
        body,
        hold,
        release,
        ..KickArgs::default()
    }
}

fn median(mut v: Vec<f32>) -> f32 {
    v.sort_by(f32::total_cmp);
    v[v.len() / 2]
}

/// The click, in dB. The target's click is what it has over 2 kHz in its
/// first 10 ms, and on a record more than the kick puts that there: the
/// hats, a master's saturation. Matched alone, it drove the click to full
/// level, where the click became the hit's peak and every level under it
/// read 3 dB low. So the click is the one whose whole score is best: the
/// pitch and the level as fitted, plus the click's own error at a tenth of
/// the weight -- what is left after that is said, not chased.
pub fn fit_click(target: &Target, k: &KickArgs) -> KickArgs {
    let cost = |db: f32| {
        let mut c = k.clone();
        c.click = db;
        match c.render(target.ms) {
            Some(y) => {
                let s = score(target, &y);
                s.cost() + 0.1 * s.click_db.abs()
            }
            None => f32::MAX,
        }
    };
    let mut best = (-80.0f32, cost(-80.0));
    for i in 0..=20 {
        let db = -60.0 + 3.0 * i as f32;
        let c = cost(db);
        if c < best.1 {
            best = (db, c);
        }
    }
    if best.0 > -80.0 {
        for i in -6..=6 {
            let db = (best.0 + 0.5 * i as f32).min(0.0);
            let c = cost(db);
            if c < best.1 {
                best = (db, c);
            }
        }
    }
    let mut out = k.clone();
    out.click = best.0;
    out
}

/// Everything a fit found.
pub struct Fit {
    pub args: KickArgs,
    pub score: Score,
    pub evals: usize,
}

/// Fit the kick node to `target`, from `from` (or a guess read off it).
/// `effort` scales how many kicks are tried: 1.0 is a few thousand.
pub fn fit(target: &Target, from: Option<&KickArgs>, effort: f32) -> Fit {
    let mut judge = Direct { target, evals: 0 };
    let budget = |n: f32| (n * effort.max(0.05)) as usize;
    let mut k = from.cloned().unwrap_or_else(|| first_guess(target));
    if from.is_none() {
        // From a few places, the guess and around it: a kick's sweep and its
        // fall each have more than one shape that fits nearly, and one
        // search from one guess can settle in the wrong one (a 34 ms
        // release on a kick that rings for 200).
        let seeds = |k: &KickArgs, vary: &dyn Fn(&mut KickArgs, f32, f32)| -> Vec<KickArgs> {
            let mut out = vec![k.clone()];
            for (a, b) in [(0.5, 0.5), (2.0, 2.0), (0.5, 2.0), (2.0, 0.5)] {
                let mut c = k.clone();
                vary(&mut c, a, b);
                out.push(c);
            }
            out
        };
        let best_of = |judge: &mut Direct, seeds: Vec<KickArgs>, dims: &[Dim], part: Part, evals: usize| {
            let mut best: Option<(f32, KickArgs)> = None;
            for s in seeds {
                let s = stage(judge, &s, dims, part, evals);
                let c = part_cost(&judge.judge(&s), part);
                if best.as_ref().is_none_or(|b| c < b.0) {
                    best = Some((c, s));
                }
            }
            best.map(|b| b.1)
        };
        let pitch_seeds = seeds(&k, &|c, a, b| {
            c.body = (c.body * a).clamp(1.0, 1000.0);
            c.start = (c.start * b).clamp(over_note(c.note) * 1.05, 5000.0);
            c.knee = c.knee.min(c.start);
        });
        k = best_of(&mut judge, pitch_seeds, &PITCH_DIMS, Part::Pitch, budget(500.0)).unwrap_or(k);
        let level_seeds = seeds(&k, &|c, a, b| {
            c.hold = (c.hold * a).clamp(0.0, 2000.0);
            c.release = (c.release * b).clamp(5.0, 5000.0);
        });
        k = best_of(&mut judge, level_seeds, &LEVEL_DIMS, Part::Level, budget(250.0)).unwrap_or(k);
    }
    k = stage(&mut judge, &k, &ALL_DIMS, Part::All, budget(2400.0));
    k = fit_click(target, &k).rounded();
    let score = judge.judge(&k);
    Fit { args: k, score, evals: judge.evals }
}

/// Refine `from` with any judge: what `tatum kick fit` does with the hit
/// rendered by the whole engine, once the node alone has been fitted.
pub fn refine(judge: &mut dyn Judge, from: &KickArgs, evals: usize) -> KickArgs {
    let click = from.click;
    let mut k = stage(judge, from, &ALL_DIMS, Part::All, evals);
    k.click = click;
    k.rounded()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A record made of a known kick: four to the bar at `bpm`, an offbeat
    /// bass on top that the average thins out, 24 seconds.
    pub(super) fn record(k: &KickArgs, bpm: f32) -> Vec<f32> {
        let one = {
            let spec = kick_spec(&k.params()).expect("a kick");
            let mut node = spec.instantiate();
            let inputs = [0.0f32; MAX_NODE_INPUTS];
            (0..(SAMPLE_RATE as usize)).map(|_| node.process(&inputs, 0)).collect::<Vec<f32>>()
        };
        let beat = 60.0 / bpm * SAMPLE_RATE;
        let n = (24.0 * SAMPLE_RATE) as usize;
        let mut out = vec![0.0f32; n];
        let mut b = 0;
        loop {
            let at = (b as f32 * beat).round() as usize + 1000;
            if at >= n {
                break;
            }
            for (i, v) in one.iter().enumerate().take(beat as usize) {
                if at + i < n {
                    out[at + i] += 0.7 * v;
                }
            }
            // The bass, from half a beat on, on a note that changes.
            let hz = [55.0, 62.0, 49.0, 73.0][b % 4];
            // Faded in and out over 5 ms, as a played note is: a sine cut
            // dead at the hit would be a click of its own on every beat.
            let (from, to) = (beat as usize / 2, beat as usize);
            let fade = (0.005 * SAMPLE_RATE) as usize;
            for i in from..to {
                if at + i < n {
                    let g = ((i - from).min(to - 1 - i) as f32 / fade as f32).min(1.0);
                    out[at + i] += 0.25 * g * (std::f32::consts::TAU * hz * i as f32 / SAMPLE_RATE).sin();
                }
            }
            b += 1;
        }
        out
    }

    fn target_of(k: &KickArgs) -> Target {
        let x = record(k, 125.0);
        Target::new(crate::hit::average_hit(&x, 125.0, 230.0).expect("a hit"))
    }

    pub(super) fn truth() -> KickArgs {
        KickArgs {
            note: 46.25,
            start: 900.0,
            knee: 160.0,
            snap: 3.0,
            body: 75.0,
            chirp: 3500.0,
            chirp_time: 0.6,
            hold: 90.0,
            sag: -2.0,
            release: 200.0,
            phase: 20.0,
            click: -80.0,
            tone: 3000.0,
        }
    }

    #[test]
    fn the_truth_scores_near_zero() {
        let k = truth();
        let t = target_of(&k);
        let s = score(&t, &k.render(t.ms).expect("renders"));
        assert!(s.pitch_st < 0.15, "{s:?}");
        assert!(s.level_db < 0.6, "{s:?}");
    }

    #[test]
    fn a_kick_of_known_arguments_is_found_again() {
        let k = truth();
        let t = target_of(&k);
        let f = fit(&t, None, 1.0);
        let got = &f.args;
        assert!(f.score.pitch_st < 0.25, "pitch {:?} {}", f.score, got.line());
        assert!(f.score.attack_st < 2.0, "attack {:?} {}", f.score, got.line());
        assert!(f.score.level_db < 1.0, "level {:?} {}", f.score, got.line());
        let st = 12.0 * (got.note / k.note).log2();
        assert!(st.abs() < 0.15, "note {} against {}", got.note, k.note);
        assert!((got.hold - k.hold).abs() < 15.0, "hold {}", got.hold);
        assert!((got.body / k.body - 1.0).abs() < 0.2, "body {}", got.body);
    }

    #[test]
    fn the_click_is_found_by_its_level() {
        // Without a chirp: a chirp is itself over 2 kHz in the first
        // milliseconds, and a quiet click under it is not told apart.
        let mut k = truth();
        k.chirp = 0.0;
        k.start = 400.0;
        k.click = -30.0;
        let t = target_of(&k);
        let mut guess = k.clone();
        guess.click = -80.0;
        let found = fit_click(&t, &guess);
        assert!((found.click - k.click).abs() < 3.0, "click {} against {}", found.click, k.click);
    }

    #[test]
    fn the_line_is_a_kick_the_language_takes() {
        let k = truth().rounded();
        let line = k.line();
        assert!(line.starts_with("kick(F#1, start=900hz"), "{line}");
        assert!(line.contains("chirp=3.5khz"), "{line}");
        let src = format!(
            "instrument x {{\n  {line} as k\n  k > out\n}}\npattern p {{ C3 - - - }}\ntrack t {{ play p using x }}\n"
        );
        tatum_core::song_engine::SongEngine::from_source(&src).expect("compiles");
        // Rounded is a fixed point: what is printed is what is scored.
        assert_eq!(k.rounded(), k);
    }

    #[test]
    fn every_point_of_the_cube_is_a_kick() {
        let k = KickArgs::default();
        for i in 0..200 {
            let u: Vec<f32> = (0..ALL_DIMS.len()).map(|j| ((i * 7 + j * 13) % 17) as f32 / 16.0).collect();
            let c = with(&k, &ALL_DIMS, &u);
            assert!(kick_spec(&c.params()).is_ok(), "{c:?}");
        }
    }
}
