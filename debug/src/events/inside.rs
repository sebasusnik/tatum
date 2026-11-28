//! One event read from the inside: the audio demodulated by the event's own
//! partial, which leaves its level and pitch at every sample.

use super::{db, Delay, Event, Floor, Ridge, Stft, SR};
use crate::fft::fft;

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
    /// two octaves up and one down from the partial.
    pub attack_ms: f32,
    /// Energy above the resonance (from 1.5 times its frequency) against the
    /// energy at the resonance, over the first 5 ms, dB. A click or a noise
    /// burst at the start reads high.
    pub burst_db: Option<f32>,
    /// From the peak to 20 and 40 dB under it; `None` when the bed or the
    /// next event is reached first.
    pub t20_ms: Option<f32>,
    pub t40_ms: Option<f32>,
    /// dB lost per millisecond over the first 30 dB of the fall, and how
    /// straight that fall is in dB (r squared): 1 is an exponential.
    pub decay_db_ms: Option<f32>,
    pub decay_r2: Option<f32>,
    pub start_hz: f32,
    pub end_hz: f32,
    pub bend_st: f32,
    pub shape: Shape,
    /// A vibrato on the partial: its rate in Hz and depth in cents, peak to peak.
    pub wobble: Option<(f32, f32)>,
    /// The -3 dB width of the partial with its glide taken out, and f / width.
    pub bandwidth_hz: f32,
    pub q: f32,
    /// The Q a resonator would need to ring with this decay: pi f tau.
    pub ring_q: Option<f32>,
    /// The partial against everything else the event adds to the spectrum, dB.
    pub tonal_db: f32,
    /// Second and third harmonics against the partial, dB; `None` past Nyquist.
    pub h2_db: Option<f32>,
    pub h3_db: Option<f32>,
    /// The partial at its peak over what was in its band just before, dB.
    pub over_bed_db: f32,
    /// The partial at its peak against the whole mix over the second around it, dB.
    pub in_mix_db: f32,
    /// Left over right in the partial's band, dB, and their correlation.
    pub pan_db: f32,
    pub corr: f32,
    /// Energy in the partial's band from 50 to 400 ms after the event has
    /// fallen 20 dB, against the event, dB. `tail_clear`: no other event
    /// near its pitch started in that time.
    pub tail_db: Option<f32>,
    pub tail_clear: bool,
    /// When the event has fallen 20 dB (or ended), seconds.
    pub direct_end_s: f32,
    /// The centroid of what the event adds, at its peak, and how far it has
    /// moved when the partial is 12 dB down, semitones.
    pub centroid_hz: f32,
    pub centroid_move_st: Option<f32>,
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
/// the ridge and low-passed at `cutoff`: the partial brought to 0 Hz.
pub(crate) struct Tracked {
    pub l: Vec<(f32, f32)>,
    pub r: Vec<(f32, f32)>,
    /// The tone's frequency at each sample.
    pub hz: Vec<f32>,
}

impl Tracked {
    pub fn mono(&self, i: usize) -> (f32, f32) {
        (0.5 * (self.l[i].0 + self.r[i].0), 0.5 * (self.l[i].1 + self.r[i].1))
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

/// `shift` moves the ridge later by that many samples: where an echo would be.
#[allow(clippy::too_many_arguments)]
pub(crate) fn track(
    left: &[f32],
    right: &[f32],
    ridge: &Ridge,
    hop: usize,
    shift: f64,
    from: usize,
    to: usize,
    cutoff: f32,
) -> Tracked {
    let len = to - from;
    let mut hz = Vec::with_capacity(len);
    let mut tone = Vec::with_capacity(len);
    let mut phase = 0.0f64;
    for i in 0..len {
        let f = ridge.hz_at((from + i) as f64 - shift, hop);
        phase += std::f64::consts::TAU * f as f64 / SR as f64;
        if phase > std::f64::consts::TAU {
            phase -= std::f64::consts::TAU;
        }
        hz.push(f);
        tone.push((phase.cos(), phase.sin()));
    }
    let run = |x: &[f32]| lowpassed(&x[from..to], &tone, cutoff);
    Tracked { l: run(left), r: run(right), hz }
}

/// How wide the low-pass around a partial at `hz` is: about two semitones
/// either side, so a ping a third away is out and a 5 ms attack still shows.
pub(crate) fn cutoff_for(hz: f32) -> f32 {
    (0.12 * hz).clamp(30.0, 500.0)
}

pub(crate) struct Ctx<'a> {
    pub left: &'a [f32],
    pub right: &'a [f32],
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

/// Read the event on `ridge`. `None` when nothing rises there after all.
pub(crate) fn read(c: &Ctx, ridge: &Ridge, over_floor_db: f32) -> Option<Event> {
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

    // The partial, followed.
    let from = centre.saturating_sub(samples(250.0));
    let to = (centre + ridge_len + samples(750.0)).min(total);
    if to < from + samples(20.0) {
        return None;
    }
    let tr = track(c.left, c.right, &ridge, hop, 0.0, from, to, cutoff);
    let level: Vec<f32> = (0..tr.l.len()).map(|i| tr.level(i)).collect();
    let rel = |n: usize| n.saturating_sub(from).min(level.len() - 1);
    let smear = (SR / cutoff) as usize;
    let bed = rms(&level[rel(early.saturating_sub(samples(120.0) + smear))..rel(early.saturating_sub(smear / 2))]);
    let (pk_i, pk) = (rel(early)..=rel(centre + ridge_len)).fold((rel(early), 0.0f32), |m, i| {
        if level[i] > m.1 {
            (i, level[i])
        } else {
            m
        }
    });
    if pk <= bed * 1.2 || pk <= 0.0 {
        return None;
    }
    let peak_n = from + pk_i;

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
    let w_bed = if before.len() < samples(5.0) { 0.0 } else { median(&mut before.to_vec()) };
    let last = w_rel((peak_n + samples(5.0)).min(centre + s.size / 8 + samples(120.0)).max(early + 1));
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
    let attack_ms = (cross(t90, at(0.9)) - cross(t10 + 1, at(0.1))).max(0.0) * 1000.0 / SR;
    let start = wa + t10 + 1;

    // What comes with the start above the resonance.
    let burst_db = (1.5 * f_on < 0.45 * SR).then(|| {
        let above = band(&c.mono[wa..wb], 1.5 * f_on, 0.47 * SR);
        let at_it = band(&c.mono[wa..wb], f_on / 1.25, f_on * 1.25);
        let power = |z: &[(f32, f32)], a: usize, b: usize| {
            z[a.min(z.len())..b.min(z.len())].iter().map(|z| (z.0 * z.0 + z.1 * z.1) as f64).sum::<f64>()
        };
        let (a, b) = (w_rel(start), w_rel(start + samples(5.0)));
        let (pa, pb) = (w_rel(early.saturating_sub(samples(70.0))), w_rel(early.saturating_sub(samples(8.0))));
        let share = (b - a) as f64 / (pb - pa).max(1) as f64;
        let up = (power(&above, a, b) - share * power(&above, pa, pb)).max(0.0);
        let it = (power(&at_it, a, b) - share * power(&at_it, pa, pb)).max(1e-20);
        db((up / it) as f32).max(-80.0)
    });

    // The fall.
    let ridge_end = (centre + ridge_len + s.size / 2).min(to - 1);
    let gone = pk * 0.01;
    let end = (pk_i..=rel(ridge_end)).find(|&i| level[i] < gone.max(bed * 1.5)).unwrap_or(rel(ridge_end));
    let fall_to = |ratio: f32| {
        (pk * ratio > bed * 1.4)
            .then(|| (pk_i..=end).find(|&i| level[i] < pk * ratio).map(|i| to_ms(i - pk_i)))
            .flatten()
    };
    let (t20_ms, t40_ms) = (fall_to(0.1), fall_to(0.01));
    let stop =
        (pk_i..=end).find(|&i| level[i] < (pk * 0.0316).max(bed * 2.0)).unwrap_or(end).max(pk_i + 1).min(level.len());
    let step = ((stop - pk_i) / 200).max(samples(0.25)).max(1);
    let points: Vec<(f32, f32)> =
        (pk_i..stop).step_by(step).map(|i| (to_ms(i - pk_i), 20.0 * (level[i] / pk).max(1e-6).log10())).collect();
    let (decay_db_ms, decay_r2) = match line(&points) {
        Some((slope, r2)) if points.len() >= 6 && slope < 0.0 => (Some(-slope), Some(r2)),
        _ => (None, None),
    };
    let direct_end = t20_ms.map_or(end, |ms| (pk_i + samples(ms)).min(end));

    // The pitch: the tone's own frequency plus how fast the phase still turns.
    let cycles = samples(2.0).max((2.0 * SR / f_on) as usize) + smear / 4;
    let v0 = (rel(start) + cycles).min(pk_i.max(rel(start) + 1));
    let quiet = (pk * 0.1).max(bed * 2.0);
    let v1 = (v0..=end).rev().find(|&i| level[i] >= quiet).unwrap_or(v0).max(v0);
    let stride = ((v1 - v0) / 400).max(samples(1.0)).max(1);
    let half = stride.max(samples(1.5));
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
    let pitch: Vec<(f32, f32)> = (v0..=v1)
        .step_by(stride)
        .map(|i| (to_ms(from + i) - to_ms(start), read_hz(i)))
        .filter(|p| p.1 > 5.0 && p.1 < SR / 2.0)
        .collect();
    let pitch = if pitch.is_empty() { vec![(to_ms(peak_n) - to_ms(start), f_on)] } else { pitch };
    let ends = (pitch.len() / 20).max((3.0 / to_ms(stride)).ceil() as usize).clamp(1, pitch.len());
    let start_hz = median(&mut pitch[..ends].iter().map(|p| p.1).collect::<Vec<_>>());
    let end_hz = median(&mut pitch[pitch.len() - ends..].iter().map(|p| p.1).collect::<Vec<_>>());
    let bend_st = 12.0 * (end_hz / start_hz).log2();
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

    // The width of the partial, its glide taken out.
    let (bandwidth_hz, peak_off) = width(&tr, rel(start), end.max(rel(start) + samples(2.0)).min(level.len()));
    let f_peak = (tr.hz[pk_i] + peak_off).max(1.0);
    let q = f_peak / bandwidth_hz.max(1e-3);
    let ring_q = decay_db_ms.map(|r| std::f32::consts::PI * f_peak * 8.686 / (r * 1000.0));

    // What the event adds to the spectrum, the floor taken off.
    let reach = ((2.0 * bandwidth_hz / s.bin_hz()).ceil() as usize).clamp(3, 12);
    let first = (start / hop).min(s.frames() - 1);
    let last = ((from + end) / hop).clamp(first, s.frames() - 1);
    let added = |u: usize, b: usize| (s.at(u, b) - c.floor.at(u, b)).max(0.0) as f64;
    let (mut tonal, mut all, mut h2, mut h3) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    let around = |u: usize, hz: f32| -> f64 {
        let b = (hz / s.bin_hz()).round() as usize;
        if b + reach >= s.bins {
            return 0.0;
        }
        (b.saturating_sub(reach).max(1)..=b + reach).map(|b| added(u, b)).sum()
    };
    let mut loudest = (first, 0.0f64);
    for u in first..=last {
        let hz = ridge.hz_at((u * hop) as f64, hop);
        let t = around(u, hz);
        if t > loudest.1 {
            loudest = (u, t);
        }
        tonal += t;
        h2 += around(u, 2.0 * hz);
        h3 += around(u, 3.0 * hz);
        all += (1..s.bins).map(|b| added(u, b)).sum::<f64>();
    }
    let tonal_db = if tonal <= 0.0 { -60.0 } else { db((tonal / (all - tonal).max(tonal * 1e-6)) as f32) };
    let harmonic = |h: f64, k: f32| (k * f_mid < 0.49 * SR && tonal > 0.0).then(|| db((h / tonal) as f32).max(-90.0));
    let centroid = |u: usize| -> Option<f32> {
        let (mut num, mut den) = (0.0f64, 0.0f64);
        for b in 1..s.bins {
            num += b as f64 * added(u, b);
            den += added(u, b);
        }
        (den > 0.0).then(|| (num / den) as f32 * s.bin_hz())
    };
    let centroid_hz = centroid(loudest.0).unwrap_or(f_peak);
    let low = (loudest.0..=last)
        .rev()
        .find(|&u| around(u, ridge.hz_at((u * hop) as f64, hop)) >= loudest.1 * 0.063)
        .filter(|&u| u > loudest.0);
    let centroid_move_st = low.and_then(centroid).map(|c| 12.0 * (c / centroid_hz).log2());

    // Against the bed and the mix.
    let over_bed_db = (20.0 * (pk / bed.max(pk * 1e-5)).log10()).min(99.0);
    let around_it = rms(&c.mono[start.saturating_sub(samples(500.0))..(start + samples(500.0)).min(total)]);
    let in_mix_db = 20.0 * (pk * std::f32::consts::FRAC_1_SQRT_2 / around_it.max(1e-9)).log10();

    // Between the speakers.
    let (mut ll, mut rr, mut lr) = (0.0f64, 0.0f64, 0.0f64);
    for i in rel(start)..=direct_end.max(rel(start)) {
        let (a, b) = (tr.l[i], tr.r[i]);
        ll += (a.0 * a.0 + a.1 * a.1) as f64;
        rr += (b.0 * b.0 + b.1 * b.1) as f64;
        lr += (a.0 * b.0 + a.1 * b.1) as f64;
    }
    let pan_db = (db((ll / rr.max(1e-20)) as f32)).clamp(-40.0, 40.0);
    let corr = (lr / (ll * rr).sqrt().max(1e-20)) as f32;

    // What is left after it.
    let (ta, tb) = (direct_end + samples(50.0), direct_end + samples(400.0));
    let tail_db = (tb < level.len()).then(|| {
        let own: f64 = level[rel(start)..=direct_end].iter().map(|v| (*v as f64).powi(2)).sum();
        let after: f64 = level[ta..tb].iter().map(|v| ((*v as f64).powi(2) - (bed as f64).powi(2)).max(0.0)).sum();
        db((after / own.max(1e-20)) as f32).max(-60.0)
    });

    let curve_from = rel(start).saturating_sub(samples(10.0));
    let curve: Vec<f32> = (curve_from..=end.max(curve_from))
        .step_by(samples(1.0).max(1))
        .take(6000)
        .map(|i| 20.0 * (level[i] / pk).max(1e-5).log10())
        .collect();

    Some(Event {
        start_s: start as f32 / SR,
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
            shape,
            wobble,
            bandwidth_hz,
            q,
            ring_q,
            tonal_db,
            h2_db: harmonic(h2, 2.0),
            h3_db: harmonic(h3, 3.0),
            over_bed_db,
            in_mix_db,
            pan_db,
            corr,
            tail_db,
            tail_clear: true,
            direct_end_s: (from + direct_end) as f32 / SR,
            centroid_hz,
            centroid_move_st,
            level: curve,
            pitch,
        },
    })
}

/// Least squares through (x, y): the slope, and r squared.
pub(crate) fn line(p: &[(f32, f32)]) -> Option<(f32, f32)> {
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
    Some(((sxy / sxx) as f32, r2 as f32))
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

/// Mark the events whose tail window another event near their pitch starts in.
pub(crate) fn tails(events: &mut [Event]) {
    let starts: Vec<(f32, f32)> = events.iter().map(|e| (e.start_s, e.inside.start_hz)).collect();
    for (i, e) in events.iter_mut().enumerate() {
        let until = e.inside.direct_end_s + 0.4;
        e.inside.tail_clear = !starts.iter().enumerate().any(|(j, &(t, hz))| {
            j != i && t > e.start_s + 0.005 && t <= until && (12.0 * (hz / e.inside.end_hz).log2()).abs() < 4.0
        });
    }
}

/// Echoes. The interval is where the band's spectrogram, the floor taken
/// off, matches itself later in time; each event is then looked for again
/// one and two intervals on, along its own partial, and the loss per repeat
/// is the median of what is found.
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
    let longest = ((1.6 / s.frame_s()) as usize).min(frames / 2);
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
    // The shortest lag that stands out: the repeats after it are its multiples.
    let best = (shortest..=longest).filter(|&d| stands(d)).max_by(|&a, &b| m[a].total_cmp(&m[b]))?;
    let first = (shortest..best).find(|&d| stands(d) && m[d] > 0.5 * m[best]).unwrap_or(best);
    let lag = first as f64 * s.hop as f64;

    let mut intervals = Vec::new();
    let (mut firsts, mut nexts) = (Vec::new(), Vec::new());
    let look = samples(15.0) + s.hop;
    for e in events {
        let peak_n = (e.start_s * SR) as usize + samples(e.inside.attack_ms);
        let cutoff = cutoff_for(e.inside.start_hz);
        let own = echo(c, e, 0.0, peak_n, look, cutoff);
        let Some((own, own_at)) = own else { continue };
        let floor = own * 10f32.powf(-e.inside.over_bed_db / 20.0);
        let Some((one, at)) = echo(c, e, lag, peak_n + lag as usize, look, cutoff) else { continue };
        if one < 2.0 * floor || one > 1.2 * own {
            continue;
        }
        firsts.push(20.0 * (one / own).log10());
        intervals.push((at as f32 - own_at as f32) * 1000.0 / SR);
        if let Some((two, _)) = echo(c, e, 2.0 * lag, peak_n + 2 * lag as usize, look, cutoff) {
            if two >= 2.0 * floor && two <= 1.2 * one {
                nexts.push(20.0 * (two / one).log10());
            }
        }
    }
    if firsts.len() < 2 || firsts.len() * 10 < events.len() * 3 {
        return None;
    }
    Some(Delay {
        interval_ms: median(&mut intervals),
        first_db: median(&mut firsts),
        per_repeat_db: (nexts.len() >= 2).then(|| median(&mut nexts)),
        confirmed: firsts.len(),
        of: events.len(),
    })
}

/// The level along `e`'s partial moved `shift` samples later, at its
/// loudest within `look` samples of `around`, and where that is.
fn echo(c: &Ctx, e: &Event, shift: f64, around: usize, look: usize, cutoff: f32) -> Option<(f32, usize)> {
    let pad = (2.0 * SR / cutoff) as usize + look;
    let (from, to) = (around.saturating_sub(pad), (around + pad).min(c.mono.len()));
    if around + look >= c.mono.len() || to <= from + 2 * look {
        return None;
    }
    let tr = track(c.left, c.right, &e.ridge, c.stft.hop, shift, from, to, cutoff);
    let (a, b) = (around - look - from, (around + look - from).min(tr.l.len() - 1));
    let (i, v) = (a..=b).fold((a, 0.0f32), |m, i| if tr.level(i) > m.1 { (i, tr.level(i)) } else { m });
    Some((v, from + i))
}
