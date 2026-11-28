//! The bubbles of `sounds/liquid/bubbles.synth`, measured: how many pings a
//! second, how uneven the gaps between them, how long each rings and where
//! it is pitched. These are the numbers the sound was made to, read off a
//! record with a script and then off our render with the same script; this
//! is that script, so the numbers stay true of the sound as the engine and
//! the patch change.
//!
//! The measure: a short-time spectrum of the band the pings are in, each
//! bin against its own median over the whole render, so what stands still
//! is the floor. A frame is active when its loudest bin stands 12 dB over
//! that floor; a run of active frames whose peak does not jump four
//! semitones is one ping, if it lasts 40 ms. It merges pings that fall
//! under 40 ms apart, so it counts fewer than the dust fires and reads
//! their gaps as more even than they are (the dust's own gaps have a
//! spread equal to their mean, which `random_sources` measures).

use tatum_core::song_engine::SongEngine;
use tatum_core::SAMPLE_RATE;

/// A ping the measure found: when it starts and how long it lasts, in
/// seconds, the pitch it starts and ends on in Hz, and its level in dB.
struct Ping {
    at: f32,
    length: f32,
    from: f32,
    to: f32,
    level: f32,
}

/// The pings in `buf` between `fmin` and `fmax` Hz.
fn pings(buf: &[f32], fmin: f32, fmax: f32) -> Vec<Ping> {
    // A window of 1024 samples at 44.1 kHz, read every eighth of it, on a
    // grid four times as fine as the window alone gives.
    let seg = (1024.0 * SAMPLE_RATE / 44100.0).round() as usize;
    let (hop, fine) = (seg / 8, seg * 4);
    let window: Vec<f32> =
        (0..seg).map(|n| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * n as f32 / seg as f32).cos()).collect();
    let scale = 1.0 / window.iter().sum::<f32>();
    let bins: Vec<usize> =
        (0..fine / 2).filter(|&k| (fmin..=fmax).contains(&(k as f32 * SAMPLE_RATE / fine as f32))).collect();
    let tables: Vec<(Vec<f32>, Vec<f32>)> = bins
        .iter()
        .map(|&k| {
            let w = 2.0 * std::f64::consts::PI * k as f64 / fine as f64;
            (0..seg)
                .map(|n| (((w * n as f64).cos()) as f32 * window[n], ((w * n as f64).sin()) as f32 * window[n]))
                .unzip()
        })
        .collect();
    // Eight sums side by side, so the compiler can do them at once.
    let dot = |a: &[f32], b: &[f32]| {
        let mut acc = [0.0f32; 8];
        for (x, y) in a.chunks_exact(8).zip(b.chunks_exact(8)) {
            for i in 0..8 {
                acc[i] += x[i] * y[i];
            }
        }
        let tail = a.len() - a.len() % 8;
        acc.iter().sum::<f32>() + a[tail..].iter().zip(&b[tail..]).map(|(x, y)| x * y).sum::<f32>()
    };
    let frames = (buf.len() - seg) / hop + 1;
    // dB of every bin in every frame.
    let mut db = vec![vec![0.0f32; frames]; bins.len()];
    for f in 0..frames {
        let x = &buf[f * hop..f * hop + seg];
        for (b, (cos, sin)) in tables.iter().enumerate() {
            let (re, im) = (dot(x, cos), dot(x, sin));
            db[b][f] = 20.0 * ((re * re + im * im).sqrt() * scale + 1e-9).log10();
        }
    }
    let floor: Vec<f32> = db
        .iter()
        .map(|row| {
            let mut sorted = row.clone();
            sorted.sort_by(|a, b| a.total_cmp(b));
            let mid = sorted.len() / 2;
            if sorted.len() % 2 == 1 {
                sorted[mid]
            } else {
                0.5 * (sorted[mid - 1] + sorted[mid])
            }
        })
        .collect();
    // The bin that stands highest over its floor, frame by frame.
    let peak: Vec<(usize, f32)> = (0..frames)
        .map(|f| {
            (0..bins.len()).map(|b| (b, db[b][f] - floor[b])).fold((0, f32::MIN), |m, v| if v.1 > m.1 { v } else { m })
        })
        .collect();
    let hz = |f: usize| bins[peak[f].0] as f32 * SAMPLE_RATE / fine as f32;
    let mut runs: Vec<Vec<usize>> = Vec::new();
    for f in (0..frames).filter(|&f| peak[f].1 > 12.0) {
        match runs.last_mut() {
            Some(run)
                if f - run[run.len() - 1] <= 2 && (12.0 * (hz(f) / hz(run[run.len() - 1])).log2()).abs() < 4.0 =>
            {
                run.push(f)
            }
            _ => runs.push(vec![f]),
        }
    }
    let dt = hop as f32 / SAMPLE_RATE;
    runs.iter()
        .map(|run| (run, (run[run.len() - 1] - run[0] + 1) as f32 * dt))
        .filter(|(_, length)| *length >= 0.04)
        .map(|(run, length)| Ping {
            at: (run[0] * hop + seg / 2) as f32 / SAMPLE_RATE,
            length,
            from: hz(run[0]),
            to: hz(run[run.len() - 1]),
            level: run.iter().map(|&f| db[peak[f].0][f]).fold(f32::MIN, f32::max),
        })
        .collect()
}

/// The 10th, 50th and 90th of a hundred, between neighbours in a straight line.
fn spread(values: &[f32]) -> [f32; 3] {
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.total_cmp(b));
    [0.1f32, 0.5, 0.9].map(|q| {
        let at = q * (v.len() - 1) as f32;
        let (i, part) = (at as usize, at.fract());
        v[i] + (v[(i + 1).min(v.len() - 1)] - v[i]) * part
    })
}

fn deviation(v: &[f32]) -> f32 {
    let mean = v.iter().sum::<f32>() / v.len() as f32;
    (v.iter().map(|x| (x - mean) * (x - mean)).sum::<f32>() / v.len() as f32).sqrt()
}

fn within(what: &str, got: f32, want: f32, by: f32) {
    assert!((got - want).abs() <= by, "{what}: {got}, and the sound was made to {want} (give or take {by})");
}

/// 24 bars of one held D#3 at 108 BPM, as the kit plays it: 51 seconds of
/// it from the first. Beside each number, what the same measure reads on
/// the record the sound was made after (Spacetime Continuum, Sea Biscuit).
#[test]
fn the_bubbles_are_as_many_as_uneven_as_long_and_as_wide_as_measured() {
    let src = format!(
        "tempo 108\n{}\npattern stream {{ D#3 ..*15 }}\ntrack t {{ play stream using bubbles out > master }}\n",
        include_str!("../../sounds/liquid/bubbles.synth")
    );
    // As the file `tatum render` writes, which is what the script read:
    // brought to the level every song is, and in 16 bits. The last bit is
    // the floor the quiet bins of the spectrum sit on.
    let mut engine = SongEngine::from_source(&src).unwrap_or_else(|e| panic!("{e}"));
    engine.set_output_gain(tatum_core::dsl::isolate::output_gain(&src).expect("the level of the song"));
    let out: Vec<f32> = engine.render(24).0.iter().map(|s| tatum_core::wav::sample_16(*s) as f32 / 32768.0).collect();
    let (from, to) = (SAMPLE_RATE as usize, (52.0 * SAMPLE_RATE) as usize);
    let found = pings(&out[from..to], 350.0, 1600.0);
    let rate = found.len() as f32 / 51.0;
    let gaps: Vec<f32> = found.windows(2).map(|w| w[1].at - w[0].at).collect();
    let cv = deviation(&gaps) / (gaps.iter().sum::<f32>() / gaps.len() as f32);
    let gap = spread(&gaps);
    let length = spread(&found.iter().map(|p| p.length * 1000.0).collect::<Vec<_>>());
    let start = spread(&found.iter().map(|p| p.from).collect::<Vec<_>>());
    let width = deviation(&found.iter().map(|p| 12.0 * p.from.log2()).collect::<Vec<_>>());
    let bends: Vec<f32> = found.iter().map(|p| 12.0 * (p.to / p.from).log2()).collect();
    let bend = spread(&bends);
    let falling = bends.iter().filter(|b| **b < -1.0).count() as f32 / bends.len() as f32;
    let rising = bends.iter().filter(|b| **b > 1.0).count() as f32 / bends.len() as f32;
    let levels = deviation(&found.iter().map(|p| p.level).collect::<Vec<_>>());
    // How far the onsets keep to the sixteenths of 108 BPM: 1 on the grid,
    // 0 anywhere at all.
    let sixteenth = 60.0 / 108.0 / 4.0;
    let (re, im) = found.iter().fold((0.0f32, 0.0f32), |(re, im), p| {
        let phase = 2.0 * std::f32::consts::PI * (p.at + 1.0) / sixteenth;
        (re + phase.cos(), im + phase.sin())
    });
    let lock = (re * re + im * im).sqrt() / found.len() as f32;
    println!(
        "{rate:.2}/s; gaps {:.3}/{:.3}/{:.3} s, CV {cv:.2}; lock {lock:.2}; length {:.0}/{:.0}/{:.0} ms; \
         start {:.0}/{:.0}/{:.0} Hz, {width:.1} st; bend {:.1}/{:.1}/{:.1} st, {:.0}% falling, {:.0}% rising; levels {levels:.1} dB",
        gap[0], gap[1], gap[2], length[0], length[1], length[2], start[0], start[1], start[2], bend[0], bend[1], bend[2],
        100.0 * falling, 100.0 * rising
    );
    // Off 44.1 kHz the dust draws its gaps in other samples and the stream
    // is another one from the same law, so the numbers are held less tightly.
    let loose = if SAMPLE_RATE == 44100.0 { 1.0 } else { 2.5 };
    // The record: 5.4 pings a second.
    within("pings a second", rate, RATE, 0.25 * loose);
    // The record: gaps of 0.06 / 0.13 / 0.36 s, their spread 0.8 to 1.0 of their mean.
    within("the shortest gaps, s", gap[0], GAP[0], 0.01 * loose);
    within("the middle gap, s", gap[1], GAP[1], 0.015 * loose);
    within("the longest gaps, s", gap[2], GAP[2], 0.04 * loose);
    within("the spread of the gaps over their mean", cv, CV, 0.05 * loose);
    // The record: 0.08 to 0.18 on the grid, where its hats are at 0.86.
    assert!(lock < 0.2, "the pings keep to the sixteenths: {lock}");
    // The record: 41 / 67 / 145 ms.
    within("the shortest pings, ms", length[0], LENGTH[0], 5.0 * loose);
    within("the middle ping, ms", length[1], LENGTH[1], 6.0 * loose);
    within("the longest pings, ms", length[2], LENGTH[2], 15.0 * loose);
    // The record: 463 / 1034 / 1507 Hz, 8.4 semitones of deviation.
    within("the lowest starts, Hz", start[0], START[0], 45.0 * loose);
    within("the middle start, Hz", start[1], START[1], 60.0 * loose);
    within("the highest starts, Hz", start[2], START[2], 60.0 * loose);
    within("the width of the starts, semitones", width, WIDTH, 0.5 * loose);
    // The record: -3.1 / 0 / +2.9 st from start to end; 20 to 35% fall and
    // 18 to 20% rise by over a semitone. (The measure reads a bend into the
    // edge of an impulse too: this is what it reads, not proof of the bend.)
    within("the deepest falls, st", bend[0], BEND[0], 0.6 * loose);
    within("the middle bend, st", bend[1], BEND[1], 0.3 * loose);
    within("the highest rises, st", bend[2], BEND[2], 0.6 * loose);
    within("the share that falls", falling, FALLING, 0.06 * loose);
    within("the share that rises", rising, RISING, 0.06 * loose);
    // The record: 4.3 to 4.6 dB.
    within("the deviation of the levels, dB", levels, LEVELS, 0.5 * loose);
}

// What the measure reads on the bubbles, and the "now" column of the table
// in the pull request that brought them. The first bubbles were a filter
// struck by impulses and read 5.55 a second, gaps of 0.06 / 0.13 / 0.35 s
// spread 0.81, 46 / 73 / 130 ms, 560 / 1023 / 1561 Hz over 7.3 st, bends of
// -3.5 / 0 / +3.0 st with 31% falling and 30% rising, levels 5.1 dB apart.
// These are swells of a resonance over a saw, so a ping starts on one of the
// saw's harmonics: that is the narrower spread of starts.
const RATE: f32 = 5.37;
const GAP: [f32; 3] = [0.056, 0.131, 0.348];
const CV: f32 = 0.92;
const LENGTH: [f32; 3] = [46.0, 67.0, 118.0];
const START: [f32; 3] = [538.0, 1012.0, 1475.0];
const WIDTH: f32 = 6.6;
const BEND: [f32; 3] = [-3.8, 0.0, 3.4];
const FALLING: f32 = 0.33;
const RISING: f32 = 0.31;
const LEVELS: f32 = 5.7;
