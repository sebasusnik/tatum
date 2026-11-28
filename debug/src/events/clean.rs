//! What should not be in a render: partials folded back from above Nyquist,
//! DC, steps in the wave, a modulation that moves in stairs, and something
//! still sounding where there should be silence. Each as a level under the
//! signal, with where it was found.

use super::inside::Ctx;
use super::sustained::Sustained;
use super::{db, Event, Stft, SR};
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
                "aliasing: {:.0} dB under the partial it comes from ({:.0} Hz at {:.2} s); loudest, harmonic {} \
                 folded to {:.0} Hz at {:.0} dB",
                a.db, a.f0, a.at_s, a.worst.0, a.worst.1, a.worst.2
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
                "zipper: a step every {} samples ({:.0} Hz), {:.0} dB under the signal",
                z.period,
                SR / z.period as f32,
                z.db
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
}

pub(crate) fn measure(c: &Ctx, events: &[Event], sustained: Option<&Sustained>, native: bool) -> Clean {
    let x = c.mono;
    let level = rms(x).max(1e-9);
    let mean = (x.iter().map(|v| *v as f64).sum::<f64>() / x.len().max(1) as f64).abs() as f32;
    let dc_db = 20.0 * (mean / level).max(1e-6).log10();
    let rest = residual(x);
    let onsets: Vec<(usize, usize)> = events
        .iter()
        .map(|e| {
            let s = (e.start_s * SR) as usize;
            (s.saturating_sub(ms(3.0)), s + ms(e.inside.attack_ms + 3.0))
        })
        .collect();
    // Folds are told from the rest by where they land, which in a mix is
    // where something else already is.
    let mut over: Vec<f32> = events.iter().map(|e| e.inside.over_bed_db).collect();
    over.sort_by(f32::total_cmp);
    let alone = over.get(over.len() / 2).is_none_or(|&m| m >= 30.0);
    let alias_unread = if !native {
        Some("the file was converted from another rate, which takes off what would fold")
    } else if !alone {
        Some("the events sit in a mix, where other sounds land on the same lines")
    } else {
        None
    };
    Clean {
        dc_db,
        alias: if alias_unread.is_none() { alias(c, events, sustained) } else { None },
        alias_unread,
        steps: steps(x, &rest, &onsets),
        zipper: zipper(&rest, x, level),
        buzz: buzz(x, level),
    }
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

/// Steps: a sample the predictor is wrong about by eight times more than
/// about any other within 3 ms, that is not a note starting (no more
/// energy after it than before it), nor at an event's own attack.
fn steps(x: &[f32], rest: &[f32], onsets: &[(usize, usize)]) -> Vec<Step> {
    let floor = 10f32.powf(-70.0 / 20.0);
    let (near, guard) = (ms(3.0), 14);
    let mut out: Vec<(usize, f32)> = Vec::new();
    let mut n = near;
    while n + near < rest.len() {
        let v = rest[n].abs();
        if v < floor {
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
        if rel < -50.0 {
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
        let k = p / 2;
        let typical = mid.select_nth_unstable_by(k, f32::total_cmp).1.max(1e-12);
        let contrast = top / typical;
        if contrast >= 4.0 && found.is_none_or(|f| contrast > 1.15 * f.1) {
            found = Some((p, contrast, top - typical));
        }
    }
    found.map(|(period, _, step)| Zipper { period, db: 20.0 * (step / level).max(1e-9).log10() })
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
/// that): a line that moves the wrong way when the note bends and sits on
/// no multiple of `f`. For every frame of every event the power where each
/// such fold would land is summed, less the power just beside it, so a bed
/// or the note's own splatter adds nothing on average.
fn alias(c: &Ctx, events: &[Event], sustained: Option<&Sustained>) -> Option<Alias> {
    let mut folds = Folds::default();
    {
        for e in events {
            let (a, b) =
                ((e.start_s * SR) as usize / c.stft.hop + 1, (e.inside.direct_end_s * SR) as usize / c.stft.hop);
            let hz_at = |u: usize| {
                let at = (u * c.stft.hop) as f32 * 1000.0 / SR - e.start_s * 1000.0;
                let p = &e.inside.pitch;
                match p.iter().position(|q| q.0 >= at) {
                    _ if p.is_empty() => e.inside.start_hz,
                    Some(0) => p[0].1,
                    None => p[p.len() - 1].1,
                    Some(i) => {
                        let u = (at - p[i - 1].0) / (p[i].0 - p[i - 1].0).max(1e-6);
                        p[i - 1].1 * (1.0 - u) + p[i].1 * u
                    }
                }
            };
            folds.add(c.stft, a, b.min(c.stft.frames().saturating_sub(1)), &hz_at, e.start_s);
        }
    }
    if let (Some(s), Some(f0)) = (sustained, sustained.and_then(|s| s.f0)) {
        let (a, b) = ((s.from_s * SR) as usize, ((s.to_s * SR) as usize).min(c.mono.len()));
        let st = Stft::new(&c.mono[a..b], 4096, 2048);
        folds.add(&st, 1, st.frames().saturating_sub(2), &|_| f0, s.from_s);
    }
    folds.result()
}

#[derive(Default)]
struct Folds {
    signal: f64,
    folded: f64,
    /// Per harmonic: what folded, and where it landed last.
    per: Vec<(f64, f32)>,
    f0: f32,
    at_s: f32,
    loudest_note: f64,
}

impl Folds {
    fn add(&mut self, s: &Stft, a: usize, b: usize, hz_at: &dyn Fn(usize) -> f32, at_s: f32) {
        let bin_hz = s.bin_hz();
        let three = |u: usize, b: usize| (b - 1..=b + 1).map(|k| s.at(u, k) as f64).sum::<f64>();
        let mut note = 0.0f64;
        let mut hz0 = 0.0;
        for u in a..=b.max(a).min(s.frames().saturating_sub(1)) {
            let f = hz_at(u);
            if !(40.0..SR * 0.4).contains(&f) {
                continue;
            }
            hz0 = f;
            let fb = s.bin(f).clamp(2, s.bins - 3);
            note += three(u, fb);
            let first = (SR / 2.0 / f).floor() as usize + 1;
            let last = ((2.5 * SR / f) as usize).min(150);
            for k in first..=last {
                let up = (k as f32 * f) % SR;
                let fold = if up > SR / 2.0 { SR - up } else { up };
                let near = (fold / f).round().max(1.0) * f;
                let fbin = (fold / bin_hz).round() as usize;
                if (fold - near).abs() < 5.0 * bin_hz || fbin < 10 || fbin + 10 >= s.bins {
                    continue;
                }
                let beside =
                    ((fbin - 8..=fbin - 5).chain(fbin + 5..=fbin + 8)).map(|k| s.at(u, k) as f64).sum::<f64>() / 8.0;
                let v = three(u, fbin) - 3.0 * beside;
                if self.per.len() <= k {
                    self.per.resize(k + 1, (0.0, 0.0));
                }
                self.per[k].0 += v;
                self.per[k].1 = fold;
                self.folded += v;
            }
        }
        self.signal += note;
        if note > self.loudest_note {
            self.loudest_note = note;
            self.f0 = hz0;
            self.at_s = at_s;
        }
    }

    fn result(&self) -> Option<Alias> {
        if self.signal <= 0.0 || self.folded <= 0.0 {
            return None;
        }
        let total = db((self.folded / self.signal) as f32);
        let (k, worst) = self.per.iter().enumerate().max_by(|a, b| a.1 .0.total_cmp(&b.1 .0))?;
        // An oscillator that folds, folds a run of harmonics: one line alone
        // where a fold would land is something else.
        let several = self.per.iter().filter(|p| p.0 >= 0.08 * self.folded).count() >= 3;
        (total > ALIAS_DB && several && worst.0 <= 0.6 * self.folded).then(|| Alias {
            db: total,
            worst: (k, worst.1, db((worst.0 / self.signal).max(1e-30) as f32)),
            f0: self.f0,
            at_s: self.at_s,
        })
    }
}
