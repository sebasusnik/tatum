//! One hit, looked at closely: the average of a record's kicks on its beat
//! grid, and the pitch and level of any hit over its first few hundred
//! milliseconds. What `tatum design` draws, and what a kick is fitted to.
//!
//! A kick on a record almost never plays alone: a bass on the offbeat, hats,
//! a pad. Averaging many hits taken on the beat grid keeps what comes with
//! every hit -- the kick -- and thins out what does not; and the first couple
//! of hundred milliseconds of a beat are usually the kick's alone, before an
//! offbeat bass comes in.

use tatum_core::SAMPLE_RATE;

use crate::reference::Biquad;

/// Milliseconds to samples at the engine's rate.
fn samples(ms: f32) -> usize {
    (ms * 0.001 * SAMPLE_RATE) as usize
}

/// The average of every hit in `mono` (at the engine's rate) on a grid of
/// `bpm`, `len_ms` long from where the hit starts. The grid's phase is where
/// the low band rises most sharply, beat after beat; each hit is then slid by
/// up to 2 ms to where it best matches the others, and the average starts
/// where it first reaches 5% of its peak. `None` with fewer than four hits.
pub fn average_hit(mono: &[f32], bpm: f32, len_ms: f32) -> Option<Vec<f32>> {
    if !(20.0..=400.0).contains(&bpm) {
        return None;
    }
    let beat = 60.0 / bpm * SAMPLE_RATE;
    let low = Biquad::lowpass(600.0).run_twice(mono);
    // How sharply the low band rises at each sample, smoothed over 0.7 ms.
    let smooth = 32usize;
    let mut level = vec![0.0f32; low.len()];
    let mut acc = 0.0f32;
    for i in 0..low.len() {
        acc += low[i].abs();
        if i >= smooth {
            acc -= low[i - smooth].abs();
        }
        level[i] = acc / smooth as f32;
    }
    let rise: Vec<f32> =
        (0..level.len()).map(|i| if i == 0 { 0.0 } else { (level[i] - level[i - 1]).max(0.0) }).collect();
    let beats = ((mono.len() as f32 - beat) / beat).floor() as usize;
    if beats < 4 {
        return None;
    }
    // The phase, to 8 samples: where the rises line up beat after beat.
    let phase = (0..beat as usize)
        .step_by(8)
        .max_by(|&a, &b| {
            let score = |p: usize| (0..beats).map(|k| rise[p + (k as f32 * beat) as usize]).sum::<f32>();
            score(a).total_cmp(&score(b))
        })
        .unwrap_or(0);
    let pre = samples(10.0);
    let len = samples(len_ms) + pre;
    let starts: Vec<usize> = (0..beats)
        .map(|k| phase + (k as f32 * beat) as usize)
        .filter(|&o| o >= pre + samples(2.0) && o - pre + len + samples(2.0) <= mono.len())
        .map(|o| o - pre)
        .collect();
    if starts.len() < 4 {
        return None;
    }
    let average = |starts: &[usize]| -> Vec<f32> {
        let mut sum = vec![0.0f64; len];
        for &s in starts {
            for (a, v) in sum.iter_mut().zip(&mono[s..s + len]) {
                *a += *v as f64;
            }
        }
        sum.iter().map(|v| (*v / starts.len() as f64) as f32).collect()
    };
    let first = average(&starts);
    let reach = samples(2.0);
    let span = samples(30.0).min(len);
    let aligned: Vec<usize> = starts
        .iter()
        .map(|&s| {
            let score = |at: usize| -> f32 {
                mono[at + pre..at + pre + span].iter().zip(&first[pre..]).map(|(a, b)| a * b).sum()
            };
            (s - reach..=s + reach).max_by(|&a, &b| score(a).total_cmp(&score(b))).unwrap_or(s)
        })
        .collect();
    let y = average(&aligned);
    let peak = y.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    if peak <= 0.0 {
        return None;
    }
    let at = y.iter().position(|v| v.abs() >= 0.05 * peak).unwrap_or(0);
    Some(y[at..(at + samples(len_ms)).min(y.len())].to_vec())
}

/// A hit's pitch and level over time, from where it starts.
#[derive(Debug, Clone, Default)]
pub struct Curves {
    /// (ms, Hz), one reading per half cycle, from the zero crossings.
    pub pitch: Vec<(f32, f32)>,
    /// (ms, dB against the hit's peak), one reading per half cycle: its peak.
    pub level: Vec<(f32, f32)>,
}

/// Read `y`'s pitch and level, until it is 60 dB down or ends. The first
/// sample is taken as the start of the hit. Both are read per half cycle,
/// between zero crossings: the pitch from its length, the level from its
/// peak, so a 40 Hz tail is as smooth as a 400 Hz snap. A single reading far
/// off its neighbours -- a crossing that noise added or took away -- is
/// replaced by the median of the five around it.
pub fn curves(y: &[f32]) -> Curves {
    let peak = y.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    if peak <= 0.0 {
        return Curves::default();
    }
    let ms = |n: f32| n * 1000.0 / SAMPLE_RATE;
    let mut crossings = vec![0.0f32];
    for i in 1..y.len() {
        if (y[i - 1] < 0.0) != (y[i] < 0.0) {
            crossings.push((i - 1) as f32 + y[i - 1] / (y[i - 1] - y[i]));
        }
    }
    let mut pitch = Vec::new();
    let mut level = Vec::new();
    for w in crossings.windows(2).filter(|w| w[1] > w[0]) {
        let (a, b) = (w[0] as usize, (w[1] as usize + 1).min(y.len()));
        let (at, p) =
            y[a..b].iter().enumerate().fold((a, 0.0f32), |m, (k, v)| if v.abs() > m.1 { (a + k, v.abs()) } else { m });
        level.push((ms(at as f32), 20.0 * (p / peak).max(1e-6).log10()));
        let hz = SAMPLE_RATE / (2.0 * (w[1] - w[0]));
        if (10.0..20_000.0).contains(&hz) {
            pitch.push((ms(0.5 * (w[0] + w[1])), hz));
        }
    }
    // A crossing that noise adds splits a half cycle in two, and the sliver
    // reads far down: each reading is the loudest of the two either side of
    // it, so a sliver cannot pull the level down and a real fall still shows.
    let level: Vec<(f32, f32)> = (0..level.len())
        .map(|i| {
            (level[i].0, level[i.saturating_sub(2)..(i + 3).min(level.len())].iter().fold(f32::MIN, |m, l| m.max(l.1)))
        })
        .collect();
    // Over where it is 60 dB down to stay.
    let over = level.iter().position(|l| l.1 < -60.0 && l.0 > 20.0).map_or(f32::MAX, |i| level[i].0);
    let level: Vec<(f32, f32)> = level.into_iter().filter(|l| l.0 <= over).collect();
    pitch.retain(|p| p.0 <= over);
    let median5 = |v: &[(f32, f32)], i: usize| {
        let lo = i.saturating_sub(2);
        let hi = (i + 3).min(v.len());
        let mut near: Vec<f32> = v[lo..hi].iter().map(|p| p.1).collect();
        near.sort_by(f32::total_cmp);
        near[near.len() / 2]
    };
    let pitch = (0..pitch.len())
        .map(|i| {
            let m = median5(&pitch, i);
            // Off its neighbours by more than a fifth: a crossing gone wrong.
            if (pitch[i].1 / m).log2().abs() > 0.6 {
                (pitch[i].0, m)
            } else {
                pitch[i]
            }
        })
        .collect();
    Curves { pitch, level }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A kick sweeping from `start` to `end` with time constant `tau` ms,
    /// held flat `hold` ms then falling, four to the bar at `bpm`, with a sine
    /// "bass" on every offbeat that the average should thin out.
    fn song(bpm: f32, seconds: f32, end: f32) -> Vec<f32> {
        let n = (seconds * SAMPLE_RATE) as usize;
        let beat = (60.0 / bpm * SAMPLE_RATE) as usize;
        let mut out = vec![0.0f32; n];
        let mut at = 0;
        let mut k = 0;
        while at + beat < n {
            let mut phase = 0.0f32;
            for i in 0..beat {
                let t = i as f32 / SAMPLE_RATE;
                let f = end + (400.0 - end) * (-t / 0.02).exp();
                let env = if t < 0.1 { 1.0 } else { (-(t - 0.1) / 0.05).exp() };
                out[at + i] += 0.8 * env * (std::f32::consts::TAU * phase).sin();
                phase = (phase + f / SAMPLE_RATE).fract();
                // An offbeat bass whose note changes every beat: not locked.
                if i >= beat / 2 {
                    let hz = [55.0, 62.0, 49.0, 73.0][k % 4];
                    out[at + i] += 0.4 * (std::f32::consts::TAU * hz * i as f32 / SAMPLE_RATE).sin();
                }
            }
            at += beat;
            k += 1;
        }
        out
    }

    #[test]
    fn the_average_hit_is_the_kick_that_made_it() {
        let x = song(125.0, 20.0, 47.0);
        let y = average_hit(&x, 125.0, 200.0).expect("a hit");
        let c = curves(&y);
        // Settled on its note, 47 Hz, by 150 ms.
        let late: Vec<f32> = c.pitch.iter().filter(|p| p.0 > 150.0).map(|p| p.1).collect();
        let tail = late.iter().sum::<f32>() / late.len() as f32;
        assert!((tail - 47.0).abs() < 1.0, "tail {tail}");
        // Early, it is still high in its sweep.
        let early = c.pitch.iter().find(|p| p.0 > 5.0).expect("a reading").1;
        assert!(early > 200.0, "at 5 ms {early}");
        // Held flat for 100 ms, then falling.
        let at = |ms: f32| c.level.iter().find(|l| l.0 >= ms).expect("a level").1;
        assert!(at(60.0) > -1.0, "held {}", at(60.0));
        assert!(at(180.0) < -6.0, "falling {}", at(180.0));
    }

    #[test]
    fn too_short_to_average_is_none() {
        assert!(average_hit(&vec![0.0; 1000], 125.0, 200.0).is_none());
    }
}
