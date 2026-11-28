//! `rand`, `wander` and `dust`: the random sources of an instrument graph,
//! and the `rate`, `draw` and `amount` ports that came with them.
//!
//! Each promises a statistic -- a density, a spread, a bound, a rate -- and
//! each test measures it on what the node puts out, at whatever rate the
//! engine is built for. All of it is drawn from seeds: the same song is the
//! same samples every time.

use tatum_core::dsl;
use tatum_core::graph::node::{port, NodeKind, NodeSpec, MAX_NODE_INPUTS};
use tatum_core::graph::voice::Instrument;
use tatum_core::graph::GraphTemplate;
use tatum_core::song_engine::SongEngine;
use tatum_core::{BLOCK_SIZE, SAMPLE_RATE};

const NONE: [f32; MAX_NODE_INPUTS] = [0.0; MAX_NODE_INPUTS];

/// `seconds` of what a source puts out as note number `hit`, struck `clock`
/// samples after its instrument started.
fn run(spec: NodeSpec, hit: u32, clock: u32, seconds: f32) -> Vec<f32> {
    let mut node = spec.instantiate();
    node.strike(&spec, hit, clock);
    (0..(seconds * SAMPLE_RATE) as usize).map(|_| node.process(&NONE, 0)).collect()
}

fn dust(density: f32, jitter: f32, spread_db: f32) -> NodeSpec {
    NodeSpec::Dust { density, jitter, spread_db, level_db: 0.0, seed: 7 }
}

fn rand(spread: f32, bias: f32, glide_ms: f32) -> NodeSpec {
    NodeSpec::Rand { spread, bias, glide_ms, seed: 11 }
}

fn wander(rate: f32, depth: f32, jitter: f32) -> NodeSpec {
    NodeSpec::Wander { rate, depth, jitter, seed: 13 }
}

/// The gaps between impulses, in seconds.
fn gaps(buf: &[f32]) -> Vec<f32> {
    let at: Vec<usize> = (0..buf.len()).filter(|&i| buf[i] != 0.0).collect();
    at.windows(2).map(|w| (w[1] - w[0]) as f32 / SAMPLE_RATE).collect()
}

fn mean(v: &[f32]) -> f32 {
    v.iter().sum::<f32>() / v.len() as f32
}

fn std(v: &[f32]) -> f32 {
    let m = mean(v);
    (v.iter().map(|x| (x - m) * (x - m)).sum::<f32>() / v.len() as f32).sqrt()
}

fn song(body: &str) -> String {
    format!(
        "tempo 120\n\
         instrument i {{\n{body}\n}}\n\
         pattern p {{ C3 - C3 - C3 - C3 - C3 - C3 - C3 - C3 - }}\n\
         track t {{ play p using i out > master }}\n"
    )
}

fn graph(body: &str) -> GraphTemplate {
    let src = song(body);
    let ast = dsl::parse(&src).unwrap_or_else(|e| panic!("{e:?}"));
    let song = dsl::compiler::compile(&ast).unwrap_or_else(|e| panic!("{e:?}"));
    song.instruments[0].as_graph().expect("an instrument graph").clone()
}

/// The error a body does not compile with, and the line of the song it is at.
fn error(body: &str) -> (String, usize) {
    let src = song(body);
    match dsl::parse(&src) {
        Err(e) => (format!("{e:?}"), e.first().map_or(0, |e| e.line)),
        Ok(ast) => match dsl::compiler::compile(&ast) {
            Ok(_) => panic!("compiled:\n{body}"),
            Err(e) => {
                (e.iter().map(|e| e.message.clone()).collect::<Vec<_>>().join("; "), e.first().map_or(0, |e| e.line))
            }
        },
    }
}

/// One note of an instrument, held: `samples` of it.
fn play(inst: &mut Instrument, note: u8, samples: usize) -> Vec<f32> {
    inst.note_on(note, 1.0);
    let mut out = Vec::with_capacity(samples);
    let mut block = [0.0f32; BLOCK_SIZE];
    while out.len() < samples {
        inst.process_block(&mut block);
        out.extend_from_slice(&block);
    }
    inst.note_off(note);
    out.truncate(samples);
    out
}

/// Frequency from the upward zero crossings, in Hz.
fn pitch(buf: &[f32]) -> f32 {
    let ups: Vec<usize> = (1..buf.len()).filter(|&i| buf[i - 1] < 0.0 && buf[i] >= 0.0).collect();
    assert!(ups.len() >= 2, "no full cycle in the window");
    (ups.len() - 1) as f32 * SAMPLE_RATE / (ups[ups.len() - 1] - ups[0]) as f32
}

// ── dust ──

/// Eight a second is eight a second over two minutes, and the gaps are as
/// uneven as a Geiger counter's: their spread equals their mean.
#[test]
fn dust_fires_at_its_density_with_gaps_as_wide_as_their_mean() {
    for density in [0.5f32, 8.0, 60.0] {
        let seconds = 960.0 / density;
        let g = gaps(&run(dust(density, 1.0, 0.0), 0, 0, seconds));
        let rate = g.len() as f32 / seconds;
        assert!((rate / density - 1.0).abs() < 0.08, "{density}/s asked, {rate}/s came");
        let cv = std(&g) / mean(&g);
        assert!((cv - 1.0).abs() < 0.1, "at {density}/s the gaps' spread is {cv} of their mean");
    }
}

/// `jitter` is that spread: 0 a clock, 0.5 half as uneven. The mean holds.
#[test]
fn jitter_is_how_uneven_the_dust_is() {
    for (jitter, low, high) in [(0.0f32, 0.0f32, 0.01f32), (0.5, 0.43, 0.57), (0.8, 0.7, 0.9)] {
        let g = gaps(&run(dust(10.0, jitter, 0.0), 0, 0, 120.0));
        let cv = std(&g) / mean(&g);
        assert!(cv >= low && cv <= high, "jitter {jitter}: spread {cv} of the mean");
        assert!((mean(&g) * 10.0 - 1.0).abs() < 0.06, "jitter {jitter}: a gap of {} s for 10/s", mean(&g));
    }
}

/// Every impulse is one sample, never taller than the level says at this
/// rate, and `spread=` scatters them evenly in dB under it.
#[test]
fn dust_is_bounded_and_spread_scatters_its_level_in_db() {
    let top = SAMPLE_RATE / 44100.0;
    let flat = run(dust(20.0, 1.0, 0.0), 0, 0, 30.0);
    assert!(flat.iter().all(|&v| v == 0.0 || v == top), "an impulse that is not {top}");
    assert!(flat.windows(2).all(|w| w[0] == 0.0 || w[1] == 0.0), "two impulses in a row");

    let spread = run(dust(20.0, 1.0, 12.0), 0, 0, 60.0);
    let db: Vec<f32> = spread.iter().filter(|&&v| v != 0.0).map(|v| 20.0 * (v / top).log10()).collect();
    let (low, high) = db.iter().fold((0.0f32, -99.0f32), |(l, h), &d| (l.min(d), h.max(d)));
    assert!(high <= 0.0 && low >= -12.0, "{low}..{high} dB for spread=12db");
    // Even between 0 and -12: a mean of -6 and a deviation of 12 / sqrt(12).
    assert!((mean(&db) + 6.0).abs() < 0.5, "mean {} dB", mean(&db));
    assert!((std(&db) - 3.46).abs() < 0.3, "deviation {} dB", std(&db));

    // `level=` is that much taller, and nothing else.
    let loud = run(NodeSpec::Dust { density: 20.0, jitter: 1.0, spread_db: 0.0, level_db: 20.0, seed: 7 }, 0, 0, 5.0);
    assert!(loud.iter().zip(flat.iter()).all(|(a, b)| (a - b * 10.0).abs() < 1e-3));
}

/// Each note plays its own stream, and the same note number the same one.
#[test]
fn each_note_has_its_own_dust_and_a_note_is_always_the_same() {
    let a = run(dust(10.0, 1.0, 6.0), 3, 0, 10.0);
    assert_eq!(a, run(dust(10.0, 1.0, 6.0), 3, 0, 10.0));
    let b = run(dust(10.0, 1.0, 6.0), 4, 0, 10.0);
    let shared = (0..a.len()).filter(|&i| a[i] != 0.0 && b[i] != 0.0).count();
    assert!(shared < 3, "{shared} impulses of note 3 fall on one of note 4");
}

/// An octave on the `rate` port is twice the dust.
#[test]
fn the_rate_port_doubles_the_dust_per_octave() {
    let count = |octaves: f32| {
        let spec = dust(10.0, 1.0, 0.0);
        let mut node = spec.instantiate();
        let mut mods = [0.0f32; port::COUNT];
        mods[port::RATE as usize] = octaves;
        let mut n = 0;
        for _ in 0..(60.0 * SAMPLE_RATE) as usize {
            node.modulate(&mods, port::bit(port::RATE));
            n += (node.process(&NONE, 0) != 0.0) as u32;
        }
        n as f32
    };
    let (slow, plain, fast) = (count(-1.0), count(0.0), count(1.0));
    assert!((fast / plain - 2.0).abs() < 0.15, "{plain} to {fast} an octave up");
    assert!((slow / plain - 0.5).abs() < 0.06, "{plain} to {slow} an octave down");
}

// ── rand ──

/// A value per note, held for the note, anywhere within the spread and
/// evenly: over two thousand notes the mean is the middle and the deviation
/// the spread over the root of three.
#[test]
fn rand_holds_one_value_per_note_within_its_spread() {
    let values: Vec<f32> = (0..2000)
        .map(|hit| {
            let held = run(rand(0.5, 0.0, 0.0), hit, 0, 0.01);
            assert!(held.iter().all(|&v| v == held[0]), "note {hit} moved while it was held");
            held[0]
        })
        .collect();
    assert!(values.iter().all(|v| v.abs() <= 0.5));
    assert!(mean(&values).abs() < 0.03, "mean {}", mean(&values));
    assert!((std(&values) - 0.5 / 3.0f32.sqrt()).abs() < 0.02, "deviation {}", std(&values));
    // No two neighbours alike, and no short cycle.
    assert!(values.windows(2).all(|w| w[0] != w[1]));
    let again = (0..values.len() - 7).filter(|&i| values[i] == values[i + 7]).count();
    assert_eq!(again, 0);
    // The same note number is the same value.
    assert_eq!(run(rand(0.5, 0.0, 0.0), 41, 0, 0.01)[0], values[41]);
}

/// `bias=` picks the side: 1 only up, -1 only down, half way three quarters.
#[test]
fn bias_picks_which_way_rand_goes() {
    let range = |bias: f32| {
        let v: Vec<f32> = (0..2000).map(|hit| run(rand(2.0, bias, 0.0), hit, 0, 0.001)[0]).collect();
        v.iter().fold((9.0f32, -9.0f32), |(l, h), &x| (l.min(x), h.max(x)))
    };
    let near = |got: (f32, f32), low: f32, high: f32| {
        assert!(got.0 >= low && got.1 <= high, "{got:?} outside {low}..{high}");
        assert!(got.0 < low + 0.02 && got.1 > high - 0.02, "{got:?} does not fill {low}..{high}");
    };
    near(range(0.0), -2.0, 2.0);
    near(range(1.0), 0.0, 2.0);
    near(range(-1.0), -2.0, 0.0);
    near(range(0.5), -1.0, 2.0);
}

/// Something rising through zero in `draw` draws again, and with `glide=`
/// the new value arrives in a straight line, in that many ms.
#[test]
fn draw_takes_a_new_value_and_glide_walks_to_it() {
    let pulse = |node: &mut NodeKind, x: f32| {
        let mut mods = [0.0f32; port::COUNT];
        mods[port::DRAW as usize] = x;
        node.modulate(&mods, port::bit(port::DRAW));
        node.process(&NONE, 0)
    };
    let spec = rand(1.0, 0.0, 0.0);
    let mut node = spec.instantiate();
    let first = pulse(&mut node, 0.0);
    assert_eq!(pulse(&mut node, 0.0), first, "it moved with nothing drawn");
    let second = pulse(&mut node, 1.0);
    assert_ne!(second, first, "an impulse drew nothing");
    assert_eq!(pulse(&mut node, 1.0), second, "it drew again while the input stayed up");
    assert_eq!(pulse(&mut node, 0.0), second);
    assert_ne!(pulse(&mut node, 0.3), second, "the next rise drew nothing");

    let spec = rand(1.0, 0.0, 50.0);
    let mut node = spec.instantiate();
    let from = pulse(&mut node, 0.0);
    let n = (0.050 * SAMPLE_RATE) as usize;
    let path: Vec<f32> = (0..n + 10).map(|i| pulse(&mut node, if i == 0 { 1.0 } else { 0.0 })).collect();
    let to = path[n + 9];
    assert_ne!(from, to);
    assert_eq!(path[n - 1], to, "not there after 50 ms");
    let half = path[n / 2 - 1];
    assert!((half - (from + to) / 2.0).abs() < 0.01 * (to - from).abs() + 1e-4, "half way it is at {half}");
    let step = (to - from) / n as f32;
    assert!(path[..n].windows(2).all(|w| ((w[1] - w[0]) - step).abs() < 1e-4), "the glide is not a straight line");
}

// ── wander ──

/// Never past its depth, in ten minutes of it, and it uses the room it has.
#[test]
fn wander_stays_within_its_depth() {
    for jitter in [0.0f32, 0.5, 1.0] {
        let w = run(wander(5.0, 0.7, jitter), 0, 0, 600.0);
        let (low, high) = w.iter().fold((0.0f32, 0.0f32), |(l, h), &x| (l.min(x), h.max(x)));
        assert!(low >= -0.7 && high <= 0.7, "{low}..{high} for a depth of 0.7");
        assert!(low < -0.65 && high > 0.65, "{low}..{high}: it never gets near its depth");
        assert!(mean(&w).abs() < 0.03, "it leans: mean {}", mean(&w));
        // Smooth: the largest step is that of an S-curve across the whole
        // depth in the shortest stretch there is, a tenth of the mean.
        let steepest = w.windows(2).map(|p| (p[1] - p[0]).abs()).fold(0.0f32, f32::max);
        assert!(steepest <= 1.5 * 1.4 * 5.0 * 10.0 / SAMPLE_RATE * 1.05, "a step of {steepest}");
    }
}

/// How often it turns round, per second, read every `every` samples: often
/// enough to see every turn, and far enough apart that the last bit of a
/// float on a level stretch is not one.
fn turns(w: &[f32], every: usize) -> f32 {
    let mut n = 0;
    let mut last = 0.0f32;
    for i in (every..w.len()).step_by(every) {
        let d = w[i] - w[i - every];
        if d != 0.0 {
            if last != 0.0 && (d > 0.0) != (last > 0.0) {
                n += 1;
            }
            last = d;
        }
    }
    n as f32 * SAMPLE_RATE / w.len() as f32
}

/// `rate` is destinations a second. A destination is a turn when it is past
/// both its neighbours, two times in three, so the curve turns at two thirds
/// of the rate; at four times the rate, four times as often.
#[test]
fn wander_heads_somewhere_new_at_its_rate() {
    for rate in [0.5f32, 2.0, 8.0] {
        let t = turns(&run(wander(rate, 1.0, 0.5), 0, 0, 1200.0 / rate), (SAMPLE_RATE / rate / 64.0) as usize);
        assert!((t / (rate * 2.0 / 3.0) - 1.0).abs() < 0.1, "at {rate} Hz it turns {t} times a second");
    }
}

/// How alike the curve is to itself `lag` samples on, -1..1.
fn autocorrelation(w: &[f32], lag: usize) -> f32 {
    let m = mean(w);
    let n = w.len() - lag;
    let num: f32 = (0..n).step_by(16).map(|i| (w[i] - m) * (w[i + lag] - m)).sum();
    let den: f32 = (0..n).step_by(16).map(|i| (w[i] - m) * (w[i] - m)).sum();
    num / den
}

/// It has no period. A tenth of a stretch on it is still nearly where it
/// was; from two stretches on it has nothing to do with where it was, and
/// that never comes back: no lag up to fifty stretches lines up again, on a
/// steady clock or an uneven one. An LFO at the same rate comes back to 1.
#[test]
fn wander_never_repeats() {
    let rate = 4.0f32;
    let stretch = (SAMPLE_RATE / rate) as usize;
    for jitter in [0.0f32, 0.5, 1.0] {
        let w = run(wander(rate, 1.0, jitter), 0, 0, 900.0);
        let near = autocorrelation(&w, stretch / 10);
        assert!(near > 0.9, "jitter {jitter}: a tenth of a stretch on, {near}");
        let mut worst = (0usize, 0.0f32);
        for k in 8..200 {
            let r = autocorrelation(&w, k * stretch / 4);
            if r.abs() > worst.1.abs() {
                worst = (k, r);
            }
        }
        assert!(
            worst.1.abs() < 0.1,
            "jitter {jitter}: {} stretches on it is {} like itself",
            worst.0 as f32 / 4.0,
            worst.1
        );
    }
    // The measure can tell: a sine at that rate lines up a whole cycle on.
    let sine: Vec<f32> = (0..(60.0 * SAMPLE_RATE) as usize)
        .map(|i| (std::f32::consts::TAU * rate * i as f32 / SAMPLE_RATE).sin())
        .collect();
    assert!(autocorrelation(&sine, stretch * 5) > 0.95);
}

/// One curve for the whole instrument: a note struck later finds it where
/// it is by then, to within what a float adds up to along the way.
#[test]
fn a_later_note_finds_the_wander_where_it_is() {
    let spec = wander(3.0, 1.0, 0.7);
    let whole = run(spec, 0, 0, 40.0);
    for seconds in [0.37f32, 5.0, 12.345, 33.3] {
        let clock = (seconds * SAMPLE_RATE) as u32;
        let late = run(spec, 9, clock, 2.0);
        let off = late.iter().zip(&whole[clock as usize..]).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(off < 2e-3, "struck at {seconds} s it is {off} away from the curve");
    }
    // And another seed is another curve.
    let other = run(NodeSpec::Wander { rate: 3.0, depth: 1.0, jitter: 0.7, seed: 14 }, 0, 0, 40.0);
    let apart = mean(&whole.iter().zip(&other).map(|(a, b)| (a - b).abs()).collect::<Vec<_>>());
    assert!(apart > 0.3, "two seeds, {apart} apart on average");
}

// ── in a graph ──

const AMP: &str = "  src > adsr(0.001, 0.1, 1.0, 0.05) as amp\n  amp > out";

/// `rand` on an oscillator's `pitch`: the same key lands on a different
/// pitch every time, within the spread, and playing it again from the top
/// lands on the same ones.
#[test]
fn every_note_gets_its_own_pitch_and_the_same_one_every_time() {
    let template = graph(&format!("  osc sine(55) as src\n  rand(0.25) as r\n  r > src.pitch\n{AMP}"));
    assert_ne!(template.random_mask, 0);
    let n = (0.2 * SAMPLE_RATE) as usize;
    let take = || {
        let mut inst = Instrument::new(template.clone());
        (0..12).map(|_| pitch(&play(&mut inst, 69, n)[n / 4..])).collect::<Vec<f32>>()
    };
    let pitches = take();
    for (i, p) in pitches.iter().enumerate() {
        let octaves = (p / 440.0).log2();
        assert!(octaves.abs() <= 0.26, "note {i} is {octaves} octaves off, with a spread of 0.25");
    }
    for w in pitches.windows(2) {
        assert!((w[1] / w[0]).log2().abs() > 0.002, "two notes in a row at {} Hz", w[0]);
    }
    let cents = std(&pitches.iter().map(|p| 1200.0 * (p / 440.0).log2()).collect::<Vec<_>>());
    assert!(cents > 80.0, "twelve notes spread {cents} cents");
    assert_eq!(pitches, take());
}

/// Two instruments written alike do not play the same values: the name is
/// stirred into the seeds.
#[test]
fn two_instruments_written_alike_differ() {
    let body = "  osc sine(55) as src\n  rand(0.25) as r\n  r > src.pitch\n  src > adsr(0.001, 0.1, 1.0, 0.05) as amp\n  amp > out";
    let src = format!(
        "tempo 120\ninstrument one {{\n{body}\n}}\ninstrument two {{\n{body}\n}}\n\
         pattern p {{ C3 }}\ntrack a {{ play p using one out > master }}\ntrack b {{ play p using two out > master }}\n"
    );
    let song = dsl::compiler::compile(&dsl::parse(&src).unwrap()).unwrap();
    let seed = |i: usize| {
        let t = song.instruments[i].as_graph().unwrap();
        t.specs.iter().find_map(|s| if let NodeSpec::Rand { seed, .. } = s { Some(*seed) } else { None }).unwrap()
    };
    assert_ne!(seed(0), seed(1));
}

/// The bubbles: one held note, a dust pinging a resonant bandpass, a `rand`
/// drawn by each impulse moving where it rings. Bounded, not silent, off the
/// grid, and the same samples on every render.
const BUBBLES: &str = r#"
tempo 110
instrument bubbles {
  dust(6hz, jitter=0.9, spread=14db, level=30db) as d
  rand(1.0) as where
  rand(0.2, glide=70ms) as bend
  wander(0.3hz, 0.5) as tide
  svf(900, 0.93, bandpass) as ping
  d > where.draw
  d > bend.draw
  d > ping
  where > ping.cutoff
  bend > ping.cutoff
  tide > ping.cutoff
  tide > d.rate
  ping > adsr(0.005, 0.1, 1.0, 0.3) as amp
  amp > out
}
pattern p { C3 ..*15 }
track t { play p using bubbles out > master }
"#;

#[test]
fn bubbles_are_bounded_irregular_and_the_same_on_every_render() {
    let render = || SongEngine::from_source(BUBBLES).unwrap_or_else(|e| panic!("{e}")).render(8).0;
    let a = render();
    assert!(a.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
    let peak = a.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(peak > 0.05, "peak {peak}");
    assert_eq!(a, render(), "two renders of one song differ");

    // Pings: where the level jumps up out of the tail before it. Their gaps
    // are uneven, which no pattern on a grid would be.
    let hop = (0.005 * SAMPLE_RATE) as usize;
    let level: Vec<f32> = a.chunks(hop).map(|c| c.iter().fold(0.0f32, |m, v| m.max(v.abs()))).collect();
    let onsets: Vec<f32> = (2..level.len())
        .filter(|&i| level[i] > 0.02 * peak && level[i] > 2.0 * level[i - 2])
        .map(|i| i as f32 * 0.005)
        .collect();
    let gaps: Vec<f32> = onsets.windows(2).map(|w| w[1] - w[0]).filter(|g| *g > 0.02).collect();
    assert!(gaps.len() > 30, "{} pings in eight bars", gaps.len());
    let cv = std(&gaps) / mean(&gaps);
    assert!(cv > 0.6, "the gaps' spread is {cv} of their mean");
}

/// An octave on an LFO's `rate` port is a vibrato twice as fast.
#[test]
fn the_rate_port_speeds_an_lfo_up() {
    let wobbles = |wire: &str| {
        let body = format!(
            "  osc sine(55) as src\n  lfo(4, 0.05) as vib\n  adsr(0.001, 10, 1.0, 0.1) as one\n{wire}\n  vib > src.pitch\n{AMP}"
        );
        let mut inst = Instrument::new(graph(&body));
        let out = play(&mut inst, 69, (4.0 * SAMPLE_RATE) as usize);
        // The pitch over time, a cycle at a time, and how often it crosses its mean.
        let ups: Vec<f32> = (1..out.len())
            .filter(|&i| out[i - 1] < 0.0 && out[i] >= 0.0)
            .map(|i| (i - 1) as f32 + out[i - 1] / (out[i - 1] - out[i]))
            .collect();
        let periods: Vec<f32> = ups.windows(2).map(|w| w[1] - w[0]).collect();
        let m = mean(&periods);
        periods.windows(2).filter(|w| w[0] < m && w[1] >= m).count() as f32 / 4.0
    };
    let plain = wobbles("");
    let fast = wobbles("  one > gain(1) > vib.rate");
    assert!((plain - 4.0).abs() < 0.6, "a 4 Hz vibrato read as {plain} Hz");
    assert!((fast / plain - 2.0).abs() < 0.2, "{plain} Hz to {fast} Hz an octave up");
}

/// A share on a gain's `amount` port: 0.5 is half as much again, -1 silence,
/// and a gain with nothing wired there is the plain one.
#[test]
fn the_amount_port_scales_a_gain_by_a_share() {
    let level = |wire: &str| {
        let body = format!(
            "  osc sine(55) as src\n  gain(0.5) as vol\n  adsr(0.001, 10, 1.0, 0.1) as one\n  src > vol\n{wire}\n  vol > adsr(0.001, 0.1, 1.0, 0.05) as amp\n  amp > out"
        );
        let mut inst = Instrument::new(graph(&body));
        let out = play(&mut inst, 69, 22050);
        out[4000..].iter().fold(0.0f32, |m, v| m.max(v.abs()))
    };
    let plain = level("");
    assert!((level("  one > gain(0.5) > vol.amount") / plain - 1.5).abs() < 0.01);
    assert!(level("  one > gain(1) as full\n  full > gain(1.5) > vol.amount") / plain > 2.4);
    assert!(level("  one > gain(0) > vol.amount") / plain > 0.999);
    // -1 and under is silence, not the sound upside down.
    let mut node = NodeSpec::Gain { amount: 0.5 }.instantiate();
    let mut through = |by: f32| {
        let mut mods = [0.0f32; port::COUNT];
        mods[port::AMOUNT as usize] = by;
        node.modulate(&mods, port::bit(port::AMOUNT));
        let mut inputs = NONE;
        inputs[0] = 1.0;
        node.process(&inputs, 1)
    };
    assert_eq!(through(-0.5), 0.25);
    assert_eq!(through(-1.0), 0.0);
    assert_eq!(through(-1.5), 0.0);
    assert_eq!(through(0.0), 0.5);
}

/// A graph with none of them is told nothing on a note, as before.
#[test]
fn a_graph_without_them_has_nothing_to_strike() {
    let t = graph(&format!("  osc saw(55) as src\n  lfo(3, 0.1) > src.pitch\n{AMP}"));
    assert_eq!(t.random_mask, 0);
}

// ── errors ──

/// An argument out of its range is an error at the node's line (the fourth
/// of the song: tempo, instrument, the oscillator, then this).
#[test]
fn out_of_range_arguments_are_errors_at_the_line() {
    for (node, word) in [
        ("dust(0hz)", "density"),
        ("dust(5000hz)", "density"),
        ("dust(5hz, jitter=1.5)", "jitter"),
        ("dust(5hz, spread=90db)", "spread"),
        ("dust(5hz, level=70db)", "level"),
        ("rand(9)", "spread"),
        ("rand(1, bias=2)", "bias"),
        ("rand(1, glide=20s)", "glide"),
        ("wander(100hz)", "rate"),
        ("wander(0.001hz)", "rate"),
        ("wander(1hz, 9)", "depth"),
        ("wander(1hz, 1, jitter=2)", "jitter"),
    ] {
        let (message, line) = error(&format!("  osc sine(55) as src\n  {node} as x\n{AMP}"));
        assert!(message.contains("out of range") && message.contains(word), "{node}: {message}");
        assert_eq!(line, 4, "{node}: {message}");
    }
    for (node, word) in
        [("dust(5hz, colour=1)", "colour"), ("rand(1, 2)", "too many"), ("wander(1hz, wobble=2)", "wobble")]
    {
        let (message, line) = error(&format!("  osc sine(55) as src\n  {node} as x\n{AMP}"));
        assert!(message.contains(word), "{node}: {message}");
        assert_eq!(line, 4, "{node}: {message}");
    }
    // A unit that is not the argument's.
    let (message, _) = error(&format!("  osc sine(55) as src\n  dust(5ms) as x\n{AMP}"));
    assert!(message.contains("hz"), "{message}");
}

/// The ports are theirs: `draw` is a `rand`'s, `rate` an LFO's and a dust's
/// (a `wander` has none: its curve is a function of time), `amount` a gain's.
#[test]
fn a_port_on_the_wrong_node_is_an_error_at_the_line() {
    for (nodes, wire, word) in [
        ("  wander(1hz, 1) as w", "one > w.rate", "no ports"),
        ("  rand(1) as r", "one > r.rate", "no `rate` port"),
        ("  dust(5hz) as d", "one > d.draw", "no `draw` port"),
        ("  lfo(1, 0.5) as l", "one > l.amount", "no `amount` port"),
    ] {
        let body = format!("  osc sine(55) as src\n  adsr(0.001, 10, 1.0, 0.1) as one\n{nodes}\n  {wire}\n{AMP}");
        let (message, line) = error(&body);
        assert!(message.contains(word), "{wire}: {message}");
        assert_eq!(line, 6, "{wire}: {message}");
    }
}

/// A `rand` or a `wander` is a control signal, as an LFO is: into a port and
/// into the sound as well is refused, where it would sit in the output as DC.
#[test]
fn a_random_control_is_kept_out_of_the_sound() {
    for source in ["rand(1)", "wander(1hz, 1)"] {
        let body = format!(
            "  osc sine(55) as src\n  {source} as r\n  r > src.pitch\n  r > mix\n  src > mix\n  mix > adsr(0.001, 0.1, 1.0, 0.05) as amp\n  amp > out"
        );
        let (message, line) = error(&body);
        assert!(message.contains("control signal"), "{source}: {message}");
        assert_eq!(line, 6, "{source}: {message}");
    }
    // A dust is sound: it rings the filter and draws the value at once.
    graph("  dust(5hz) as d\n  rand(1) as r\n  svf(800, 0.9, bandpass) as f\n  d > r.draw\n  d > f\n  r > f.cutoff\n  f > adsr(0.001, 0.1, 1.0, 0.05) as amp\n  amp > out");
}

/// Without their parentheses the three words are names, as they always were.
#[test]
fn a_node_named_dust_stays_a_name() {
    let t = graph(
        "  osc saw(55) as rand\n  gain(0.5) as dust\n  lowpass(900, 0.1) as wander\n  rand > dust\n  dust > wander\n  wander > adsr(0.001, 0.1, 1.0, 0.05) as amp\n  amp > out",
    );
    assert_eq!(t.random_mask, 0);
}

// ── the wander and the song's clock ──

/// Frequency from the upward zero crossings, each placed between its two
/// samples: fine enough to compare two pitches at any rate.
fn pitch_between(buf: &[f32]) -> f32 {
    let ups: Vec<f32> = (1..buf.len())
        .filter(|&i| buf[i - 1] < 0.0 && buf[i] >= 0.0)
        .map(|i| (i - 1) as f32 + buf[i - 1] / (buf[i - 1] - buf[i]))
        .collect();
    assert!(ups.len() >= 2, "no full cycle in the window");
    (ups.len() - 1) as f32 * SAMPLE_RATE / (ups[ups.len() - 1] - ups[0])
}

/// A track `t` whose filter wanders, in a song of scenes: `a` has it and
/// `b` does not. Its note is over well before the bar is, so a scene finds
/// it silent, and it is quiet enough to stay under the limiter at the
/// output, whose gain would remember the bars before. At 120 BPM a bar is
/// two seconds.
fn scenes(arrange: &str) -> String {
    format!(
        "tempo 120\n\
         instrument i {{\n  osc saw(110) as a\n  svf(400hz, 0.5) as f\n  wander(2hz, 2) as w\n  adsr(0.01, 0.2, 1, 0.05) as amp\n  \
         a > f\n  w > f.cutoff\n  f > amp\n  amp > out\n}}\n\
         instrument j {{\n  osc sine(110) as a\n  adsr(0.01, 0.2, 0.6, 0.3) as amp\n  a > amp\n  amp > out\n}}\n\
         pattern p {{ C3 ..*5 - - E3 ..*5 - - }}\n\
         pattern q {{ C2 - - - }}\n\
         track t {{ play p using i level 0.2 out > master }}\n\
         track u {{ play q using j level 0.0 }}\n\
         scene a {{ track t {{ play p using i level 0.2 }} track u {{ play q using j }} }}\n\
         scene b {{ track u {{ play q using j }} }}\n\
         arrange {{ {arrange} }}\n"
    )
}

/// A song in blocks of `chunk` samples, as a sound card or a set asks for it.
fn in_blocks(src: &str, bars: usize, chunk: usize) -> Vec<f32> {
    let mut e = SongEngine::from_source(src).unwrap_or_else(|e| panic!("{e}"));
    let total = bars * (16.0 * SAMPLE_RATE * 60.0 / 120.0 / 4.0) as usize;
    e.start();
    let mut out = Vec::with_capacity(total);
    while out.len() < total {
        let c = chunk.min(total - out.len());
        let (mut l, mut r) = ([0.0f32; BLOCK_SIZE], [0.0f32; BLOCK_SIZE]);
        e.process_block_stereo(&mut l[..c], &mut r[..c]);
        out.extend_from_slice(&l[..c]);
    }
    out
}

fn first_difference(a: &[f32], b: &[f32]) -> Option<usize> {
    a.iter().zip(b).position(|(x, y)| x.to_bits() != y.to_bits())
}

/// The curve is the song's, not the track's: a track that sits two bars out
/// comes back to it where a track that never left has it. It used to stop
/// with the track, and bar 3 after two bars out was bar 1 played again.
#[test]
fn a_wander_goes_on_while_its_track_sits_out_a_scene() {
    let bar = (16.0 * SAMPLE_RATE * 60.0 / 120.0 / 4.0) as usize;
    let always = in_blocks(&scenes("a x1 a x2 a x2"), 5, BLOCK_SIZE);
    let away = in_blocks(&scenes("a x1 b x2 a x2"), 5, BLOCK_SIZE);
    assert!(away[bar + bar / 8..3 * bar].iter().all(|v| *v == 0.0), "the track is out for bars 1 and 2");
    let heard = always[3 * bar..].iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(heard > 0.005, "the track is back from bar 3: {heard}");
    if let Some(i) = first_difference(&always[3 * bar..], &away[3 * bar..]) {
        let at = 3 * bar + i;
        panic!(
            "back in the scene it differs from the track that never left at sample {at}: {} against {}",
            away[at], always[at]
        );
    }
    // The curve does move in that time: bar 3 is not bar 1.
    let moved =
        always[bar..2 * bar].iter().zip(&always[3 * bar..4 * bar]).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
    assert!(moved > 0.05 * heard, "bar 1 and bar 3 are {moved} apart at the most");
}

/// A sound card asks for blocks of its own size and a set cuts them at its
/// steps. The song is the same samples whatever the size, scenes and all:
/// where the curve is does not depend on how many blocks a track faded over.
#[test]
fn a_wander_is_the_same_at_any_block_size() {
    for arrange in ["a x1 b x1 a x2", "a x4"] {
        let src = scenes(arrange);
        let whole = in_blocks(&src, 4, BLOCK_SIZE);
        for chunk in [100, 64, 37] {
            let cut = in_blocks(&src, 4, chunk);
            if let Some(i) = first_difference(&whole, &cut) {
                panic!(
                    "`{arrange}` in blocks of {chunk} differs from blocks of {BLOCK_SIZE} at sample {i}: {} against {}",
                    cut[i], whole[i]
                );
            }
        }
    }
}

/// Started at a bar, the song finds the curve where it is at that bar when
/// played from the top. The wander is on the pitch, which is what is
/// compared: a song started at a bar has its own fades and level.
#[test]
fn a_song_started_at_a_bar_finds_the_wander_there() {
    let bar = (16.0 * SAMPLE_RATE * 60.0 / 120.0 / 4.0) as usize;
    let src = "tempo 120
         instrument i {
  osc sine(220) as src
  wander(2hz, 1) as w
  w > src.pitch
  src > adsr(0.01, 0.2, 1, 0.05) as amp
  amp > out
}
         pattern p { C4 ..*13 - - }
         track t { play p using i out > master }
";
    let whole = in_blocks(src, 5, BLOCK_SIZE);
    let mut e = SongEngine::from_source(src).unwrap_or_else(|e| panic!("{e}"));
    e.start_from_bar(3);
    let (late, _) = e.render_steps(32);
    let window = (0.05 * SAMPLE_RATE) as usize;
    let mut apart = 0.0f32;
    for seconds in [0.1f32, 0.4, 0.75, 1.1, 1.5, 2.2, 2.9, 3.4] {
        let at = (seconds * SAMPLE_RATE) as usize;
        let (here, there) =
            (pitch_between(&late[at..at + window]), pitch_between(&whole[3 * bar + at..3 * bar + at + window]));
        assert!((here / there - 1.0).abs() < 0.003, "{seconds} s after bar 3: {here} Hz, and {there} Hz from the top");
        apart = apart.max((here / pitch_between(&whole[at..at + window]) - 1.0).abs());
    }
    // And bar 3 is not bar 0: the curve is somewhere else by then.
    assert!(apart > 0.05, "bars 0 and 3 are within {apart} of each other in pitch");
}

/// A voice that was not run for a while -- its track out of the scene with
/// the note still ringing -- is put back on the curve when it runs again,
/// and a voice that ran all along is left alone.
#[test]
fn a_voice_that_was_not_run_is_put_back_on_the_curve() {
    let template = graph("  osc sine(220) as src\n  wander(3hz, 1) as w\n  w > src.pitch\n  src > adsr(0.001, 0.1, 1.0, 0.05) as amp\n  amp > out");
    let block = |inst: &mut Instrument| {
        let mut b = [0.0f32; BLOCK_SIZE];
        inst.process_block(&mut b);
        b
    };
    // One instrument plays every block. The other is given the time, as the
    // song engine gives it, after sitting 5000 blocks out.
    let mut ran = Instrument::new(template.clone());
    let mut sat = Instrument::new(template.clone());
    ran.note_on(57, 1.0);
    sat.note_on(57, 1.0);
    for _ in 0..10 {
        assert_eq!(block(&mut ran), block(&mut sat));
    }
    for _ in 0..5000 {
        block(&mut ran);
    }
    sat.set_clock(5010 * BLOCK_SIZE as u32);
    // The oscillator's phase is wherever each left it, so the pitch is what
    // is compared, cycle by cycle: where the curve has the oscillator over
    // half a second.
    let half_a_second = |inst: &mut Instrument| {
        let mut out = Vec::new();
        for _ in 0..(0.5 * SAMPLE_RATE) as usize / BLOCK_SIZE {
            out.extend_from_slice(&block(inst));
        }
        out
    };
    // (when, Hz) of every cycle: its middle and one over its length.
    let cycles = |buf: &[f32]| -> Vec<(f32, f32)> {
        let ups: Vec<f32> = (1..buf.len())
            .filter(|&i| buf[i - 1] < 0.0 && buf[i] >= 0.0)
            .map(|i| (i - 1) as f32 + buf[i - 1] / (buf[i - 1] - buf[i]))
            .collect();
        ups.windows(2).map(|w| (0.5 * (w[0] + w[1]), SAMPLE_RATE / (w[1] - w[0]))).collect()
    };
    // How far `a` is from `b` at the worst, as a share: each cycle of `a`
    // against `b` at the same moment, between its two nearest cycles.
    let apart = |a: &[(f32, f32)], b: &[(f32, f32)]| {
        let mut worst = 0.0f32;
        for &(t, hz) in a {
            let Some(i) = b.windows(2).position(|w| w[0].0 <= t && t < w[1].0) else { continue };
            let there = b[i].1 + (b[i + 1].1 - b[i].1) * (t - b[i].0) / (b[i + 1].0 - b[i].0);
            worst = worst.max((hz / there - 1.0).abs());
        }
        worst
    };
    let (a, b) = (cycles(&half_a_second(&mut ran)), cycles(&half_a_second(&mut sat)));
    assert!(a.len() > 40 && b.len() > 40, "half a second of cycles: {} and {}", a.len(), b.len());
    let off = apart(&a, &b);
    assert!(off < 0.005, "the voice that sat out is {off} of its pitch away from the one that ran");
    // Without the time it is told, it would still be where it stopped.
    let mut stale = Instrument::new(template);
    stale.note_on(57, 1.0);
    for _ in 0..10 {
        block(&mut stale);
    }
    let off = apart(&a, &cycles(&half_a_second(&mut stale)));
    assert!(off > 0.05, "the curve had not moved in 5000 blocks: {off} apart");
}

// ── the dust at the ends of its range ──

/// What a dust at `density` does over `seconds` with `octaves` in its
/// `rate` port: impulses, and pairs of them on neighbouring samples.
fn pushed(density: f32, octaves: f32, hit: u32, seconds: f32) -> (usize, usize) {
    let spec = dust(density, 1.0, 0.0);
    let mut node = spec.instantiate();
    node.strike(&spec, hit, 0);
    let mut mods = [0.0f32; port::COUNT];
    mods[port::RATE as usize] = octaves;
    let (mut n, mut pairs, mut last) = (0, 0, false);
    for _ in 0..(seconds * SAMPLE_RATE) as usize {
        node.modulate(&mods, port::bit(port::RATE));
        let v = node.process(&NONE, 0);
        assert!(v.is_finite());
        let fired = v != 0.0;
        n += fired as usize;
        pairs += (fired && last) as usize;
        last = fired;
    }
    (n, pairs)
}

/// The `rate` port moves the density inside the range one can be written
/// in. Eight octaves over 60 Hz asked for 15360 a second and got 14912 with
/// 126629 pairs of impulses on neighbouring samples; over 2000 Hz it was an
/// impulse on every sample, a level and not dust.
#[test]
fn the_rate_port_keeps_the_dust_within_its_range() {
    for (density, octaves) in [(60.0, 8.0), (2000.0, 8.0), (2000.0, 3.0), (5.0, 40.0), (60.0, f32::INFINITY)] {
        let (n, pairs) = pushed(density, octaves, 0, 20.0);
        let rate = n as f32 / 20.0;
        assert_eq!(pairs, 0, "dust({density}) with {octaves} octaves in `rate` fired on neighbouring samples");
        assert!(rate < 2000.0 * 1.03, "dust({density}) with {octaves} octaves in `rate`: {rate} a second");
        assert!(rate > 2000.0 * 0.9, "dust({density}) with {octaves} octaves in `rate`: only {rate} a second");
    }
    // An octave inside the range is still an octave.
    let (plain, _) = pushed(500.0, 0.0, 0, 20.0);
    let (up, pairs) = pushed(500.0, 1.0, 0, 20.0);
    assert!((up as f32 / plain as f32 - 2.0).abs() < 0.1, "{plain} to {up} an octave up");
    assert_eq!(pairs, 0);
    // The low end: twenty octaves under 5 Hz would be one impulse in two
    // days. It is one in a hundred seconds, the slowest a dust is written.
    let mut n = 0;
    for hit in 0..12 {
        n += pushed(5.0, -20.0, hit, 500.0).0;
    }
    let rate = n as f32 / (12.0 * 500.0);
    assert!((rate / 0.01 - 1.0).abs() < 0.5, "{n} impulses in 6000 s, {rate} a second for 0.01");
    // Not a number in the port is nothing in the port.
    let (odd, _) = pushed(500.0, f32::NAN, 0, 20.0);
    assert_eq!(odd, plain);
}

/// Notes of `dust(0.01hz)` (seed 7) whose first impulse comes more than
/// 4.1 mean gaps in: over 2^24 samples at 44.1 kHz.
const LONG_FIRST_GAPS: [u32; 2] = [7, 313];

/// The countdown to the next impulse is counted in whole numbers. As an
/// `f32` it stopped changing once the gap was over 2^24 samples (six minutes
/// at 44.1 kHz): at a hundredth of a Hz one note in forty drew such a gap
/// and its dust never fired again. These notes are ones whose first gap is
/// that long; the longest a gap can be is eight times the mean.
#[test]
fn the_longest_gap_of_the_slowest_dust_ends() {
    let spec = dust(0.01, 1.0, 0.0);
    let mean = SAMPLE_RATE as f64 * 100.0;
    let mut long = 0;
    for hit in LONG_FIRST_GAPS {
        let mut node = spec.instantiate();
        node.strike(&spec, hit, 0);
        let limit = (8.0 * mean) as usize + 2;
        let at = (0..limit).position(|_| node.process(&NONE, 0) != 0.0);
        let at = at.unwrap_or_else(|| panic!("note {hit} of dust(0.01hz) had not fired after {limit} samples"));
        long += (at > 1 << 24) as usize;
    }
    if SAMPLE_RATE == 44100.0 {
        assert_eq!(long, LONG_FIRST_GAPS.len(), "these notes wait over 2^24 samples for their first impulse");
    }
}

// ── what the reference says ──

/// The section of docs/DSL.md on the random sources.
fn the_section() -> String {
    let doc = include_str!("../../docs/DSL.md");
    let from = doc.find("**Bubbles, drops and sweeps that never repeat").expect("the section on the random sources");
    let to = from + doc[from..].find("**A deep minimal kick.**").expect("the section after it");
    doc[from..to].to_string()
}

/// Every example the reference gives for these nodes compiles: the patch in
/// its fence, and each wire written in a sentence (`rand(0.5) > filt.cutoff`),
/// put into an instrument that has the nodes the sentences name. One of
/// them said `rand(1/12)`, which is a rhythm division and an error.
#[test]
fn every_example_in_the_reference_compiles() {
    let section = the_section();
    // The fenced patch, as a song.
    let fence = section.split("```").nth(1).expect("the bubbles in a fence");
    let src = format!("tempo 108\n{fence}\ntrack t {{ play stream using bubbles out > master }}\n");
    let (out, _) = SongEngine::from_source(&src).unwrap_or_else(|e| panic!("the fenced example: {e}")).render(2);
    assert!(out.iter().any(|v| v.abs() > 1e-3), "the fenced example is silent");

    // The wires in the prose: code spans that send a source into a port.
    let prose: String = section.split("```").step_by(2).collect::<Vec<_>>().join(" ");
    let prose = prose.split_whitespace().collect::<Vec<_>>().join(" ");
    let wires: Vec<&str> = prose
        .split('`')
        .skip(1)
        .step_by(2)
        .filter(|span| span.contains(" > ") && ["rand(", "wander(", "dust("].iter().any(|n| span.starts_with(n)))
        .collect();
    assert!(wires.len() >= 4, "the wires the section writes out: {wires:?}");
    for wire in wires {
        let (source, to) = wire.split_once(" > ").expect("a wire");
        let body = format!(
            "  osc saw(110) as a\n  svf(800hz, 0.3) as filt\n  gain(1) as vol\n  dust(5hz, level=30db) as d\n  \
             {source} as it\n  it > {to}\n  a > filt\n  d > filt\n  filt > vol\n  vol > adsr(0.01, 0.1, 1, 0.1) as amp\n  amp > out"
        );
        let src = song(&body);
        if let Err(e) = SongEngine::from_source(&src) {
            panic!("`{wire}` in docs/DSL.md does not compile: {e}");
        }
    }
}
