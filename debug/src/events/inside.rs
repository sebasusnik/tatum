//! One event read from the inside: the audio demodulated by the event's own
//! partial, which leaves its level and pitch at every sample.
//!
//! Every figure is a [`Fig`]: a value where the estimator is right within
//! what the docs state, a bound where the event cannot be told from the bed
//! under it or from its own decay's skirt, and nothing, with the reason,
//! outside the estimator's domain.

use super::{db, Delay, Event, Floor, Ridge, Stft, SR};
use crate::fft::fft;

/// A measured figure, or why there is not one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fig {
    Value(f32),
    /// The truth is at most this: what is there cannot be told from what
    /// the bed and the event's own skirt put there.
    AtMost(f32),
    /// The truth is at least this.
    AtLeast(f32),
    /// Not measurable here, and why.
    No(&'static str),
}

impl Fig {
    pub fn value(self) -> Option<f32> {
        match self {
            Fig::Value(v) => Some(v),
            _ => None,
        }
    }

    /// A value, or `No(why)` for none.
    pub fn or(v: Option<f32>, why: &'static str) -> Fig {
        v.filter(|v| v.is_finite()).map_or(Fig::No(why), Fig::Value)
    }

    /// Through a function that keeps the order.
    pub fn map(self, f: impl Fn(f32) -> f32) -> Fig {
        match self {
            Fig::Value(v) => Fig::Value(f(v)),
            Fig::AtMost(v) => Fig::AtMost(f(v)),
            Fig::AtLeast(v) => Fig::AtLeast(f(v)),
            no => no,
        }
    }

    /// Through a function that turns the order round (a rate into a time).
    pub fn map_down(self, f: impl Fn(f32) -> f32) -> Fig {
        match self {
            Fig::Value(v) => Fig::Value(f(v)),
            Fig::AtMost(v) => Fig::AtLeast(f(v)),
            Fig::AtLeast(v) => Fig::AtMost(f(v)),
            no => no,
        }
    }

    /// The number, whether a value or a bound.
    pub fn any(self) -> Option<f32> {
        match self {
            Fig::Value(v) | Fig::AtMost(v) | Fig::AtLeast(v) => Some(v),
            Fig::No(_) => None,
        }
    }

    /// A value or a bound becomes `No(why)`; an earlier reason stays.
    pub fn withhold(self, why: &'static str) -> Fig {
        match self {
            Fig::Value(_) | Fig::AtMost(_) | Fig::AtLeast(_) => Fig::No(why),
            no => no,
        }
    }
}

/// How the pitch gets from its start to its end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Flat,
    Straight,
    /// Most of the bend early: an exponential towards the end note.
    FastThenSlow,
    SlowThenFast,
    /// Up and back.
    Arch,
    /// Down and back.
    Dip,
}

impl Shape {
    pub fn word(self) -> &'static str {
        match self {
            Shape::Flat => "flat",
            Shape::Straight => "straight",
            Shape::FastThenSlow => "fast then slow",
            Shape::SlowThenFast => "slow then fast",
            Shape::Arch => "arch",
            Shape::Dip => "dip",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Inside {
    /// From 10% to 90% of the peak, over what was there before, in a band
    /// an octave down and two up from the partial.
    pub attack_ms: Fig,
    /// What starts above the resonance (from 1.5 times its frequency) in the
    /// first 5 ms, beyond what the partial's own onset puts there, against
    /// the resonance in those 5 ms, dB.
    pub burst_db: Fig,
    /// From the peak to 20 and 40 dB under it.
    pub t20_ms: Fig,
    pub t40_ms: Fig,
    /// dB lost per millisecond over the first 30 dB of the fall, and how
    /// straight that fall is in dB (r squared): 1 is an exponential.
    pub decay_db_ms: Fig,
    pub decay_r2: Fig,
    /// The pitch as read, for the sheet and the other checks.
    pub start_hz: f32,
    pub end_hz: f32,
    pub bend_st: f32,
    /// The same as figures: a start extrapolated past the fastest sweep
    /// followed is a bound, a note of too few cycles has none.
    pub start: Fig,
    pub end: Fig,
    pub bend: Fig,
    pub shape: Shape,
    /// A vibrato on the partial: its rate in Hz and depth in cents, peak to peak.
    pub wobble: Option<(f32, f32)>,
    /// The -3 dB width of the partial with its glide taken out, and f / width.
    pub bandwidth_hz: Fig,
    pub q: Fig,
    /// The Q a resonator would need to ring with this decay: pi f tau.
    pub ring_q: Fig,
    /// The partial against what else the event adds -- the bed and the
    /// partial's own skirt taken off -- dB.
    pub tonal_db: Fig,
    /// Second and third harmonics against the partial, the bed and the
    /// partial's skirt taken off, dB.
    pub h2_db: Fig,
    pub h3_db: Fig,
    /// The partial at its peak over what was in its band just before, dB.
    pub over_bed_db: f32,
    /// The partial at its peak against the mix over the second around it,
    /// its own stretch left out of the mix.
    pub in_mix_db: Fig,
    /// Left over right in the partial's band, dB.
    pub pan_db: f32,
    /// Side over mid in the partial's band, dB: the width.
    pub side_mid_db: Fig,
    /// The correlation of left and right in the partial's band: the cosine
    /// of the phase between them times how alike they are.
    pub corr: Fig,
    /// Energy in the partial's band from 50 to 400 ms after the fitted decay
    /// is 20 dB down, beyond what that decay itself still leaves, against
    /// the event, dB. `tail_clear`: no other event near its pitch started then.
    pub tail_db: Fig,
    pub tail_clear: bool,
    /// When the fitted decay is 20 dB down (or the event ended), seconds.
    pub direct_end_s: f32,
    /// The centroid of what the event adds over the bed, at its peak, and how
    /// far it has moved when the partial is 12 dB down, semitones.
    pub centroid_hz: Fig,
    pub centroid_move_st: Fig,
    /// Why the event is not alone, when it is not: the figures that need it
    /// alone are withheld.
    pub overlap: Option<&'static str>,
    /// The partial moved faster than the band that follows it.
    pub lost: bool,
    /// The extent the spectral figures were read over, seconds: other events
    /// in it make the event not alone.
    pub(crate) span_s: (f32, f32),
    /// Level against the peak, dB, every millisecond from 10 ms before the start.
    pub level: Vec<f32>,
    /// (ms from the start, Hz).
    pub pitch: Vec<(f32, f32)>,
}

pub(crate) fn ifft(re: &mut [f64], im: &mut [f64]) {
    for v in im.iter_mut() {
        *v = -*v;
    }
    fft(re, im);
    let n = re.len() as f64;
    for (r, i) in re.iter_mut().zip(im.iter_mut()) {
        *r /= n;
        *i = -*i / n;
    }
}

fn mag(z: (f32, f32)) -> f32 {
    (z.0 * z.0 + z.1 * z.1).sqrt()
}

/// `x` between `lo` and `hi` Hz as an analytic signal: its magnitude is the
/// level of what is in the band, with no ripple at the carrier. The edges
/// roll off over a sixth of an octave.
pub(crate) fn band(x: &[f32], lo: f32, hi: f32) -> Vec<(f32, f32)> {
    let n = x.len().next_power_of_two().max(2);
    let (mut re, mut im) = (vec![0.0f64; n], vec![0.0f64; n]);
    for (a, v) in re.iter_mut().zip(x) {
        *a = *v as f64;
    }
    fft(&mut re, &mut im);
    let edge = 2f32.powf(1.0 / 6.0);
    for k in 0..n {
        let hz = k as f32 * SR / n as f32;
        let m = if k > n / 2 || hz < lo / edge || hz > hi * edge {
            0.0
        } else if hz < lo {
            (0.5 * std::f32::consts::PI * (hz * edge / lo).ln() / edge.ln()).sin().powi(2)
        } else if hz > hi {
            (0.5 * std::f32::consts::PI * (hi * edge / hz).ln() / edge.ln()).sin().powi(2)
        } else {
            1.0
        };
        re[k] *= 2.0 * m as f64;
        im[k] *= 2.0 * m as f64;
    }
    ifft(&mut re, &mut im);
    (0..x.len()).map(|i| (re[i] as f32, im[i] as f32)).collect()
}

/// Left and right from sample `from`, each multiplied by a tone that follows
/// the partial and low-passed at `cutoff`: the partial brought to 0 Hz.
pub(crate) struct Tracked {
    pub l: Vec<(f32, f32)>,
    pub r: Vec<(f32, f32)>,
    /// The tone's frequency at each sample.
    pub hz: Vec<f32>,
    /// -1 when the channels are read on their difference.
    pub sign: f32,
}

impl Tracked {
    pub fn mono(&self, i: usize) -> (f32, f32) {
        let k = self.sign;
        (0.5 * (self.l[i].0 + k * self.r[i].0), 0.5 * (self.l[i].1 + k * self.r[i].1))
    }

    pub fn level(&self, i: usize) -> f32 {
        mag(self.mono(i))
    }
}

/// `x` times a tone (cosine and sine per sample), low-passed at `cutoff` Hz
/// with an edge half as wide again: what was at the tone's frequency, at 0 Hz.
pub(crate) fn lowpassed(x: &[f32], tone: &[(f64, f64)], cutoff: f32) -> Vec<(f32, f32)> {
    let len = x.len();
    let n = len.next_power_of_two().max(2);
    let (mut re, mut im) = (vec![0.0f64; n], vec![0.0f64; n]);
    for i in 0..len {
        let v = 2.0 * x[i] as f64;
        re[i] = v * tone[i].0;
        im[i] = -v * tone[i].1;
    }
    fft(&mut re, &mut im);
    for k in 0..n {
        let f = (if k <= n / 2 { k as f32 } else { k as f32 - n as f32 }) * SR / n as f32;
        let m = if f.abs() <= cutoff {
            1.0
        } else if f.abs() < 1.5 * cutoff {
            (std::f32::consts::PI * (f.abs() - cutoff) / cutoff).cos().powi(2)
        } else {
            0.0
        };
        re[k] *= m as f64;
        im[k] *= m as f64;
    }
    ifft(&mut re, &mut im);
    (0..len).map(|i| (re[i] as f32, im[i] as f32)).collect()
}

/// Both channels from `from`, demodulated by a tone at `path` Hz, one value
/// per sample from `from`.
pub(crate) fn track_path(c: &Ctx, path: &[f32], from: usize, cutoff: f32) -> Tracked {
    let len = path.len();
    let mut tone = Vec::with_capacity(len);
    let mut phase = 0.0f64;
    for &f in path {
        phase += std::f64::consts::TAU * f as f64 / SR as f64;
        if phase > std::f64::consts::TAU {
            phase -= std::f64::consts::TAU;
        }
        tone.push((phase.cos(), phase.sin()));
    }
    let run = |x: &[f32]| lowpassed(&x[from..from + len], &tone, cutoff);
    Tracked { l: run(c.left), r: run(c.right), hz: path.to_vec(), sign: c.sign }
}

/// `shift` moves the ridge later by that many samples: where an echo would be.
#[allow(clippy::too_many_arguments)]
pub(crate) fn track(
    c: &Ctx,
    ridge: &Ridge,
    hop: usize,
    shift: f64,
    from: usize,
    to: usize,
    cutoff: f32,
) -> Tracked {
    let path: Vec<f32> = (from..to).map(|n| ridge.hz_at(n as f64 - shift, hop)).collect();
    track_path(c, &path, from, cutoff)
}

/// How wide the low-pass around a partial at `hz` is: about two semitones
/// either side, so a ping a third away is out and a 5 ms attack still shows.
pub(crate) fn cutoff_for(hz: f32) -> f32 {
    (0.12 * hz).clamp(30.0, 500.0)
}

/// The fastest decay the band around a partial at `hz` follows, dB/ms: the
/// decay's own width (1 / (2 pi tau)) a third of the low-pass. A faster one
/// reads slower than it is.
pub fn fastest_decay(hz: f32) -> f32 {
    let tau_s = 3.0 / (std::f32::consts::TAU * cutoff_for(hz));
    8.686 / (tau_s * 1000.0)
}

/// The shortest attack the attack band resolves at `hz`, ms: eight tenths of
/// a period, and never under 0.3 ms.
pub fn attack_floor_ms(hz: f32) -> f32 {
    (800.0 / hz.max(1.0)).max(0.3)
}

/// The fastest pitch sweep the start pitch is read from, as the share of
/// its frequency the pitch moves in one cycle.
pub const FASTEST_SWEEP: f32 = 0.1;

pub(crate) struct Ctx<'a> {
    pub left: &'a [f32],
    pub right: &'a [f32],
    /// -1 when left and right cancel and are read on their difference.
    pub sign: f32,
    pub mono: &'a [f32],
    pub stft: &'a Stft,
    pub floor: &'a Floor,
}

fn median(v: &mut [f32]) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f32::total_cmp);
    v[v.len() / 2]
}

fn rms(x: &[f32]) -> f32 {
    if x.is_empty() {
        return 0.0;
    }
    (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len() as f64).sqrt() as f32
}

fn samples(ms: f32) -> usize {
    (ms * 0.001 * SR) as usize
}

fn to_ms(n: usize) -> f32 {
    n as f32 * 1000.0 / SR
}

/// Of the energy of a partial whose envelope rises in a straight line over
/// `ramp` seconds and falls as exp(-t / tau), the share at offsets from it
/// between `lo` and `hi` Hz, both sides counted. Its spectrum is
/// (1 - exp(-s ramp)) / (ramp s^2), s = 1 / tau + 2 pi i f; integrated in
/// the angle atan(2 pi f tau), where the decay's own Lorentzian is flat.
pub(crate) fn skirt(tau: f64, ramp: f64, lo: f64, hi: f64) -> f64 {
    let tau = tau.max(1e-5);
    let density = |theta: f64| -> f64 {
        let f = theta.tan() / (std::f64::consts::TAU * tau);
        if ramp < 1e-7 {
            return 1.0;
        }
        let (a, w) = (ramp / tau, std::f64::consts::TAU * f * ramp);
        let num = 1.0 - 2.0 * (-a).exp() * w.cos() + (-2.0 * a).exp();
        let s2 = (1.0 / (tau * tau) + (std::f64::consts::TAU * f).powi(2)) * ramp * ramp;
        num / s2
    };
    let part = |lo: f64, hi: f64, steps: usize| -> f64 {
        let (t0, t1) = ((std::f64::consts::TAU * lo * tau).atan(), (std::f64::consts::TAU * hi * tau).atan());
        let h = (t1 - t0) / steps as f64;
        (0..steps).map(|i| density(t0 + (i as f64 + 0.5) * h)).sum::<f64>() * h
    };
    let total = part(0.0, 1e9, 6000);
    (part(lo.max(0.0), hi.max(lo), 1500) / total.max(1e-30)).clamp(0.0, 1.0)
}

/// The bed's spectrum just before an event: per bin its mean power and how
/// far one frame strays from it.
struct Bed {
    mean: Vec<f64>,
    spread: Vec<f64>,
    frames: usize,
}

/// The frames whose window ends before sample `before`, up to 0.6 s of them.
fn bed_spectrum(s: &Stft, before: usize) -> Option<Bed> {
    let last = (before.checked_sub(s.size / 2)? / s.hop).min(s.frames() - 1);
    let count = ((0.6 / s.frame_s()) as usize).max(8).min(last + 1);
    if count < 6 {
        return None;
    }
    let first = last + 1 - count;
    let mut mean = vec![0.0f64; s.bins];
    let mut sq = vec![0.0f64; s.bins];
    for u in first..=last {
        for (b, p) in s.row(u).iter().enumerate() {
            mean[b] += *p as f64;
            sq[b] += (*p as f64).powi(2);
        }
    }
    let n = count as f64;
    let spread = mean
        .iter_mut()
        .zip(&sq)
        .map(|(m, q)| {
            *m /= n;
            // The spread of a few correlated frames reads low: half as much again.
            (1.5 * (q / n - *m * *m).max(0.0).sqrt()).max(0.05 * *m)
        })
        .collect();
    Some(Bed { mean, spread, frames: count })
}

/// A sum of `(power - bed)` over frames and bins, with the variance a
/// stationary bed would give it.
#[derive(Default, Clone, Copy)]
struct Excess {
    sum: f64,
    var: f64,
}

impl Excess {
    /// Three standard deviations of what the bed alone would leave: frames
    /// overlap eight to one and neighbouring bins share a third of their
    /// power (7.5), and the bed's own mean is read from `m` frames.
    fn floor(&self, n: usize, m: usize) -> f64 {
        3.0 * (7.5 * self.var * (1.0 + n as f64 / m.max(1) as f64)).sqrt()
    }
}

/// A share read against what cannot be told from it: `Value` when it stands
/// twice its uncertainty clear, a bound otherwise. `excess` and `unsure` in
/// the same unit as `of`.
fn share_db(excess: f64, unsure: f64, of: f64) -> Fig {
    if of <= 0.0 {
        return Fig::No("its partial does not stand over the bed");
    }
    if excess > 2.0 * unsure {
        Fig::Value(db((excess / of) as f32))
    } else {
        Fig::AtMost(db(((excess.max(0.0) + unsure) / of) as f32))
    }
}

/// Read the event on `ridge`. `None` when nothing rises there after all.
/// `onsets` are every event's first frame, in samples, and its frequency:
/// the next one near this partial ends what is read of it.
pub(crate) fn read(c: &Ctx, ridge: &Ridge, over_floor_db: f32, onsets: &[(usize, f32)]) -> Option<Event> {
    let (s, hop) = (c.stft, c.stft.hop);
    let total = c.mono.len();
    // One frame off its neighbours is a peak picked wrong.
    let mut ridge = ridge.clone();
    let raw = ridge.hz.clone();
    for i in 1..raw.len().saturating_sub(1) {
        ridge.hz[i] = median(&mut [raw[i - 1], raw[i], raw[i + 1]]);
    }
    let centre = ridge.frame * hop;
    let ridge_len = ridge.hz.len() * hop;
    let f_on = median(&mut ridge.hz[..ridge.hz.len().min(3)].to_vec());
    let f_mid = median(&mut ridge.hz.clone());
    let cutoff = cutoff_for(f_mid);
    // The flux peaks with the onset in the first half of its window.
    let early = centre.saturating_sub(s.size / 4);
    // The next event's rise: near this partial it ends the level read; any
    // other ends the stretch the pitch is read from in a wide band.
    let rise = |n: usize| n.saturating_sub(s.size / 4);
    let later = |near: bool| {
        onsets
            .iter()
            .filter(|&&(n, hz)| {
                rise(n) > centre + samples(5.0)
                    && (!near || (hz - ridge.hz_at(n as f64, hop)).abs() < 2.0 * cutoff.max(0.06 * hz))
            })
            .map(|&(n, _)| rise(n))
            .min()
    };
    let (next_near, next_any) = (later(true), later(false));

    // The partial, followed along the ridge.
    let from = centre.saturating_sub(samples(250.0));
    let to = (centre + ridge_len + samples(750.0)).min(total);
    if to < from + samples(20.0) {
        return None;
    }
    let coarse: Vec<f32> = (from..to).map(|n| ridge.hz_at(n as f64, hop)).collect();
    let mut tr = track_path(c, &coarse, from, cutoff);
    let mut level: Vec<f32> = (0..tr.l.len()).map(|i| tr.level(i)).collect();
    let rel = |n: usize, len: usize| n.saturating_sub(from).min(len - 1);
    let smear = (SR / cutoff) as usize;
    let bed_range = |len: usize| {
        (rel(early.saturating_sub(samples(120.0) + smear), len), rel(early.saturating_sub(smear / 2), len))
    };
    let peak_of = |level: &[f32], until: usize| {
        (rel(early, level.len())..=rel(until, level.len())).fold((rel(early, level.len()), 0.0f32), |m, i| {
            if level[i] > m.1 {
                (i, level[i])
            } else {
                m
            }
        })
    };
    let ridge_end = (centre + ridge_len + s.size / 2).min(to - 1).min(next_near.unwrap_or(usize::MAX));
    let (a, b) = bed_range(level.len());
    let bed0 = rms(&level[a..b]);
    let (pk0_i, pk0) = peak_of(&level, centre + ridge_len);
    if pk0 <= bed0 * 1.2 || pk0 <= 0.0 {
        return None;
    }

    // The attack, in a band wide enough to rise in a fraction of a millisecond.
    let wa = early.saturating_sub(samples(90.0));
    let wb = (centre + s.size / 8 + samples(140.0)).min(total);
    let wide_hi = (4.0 * f_on).min(0.47 * SR);
    let smooth = |z: Vec<(f32, f32)>| -> Vec<f32> {
        let w = samples(0.25).max(1);
        let p: Vec<f32> = z.iter().map(|z| z.0 * z.0 + z.1 * z.1).collect();
        (0..p.len())
            .map(|i| {
                let (a, b) = (i.saturating_sub(w / 2), (i + w / 2 + 1).min(p.len()));
                (p[a..b].iter().sum::<f32>() / (b - a) as f32).sqrt()
            })
            .collect()
    };
    let wide = smooth(band(&c.mono[wa..wb], 0.5 * f_on, wide_hi));
    let w_rel = |n: usize| n.saturating_sub(wa).min(wide.len() - 1);
    let before = &wide[w_rel(early.saturating_sub(samples(70.0)))..w_rel(early.saturating_sub(samples(8.0)))];
    let (w_bed, w_stray) = if before.len() < samples(5.0) {
        (0.0, 0.0)
    } else {
        let m = median(&mut before.to_vec());
        let sd = (before.iter().map(|v| (v - m).powi(2)).sum::<f32>() / before.len() as f32).sqrt();
        (m, sd)
    };
    let last = w_rel((from + pk0_i + samples(5.0)).min(centre + s.size / 8 + samples(120.0)).max(early + 1));
    let (wp_i, wp) =
        (w_rel(early)..=last).fold((w_rel(early), 0.0f32), |m, i| if wide[i] > m.1 { (i, wide[i]) } else { m });
    if wp <= w_bed {
        return None;
    }
    let at = |share: f32| w_bed + share * (wp - w_bed);
    let mut t10 = wp_i;
    while t10 > 0 && wide[t10] > at(0.1) {
        t10 -= 1;
    }
    let t90 = (t10..=wp_i).find(|&i| wide[i] >= at(0.9)).unwrap_or(wp_i);
    // Between samples, so an attack of a few samples is not a whole number.
    let cross = |i: usize, v: f32| {
        if i == 0 || wide[i] <= wide[i - 1] {
            i as f32
        } else {
            i as f32 - 1.0 + ((v - wide[i - 1]) / (wide[i] - wide[i - 1])).clamp(0.0, 1.0)
        }
    };
    let attack = (cross(t90, at(0.9)) - cross(t10 + 1, at(0.1))).max(0.0) * 1000.0 / SR;
    let start = wa + t10 + 1;
    let attack_ms = if 0.1 * (wp - w_bed) < 3.0 * w_stray {
        Fig::No("its start is within 20 dB of the bed in its band: the 10% point is in the bed")
    } else if attack < 1.25 * attack_floor_ms(f_on) {
        // The band cannot rise faster than this at this pitch: the truth is
        // anywhere under what was read.
        Fig::AtMost(attack)
    } else {
        Fig::Value(attack)
    };

    // The pitch sample by sample from the phase of a wide band around it,
    // for a sweep faster than the spectrogram's frames: the path that keeps
    // more of the event in the narrow band is the one followed.
    let lo_hz = 0.5 * ridge.hz.iter().fold(f32::MAX, |m, v| m.min(*v));
    let hi_hz = (4.0 * ridge.hz.iter().fold(0.0f32, |m, v| m.max(*v))).min(0.45 * SR);
    let until = next_any.unwrap_or(to).clamp(start + 1, to);
    let fine = phase_path(c.mono, from, to, start, until, lo_hz, hi_hz, &coarse);
    let kept = |level: &[f32]| -> f64 {
        let (a, b) = (rel(start, level.len()), rel(ridge_end.max(start + samples(5.0)), level.len()));
        level[a..=b.max(a)].iter().map(|v| (*v as f64).powi(2)).sum()
    };
    let mut swept = false;
    if let Some(fine) = &fine {
        let ft = track_path(c, fine, from, cutoff);
        let fl: Vec<f32> = (0..ft.l.len()).map(|i| ft.level(i)).collect();
        if kept(&fl) > 1.25 * kept(&level) {
            (tr, level, swept) = (ft, fl, true);
        }
    }
    let (a, b) = bed_range(level.len());
    let bed_part = &level[a..b];
    let bed = rms(bed_part);
    let (pk_i, pk) = peak_of(&level, if swept { until } else { centre + ridge_len });
    if pk <= bed * 1.2 || pk <= 0.0 {
        return None;
    }
    let peak_n = from + pk_i;
    let ridge_end = if swept { until.min(next_near.unwrap_or(usize::MAX)).min(to - 1) } else { ridge_end };

    // In a mix the partial can peak in its narrow band before the wide one
    // has risen: the event is never taken to start after its own peak.
    let begin = rel(start, level.len()).min(pk_i);

    // The fall.
    let gone = pk * 0.01;
    let end = (pk_i..=rel(ridge_end, level.len()))
        .find(|&i| level[i] < gone.max(bed * 1.5))
        .unwrap_or(rel(ridge_end, level.len()));
    let cut_short = next_near.is_some_and(|n| rel(n, level.len()) <= end + 1);
    // Cut short by the next strike, the fall is read up to where that
    // strike's own rise, smeared back by the band's low-pass, begins.
    let fall_end = if cut_short { end.saturating_sub(smear).max(pk_i + 1) } else { end };
    let stop = (pk_i..=fall_end)
        .find(|&i| level[i] < (pk * 0.0316).max(bed * 2.0))
        .unwrap_or(fall_end)
        .max(pk_i + 1)
        .min(level.len());
    let step = ((stop - pk_i) / 200).max(samples(0.25)).max(1);
    let points: Vec<(f32, f32)> =
        (pk_i..stop).step_by(step).map(|i| (to_ms(i - pk_i), 20.0 * (level[i] / pk).max(1e-6).log10())).collect();
    // The event's own decay is its first stage: the line through the fall
    // to -10 dB, carried on while each reading stays within 2 dB of it (up
    // to the first 30 dB). A reverb or a slow second stage under it bends
    // the fall away from the line and is left to the tail.
    let all_fit = line(&points);
    let points: Vec<(f32, f32)> = {
        let mut used = points.iter().position(|p| p.1 < -10.0).unwrap_or(points.len()).clamp(6.min(points.len()), points.len());
        if let Some(l) = line(&points[..used]) {
            while used < points.len() && (points[used].1 - (l.intercept + l.slope * points[used].0)).abs() <= 2.0 {
                used += 1;
            }
        }
        points[..used].to_vec()
    };
    let fit = line(&points).filter(|f| points.len() >= 6 && f.slope < 0.0);
    let fastest = fastest_decay(f_mid);
    // Under 10 dB of fall the line is mostly the rounded top of the peak.
    let fell = points.last().map_or(0.0, |p| -p.1);
    let decay_db_ms = match fit {
        None => Fig::No("no fall of 6 readings before the bed or the next event"),
        Some(_) if fell < 10.0 => Fig::No("it falls under 10 dB before the bed or the next event"),
        Some(f) if -f.slope > 0.8 * fastest => Fig::AtLeast(-f.slope),
        Some(f) => Fig::Value(-f.slope),
    };
    let decay_r2 = match (decay_db_ms, fit) {
        (Fig::Value(_), Some(_)) => Fig::Value(all_fit.map_or(1.0, |f| f.r2)),
        _ => Fig::No("the decay is not read"),
    };
    let fall_to = |ratio: f32| -> Fig {
        if pk * ratio <= bed * 1.4 {
            return Fig::No("the bed is reached first");
        }
        match (pk_i..=end).find(|&i| level[i] < pk * ratio) {
            None if cut_short => Fig::No("the next event near its pitch comes first"),
            None => Fig::No("it does not fall that far before it ends"),
            Some(i) => match decay_db_ms {
                Fig::AtLeast(_) => Fig::AtMost(to_ms(i - pk_i)),
                _ => Fig::Value(to_ms(i - pk_i)),
            },
        }
    };
    let (t20_ms, t40_ms) = (fall_to(0.1), fall_to(0.01));
    // Where the fitted decay is 20 dB down: what comes after is the tail.
    let direct_end = match fit {
        Some(f) => (pk_i + samples(((-20.0 - f.intercept) / f.slope).max(0.0))).min(end.max(pk_i)),
        None => end,
    };

    // The pitch: the tone's own frequency plus how fast the phase still turns.
    let cycles = samples(2.0).max((2.0 * SR / f_on) as usize) + smear / 2;
    // A sweep is read from three quarters of a cycle in, at the frequency
    // the phase gives there: its first reading averages its first cycle and
    // a half.
    let first_cycle = |i: usize| (0.75 * SR / tr.hz[(i + samples(1.0)).min(tr.hz.len() - 1)].max(1.0)) as usize;
    let v0 = (begin + if swept { first_cycle(begin).max(samples(0.5)) } else { cycles }).min(pk_i.max(begin + 1));
    let quiet = (pk * 0.1).max(bed * 2.0);
    let v1 = (v0..=end).rev().find(|&i| level[i] >= quiet).unwrap_or(v0).max(v0);
    let stride = ((v1 - v0) / 400).max(samples(1.0).min(if swept { samples(0.5) } else { usize::MAX })).max(1);
    let half = if swept { stride.max(samples(0.5)) } else { stride.max(samples(1.5)) };
    let read_hz = |i: usize| -> f32 {
        let (a, b) = (i.saturating_sub(half).max(1), (i + half).min(level.len() - 1));
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for k in a..=b {
            let (z, y) = (tr.mono(k), tr.mono(k - 1));
            re += (z.0 * y.0 + z.1 * y.1) as f64;
            im += (z.1 * y.0 - z.0 * y.1) as f64;
        }
        tr.hz[i] + (im.atan2(re) as f32) * SR / std::f32::consts::TAU
    };
    let pitch: Vec<(f32, f32, f32)> = (v0..=v1)
        .step_by(stride)
        .map(|i| (to_ms(from + i) - to_ms(start), read_hz(i), tr.hz[i]))
        .filter(|p| p.1 > 5.0 && p.1 < SR / 2.0)
        .collect();
    // Lost: the reading sits half the band off the tone that follows it.
    let lost = pitch.iter().filter(|p| (p.1 - p.2).abs() > 0.5 * cutoff).count() * 4 > pitch.len().max(1);
    let pitch: Vec<(f32, f32)> = pitch.iter().map(|p| (p.0, p.1)).collect();
    let pitch = if pitch.is_empty() { vec![(to_ms(peak_n) - to_ms(start), f_on)] } else { pitch };
    let ends = (pitch.len() / 20).max((3.0 / to_ms(stride)).ceil() as usize).clamp(1, pitch.len());
    // The start over its first 3 ms of readings (a glide later on is not in it).
    let first_ends = pitch.iter().take_while(|p| p.0 <= pitch[0].0 + 3.0).count().clamp(1, ends);
    let mut start_hz = median(&mut pitch[..first_ends].iter().map(|p| p.1).collect::<Vec<_>>());
    let end_hz = median(&mut pitch[pitch.len() - ends..].iter().map(|p| p.1).collect::<Vec<_>>());
    // A sweep that has moved on by the first reading: its pitch at the
    // start is the line through the first readings, taken back to it.
    let mut sweep_rate = 0.0f32;
    // The first reading is some way after the start. On a steady note that
    // does not matter; on a glide the start is taken back over that gap
    // along the line through the first cycle of readings, which holds when
    // the gap is under a cycle and the line fits; otherwise all that is
    // known of a fall is that it started at least as high as first read.
    let mut start_bound: Option<Fig> = None;
    // Read near the start: the demodulated readings of a sweep, or else the
    // phase of the wide band itself from three quarters of a cycle in (the
    // narrow band's low-pass keeps its own readings a few cycles off the
    // start).
    // The wide band's phase is the partial's only where the partial
    // rules the band: it has to agree with the narrow band's first reading.
    let agrees_at = |fp: &[f32]| {
        pitch.first().is_some_and(|p0| {
            let i = rel(start + samples(p0.0.max(0.0)), fp.len());
            (fp[i] / p0.1 - 1.0).abs() < 0.02
        })
    };
    let near_start: Vec<(f32, f32)> = match &fine {
        Some(fp) if swept || agrees_at(fp) => {
            let rel_start = rel(start, fp.len());
            let i0 = rel_start + (0.75 * SR / fp[(rel_start + samples(1.0)).min(fp.len() - 1)].max(1.0)) as usize;
            let i1 = (i0 + samples(12.0)).min(end.max(i0));
            (i0..i1).step_by(samples(0.25).max(1)).map(|i| (to_ms(from + i) - to_ms(start), fp[i])).collect()
        }
        _ => pitch.clone(),
    };
    if near_start.len() >= 3 {
        let span_ms = (2000.0 / near_start[0].1.max(1.0)).max(3.0);
        let first: Vec<(f32, f32)> =
            near_start.iter().take_while(|p| p.0 <= near_start[0].0 + span_ms).map(|p| (p.0, p.1.log2())).collect();
        if let Some(f) = line(&first).filter(|_| first.len() >= 3) {
            let gap_ms = near_start[0].0.max(0.0);
            // A glide at the start: it moves 0.3 st or more over the first
            // readings themselves.
            let moved_st = 12.0 * f.slope.abs() * (first[first.len() - 1].0 - first[0].0).max(gap_ms);
            if moved_st > 0.3 {
                let back = 2f32.powf(f.intercept);
                // Octaves per ms times ms per cycle: the share it moves in a cycle.
                sweep_rate = (f.slope.abs() * std::f32::consts::LN_2) * 1000.0 / back.max(1.0);
                let gap_cycles = gap_ms * 0.001 * back;
                let fits = f.r2 >= 0.9;
                // A second reading that errs another way: the first whole
                // cycle of the wide band, crossing to crossing. A start the
                // line puts more than 8% off it is not trusted.
                let agrees = first_cycle_hz(c.mono, start, lo_hz, hi_hz).is_some_and(|zc| (back / zc - 1.0).abs() < 0.08);
                if gap_cycles <= 1.0 && fits && agrees && sweep_rate <= FASTEST_SWEEP {
                    start_hz = back;
                } else {
                    let firsts = first.iter().map(|p| 2f32.powf(p.1));
                    start_bound = Some(if f.slope < 0.0 {
                        Fig::AtLeast(firsts.fold(0.0f32, f32::max))
                    } else {
                        Fig::AtMost(firsts.fold(f32::MAX, f32::min))
                    });
                }
            }
        }
    }
    // A note that glides but whose first readings did not show it moving
    // (a sweep faster than the readings' own window): the start read is
    // held against the first whole cycle, and is a bound when they part.
    if start_bound.is_none() && (12.0 * (end_hz / start_hz).log2()).abs() > 1.0 {
        if let Some(zc) = first_cycle_hz(c.mono, start, lo_hz, hi_hz) {
            if (start_hz / zc - 1.0).abs() >= 0.08 {
                start_bound = Some(if end_hz < start_hz {
                    Fig::AtLeast(start_hz.max(zc))
                } else {
                    Fig::AtMost(start_hz.min(zc))
                });
            }
        }
    }
    let bend_st = 12.0 * (end_hz / start_hz).log2();
    let read_cycles = (pitch[pitch.len() - 1].0 - pitch[0].0 + to_ms(2 * half)) * 0.001 * f_mid;
    let (start_fig, end_fig, bend) = if lost {
        let no = Fig::No("the partial moves faster than its band follows it");
        (no, no, no)
    } else if read_cycles < 4.0 {
        let no = Fig::No("under four cycles above -20 dB: too few to read a pitch");
        (no, no, no)
    } else if let Some(bound) = start_bound {
        // Past the fastest sweep read, or read too long after it began, the
        // start is a bound, and so is the bend.
        let b = match bound {
            Fig::AtLeast(v) => Fig::AtMost(12.0 * (end_hz / v).log2()),
            Fig::AtMost(v) => Fig::AtLeast(12.0 * (end_hz / v).log2()),
            other => other,
        };
        (bound, Fig::Value(end_hz), b)
    } else {
        (Fig::Value(start_hz), Fig::Value(end_hz), Fig::Value(bend_st))
    };
    let mid = {
        let (a, b) = (pitch.len() * 9 / 20, (pitch.len() * 11 / 20 + 1).min(pitch.len()));
        median(&mut pitch[a..b].iter().map(|p| p.1).collect::<Vec<_>>())
    };
    let off_st = 12.0 * (mid / (start_hz * end_hz).sqrt()).log2();
    let shape = if bend_st.abs() < 0.5 {
        match off_st {
            o if o > 0.5 => Shape::Arch,
            o if o < -0.5 => Shape::Dip,
            _ => Shape::Flat,
        }
    } else if off_st.abs() < 0.12 * bend_st.abs() {
        Shape::Straight
    } else if (off_st > 0.0) == (bend_st > 0.0) {
        Shape::FastThenSlow
    } else {
        Shape::SlowThenFast
    };
    let wobble = wobble(&pitch);

    // The width of the partial, its glide taken out. A window of T seconds
    // cannot read under 0.886 / T: near that, the width is a bound.
    let w_end = end.max(begin + samples(2.0)).min(level.len());
    let (width_hz, peak_off) = width(&tr, begin, w_end);
    let f_peak = (tr.hz[pk_i] + peak_off).max(1.0);
    let window_floor = 0.886 * SR / (w_end - begin).max(1) as f32;
    let bandwidth_hz = if lost {
        Fig::No("the partial moves faster than its band follows it")
    } else if matches!(decay_db_ms, Fig::AtLeast(_)) {
        // The band clips a decay faster than it follows: wider than read.
        Fig::AtLeast(width_hz)
    } else if width_hz < 1.5 * window_floor {
        Fig::AtMost(width_hz)
    } else {
        Fig::Value(width_hz)
    };
    let q = bandwidth_hz.map_down(|w| f_peak / w.max(1e-3));
    let ring_q = decay_db_ms.map_down(|r| std::f32::consts::PI * f_peak * 8.686 / (r * 1000.0));
    let decay_db_ms = if lost { decay_db_ms.withhold("the partial moves faster than its band follows it") } else { decay_db_ms };

    // What the event adds to the spectrum: the bed just before it taken off
    // bin by bin, over every frame that touches it.
    let fa = start.saturating_sub(s.size / 2) / hop;
    let fb = ((from + end + s.size / 2) / hop).min(s.frames() - 1).max(fa);
    let path_at = |n: usize| tr.hz[n.saturating_sub(from).min(tr.hz.len() - 1)];
    let bin_hz = s.bin_hz();
    let reach = ((2.0 * width_hz / bin_hz).ceil() as usize).clamp(3, 12);
    // The partial's own envelope: the attack where it is read, else none
    // (the most skirt), and the energy's own time constant.
    let energy: f64 = level[begin..=end.max(begin)].iter().map(|v| (*v as f64).powi(2)).sum();
    let tau = 2.0 * energy / (pk as f64).powi(2) / SR as f64;
    let ramp = attack_ms.value().map_or(0.0, |a| a as f64 / 800.0);
    let skirt_known = matches!(decay_db_ms, Fig::Value(_));
    let spectral = spectral(c, from, fa, fb, start, reach, f_mid, tau, ramp, path_at);
    let (mut tonal_db, mut h2_db, mut h3_db, centroid_hz, centroid_move_st) = spectral;
    if !skirt_known {
        // The partial's own skirt is modelled from its decay; a decay the
        // band cannot follow leaves it unknown, and it is most of the rest.
        let why = "its decay is faster than its band follows: its own skirt cannot be taken off";
        for f in [&mut tonal_db, &mut h2_db, &mut h3_db] {
            *f = f.withhold(why);
        }
    }

    // What starts above the resonance, beyond the partial's own onset.
    let burst_db = if 1.5 * f_on >= 0.45 * SR {
        Fig::No("1.5 times its pitch is past what the file holds")
    } else if !skirt_known {
        Fig::No("its decay is faster than its band follows: its own onset's skirt cannot be taken off")
    } else {
        let above = band(&c.mono[wa..wb], 1.5 * f_on, 0.47 * SR);
        let at_it = band(&c.mono[wa..wb], f_on / 1.25, f_on * 1.25);
        let power = |z: &[(f32, f32)], a: usize, b: usize| {
            z[a.min(z.len())..b.min(z.len())].iter().map(|z| (z.0 * z.0 + z.1 * z.1) as f64).sum::<f64>()
        };
        let five = samples(5.0);
        let (a, b) = (w_rel(start), w_rel(start + five));
        let (pa, pb) = (w_rel(early.saturating_sub(samples(70.0))), w_rel(early.saturating_sub(samples(8.0))));
        // The bed in 5 ms pieces before it: its mean, and how far a piece strays.
        let pieces: Vec<f64> = (pa..pb).step_by(five.max(1)).filter(|&p| p + five <= pb).map(|p| power(&above, p, p + five)).collect();
        let (mean, stray) = if pieces.len() >= 4 {
            let m = pieces.iter().sum::<f64>() / pieces.len() as f64;
            let sd = (pieces.iter().map(|p| (p - m).powi(2)).sum::<f64>() / pieces.len() as f64).sqrt();
            (m, 1.5 * sd * (1.0 + 1.0 / pieces.len() as f64).sqrt())
        } else if pa == pb {
            (0.0, 0.0)
        } else {
            (power(&above, pa, pb) * (b - a) as f64 / (pb - pa).max(1) as f64, f64::INFINITY)
        };
        let it_bed = power(&at_it, pa, pb) * (b - a) as f64 / (pb - pa).max(1) as f64;
        let it = power(&at_it, a, b) - it_bed;
        // What the partial's own onset puts above 1.5 f: its skirt from half
        // its frequency out (and from 2.5 f out of its mirror image).
        let own = energy * (skirt(tau, ramp, 0.5 * f_on as f64, 1e9) + skirt(tau, ramp, 2.5 * f_on as f64, 1e9)) * 0.5;
        if it <= 0.0 || !stray.is_finite() {
            Fig::No("nothing of the resonance in its first 5 ms, or no bed before it to take off")
        } else {
            share_db(power(&above, a, b) - mean - own, 3.0 * stray + 0.5 * own + 1e-7 * it, it)
        }
    };

    // Against the bed and the mix.
    let over_bed_db = (20.0 * (pk / bed.max(pk * 1e-5)).log10()).min(99.0);
    let around = (start.saturating_sub(samples(500.0)), (start + samples(500.0)).min(total));
    let own_span = (from + begin, from + end + 1);
    let rest: Vec<f32> = (around.0..around.1)
        .filter(|&n| n < own_span.0 || n >= own_span.1)
        .map(|n| c.mono[n])
        .collect();
    let under = rms(&c.mono[start.saturating_sub(samples(120.0))..start]);
    let in_mix_db = if under < pk * 0.01 || rest.len() < samples(100.0) {
        Fig::No("alone: nothing sounds under it (a solo render), so the mix around it is itself")
    } else {
        Fig::Value(20.0 * (pk * std::f32::consts::FRAC_1_SQRT_2 / rms(&rest).max(1e-9)).log10())
    };

    // Between the speakers.
    let (mut ll, mut rr, mut lr, mut side) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for i in begin..=direct_end.max(begin) {
        let (a, b) = (tr.l[i], tr.r[i]);
        ll += (a.0 * a.0 + a.1 * a.1) as f64;
        rr += (b.0 * b.0 + b.1 * b.1) as f64;
        lr += (a.0 * b.0 + a.1 * b.1) as f64;
        side += (((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)) * 0.25) as f64;
    }
    let mid_power = 0.25 * (ll + rr + 2.0 * lr);
    let pan_db = (db((ll / rr.max(1e-20)) as f32)).clamp(-40.0, 40.0);
    let side_mid_db = if mid_power <= 1e-12 * (ll + rr) {
        Fig::AtLeast(60.0)
    } else {
        Fig::Value(db((side / mid_power) as f32).clamp(-60.0, 60.0))
    };
    let corr = if pan_db.abs() > 30.0 {
        Fig::No("one side is more than 30 dB under the other: there is nothing to correlate")
    } else {
        Fig::Value((lr / (ll * rr).sqrt().max(1e-20)) as f32)
    };

    // What is left after it, beyond what its own decay still leaves.
    let tail_db = tail(&level, bed_part, begin, pk_i, pk, direct_end, fit, decay_db_ms, &points, cutoff, ramp);

    let curve_from = begin.saturating_sub(samples(10.0));
    let curve: Vec<f32> = (curve_from..=end.max(curve_from))
        .step_by(samples(1.0).max(1))
        .take(6000)
        .map(|i| 20.0 * (level[i] / pk).max(1e-5).log10())
        .collect();

    // Its level rising again after its peak: more strikes than one inside it.
    let mut low = pk;
    let mut rises = 0;
    let mut armed = true;
    for i in (pk_i..=end).step_by(samples(1.0).max(1)) {
        low = low.min(level[i]);
        if armed && level[i] > 2.0 * low && level[i] > 0.1 * pk && level[i] > 4.0 * bed {
            rises += 1;
            armed = false;
        } else if level[i] < 0.7 * level[i.saturating_sub(samples(1.0))] {
            armed = true;
            low = level[i];
        }
    }
    // The bed just before it falling: something near its pitch still rings
    // out under it, and the bed it is read against is not a bed.
    let ringing = {
        let (a, b) = bed_range(level.len());
        let mid = (a + b) / 2;
        b > a + samples(20.0) && bed > pk * 0.01 && rms(&level[a..mid]) > 1.41 * rms(&level[mid..b])
    };
    let overlap = if rises > 0 {
        Some("its level falls and rises again after its peak (a second strike, or partials beating)")
    } else if ringing {
        Some("it starts while something near its pitch still rings")
    } else if cut_short {
        Some("the next event near its pitch starts before it has died away")
    } else {
        None
    };

    let start_s = start as f32 / SR;
    Some(Event {
        start_s,
        end_s: (from + end).max(start + 1) as f32 / SR,
        over_floor_db,
        ridge,
        inside: Inside {
            attack_ms,
            burst_db,
            t20_ms,
            t40_ms,
            decay_db_ms,
            decay_r2,
            start_hz,
            end_hz,
            bend_st,
            start: start_fig,
            end: end_fig,
            bend,
            shape,
            wobble,
            bandwidth_hz,
            q,
            ring_q,
            tonal_db,
            h2_db,
            h3_db,
            over_bed_db,
            in_mix_db,
            pan_db,
            side_mid_db,
            corr,
            tail_db,
            tail_clear: true,
            direct_end_s: (from + direct_end) as f32 / SR,
            centroid_hz,
            centroid_move_st,
            overlap,
            lost,
            span_s: ((fa * hop).saturating_sub(s.size / 2) as f32 / SR, (fb * hop + s.size / 2) as f32 / SR),
            level: curve,
            pitch,
        },
    })
}

/// The frequency of the first whole cycle of `lo..hi` Hz from sample
/// `start`: between its first two upward crossings of zero, each placed
/// between samples. `None` when the band has no two crossings in 60 ms.
fn first_cycle_hz(x: &[f32], start: usize, lo: f32, hi: f32) -> Option<f32> {
    let n = samples(60.0).min(x.len().saturating_sub(start));
    if n < samples(2.0) || hi <= lo * 1.2 {
        return None;
    }
    let z = band(&x[start..start + n], lo, hi);
    // From the first tenth of the peak on: before it the band is the bed.
    let top = z.iter().fold(0.0f32, |m, v| m.max(mag(*v)));
    let from = z.iter().position(|v| mag(*v) >= 0.1 * top)?;
    let ups: Vec<f32> = (from + 1..z.len())
        .filter(|&i| z[i - 1].0 < 0.0 && z[i].0 >= 0.0)
        .map(|i| i as f32 - 1.0 + z[i - 1].0 / (z[i - 1].0 - z[i].0))
        .take(2)
        .collect();
    (ups.len() == 2 && ups[1] > ups[0]).then(|| SR / (ups[1] - ups[0]))
}

/// The partial's frequency sample by sample, `from..to`, from the turn of
/// the phase of `lo..hi` Hz averaged over a cycle and a half, read between
/// `start` and `until` and held outside. `None` when the stretch is short.
#[allow(clippy::too_many_arguments)]
fn phase_path(
    x: &[f32],
    from: usize,
    to: usize,
    start: usize,
    until: usize,
    lo: f32,
    hi: f32,
    guess: &[f32],
) -> Option<Vec<f32>> {
    if until <= start + samples(3.0) || hi <= lo * 1.2 {
        return None;
    }
    let z = band(&x[from..to], lo, hi);
    let mut pre = vec![(0.0f64, 0.0f64); z.len() + 1];
    for n in 0..z.len() {
        let (re, im) = if n == 0 {
            (0.0, 0.0)
        } else {
            let (a, b) = (z[n], z[n - 1]);
            ((a.0 * b.0 + a.1 * b.1) as f64, (a.1 * b.0 - a.0 * b.1) as f64)
        };
        pre[n + 1] = (pre[n].0 + re, pre[n].1 + im);
    }
    // Nothing before the start is read: the band there is the bed's.
    let (a, b) = (start - from, until - from);
    let read = |n: usize, w: usize| -> f32 {
        let (i, j) = (n.saturating_sub(w / 2).max(a).max(1), (n + w / 2 + 1).min(z.len()).max(a.max(1) + 2));
        let (re, im) = (pre[j].0 - pre[i].0, pre[j].1 - pre[i].1);
        (im.atan2(re) as f32 * SR / std::f32::consts::TAU).clamp(lo, hi)
    };
    let one: Vec<f32> = (a..b).map(|n| read(n, (1.5 * SR / guess[n].max(lo)) as usize)).collect();
    let two: Vec<f32> = (a..b).map(|n| read(n, (1.5 * SR / one[n - a]) as usize)).collect();
    let mut path = vec![two[0]; to - from];
    path[a..b].copy_from_slice(&two);
    let last = two[two.len() - 1];
    for v in path[b..].iter_mut() {
        *v = last;
    }
    Some(path)
}

/// Purity, harmonics and centroid from the spectrogram, the bed before the
/// event taken off bin by bin and the partial's own skirt (from its
/// envelope: `tau` s, `ramp` s) taken off what lies beside it.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn spectral(
    c: &Ctx,
    from: usize,
    fa: usize,
    fb: usize,
    start: usize,
    reach: usize,
    f_mid: f32,
    tau: f64,
    ramp: f64,
    path_at: impl Fn(usize) -> f32,
) -> (Fig, Fig, Fig, Fig, Fig) {
    let s = c.stft;
    let bin_hz = s.bin_hz();
    let none = |why: &'static str| (Fig::No(why), Fig::No(why), Fig::No(why), Fig::No(why), Fig::No(why));
    let Some(bed) = bed_spectrum(s, start.saturating_sub(s.size / 4)) else {
        return none("the bed before it is not in the file: it starts too near the beginning");
    };
    let n = fb + 1 - fa;
    let (mut t, mut all, mut h2, mut h3) = (Excess::default(), Excess::default(), Excess::default(), Excess::default());
    let mut fast = 0.0f64;
    let mut frames_t: Vec<f64> = Vec::with_capacity(n);
    let region = |u: usize, hz: f32, e: &mut Excess| -> f64 {
        let b = (hz / bin_hz).round() as usize;
        if b + reach >= s.bins {
            return 0.0;
        }
        let mut got = 0.0;
        for k in b.saturating_sub(reach).max(1)..=b + reach {
            got += s.at(u, k) as f64 - bed.mean[k];
            e.var += bed.spread[k].powi(2);
        }
        e.sum += got;
        got
    };
    for u in fa..=fb {
        let hz = path_at(u * s.hop);
        let got = region(u, hz, &mut t);
        frames_t.push(got);
        region(u, 2.0 * hz, &mut h2);
        region(u, 3.0 * hz, &mut h3);
        for k in 1..s.bins {
            all.sum += s.at(u, k) as f64 - bed.mean[k];
            all.var += bed.spread[k].powi(2);
        }
        // A pitch that moves more than the partial's reach within one
        // window smears it over the rest.
        let moved = (path_at((u * s.hop + s.size / 2).max(from)) - path_at((u * s.hop).saturating_sub(s.size / 2)))
            .abs();
        if moved > reach as f32 * bin_hz {
            fast += got.max(0.0);
        }
    }
    let m = bed.frames;
    if t.sum <= 3.0 * t.floor(n, m) {
        return none("its partial does not stand over the bed in the spectrum");
    }
    if fast > 0.1 * t.sum {
        return none("its pitch moves more within one spectrum frame than its partial is wide");
    }
    let f_bins = f_mid / bin_hz;
    // The envelope's spectrum: the share of the partial within its reach,
    // and where its skirt lands.
    let r = reach as f64 * bin_hz as f64;
    let inside = skirt(tau, ramp, 0.0, r).max(1e-6);
    let whole = t.sum / inside;
    let numeric = 1e-7 * t.sum;
    let rest = Excess { sum: all.sum - t.sum, var: (all.var - t.var).max(0.0) };
    let own_rest = whole * (1.0 - inside);
    let tonal = {
        let excess = rest.sum - own_rest;
        let unsure = rest.floor(n, m) + 0.5 * own_rest + numeric;
        if excess > 2.0 * unsure {
            Fig::Value(db((t.sum / excess) as f32))
        } else {
            Fig::AtLeast(db((t.sum / (excess.max(0.0) + unsure)) as f32))
        }
    };
    let harmonic = |e: &Excess, k: f64| -> Fig {
        if (k as f32) * f_mid >= 0.49 * SR {
            return Fig::No("past what the file holds");
        }
        if (reach as f32) * 2.0 >= f_bins {
            return Fig::No("its harmonics are closer to it than the spectrum resolves at this pitch");
        }
        let f = f_mid as f64;
        let own = whole * (skirt(tau, ramp, (k - 1.0) * f - r, (k - 1.0) * f + r) + skirt(tau, ramp, (k + 1.0) * f - r, (k + 1.0) * f + r)) * 0.5;
        share_db(e.sum - own, e.floor(n, m) + 0.5 * own + numeric, t.sum)
    };
    let (h2_db, h3_db) = (harmonic(&h2, 2.0), harmonic(&h3, 3.0));

    // The centroid over three frames around the loudest, with how far the
    // bed alone could move it.
    let centroid = |u0: usize| -> Option<(f32, f32)> {
        let (a, b) = (u0.saturating_sub(1).max(fa), (u0 + 1).min(fb));
        let (mut num, mut den, mut spread_sum) = (0.0f64, 0.0f64, 0.0f64);
        for u in a..=b {
            for k in 1..s.bins {
                let v = s.at(u, k) as f64 - bed.mean[k];
                num += k as f64 * v;
                den += v;
            }
        }
        if den <= 0.0 {
            return None;
        }
        let cen = num / den;
        let frames = (b + 1 - a) as f64;
        for k in 1..s.bins {
            spread_sum += (k as f64 - cen).powi(2) * bed.spread[k].powi(2);
        }
        let sd = (7.5 * spread_sum * frames * (1.0 + frames / m as f64)).sqrt() / den;
        let hz = (cen * bin_hz as f64) as f32;
        // Semitones the centroid could be off by, three deviations.
        let off = 12.0 * ((cen + 3.0 * sd) / cen).log2() as f32;
        Some((hz, off))
    };
    let loudest = fa + frames_t.iter().enumerate().fold((0, f64::MIN), |m, (i, v)| if *v > m.1 { (i, *v) } else { m }).0;
    let at_peak = centroid(loudest);
    let centroid_hz = match at_peak {
        None => Fig::No("nothing over the bed at its peak"),
        Some((_, off)) if off > 0.25 => Fig::No("the bed could move the centroid by more than a quarter of a semitone"),
        Some((hz, _)) => Fig::Value(hz),
    };
    let down = frames_t[loudest - fa] * 0.063;
    let low_frame = (loudest..=fb).rev().find(|&u| frames_t[u - fa] >= down).filter(|&u| u > loudest);
    let centroid_move_st = match (at_peak, low_frame.and_then(centroid)) {
        (Some((a, ea)), Some((b, eb))) if (ea * ea + eb * eb).sqrt() <= 1.0 => Fig::Value(12.0 * (b / a).log2()),
        (Some(_), Some(_)) => Fig::No("the bed moves the centroid by more than a semitone 12 dB down"),
        _ => Fig::No("it does not fall 12 dB clear of the bed"),
    };
    (tonal, h2_db, h3_db, centroid_hz, centroid_move_st)
}

/// The energy in the partial's band from 50 to 400 ms after the fitted decay
/// is 20 dB down, beyond what that decay itself leaves there and beyond the
/// bed, against the event. The decay is a model -- the attack read, then the
/// fitted exponential -- put through the same low-pass as the level, so the
/// band's own ringing is in it. A bound when the excess is within what the
/// bed's stray and the decay's own slope leave unsure.
#[allow(clippy::too_many_arguments)]
fn tail(
    level: &[f32],
    bed_part: &[f32],
    begin: usize,
    pk_i: usize,
    pk: f32,
    direct_end: usize,
    fit: Option<Fit>,
    decay: Fig,
    points: &[(f32, f32)],
    cutoff: f32,
    ramp: f64,
) -> Fig {
    let (ta, tb) = (direct_end + samples(50.0), direct_end + samples(400.0));
    if tb >= level.len() {
        return Fig::No("the file, or what is read of it, ends inside the 400 ms after it");
    }
    let (Some(fit), Fig::Value(_)) = (fit, decay) else {
        return Fig::No("its decay is not read, so what lies beyond it cannot be told");
    };
    let own: f64 = level[begin..=direct_end].iter().map(|v| (*v as f64).powi(2)).sum();
    let bed2: Vec<f64> = bed_part.iter().map(|v| (*v as f64).powi(2)).collect();
    let bed_mean = if bed2.is_empty() { 0.0 } else { bed2.iter().sum::<f64>() / bed2.len() as f64 };
    let bed_sd = if bed2.len() < 2 {
        0.0
    } else {
        (bed2.iter().map(|v| (v - bed_mean).powi(2)).sum::<f64>() / bed2.len() as f64).sqrt()
    };
    // The dry event this decay would be, seen through the band.
    let rise = (ramp * SR as f64) as usize;
    let model = |slope: f32| -> Vec<f64> {
        let per = slope as f64 / 20.0 * 1000.0 / SR as f64;
        let env: Vec<f32> = (0..level.len())
            .map(|n| {
                if n < begin {
                    0.0
                } else {
                    let t = (n - begin) as f64;
                    let up = if rise > 0 { (t / rise as f64).min(1.0) } else { 1.0 };
                    (up * 10f64.powf(per * t)) as f32
                }
            })
            .collect();
        let ones = vec![(1.0f64, 0.0f64); env.len()];
        let z = lowpassed(&env, &ones, cutoff);
        let lv: Vec<f64> = z.iter().map(|z| 0.5 * mag(*z) as f64).collect();
        let top = lv[begin..=pk_i.max(begin)].iter().fold(0.0f64, |m, v| m.max(*v)).max(1e-30);
        let k = pk as f64 / top;
        lv.iter().map(|v| (v * k).powi(2)).collect()
    };
    let direct = model(fit.slope);
    let excess: f64 = (ta..tb).map(|i| (level[i] as f64).powi(2) - direct[i] - bed_mean).sum();
    // How unsure the decay is: its slope three standard errors slower.
    let n = points.len() as f64;
    let se = if n > 2.0 {
        let res: f64 = points
            .iter()
            .map(|p| (p.1 as f64 - (fit.intercept + fit.slope * p.0) as f64).powi(2))
            .sum::<f64>()
            / (n - 2.0);
        // The readings are a smooth curve, a few independent ones in all.
        (res / fit.sxx as f64).sqrt() * (n / 6.0).max(1.0).sqrt()
    } else {
        0.0
    };
    let slower = model((fit.slope + 3.0 * se as f32).min(-1e-6));
    let unsure_direct: f64 = (ta..tb).map(|i| (slower[i] - direct[i]).abs()).sum::<f64>() + 0.05 * (ta..tb).map(|i| direct[i]).sum::<f64>();
    let width = (tb - ta) as f64;
    let span = (SR / (2.0 * cutoff)) as f64;
    let unsure_bed =
        3.0 * bed_sd * (width * span).sqrt() + 3.0 * width * bed_sd * (span / bed2.len().max(1) as f64).sqrt();
    let fig = share_db(excess, unsure_direct + unsure_bed + 1e-7 * own, own);
    match fig {
        Fig::AtMost(v) => Fig::AtMost(v.max(-60.0)),
        Fig::Value(v) => Fig::Value(v.max(-60.0)),
        other => other,
    }
}

/// A straight line through points.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Fit {
    pub slope: f32,
    pub intercept: f32,
    pub r2: f32,
    pub sxx: f32,
}

/// Least squares through (x, y).
pub(crate) fn line(p: &[(f32, f32)]) -> Option<Fit> {
    let n = p.len() as f64;
    if p.len() < 3 {
        return None;
    }
    let (mx, my) = (p.iter().map(|v| v.0 as f64).sum::<f64>() / n, p.iter().map(|v| v.1 as f64).sum::<f64>() / n);
    let sxx: f64 = p.iter().map(|v| (v.0 as f64 - mx).powi(2)).sum();
    let syy: f64 = p.iter().map(|v| (v.1 as f64 - my).powi(2)).sum();
    let sxy: f64 = p.iter().map(|v| (v.0 as f64 - mx) * (v.1 as f64 - my)).sum();
    if sxx <= 0.0 {
        return None;
    }
    let r2 = if syy <= 1e-12 { 1.0 } else { sxy * sxy / (sxx * syy) };
    let slope = sxy / sxx;
    Some(Fit { slope: slope as f32, intercept: (my - slope * mx) as f32, r2: r2 as f32, sxx: sxx as f32 })
}

/// The -3 dB width of the tracked partial between samples `a` and `b`, in
/// Hz, and how far its peak sits from the tone that tracked it.
fn width(tr: &Tracked, a: usize, b: usize) -> (f32, f32) {
    let len = (b.saturating_sub(a)).clamp(2, 1 << 17);
    let n = (8 * len).next_power_of_two();
    let (mut re, mut im) = (vec![0.0f64; n], vec![0.0f64; n]);
    for i in 0..len.min(tr.l.len() - a) {
        let z = tr.mono(a + i);
        re[i] = z.0 as f64;
        im[i] = z.1 as f64;
    }
    fft(&mut re, &mut im);
    let p = |k: isize| {
        let k = k.rem_euclid(n as isize) as usize;
        re[k] * re[k] + im[k] * im[k]
    };
    let top = (-(n as isize) / 2..n as isize / 2).max_by(|&x, &y| p(x).total_cmp(&p(y))).unwrap_or(0);
    let half = 0.5 * p(top);
    let edge = |dir: isize| -> f64 {
        let mut k = top;
        for _ in 0..n / 2 {
            if p(k + dir) < half {
                let (hi, lo) = (p(k), p(k + dir));
                return (k - top).abs() as f64 + (hi - half) / (hi - lo).max(1e-30);
            }
            k += dir;
        }
        (n / 2) as f64
    };
    let hz = SR as f64 / n as f64;
    (((edge(1) + edge(-1)) * hz) as f32, (top as f64 * hz) as f32)
}

/// A steady vibrato in a pitch curve: (Hz, cents peak to peak). The curve's
/// slow bend is taken out first, and a rate counts when one line holds four
/// tenths of what is left and swings 8 cents or more.
fn wobble(pitch: &[(f32, f32)]) -> Option<(f32, f32)> {
    let span = pitch.last()?.0 - pitch.first()?.0;
    if pitch.len() < 40 || span < 150.0 {
        return None;
    }
    let step = span / (pitch.len() - 1) as f32;
    let cents: Vec<f32> = pitch.iter().map(|p| 1200.0 * p.1.log2()).collect();
    let window_ms = if span >= 600.0 { 300.0 } else { 120.0 };
    let w = ((window_ms / step) as usize / 2).max(1);
    let rest: Vec<f32> = (0..cents.len())
        .map(|i| {
            let (a, b) = (i.saturating_sub(w), (i + w + 1).min(cents.len()));
            cents[i] - cents[a..b].iter().sum::<f32>() / (b - a) as f32
        })
        .collect();
    let rest = &rest[w.min(rest.len() / 4)..rest.len() - w.min(rest.len() / 4)];
    let var = rest.iter().map(|v| v * v).sum::<f32>() / rest.len() as f32;
    let nyquist = 500.0 / step;
    let mut best = (0.0f32, 0.0f32);
    let mut rate = (2000.0 / span).max(2.5);
    while rate <= nyquist.min(45.0) {
        let (mut re, mut im) = (0.0f32, 0.0f32);
        for (i, v) in rest.iter().enumerate() {
            let ph = std::f32::consts::TAU * rate * i as f32 * step * 0.001;
            re += v * ph.cos();
            im += v * ph.sin();
        }
        let amp = 2.0 * (re * re + im * im).sqrt() / rest.len() as f32;
        if amp > best.1 {
            best = (rate, amp);
        }
        rate += 0.25;
    }
    let (rate, amp) = best;
    // What the moving average took of the vibrato itself comes back.
    let x = std::f32::consts::PI * rate * (2 * w + 1) as f32 * step * 0.001;
    let kept = (1.0 - x.sin() / x).clamp(0.3, 1.5);
    (var > 0.0 && 0.5 * amp * amp / var > 0.4 && 2.0 * amp / kept >= 8.0).then_some((rate, 2.0 * amp / kept))
}

/// Mark the events whose tail window another event near their pitch starts
/// in, and the events another event sounds inside: what needs the event
/// alone is withheld on those.
pub(crate) fn tails(events: &mut [Event]) {
    let starts: Vec<(f32, f32, f32)> = events.iter().map(|e| (e.start_s, e.end_s, e.inside.start_hz)).collect();
    let cut: Vec<bool> =
        events.iter().map(|e| e.inside.overlap.is_some_and(|o| o.starts_with("the next event near its pitch"))).collect();
    for (i, e) in events.iter_mut().enumerate() {
        // The band's own low-pass rings ahead of the next onset by about two
        // of its periods: that is inside the window too.
        let until = e.inside.direct_end_s + 0.4 + 2.0 / cutoff_for(e.inside.end_hz);
        e.inside.tail_clear = !starts.iter().enumerate().any(|(j, &(t, _, hz))| {
            // An onset spreads over the band around it: anything within an
            // octave starting there is in the tail's band.
            j != i && t > e.start_s + 0.005 && t <= until && (12.0 * (hz / e.inside.end_hz).log2()).abs() < 12.0
        });
        let (a, b) = e.inside.span_s;
        let inside_it = starts.iter().enumerate().any(|(j, &(t0, t1, _))| j != i && t0 < b && t1 > a);
        // The one before it near its pitch was cut short by it: this one
        // starts on that one's ring, which its bed, its decay and its fall
        // are read against.
        let on_a_ring = i > 0
            && cut.iter().enumerate().take(i).any(|(j, &c)| {
                // Cut short by it, or still sounding when it starts.
                let near = (12.0 * (starts[j].2 / e.inside.start_hz).log2()).abs() < 4.0;
                near && (e.start_s - starts[j].1) < 0.05 && (c || starts[j].1 > e.start_s - 0.005)
            });
        if on_a_ring {
            e.inside.overlap = Some("it starts while the event before it near its pitch still rings");
        } else if e.inside.overlap.is_none() && inside_it {
            e.inside.overlap = Some("another event sounds while it does");
        }
        let x = &mut e.inside;
        if !x.tail_clear {
            x.tail_db = x.tail_db.withhold("another event within an octave of it starts in the 400 ms after it");
        }
        if let Some(why) = x.overlap {
            for f in [&mut x.tonal_db, &mut x.h2_db, &mut x.h3_db, &mut x.centroid_hz, &mut x.centroid_move_st, &mut x.tail_db, &mut x.burst_db] {
                *f = f.withhold(why);
            }
            // The width is the window's when the next strike cuts the event;
            // the decay read before the cut still gives a Q.
            x.bandwidth_hz = x.bandwidth_hz.withhold(why);
            x.q = x.q.withhold(why);
            x.t40_ms = x.t40_ms.withhold(why);
            if why.starts_with("it starts while") {
                // Its bed is another event's ring: its start and its fall are
                // read against something that is not a bed.
                for f in [&mut x.decay_db_ms, &mut x.decay_r2, &mut x.ring_q, &mut x.t20_ms, &mut x.attack_ms] {
                    *f = f.withhold(why);
                }
            } else if why.starts_with("its level falls and rises") {
                // The first stage of its fall, before it rises, is still its
                // own (the decay is read only while it stays within 2 dB of
                // a line); where it crosses -20 dB may be a notch.
                x.t20_ms = x.t20_ms.withhold(why);
            }
        }
    }
}

/// Echoes. Candidate intervals are where the band's spectrogram, the floor
/// taken off, matches itself later in time; each event is then looked for
/// again one and two intervals on, along its own partial: a repeat has to be
/// a quieter copy of the event (its level curve shaped the same) and the
/// second quieter than the first. The interval most events confirm wins; on
/// one pitch on a grid, an interval on the grid's half steps cannot be told
/// from the notes and is marked so.
pub(crate) fn delay(c: &Ctx, events: &[Event], band: (f32, f32)) -> Option<Delay> {
    if events.len() < 2 {
        return None;
    }
    let s = c.stft;
    let (b0, b1) = (s.bin(band.0), s.bin(band.1));
    let frames = s.frames();
    let over: Vec<f32> = (0..frames)
        .flat_map(|t| (b0..=b1).map(move |b| (t, b)))
        .map(|(t, b)| (s.at(t, b).sqrt() - 2.0 * c.floor.at(t, b).sqrt()).max(0.0))
        .collect();
    let width = b1 - b0 + 1;
    let mut lengths: Vec<f32> = events.iter().map(|e| e.end_s - e.start_s).collect();
    let typical = median(&mut lengths);
    let shortest = ((typical.max(0.04) / s.frame_s()) as usize).max(2);
    let longest = ((LONGEST_DELAY_S / s.frame_s()) as usize).min(frames / 2);
    if longest <= shortest + 4 {
        return None;
    }
    let match_at = |d: usize| -> f64 {
        over[..(frames - d) * width].iter().zip(&over[d * width..]).map(|(a, b)| (*a as f64) * (*b as f64)).sum()
    };
    let zero = match_at(0);
    if zero <= 0.0 {
        return None;
    }
    let m: Vec<f32> = (0..=longest).map(|d| (match_at(d) / zero) as f32).collect();
    let mut rest: Vec<f32> = m[shortest..].to_vec();
    let base = median(&mut rest);
    let mut dev: Vec<f32> = m[shortest..].iter().map(|v| (v - base).abs()).collect();
    let mad = median(&mut dev);
    let near = ((0.02 / s.frame_s()) as usize).max(1);
    let stands = |d: usize| {
        m[d] > base + 6.0 * mad
            && m[d] > 0.03
            && (d.saturating_sub(near).max(shortest)..=(d + near).min(longest)).all(|u| m[u] <= m[d])
    };
    let mut candidates: Vec<usize> = (shortest..=longest).filter(|&d| stands(d)).collect();
    candidates.sort_by(|a, b| m[*b].total_cmp(&m[*a]));
    candidates.truncate(8);

    let look = samples(15.0) + s.hop;
    let onsets: Vec<usize> = events.iter().map(|e| (e.start_s * SR) as usize).collect();
    let lands_on_note = |n: usize| onsets.iter().any(|&o| o.abs_diff(n) <= look);
    let mut best: Option<(usize, Delay)> = None;
    for &d in &candidates {
        let lag = d as f64 * s.hop as f64;
        let mut intervals = Vec::new();
        let (mut firsts, mut nexts) = (Vec::new(), Vec::new());
        for e in events {
            let peak_n = (e.start_s * SR) as usize + samples(e.inside.attack_ms.any().unwrap_or(0.0));
            let cutoff = cutoff_for(e.inside.start_hz);
            let Some((own, own_at, own_curve)) = echo(c, e, 0.0, peak_n, look, cutoff) else { continue };
            let floor = own * 10f32.powf(-e.inside.over_bed_db / 20.0);
            let Some((one, at, one_curve)) = echo(c, e, lag, peak_n + lag as usize, look, cutoff) else { continue };
            // A quieter copy: at least 1 dB down, shaped like the event.
            if one < 2.0 * floor || one > 0.89 * own || shape_match(&own_curve, &one_curve) < 0.8 {
                continue;
            }
            let second = echo(c, e, 2.0 * lag, peak_n + 2 * lag as usize, look, cutoff);
            if let Some((two, _, _)) = second {
                // The next repeat quieter again, unless a note of its own lands there.
                if two > 1.12 * one && !lands_on_note((e.start_s * SR) as usize + 2 * lag as usize) {
                    continue;
                }
                if two >= 2.0 * floor && two <= 0.89 * one {
                    nexts.push(20.0 * (two / one).log10());
                }
            }
            firsts.push(20.0 * (one / own).log10());
            intervals.push((at as f32 - own_at as f32) * 1000.0 / SR);
        }
        if firsts.len() < 2 || firsts.len() * 10 < events.len() * 3 {
            continue;
        }
        let found = Delay {
            interval_ms: median(&mut intervals),
            first_db: median(&mut firsts),
            per_repeat_db: (nexts.len() >= 2).then(|| median(&mut nexts)),
            confirmed: firsts.len(),
            of: events.len(),
            ambiguous: None,
        };
        if best.as_ref().is_none_or(|(_, b)| found.confirmed > b.confirmed) {
            best = Some((d, found));
        }
    }
    let (_, mut found) = best?;
    // One pitch on a grid: an interval on the grid's half steps lands where
    // the notes and each other's repeats are, and is not told from them.
    let mut hz: Vec<f32> = events.iter().map(|e| e.inside.start_hz).collect();
    let mid_hz = median(&mut hz);
    let one_pitch = events.iter().filter(|e| (12.0 * (e.inside.start_hz / mid_hz).log2()).abs() < 1.0).count() * 5
        >= events.len() * 4;
    let mut gaps: Vec<f32> = events.windows(2).map(|w| (w[1].start_s - w[0].start_s) * 1000.0).collect();
    let ioi = median(&mut gaps);
    let on_grid = gaps.iter().filter(|g| (*g / ioi - 1.0).abs() < 0.05).count() * 2 >= gaps.len();
    let tol = to_ms(look);
    let half_steps = found.interval_ms / (0.5 * ioi);
    if one_pitch && on_grid && ioi > 0.0 && (half_steps - half_steps.round()).abs() * 0.5 * ioi < tol {
        found.ambiguous = Some("its interval is a multiple of half the note spacing, on one pitch: it cannot be told from the notes");
    }
    Some(found)
}

/// Repeats are looked for up to this far after an event.
pub const LONGEST_DELAY_S: f32 = 1.6;

/// The correlation of two level curves, in dB above their own floor.
fn shape_match(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    if n < 4 {
        return 0.0;
    }
    let (ma, mb) = (a[..n].iter().sum::<f32>() / n as f32, b[..n].iter().sum::<f32>() / n as f32);
    let (mut sab, mut saa, mut sbb) = (0.0f32, 0.0f32, 0.0f32);
    for i in 0..n {
        let (x, y) = (a[i] - ma, b[i] - mb);
        sab += x * y;
        saa += x * x;
        sbb += y * y;
    }
    if saa <= 0.0 || sbb <= 0.0 {
        return 0.0;
    }
    sab / (saa * sbb).sqrt()
}

/// The level along `e`'s partial moved `shift` samples later, at its
/// loudest within `look` samples of `around`, where that is, and the level
/// curve from there over the next 30 ms, every millisecond.
fn echo(c: &Ctx, e: &Event, shift: f64, around: usize, look: usize, cutoff: f32) -> Option<(f32, usize, Vec<f32>)> {
    let pad = (2.0 * SR / cutoff) as usize + look + samples(30.0);
    let (from, to) = (around.saturating_sub(pad), (around + pad).min(c.mono.len()));
    if around + look >= c.mono.len() || to <= from + 2 * look {
        return None;
    }
    let tr = track(c, &e.ridge, c.stft.hop, shift, from, to, cutoff);
    let (a, b) = (around.saturating_sub(look).saturating_sub(from), (around + look - from).min(tr.l.len() - 1));
    let (i, v) = (a..=b).fold((a, 0.0f32), |m, i| if tr.level(i) > m.1 { (i, tr.level(i)) } else { m });
    let curve: Vec<f32> = (i..(i + samples(30.0)).min(tr.l.len()))
        .step_by(samples(1.0).max(1))
        .map(|k| tr.level(k))
        .collect();
    Some((v, from + i, curve))
}
