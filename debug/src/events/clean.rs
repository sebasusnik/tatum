//! What should not be in a render: partials folded back from above Nyquist,
//! DC, steps in the wave, a modulation that moves in stairs, and something
//! still sounding where there should be silence. Each as a level under the
//! signal, with where it was found.

use super::inside::Ctx;
use super::sustained::Sustained;
use super::{db, Event, SR};
use crate::fft::fft;

#[derive(Clone, Debug)]
pub struct Alias {
    /// Folded partials against the partial they are harmonics of, dB.
    pub db: f32,
    /// The harmonic that folds loudest, where it lands in Hz, and its level, dB.
    pub worst: (usize, f32, f32),
    /// The note the harmonics are of, Hz, and when, seconds.
    pub f0: f32,
    pub at_s: f32,
    /// How many folded lines stand over their floor.
    pub lines: usize,
}

#[derive(Clone, Debug)]
pub struct Step {
    pub at_s: f32,
    /// The step against the level of the sound around it, dB.
    pub db: f32,
}

#[derive(Clone, Debug)]
pub struct Zipper {
    /// Samples between steps.
    pub period: usize,
    /// The steps against the level of the signal, dB.
    pub db: f32,
}

#[derive(Clone, Debug)]
pub struct Buzz {
    pub hz: f32,
    /// Against the level of the signal, dB.
    pub db: f32,
    pub at_s: f32,
}

#[derive(Clone, Debug, Default)]
pub struct Clean {
    /// The mean of the wave against its level, dB.
    pub dc_db: f32,
    /// `None`: nothing folds, or it could not be looked for (`alias_unread`).
    pub alias: Option<Alias>,
    /// Why folds were not looked for, when they were not.
    pub alias_unread: Option<&'static str>,
    /// When they were and none were found: how many fold positions were
    /// examined, and the level a fold would have had to reach to be seen, dB
    /// under the partial it comes from.
    pub alias_floor: Option<(usize, f32)>,
    pub steps: Vec<Step>,
    pub zipper: Option<Zipper>,
    pub buzz: Option<Buzz>,
}

/// Under these nothing is reported.
pub const DC_DB: f32 = -40.0;
pub const ALIAS_DB: f32 = -60.0;

impl Clean {
    /// One line per thing found; empty when the render is clean.
    pub fn findings(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(a) = &self.alias {
            out.push(format!(
                "aliasing: {} folded lines, together {:.0} dB under the partial they come from ({:.0} Hz at {:.2} s); \
                 loudest, harmonic {} folded to {:.0} Hz at {:.0} dB",
                a.lines, -a.db, a.f0, a.at_s, a.worst.0, a.worst.1, a.worst.2
            ));
        }
        if self.dc_db > DC_DB {
            out.push(format!("DC: the wave's mean is {:.0} dB under its level", self.dc_db));
        }
        if !self.steps.is_empty() {
            let at: Vec<String> =
                self.steps.iter().take(5).map(|s| format!("{:.3} s ({:.0} dB)", s.at_s, s.db)).collect();
            out.push(format!(
                "steps in the wave: {}, against the sound around each: {}",
                self.steps.len(),
                at.join(", ")
            ));
        }
        if let Some(z) = &self.zipper {
            out.push(format!(
                "zipper: a step every {} samples ({:.0} Hz), {:.0} dB {} the signal",
                z.period,
                SR / z.period as f32,
                z.db.abs(),
                if z.db <= 0.0 { "under" } else { "over" }
            ));
        }
        if let Some(b) = &self.buzz {
            out.push(format!(
                "in the quiet at {:.2} s: a tone at {:.0} Hz, {:.0} dB under the signal",
                b.at_s, b.hz, b.db
            ));
        }
        out
    }

    /// The findings, or what was looked for and not found, and what could
    /// not be looked at: never "nothing" where nothing was examined.
    pub fn lines(&self) -> Vec<String> {
        let mut out = self.findings();
        if out.is_empty() {
            out.push(format!(
                "nothing found: DC {:.0} dB under the signal, no steps, no zipper, nothing in the quiet",
                -self.dc_db
            ));
        }
        if self.alias.is_none() {
            match (self.alias_unread, self.alias_floor) {
                (Some(why), _) => out.push(format!("aliasing {why}")),
                (None, Some((n, floor))) => out.push(format!(
                    "aliasing: nothing folded back over {:.0} dB under its partial, at the {n} fold positions examined",
                    -floor
                )),
                (None, None) => out.push("aliasing not examined".into()),
            }
        }
        out
    }
}

pub(crate) fn measure(c: &Ctx, events: &[Event], sustained: Option<&Sustained>, native: bool) -> Clean {
    let x = c.mono;
    let level = rms(x).max(1e-9);
    let dc_db = dc(x, level);
    let rest = residual(x);
    let onsets: Vec<(usize, usize)> = events
        .iter()
        .map(|e| {
            let s = (e.start_s * SR) as usize;
            (s.saturating_sub(ms(3.0)), s + ms(e.inside.attack_ms.any().unwrap_or(0.0) + 3.0))
        })
        .collect();
    // Folds are told from the rest by where they land, which in a mix is
    // where something else already is.
    let mut over: Vec<f32> = events.iter().map(|e| e.inside.over_bed_db).collect();
    over.sort_by(f32::total_cmp);
    let alone = over.get(over.len() / 2).is_none_or(|&m| m >= 30.0);
    let mut alias_unread = if !native {
        Some("not examined: the file was converted from another rate, which takes off what would fold")
    } else if !alone {
        Some("not examined: the events stand under 30 dB over what is in their band (a mix, or notes that overlap), and other lines land where folds would")
    } else {
        None
    };
    let (found, examined) = if alias_unread.is_none() { alias(c, events, sustained) } else { (None, Err("")) };
    let alias_floor = match examined {
        Ok(v) => Some(v),
        Err(why) => {
            if alias_unread.is_none() && found.is_none() {
                alias_unread = Some(why);
            }
            None
        }
    };
    Clean {
        dc_db,
        alias: found,
        alias_unread,
        alias_floor,
        steps: steps(x, &bl_residual(x), &onsets),
        zipper: zipper(&rest, x, level),
        buzz: buzz(x, level),
    }
}

/// The wave's mean against its level, dB: the median of the means of every
/// tenth of a second, quiet ones included. An offset in the render is in
/// all of them; a low note that starts on one side of zero leaves a mean
/// only where it sounds, and in a file with any quiet in it that is not the
/// median.
fn dc(x: &[f32], level: f32) -> f32 {
    let win = ms(100.0).max(1);
    let mut means: Vec<f32> = x
        .chunks(win)
        .filter(|c| c.len() == win)
        .map(|c| (c.iter().map(|v| *v as f64).sum::<f64>() / c.len() as f64) as f32)
        .collect();
    let mean = if means.is_empty() {
        (x.iter().map(|v| *v as f64).sum::<f64>() / x.len().max(1) as f64) as f32
    } else {
        means.sort_by(f32::total_cmp);
        means[means.len() / 2]
    };
    20.0 * (mean.abs() / level).max(1e-6).log10()
}

fn ms(v: f32) -> usize {
    (v * 0.001 * SR) as usize
}

fn rms(x: &[f32]) -> f32 {
    if x.is_empty() {
        return 0.0;
    }
    (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len() as f64).sqrt() as f32
}

/// What a 12th-order linear predictor, fitted to each 4096 samples, cannot
/// tell from the samples before: a tone, however bright, is predicted and
/// leaves nothing; a step is not.
pub(crate) fn residual(x: &[f32]) -> Vec<f32> {
    const ORDER: usize = 12;
    const BLOCK: usize = 4096;
    let mut out = vec![0.0f32; x.len()];
    let mut at = 0;
    while at < x.len() {
        let end = (at + BLOCK).min(x.len());
        let seg = &x[at..end];
        let mut r = [0.0f64; ORDER + 1];
        for (k, v) in r.iter_mut().enumerate() {
            *v = seg.iter().zip(seg.iter().skip(k)).map(|(a, b)| *a as f64 * *b as f64).sum();
        }
        // A little white noise in the fit: a pure tone alone has no unique one.
        r[0] = r[0] * (1.0 + 1e-7) + 1e-12;
        let mut a = [0.0f64; ORDER + 1];
        a[0] = 1.0;
        let mut err = r[0];
        for i in 1..=ORDER {
            let k = -(0..i).map(|j| a[j] * r[i - j]).sum::<f64>() / err;
            let prev = a;
            for j in 1..=i {
                a[j] = prev[j] + k * prev[i - j];
            }
            err *= 1.0 - k * k;
            if err <= 0.0 {
                break;
            }
        }
        for n in at..end {
            let mut e = 0.0f64;
            for (k, c) in a.iter().enumerate() {
                if n >= k {
                    e += c * x[n - k] as f64;
                }
            }
            out[n] = e as f32;
        }
        at = end;
    }
    out
}

/// Half the taps of the interpolator `bl_residual` uses.
const TAPS: usize = 32;

/// What each sample is off from the band-limited interpolation of its
/// neighbours, `TAPS` either side: a least-squares fit to everything up to
/// 0.45 of the rate. Anything a render can hold below that -- however steep
/// its edges -- is interpolated to within -60 dB; a discontinuity is not.
pub(crate) fn bl_residual(x: &[f32]) -> Vec<f32> {
    let h = interpolator();
    let mut out = vec![0.0f32; x.len()];
    for n in TAPS..x.len().saturating_sub(TAPS) {
        let mut p = 0.0f64;
        for (j, c) in h.iter().enumerate() {
            p += c * (x[n - j - 1] as f64 + x[n + j + 1] as f64);
        }
        out[n] = (x[n] as f64 - p) as f32;
    }
    out
}

/// The taps: minimise the error of x[n] = sum h_j (x[n-j] + x[n+j]) over
/// 0..0.9 pi, by the normal equations.
fn interpolator() -> Vec<f64> {
    let w = 0.9 * std::f64::consts::PI;
    let cc = |j: f64, k: f64| -> f64 {
        // The integral over 0..w of cos(j t) cos(k t).
        let s = |m: f64| if m == 0.0 { w } else { (m * w).sin() / m };
        0.5 * (s(j - k) + s(j + k))
    };
    let m = TAPS;
    let mut g = vec![vec![0.0f64; m + 1]; m];
    for j in 0..m {
        for k in 0..m {
            g[j][k] = 4.0 * cc((j + 1) as f64, (k + 1) as f64) + if j == k { 1e-9 } else { 0.0 };
        }
        g[j][m] = 2.0 * ((j + 1) as f64 * w).sin() / (j + 1) as f64;
    }
    // Gaussian elimination with partial pivoting.
    for col in 0..m {
        let piv = (col..m).max_by(|&a, &b| g[a][col].abs().total_cmp(&g[b][col].abs())).unwrap_or(col);
        g.swap(col, piv);
        for r in 0..m {
            if r != col {
                let f = g[r][col] / g[col][col];
                for k in col..=m {
                    g[r][k] -= f * g[col][k];
                }
            }
        }
    }
    (0..m).map(|j| g[j][m] / g[j][j]).collect()
}

/// Steps: a sample the band-limited interpolation of its neighbours is
/// wrong about by eight times more than about any other within 3 ms, that
/// is not a note starting (no more energy after it than before it), not at
/// an event's own attack, and not the wave's own edge coming round again
/// every period.
fn steps(x: &[f32], rest: &[f32], onsets: &[(usize, usize)]) -> Vec<Step> {
    let floor = 10f32.powf(-70.0 / 20.0);
    let (near, guard) = (ms(3.0), TAPS + 2);
    let mut out: Vec<(usize, f32)> = Vec::new();
    let mut n = near.max(guard + 1);
    while n + near < rest.len() {
        let v = rest[n].abs();
        if v < floor {
            n += 1;
            continue;
        }
        // The interpolator's own taps spread a step over the samples around
        // it: the step is where the miss is largest.
        if rest[n - guard.min(n)..(n + guard + 1).min(rest.len())].iter().any(|r| r.abs() > v) {
            n += 1;
            continue;
        }
        let around = rest[n - near..n - guard.min(near)]
            .iter()
            .chain(&rest[n + guard.min(near)..n + near])
            .fold(0.0f32, |m, r| m.max(r.abs()));
        if v < 8.0 * around || onsets.iter().any(|&(a, b)| n >= a && n <= b) {
            n += 1;
            continue;
        }
        let w = ms(12.0);
        let before = rms(&x[n.saturating_sub(w)..n.saturating_sub(4)]);
        let after = rms(&x[(n + 4).min(x.len())..(n + w).min(x.len())]);
        // A note starting gains level; a step in a sound leaves it as it was.
        if after > 1.4 * before || before < 1e-5 {
            n += 1;
            continue;
        }
        let rel = 20.0 * (v / before.max(after).max(1e-9)).log10();
        if rel < -50.0 || recurs(x, rest, n, v) {
            n += 1;
            continue;
        }
        out.push((n, rel));
        n += ms(5.0);
    }
    out.sort_by(|a, b| b.1.total_cmp(&a.1));
    out.truncate(50);
    out.iter().map(|&(n, d)| Step { at_s: n as f32 / SR, db: d }).collect()
}

/// Whether what the interpolation misses at `n` comes back a period of the
/// wave later or earlier: the edge of a naive oscillator, which aliasing
/// reports, not a step.
fn recurs(x: &[f32], rest: &[f32], n: usize, v: f32) -> bool {
    let w = ms(30.0);
    let (a, b) = (n.saturating_sub(w), (n + w).min(x.len()));
    let seg = &x[a..b];
    let energy: f64 = seg.iter().map(|v| (*v as f64).powi(2)).sum();
    if energy <= 0.0 {
        return false;
    }
    let (lo, hi) = (ms(0.5).max(2), ms(25.0).min(seg.len() / 2));
    let mut best = (0usize, 0.0f64);
    for lag in lo..hi {
        let c: f64 = seg.iter().zip(&seg[lag..]).map(|(p, q)| (*p as f64) * (*q as f64)).sum();
        let norm = c / energy * seg.len() as f64 / (seg.len() - lag) as f64;
        if norm > best.1 {
            best = (lag, norm);
        }
    }
    if best.1 < 0.8 {
        return false;
    }
    let p = best.0;
    let peak = |m: isize| -> f32 {
        if m < 2 || m as usize + 3 >= rest.len() {
            return 0.0;
        }
        let m = m as usize;
        rest[m - 2..=m + 2].iter().fold(0.0f32, |acc, r| acc.max(r.abs()))
    };
    let n = n as isize;
    let p = p as isize;
    [n - p, n + p, n - 2 * p, n + 2 * p].iter().any(|&m| peak(m) >= 0.5 * v)
}

/// A modulation worked out once a block and held: the predictor is wrong at
/// the same place in every block. Looked for at the block sizes an engine
/// would use; the period is the shortest that shows it.
fn zipper(rest: &[f32], x: &[f32], level: f32) -> Option<Zipper> {
    let mut periods: Vec<usize> = vec![16, 32, 64, 128, 256, 512, 1024, 2048];
    periods.push(tatum_core::BLOCK_SIZE);
    periods.sort_unstable();
    periods.dedup();
    let gate = level * 0.03;
    // Only where there is sound, by hundredths of a second.
    let win = ms(10.0).max(1);
    let sounding: Vec<bool> = x.chunks(win).map(|c| rms(c) > gate).collect();
    let mut found: Option<(usize, f32, f32)> = None;
    for &p in &periods {
        // What the predictor misses at each place in the block, in the
        // middle block of all that sound: a note starting lands on one
        // place once, a held modulation steps there every time.
        let mut at: Vec<Vec<f32>> = vec![Vec::new(); p];
        for (n, r) in rest.iter().enumerate() {
            if sounding[n / win] {
                at[n % p].push(r.abs());
            }
        }
        if at.iter().any(|v| v.len() < 20) {
            continue;
        }
        let mut mid: Vec<f32> = at
            .iter_mut()
            .map(|v| {
                let k = v.len() / 2;
                *v.select_nth_unstable_by(k, f32::total_cmp).1
            })
            .collect();
        let top = mid.iter().fold(0.0f32, |m, v| m.max(*v));
        // A wave whose own period is the block's, or a few blocks', repeats
        // its miss on the same places in the block.
        if (1..=8).any(|m| periodic_at(x, &sounding, win, m * p)) {
            continue;
        }
        let k = p / 2;
        let typical = mid.select_nth_unstable_by(k, f32::total_cmp).1.max(1e-12);
        let contrast = top / typical;
        if contrast >= 4.0 && found.is_none_or(|f| contrast > 1.15 * f.1) {
            found = Some((p, contrast, top - typical));
        }
    }
    found.map(|(period, _, step)| Zipper { period, db: 20.0 * (step / level).max(1e-9).log10() })
}

/// Whether the sounding wave repeats itself every `p` samples.
fn periodic_at(x: &[f32], sounding: &[bool], win: usize, p: usize) -> bool {
    let (mut c, mut e) = (0.0f64, 0.0f64);
    for n in 0..x.len().saturating_sub(p) {
        if sounding[n / win] {
            c += x[n] as f64 * x[n + p] as f64;
            e += (x[n] as f64).powi(2);
        }
    }
    e > 0.0 && c / e > 0.95
}

/// The quietest tenth of a second, when it is 60 dB under the signal and not
/// digital silence: a line standing 15 dB over the rest of its spectrum.
fn buzz(x: &[f32], level: f32) -> Option<Buzz> {
    let win = 4096;
    if x.len() < 2 * win {
        return None;
    }
    let (at, quiet) =
        (0..x.len() - win).step_by(win / 2).map(|a| (a, rms(&x[a..a + win]))).min_by(|a, b| a.1.total_cmp(&b.1))?;
    if quiet <= 0.0 || quiet > level * 1e-3 {
        return None;
    }
    let mut re: Vec<f64> = (0..win)
        .map(|i| x[at + i] as f64 * (0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / win as f64).cos()))
        .collect();
    let mut im = vec![0.0f64; win];
    fft(&mut re, &mut im);
    let p: Vec<f64> = (2..win / 2).map(|b| re[b] * re[b] + im[b] * im[b]).collect();
    let (top, peak) = p.iter().enumerate().fold((0, 0.0f64), |m, (i, v)| if *v > m.1 { (i, *v) } else { m });
    let mut sorted = p.clone();
    sorted.sort_by(f64::total_cmp);
    let typical = sorted[sorted.len() / 2].max(1e-30);
    (peak > 30.0 * typical).then(|| Buzz {
        hz: (top + 2) as f32 * SR / win as f32,
        db: 20.0 * (quiet / level).log10(),
        at_s: at as f32 / SR,
    })
}

/// Folded partials. A harmonic `k` of a note at `f` that the oscillator
/// makes above Nyquist comes back at `rate - k f` (and folds again past
/// that): every such line sits `rate mod f` off the note's own harmonics. A
/// note is examined when it holds steady long enough for a spectrum to put
/// a few bins between the two, and has harmonics of its own to fold; each
/// fold position counts as a line when it stands 10 dB over the bins around
/// it. An oscillator that folds puts lines on most of them; six are needed.
fn alias(c: &Ctx, events: &[Event], sustained: Option<&Sustained>) -> (Option<Alias>, Result<(usize, f32), &'static str>) {
    let mut notes: Vec<(usize, usize, f32, f32)> = Vec::new();
    for e in events {
        let (Some(_), true) = (e.inside.start.value(), e.inside.overlap.is_none()) else { continue };
        if e.inside.bend_st.abs() > 0.1 {
            continue;
        }
        let a = (e.start_s * SR) as usize + ms(e.inside.attack_ms.any().unwrap_or(0.0) + 2.0);
        let b = ((e.inside.direct_end_s * SR) as usize).min(c.mono.len());
        if b > a {
            notes.push((a, b, e.inside.start_hz, e.start_s));
        }
    }
    if let Some(s) = sustained {
        let (a, b) = ((s.from_s * SR) as usize, ((s.to_s * SR) as usize).min(c.mono.len()));
        if b > a {
            notes.push((a, b, s.f0.unwrap_or(0.0), s.from_s));
        }
    }
    let mut why = "not examined: no note steady and alone long enough";
    let (mut examined, mut floors, mut signal, mut folded, mut lines) = (0usize, Vec::new(), 0.0f64, 0.0f64, 0usize);
    let mut worst = (0usize, 0.0f32, 0.0f64);
    let mut loudest = (0.0f64, 0.0f32, 0.0f32);
    for (a, b, hint, at_s) in notes {
        match fold_lines(&c.mono[a..b], hint) {
            Err(w) => why = w,
            Ok(n) => {
                examined += n.examined;
                floors.push(n.floor_db);
                if n.lines >= 6 && n.has_harmonics {
                    signal += n.fundamental;
                    folded += n.folded;
                    lines = lines.max(n.lines);
                    if n.worst.2 > worst.2 {
                        worst = n.worst;
                    }
                    if n.fundamental > loudest.0 {
                        loudest = (n.fundamental, n.f0, at_s);
                    }
                }
            }
        }
    }
    if examined == 0 {
        return (None, Err(why));
    }
    let mut floors = floors;
    floors.sort_by(f32::total_cmp);
    let floor = floors[floors.len() / 2];
    if signal <= 0.0 || folded <= 0.0 {
        return (None, Ok((examined, floor)));
    }
    let total = db((folded / signal) as f32);
    if total <= ALIAS_DB {
        return (None, Ok((examined, floor)));
    }
    (
        Some(Alias {
            db: total,
            worst: (worst.0, worst.1, db((worst.2 / loudest.0.max(1e-30)) as f32)),
            f0: loudest.1,
            at_s: loudest.2,
            lines,
        }),
        Ok((examined, floor)),
    )
}

struct NoteFolds {
    examined: usize,
    lines: usize,
    /// Power at the note and summed over the lines, and the loudest line:
    /// (harmonic, where it lands, its power).
    fundamental: f64,
    folded: f64,
    worst: (usize, f32, f64),
    f0: f32,
    /// What a line would have needed to be seen, dB under the note.
    floor_db: f32,
    /// A harmonic between the 2nd and the 5th within 50 dB of the note:
    /// something to fold.
    has_harmonics: bool,
}

fn fold_lines(x: &[f32], hint: f32) -> Result<NoteFolds, &'static str> {
    if x.len() < 4096 {
        return Err("not examined: no note steady and alone long enough to resolve its folds");
    }
    // A first spectrum to find the note: the strongest peak near the hint, or
    // the strongest of all.
    let n0 = (x.len().next_power_of_two() / 2).clamp(4096, 65536);
    let spec = |n: usize| -> Vec<f64> {
        let (mut re, mut im) = (vec![0.0f64; n], vec![0.0f64; n]);
        for i in 0..n.min(x.len()) {
            re[i] = x[i] as f64 * (0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos());
        }
        fft(&mut re, &mut im);
        (0..n / 2).map(|k| re[k] * re[k] + im[k] * im[k]).collect()
    };
    let p = spec(n0);
    let bin = SR / n0 as f32;
    let peak_near = |p: &[f64], hz: f32, reach: f32, bin: f32| -> Option<f32> {
        let (a, b) = (((hz - reach) / bin).floor().max(2.0) as usize, ((hz + reach) / bin).ceil() as usize);
        let b = b.min(p.len() - 2);
        if a >= b {
            return None;
        }
        let k = (a..=b).max_by(|&i, &j| p[i].total_cmp(&p[j]))?;
        let (l, c, r) = (p[k - 1].max(1e-30).ln(), p[k].max(1e-30).ln(), p[k + 1].max(1e-30).ln());
        let d = if 2.0 * c - l - r > 1e-12 { (0.5 * (l - r) / (l - 2.0 * c + r)).clamp(-0.5, 0.5) } else { 0.0 };
        Some((k as f32 + d as f32) * bin)
    };
    let strongest = (2..p.len() - 2).max_by(|&i, &j| p[i].total_cmp(&p[j])).unwrap_or(2) as f32 * bin;
    let guess = if hint > 20.0 { hint } else { strongest };
    let mut f = peak_near(&p, guess, (0.03 * guess).max(2.0 * bin), bin).ok_or("not examined: no note found")?;
    // Taken up the harmonics, each read to its own bin: k f to a bin is f to
    // a k-th of one.
    let mut k = 2usize;
    while (k as f32) * f < 0.45 * SR && k <= 200 {
        let at = k as f32 * f;
        match peak_near(&p, at, (2.0 * bin).max(0.002 * at), bin) {
            Some(hz) if p[(hz / bin).round() as usize] > 1e-8 * p[(f / bin).round() as usize] => f = hz / k as f32,
            _ => {}
        }
        k += 1;
    }
    if f < 20.0 {
        return Err("not examined: no note found");
    }
    let r = SR % f;
    let off = r.min(f - r);
    // Four bins between a fold and the nearest harmonic.
    let need = ((4.0 * SR / off.max(1e-3)) as usize).next_power_of_two().max(4096);
    if need > 65536 || need > x.len() {
        return Err(if need > 65536 {
            "not examined: at this pitch its folds land on its own harmonics"
        } else {
            "not examined: no note held long enough to resolve its folds from its harmonics"
        });
    }
    let p = if need == n0 { p } else { spec(need) };
    let bin = SR / need as f32;
    let at = |hz: f32| (hz / bin).round() as usize;
    let three = |k: usize| p[k - 1..=k + 1].iter().sum::<f64>();
    let fundamental = three(at(f).clamp(1, p.len() - 2));
    let has_harmonics = (2..=5).any(|h| {
        let hz = h as f32 * f;
        hz < 0.45 * SR && three(at(hz).clamp(1, p.len() - 2)) > 1e-5 * fundamental
    });
    let local = |k: usize| -> f64 {
        let mut v: Vec<f64> = (k.saturating_sub(24)..(k + 25).min(p.len())).filter(|&j| j.abs_diff(k) > 2).map(|j| p[j]).collect();
        let mid = v.len() / 2;
        *v.select_nth_unstable_by(mid, f64::total_cmp).1
    };
    let first = (SR / 2.0 / f).floor() as usize + 1;
    let (mut examined, mut lines, mut folded) = (0usize, 0usize, 0.0f64);
    let mut worst = (0usize, 0.0f32, 0.0f64);
    let mut floors = Vec::new();
    for k in first..first + 300 {
        let up = (k as f32 * f) % SR;
        let fold = if up > SR / 2.0 { SR - up } else { up };
        let kb = at(fold);
        let harmonic = (fold / f).round() * f;
        if kb < 30 || kb + 30 >= p.len() || (fold - harmonic).abs() < 3.0 * bin {
            continue;
        }
        examined += 1;
        let floor = local(kb).max(1e-30);
        floors.push(floor);
        let line = three(kb);
        if line > 3.0 * 10.0 * floor {
            lines += 1;
            let v = line - 3.0 * floor;
            folded += v;
            if v > worst.2 {
                worst = (k, fold, v);
            }
        }
    }
    if examined == 0 {
        return Err("not examined: its folds land outside what the spectrum resolves");
    }
    floors.sort_by(f64::total_cmp);
    let floor_db = db((10.0 * 3.0 * floors[floors.len() / 2] / fundamental.max(1e-30)) as f32);
    Ok(NoteFolds { examined, lines, fundamental, folded, worst, f0: f, floor_db, has_harmonics })
}
