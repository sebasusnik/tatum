//! What holds: a pad, a drone, a held note. Its spectrum in third octaves,
//! the partials of its steadiest stretch and how each one moves, how much of
//! it is noise, how wide it is band by band.

use super::inside::{lowpassed, line};
use super::{db, Stft, SR};
use crate::fft::fft;
use crate::reference::BAND_HZ;

#[derive(Clone, Debug)]
pub struct Partial {
    pub hz: f32,
    /// Against the strongest partial, dB.
    pub db: f32,
    /// How far its level swings (5th to 95th percentile), dB, and at what
    /// rate when one rate holds a third of the swing.
    pub am_db: f32,
    pub am_hz: Option<f32>,
    /// The same for its pitch, in cents.
    pub fm_cents: f32,
    pub fm_hz: Option<f32>,
}

#[derive(Clone, Debug)]
pub struct Sustained {
    /// The steadiest stretch, which the partials are read from, seconds.
    pub from_s: f32,
    pub to_s: f32,
    /// (centre Hz, dB against the whole) per third octave.
    pub thirds: Vec<(f32, f32)>,
    /// The partials' power against everything between them, dB.
    pub hnr_db: f32,
    /// The level between the partials, per bin, under the strongest one, dB.
    pub noise_db: f32,
    pub partials: Vec<Partial>,
    /// The note the partials are multiples of, when they are.
    pub f0: Option<f32>,
    /// Beats: partials closer than 20 Hz, and levels that swing at one rate, Hz.
    pub beats: Vec<f32>,
    /// How much the spectrum changes from one 46 ms frame to the next:
    /// 0 a held tone, towards 1 noise or a new sound every frame.
    pub flux: f32,
    pub centroid_hz: f32,
    /// How far the centroid wanders, semitones (standard deviation).
    pub centroid_move_st: f32,
    /// (centre Hz, side against mid dB, correlation) per octave.
    pub width: Vec<(f32, f32, f32)>,
}

/// `None` for silence, or less than half a second.
pub fn measure(left: &[f32], right: &[f32]) -> Option<Sustained> {
    let mono: Vec<f32> = left.iter().zip(right).map(|(a, b)| 0.5 * (a + b)).collect();
    let level = (mono.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / mono.len().max(1) as f64).sqrt();
    if mono.len() < (0.5 * SR) as usize || level < 1e-5 {
        return None;
    }

    // Left, right and what they share, averaged over the whole stretch.
    let size = 8192.min(mono.len().next_power_of_two() / 2).max(1024);
    let bins = size / 2 + 1;
    let (mut ll, mut rr, mut lr) = (vec![0.0f64; bins], vec![0.0f64; bins], vec![0.0f64; bins]);
    let hann: Vec<f64> =
        (0..size).map(|i| 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / size as f64).cos()).collect();
    let mut at = 0;
    while at + size <= mono.len() {
        let spectrum = |x: &[f32]| {
            let mut re: Vec<f64> = x[at..at + size].iter().zip(&hann).map(|(v, w)| *v as f64 * w).collect();
            let mut im = vec![0.0f64; size];
            fft(&mut re, &mut im);
            (re, im)
        };
        let ((lre, lim), (rre, rim)) = (spectrum(left), spectrum(right));
        for b in 0..bins {
            ll[b] += lre[b] * lre[b] + lim[b] * lim[b];
            rr[b] += rre[b] * rre[b] + rim[b] * rim[b];
            lr[b] += lre[b] * rre[b] + lim[b] * rim[b];
        }
        at += size / 2;
    }
    let bin_hz = SR / size as f32;
    let range = |lo: f32, hi: f32| ((lo / bin_hz).ceil() as usize).max(1)..((hi / bin_hz).ceil() as usize).min(bins);
    // Mid is (l + r) / 2 and side (l - r) / 2.
    let mid = |b: usize| 0.25 * (ll[b] + rr[b] + 2.0 * lr[b]);
    let side = |b: usize| 0.25 * (ll[b] + rr[b] - 2.0 * lr[b]);
    let whole: f64 = (1..bins).map(mid).sum::<f64>().max(1e-30);
    let sixth = 2f32.powf(1.0 / 6.0);
    let thirds: Vec<(f32, f32)> = (0..40)
        .map(|k| 25.0 * 2f32.powf(k as f32 / 3.0))
        .filter(|&hz| hz * sixth < SR / 2.0)
        .map(|hz| (hz, db((range(hz / sixth, hz * sixth).map(mid).sum::<f64>() / whole) as f32).max(-120.0)))
        .collect();
    let width: Vec<(f32, f32, f32)> = BAND_HZ
        .iter()
        .filter(|&&hz| hz * 1.41 < SR / 2.0)
        .map(|&hz| {
            let r = range(hz / 1.414, hz * 1.414);
            let (m, s): (f64, f64) = (r.clone().map(mid).sum(), r.clone().map(side).sum());
            let (a, b, c): (f64, f64, f64) =
                (r.clone().map(|b| ll[b]).sum(), r.clone().map(|b| rr[b]).sum(), r.map(|b| lr[b]).sum());
            (hz, db((s / m.max(1e-30)) as f32).max(-90.0), (c / (a * b).sqrt().max(1e-30)) as f32)
        })
        .collect();

    // How much it moves, and where it is steadiest.
    let s = Stft::new(&mono, 2048, 2048);
    let frames = s.frames();
    let mut flux = vec![0.0f32; frames];
    let mut centroids = Vec::new();
    for (t, moved) in flux.iter_mut().enumerate() {
        let (mut num, mut den, mut dot, mut a2, mut b2) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
        for b in 1..s.bins {
            let p = s.at(t, b) as f64;
            num += b as f64 * p;
            den += p;
            if t > 0 {
                let q = s.at(t - 1, b) as f64;
                dot += (p * q).sqrt();
                a2 += p;
                b2 += q;
            }
        }
        if den > 1e-12 {
            centroids.push((num / den) as f32 * s.bin_hz());
        }
        *moved = if a2 > 1e-12 && b2 > 1e-12 { (1.0 - dot / (a2 * b2).sqrt()) as f32 } else { 0.0 };
    }
    let loud = |t: usize| (1..s.bins).map(|b| s.at(t, b)).sum::<f32>() > 1e-8;
    let moving: Vec<f32> = (1..frames).filter(|&t| loud(t) && loud(t - 1)).map(|t| flux[t]).collect();
    let mean_flux = if moving.is_empty() { 0.0 } else { moving.iter().sum::<f32>() / moving.len() as f32 };
    let centroid_hz = if centroids.is_empty() {
        0.0
    } else {
        (centroids.iter().map(|c| c.ln()).sum::<f32>() / centroids.len() as f32).exp()
    };
    let centroid_move_st = if centroids.len() < 2 {
        0.0
    } else {
        let m = centroid_hz.log2();
        12.0 * (centroids.iter().map(|c| (c.log2() - m).powi(2)).sum::<f32>() / centroids.len() as f32).sqrt()
    };

    // The steadiest stretch: the least flux, and nothing silent in it.
    let long = 65536.min(mono.len().next_power_of_two() / 2);
    let span = (long / 2048).max(1);
    let score = |t: usize| (t..t + span).map(|u| if loud(u) { flux[u] } else { 1.0 }).sum::<f32>();
    let first = (0..frames.saturating_sub(span + 1)).min_by(|&a, &b| score(a).total_cmp(&score(b))).unwrap_or(0) * 2048;
    let first = first.min(mono.len() - long);
    let (mut re, mut im) = (vec![0.0f64; long], vec![0.0f64; long]);
    for i in 0..long {
        re[i] = mono[first + i] as f64 * (0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / long as f64).cos());
    }
    fft(&mut re, &mut im);
    let half = long / 2;
    let power: Vec<f32> =
        (0..half).map(|b| ((re[b] * re[b] + im[b] * im[b]) / (long as f64 / 4.0).powi(2)) as f32).collect();
    let fine_hz = SR / long as f32;
    // What lies between the partials: the median of the bins around each one.
    let stride = 8;
    let mut scratch: Vec<f32> = Vec::new();
    let between: Vec<f32> = (0..half.div_ceil(stride))
        .map(|k| {
            let b = k * stride;
            let reach = ((b as f32 * 0.06) as usize).max(60);
            scratch.clear();
            scratch.extend(power[b.saturating_sub(reach)..(b + reach).min(half)].iter().step_by(3));
            let mid = scratch.len() / 2;
            *scratch.select_nth_unstable_by(mid, f32::total_cmp).1
        })
        .collect();
    let floor = |b: usize| between[(b / stride).min(between.len() - 1)].max(1e-20);
    let lowest = (20.0 / fine_hz) as usize + 1;
    let strongest = power[lowest..].iter().fold(0.0f32, |m, v| m.max(*v)).max(1e-20);
    let mut peaks: Vec<(f32, f32)> = Vec::new();
    let (mut in_peaks, mut total) = (0.0f64, 0.0f64);
    for b in lowest..half - 3 {
        total += power[b] as f64;
        let p = power[b];
        if p > 30.0 * floor(b) && p > 1e-6 * strongest && (b - 3..=b + 3).all(|k| power[k] <= p) {
            let (l, c, r) = (power[b - 1].max(1e-20).ln(), p.ln(), power[b + 1].max(1e-20).ln());
            let d = if 2.0 * c - l - r > 1e-9 { (0.5 * (l - r) / (l - 2.0 * c + r)).clamp(-0.5, 0.5) } else { 0.0 };
            peaks.push(((b as f32 + d) * fine_hz, p));
            in_peaks += (b - 3..=b + 3).map(|k| power[k] as f64).sum::<f64>();
        }
    }
    let hnr_db = db((in_peaks / (total - in_peaks).max(in_peaks * 1e-9).max(1e-30)) as f32);
    let mut quiet: Vec<f32> = ((100.0 / fine_hz) as usize..((10_000.0f32).min(0.45 * SR) / fine_hz) as usize)
        .step_by(stride)
        .map(floor)
        .collect();
    quiet.sort_by(f32::total_cmp);
    let noise_db = db(quiet.get(quiet.len() / 2).copied().unwrap_or(1e-20) / strongest);

    peaks.sort_by(|a, b| b.1.total_cmp(&a.1));
    peaks.truncate(12);
    peaks.sort_by(|a, b| a.0.total_cmp(&b.0));
    let f0 = fundamental(&peaks);

    // Each partial brought to 0 Hz over the steady stretch and as much again
    // around it -- three seconds: a few cycles of a slow vibrato, and few
    // enough that a held chord has not moved on -- and what its level and
    // pitch do there.
    let take = (2 * long).min(mono.len());
    let begin = (first + long / 2).saturating_sub(take / 2).min(mono.len() - take);
    let mut beats: Vec<f32> = Vec::new();
    let partials: Vec<Partial> = peaks
        .iter()
        .enumerate()
        .map(|(i, &(hz, p))| {
            let gap = peaks
                .iter()
                .enumerate()
                .filter(|(j, q)| *j != i && (q.0 - hz).abs() > 20.0)
                .map(|(_, q)| (q.0 - hz).abs())
                .fold(f32::MAX, f32::min);
            let cutoff = (0.4 * gap).clamp(8.0, 45.0);
            let tone: Vec<(f64, f64)> = (0..take)
                .map(|n| {
                    let ph = std::f64::consts::TAU * (hz as f64 * n as f64 / SR as f64).fract();
                    (ph.cos(), ph.sin())
                })
                .collect();
            let z = lowpassed(&mono[begin..begin + take], &tone, cutoff);
            // 200 readings a second, the ends (where the filter wraps) left out.
            let step = (SR / 200.0) as usize;
            let edge = ((0.12 * SR) as usize).min(take / 8);
            let mut level = Vec::new();
            let mut cents = Vec::new();
            let mut n = edge + step;
            while n + edge < take {
                let (a, b) = (z[n], z[n - step]);
                level.push(db(a.0 * a.0 + a.1 * a.1));
                let turn = (a.1 * b.0 - a.0 * b.1).atan2(a.0 * b.0 + a.1 * b.1);
                cents.push(1200.0 * ((hz + turn * 200.0 / std::f32::consts::TAU) / hz).max(1e-3).log2());
                n += step;
            }
            let (am_db, am_hz) = swing(&level, 200.0);
            let (fm_cents, fm_hz) = swing(&cents, 200.0);
            if let Some(r) = am_hz.filter(|_| am_db >= 3.0) {
                beats.push(r);
            }
            Partial { hz, db: db(p / strongest), am_db, am_hz, fm_cents, fm_hz }
        })
        .collect();
    for w in peaks.windows(2) {
        if w[1].0 - w[0].0 < 20.0 {
            beats.push(w[1].0 - w[0].0);
        }
    }
    beats.sort_by(f32::total_cmp);
    beats.dedup_by(|a, b| (*a - *b).abs() < 0.3);

    Some(Sustained {
        from_s: first as f32 / SR,
        to_s: (first + long) as f32 / SR,
        thirds,
        hnr_db,
        noise_db,
        partials,
        f0,
        beats,
        flux: mean_flux,
        centroid_hz,
        centroid_move_st,
        width,
    })
}

/// How far a series swings between its 5th and 95th percentiles, its slow
/// drift taken out, and the rate of the swing when one rate between 0.3 and
/// 40 Hz holds a third of it.
fn swing(v: &[f32], per_second: f32) -> (f32, Option<f32>) {
    if v.len() < 20 {
        return (0.0, None);
    }
    let points: Vec<(f32, f32)> = v.iter().enumerate().map(|(i, y)| (i as f32, *y)).collect();
    let slope = line(&points).map_or(0.0, |l| l.0);
    let mean = v.iter().sum::<f32>() / v.len() as f32;
    let mid = (v.len() - 1) as f32 / 2.0;
    let rest: Vec<f32> = v.iter().enumerate().map(|(i, y)| y - mean - slope * (i as f32 - mid)).collect();
    let mut sorted = rest.clone();
    sorted.sort_by(f32::total_cmp);
    let depth = sorted[sorted.len() * 95 / 100] - sorted[sorted.len() * 5 / 100];
    let var = rest.iter().map(|x| x * x).sum::<f32>() / rest.len() as f32;
    let seconds = v.len() as f32 / per_second;
    let mut best = (0.0f32, 0.0f32);
    let mut rate = (1.5 / seconds).max(0.3);
    while rate <= 40.0 {
        let (mut re, mut im) = (0.0f32, 0.0f32);
        for (i, x) in rest.iter().enumerate() {
            let ph = std::f32::consts::TAU * rate * i as f32 / per_second;
            re += x * ph.cos();
            im += x * ph.sin();
        }
        let amp = 2.0 * (re * re + im * im).sqrt() / rest.len() as f32;
        if amp > best.1 {
            best = (rate, amp);
        }
        rate += if rate < 5.0 { 0.05 } else { 0.25 };
    }
    (depth, (var > 1e-9 && 0.5 * best.1 * best.1 / var > 0.33).then_some(best.0))
}

/// The note most of the partials' power sits on multiples of, within 1.5%.
/// `None` when under half of it does: a chord, a cluster, a bell.
fn fundamental(peaks: &[(f32, f32)]) -> Option<f32> {
    let total: f32 = peaks.iter().map(|p| p.1.sqrt()).sum();
    let mut best: Option<(f32, f32)> = None;
    for &(hz, _) in peaks {
        for k in 1..=6 {
            let f = hz / k as f32;
            if f < 20.0 {
                break;
            }
            let on: f32 = peaks
                .iter()
                .filter(|p| {
                    let n = (p.0 / f).round();
                    n >= 1.0 && (p.0 / (n * f) - 1.0).abs() < 0.015
                })
                .map(|p| p.1.sqrt())
                .sum();
            // The highest note that explains them: half of any note does too,
            // and loses a little for each division.
            let score = on / total - 0.02 * k as f32;
            if on / total > 0.5 && best.is_none_or(|b| score > b.1 + 1e-6) {
                best = Some((f, score));
            }
        }
    }
    best.map(|b| b.0)
}
