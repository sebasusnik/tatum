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
/// where the hit rises from what rang before it. `None` with fewer than four
/// hits.
pub fn average_hit(mono: &[f32], bpm: f32, len_ms: f32) -> Option<Vec<f32>> {
    if !(20.0..=400.0).contains(&bpm) {
        return None;
    }
    let beat = 60.0 / bpm * SAMPLE_RATE;
    let low = Biquad::lowpass(600.0).run_twice(mono);
    // How sharply the low band rises at each sample, its level held at its
    // peak over the last 25 ms: longer than a cycle of the lowest kick, so a
    // steady or falling body never rises, and a hit rises at once, in the
    // first quarter cycle of its body. A level averaged over a short window
    // rose with every half cycle of a 94 Hz body, and over a long one rippled
    // as the sweep slowed: either way the grid could land 10 to 60 ms into
    // the kick, past the lead-in, with the attack cut off.
    let hold = samples(25.0).max(1);
    let mut level = vec![0.0f32; low.len()];
    let mut window: std::collections::VecDeque<(usize, f32)> = std::collections::VecDeque::new();
    for (i, v) in low.iter().enumerate() {
        let v = v.abs();
        while window.back().is_some_and(|b| b.1 <= v) {
            window.pop_back();
        }
        window.push_back((i, v));
        while window.front().is_some_and(|f| f.0 + hold <= i) {
            window.pop_front();
        }
        level[i] = window.front().map_or(0.0, |f| f.1);
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
    // The mark is where that level rises fastest, a few ms into the hit at
    // most: the lead-in is longer, so the hit starts inside it.
    let pre = samples(25.0);
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
    // That alignment is to the body: over 30 ms the low end decides it, and
    // a hit a few samples off still matches. The attack does not: a 900 Hz
    // start is 49 samples a cycle, and hits a few samples apart cancel it
    // in the average -- with the tempo measured 0.01 BPM off, a minimal
    // kick's 900 Hz start averaged into a flat 94 Hz. So each hit is slid
    // again, by at most half a millisecond, to match the average over its
    // first few milliseconds, twice.
    let mut aligned = aligned;
    for _ in 0..2 {
        let y = average(&aligned);
        let h = hit_start(&y).unwrap_or(pre);
        let (w0, w1) = (h.saturating_sub(samples(1.0)), (h + samples(8.0)).min(len));
        let fine = samples(0.5);
        aligned = aligned
            .iter()
            .map(|&s| {
                let score = |at: usize| -> f32 {
                    mono.get(at + w0..at + w1).map_or(f32::MIN, |x| x.iter().zip(&y[w0..w1]).map(|(a, b)| a * b).sum())
                };
                (s.saturating_sub(fine)..=s + fine).max_by(|&a, &b| score(a).total_cmp(&score(b))).unwrap_or(s)
            })
            .filter(|&s| s + len <= mono.len())
            .collect();
    }
    let y = average(&aligned);
    let peak = y.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    if peak <= 0.0 {
        return None;
    }
    // With no step to find, the hit is taken to start at the grid's mark.
    let at = hit_start(&y).unwrap_or(pre);
    Some(y[at..(at + samples(len_ms)).min(y.len())].to_vec())
}

/// Where a hit starts in `y`. Not where it first reaches 5% of its peak: an
/// offbeat bass ringing up to the beat is louder than that, and an average
/// cut there began on the bass, the hit some milliseconds later. The hit is
/// the first sample that is both 30% of the peak and three times the mean
/// level of the 3 ms before it (to half a millisecond before): a sine, however
/// loud, never peaks over 1.6 times its own mean level; a hit coming in over
/// it does. It is looked for in the 20 ms before the hit first comes near
/// its peak. From there it is followed back down its first half cycle, while
/// each sample is quieter than the one after it and of the same sign, for at
/// most a millisecond: to its zero crossing, or to the step where it came in
/// at a phase over the bass. `None` when nothing comes in over what was
/// already sounding: a kick retriggered over its own long tail, or over a
/// rumble as loud as itself, has no step to find.
pub fn hit_start(y: &[f32]) -> Option<usize> {
    if y.is_empty() {
        return None;
    }
    let peak = y.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let (back, gap) = (samples(3.0), samples(0.5).max(1));
    let recent = |i: usize| {
        let w = &y[i.saturating_sub(back)..i.saturating_sub(gap)];
        w.iter().map(|v| v.abs()).sum::<f32>() / w.len().max(1) as f32
    };
    // Looked for in the 20 ms before the hit first comes near its peak, its
    // first cycle: a kick held flat has its peak anywhere along the top. An
    // average's lead-in is 25 ms, so the search starts inside it, never on
    // its first sample, where there would be nothing to compare with.
    let near = y.iter().position(|v| v.abs() >= 0.9 * peak).unwrap_or(0);
    let from = near.saturating_sub(samples(20.0));
    // Before half a millisecond there is nothing to compare with: a hit at
    // the very start of `y` is a hit, as in a kick rendered on its own.
    let sure = (from..=near).find(|&i| y[i].abs() >= 0.3 * peak && (i < gap || y[i].abs() >= 3.0 * recent(i)))?;
    let mut at = sure;
    while at > 0 && sure - at < samples(1.0) && y[at - 1].abs() < y[at].abs() && (y[at - 1] < 0.0) == (y[at] < 0.0) {
        at -= 1;
    }
    Some(at)
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
    fn nothing_has_no_start() {
        assert_eq!(hit_start(&[]), None);
        assert!(crate::fit::cut_like_an_average(&[], 100.0).is_empty());
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
    fn the_average_starts_on_the_hit_not_on_the_bass_before_it() {
        // The bass rings up to the next beat, louder than 5% of the kick.
        let x = song(125.0, 20.0, 47.0);
        let y = average_hit(&x, 125.0, 200.0).expect("a hit");
        let peak = y.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        // The kick starts at its zero crossing and is at full level within a
        // few samples, its first cycle near 400 Hz: the cut is on that rise.
        assert!(y[..8].iter().any(|v| v.abs() > 0.2 * peak), "{:?}", &y[..8]);
        let c = curves(&y);
        assert!(c.pitch[0].1 > 250.0, "the first half cycle is the hit's: {:?}", &c.pitch[..3]);
    }

    #[test]
    fn too_short_to_average_is_none() {
        assert!(average_hit(&vec![0.0; 1000], 125.0, 200.0).is_none());
    }
}
