//! The delay's wow and flutter: a bucket-brigade wobble on the repeats.
//!
//! At 0 the delay must play exactly what it did before it had them, so every
//! song written before them sounds the same. Above 0 the repeats must bend in
//! pitch, slowly for wow and quickly for flutter, the same way every render,
//! at whatever rate the engine was built for.

use tatum_core::dsl::{self, compiler};
use tatum_core::effects::delay::{Delay, DelaySync};
use tatum_core::graph::node::NodeSpec;
use tatum_core::song_engine::SongEngine;
use tatum_core::SAMPLE_RATE;

/// The delay as it was before wow and flutter, line for line: the reference
/// a delay with both at 0 must match sample for sample.
mod before {
    pub struct Delay {
        buffer_l: Vec<f32>,
        buffer_r: Vec<f32>,
        write_pos: usize,
        delay_samples: usize,
        feedback: f32,
        mix: f32,
        fb_filter_l: f32,
        fb_filter_r: f32,
        fb_filter_coeff: f32,
    }

    impl Delay {
        pub fn new(sample_rate: f32, max_time: f32) -> Self {
            let max_samples = (sample_rate * max_time) as usize + 1;
            Self {
                buffer_l: vec![0.0; max_samples],
                buffer_r: vec![0.0; max_samples],
                write_pos: 0,
                delay_samples: (sample_rate * 0.3) as usize,
                feedback: 0.35,
                mix: 0.2,
                fb_filter_l: 0.0,
                fb_filter_r: 0.0,
                fb_filter_coeff: 0.0,
            }
        }
        pub fn set_time(&mut self, time: f32, sample_rate: f32) {
            let samples = (sample_rate * time) as usize;
            self.delay_samples = if samples >= self.buffer_l.len() { self.buffer_l.len() - 1 } else { samples };
        }
        pub fn set_feedback(&mut self, feedback: f32) {
            self.feedback = feedback.clamp(0.0, 0.95);
        }
        pub fn set_mix(&mut self, mix: f32) {
            self.mix = mix.clamp(0.0, 1.0);
        }
        pub fn set_filter(&mut self, amount: f32) {
            self.fb_filter_coeff = amount.clamp(0.0, 0.95);
        }
        fn process_core(&mut self, input_l: f32, input_r: f32) -> (f32, f32) {
            let buf_len = self.buffer_l.len();
            let read_pos = if self.write_pos >= self.delay_samples {
                self.write_pos - self.delay_samples
            } else {
                buf_len - (self.delay_samples - self.write_pos)
            };
            let delayed_l = self.buffer_l[read_pos % buf_len];
            let delayed_r = self.buffer_r[read_pos % buf_len];
            let fb_l = delayed_r * self.feedback;
            let fb_r = delayed_l * self.feedback;
            self.fb_filter_l = fb_l + self.fb_filter_coeff * (self.fb_filter_l - fb_l);
            self.fb_filter_r = fb_r + self.fb_filter_coeff * (self.fb_filter_r - fb_r);
            self.buffer_l[self.write_pos % buf_len] = input_l + self.fb_filter_l;
            self.buffer_r[self.write_pos % buf_len] = input_r + self.fb_filter_r;
            self.write_pos += 1;
            if self.write_pos >= buf_len {
                self.write_pos = 0;
            }
            (delayed_l, delayed_r)
        }
        pub fn process_stereo(&mut self, input_l: f32, input_r: f32) -> (f32, f32) {
            let (delayed_l, delayed_r) = self.process_core(input_l, input_r);
            let dry = 1.0 - self.mix;
            let wet = self.mix;
            (input_l * dry + delayed_l * wet, input_r * dry + delayed_r * wet)
        }
        pub fn process_stereo_wet(&mut self, input_l: f32, input_r: f32) -> (f32, f32) {
            self.process_core(input_l, input_r)
        }
        pub fn process(&mut self, input: f32) -> f32 {
            let (l, r) = self.process_stereo(input, input);
            (l + r) * 0.5
        }
    }
}

/// A plucky, deterministic test signal: decaying bursts of a few partials,
/// different left and right, with silences between.
fn plucks(i: usize) -> (f32, f32) {
    let sr = SAMPLE_RATE;
    let period = (sr * 0.37) as usize;
    let k = i % period;
    let n = (i / period) as f32;
    let t = k as f32 / sr;
    let env = (-t * 18.0).exp();
    let f = 220.0 * (1.0 + (n % 5.0) * 0.25);
    let tone = (t * f * core::f32::consts::TAU).sin() + 0.4 * (t * f * 3.01 * core::f32::consts::TAU).sin();
    (0.4 * env * tone, 0.3 * env * (t * f * 1.5 * core::f32::consts::TAU).sin())
}

fn seconds(s: f32) -> usize {
    (SAMPLE_RATE * s) as usize
}

#[test]
fn at_zero_wow_and_flutter_the_delay_plays_exactly_as_before() {
    // (max_time, time, feedback, filter, mix): short and long lines, a time
    // past the end of the line, no feedback and the most, dark and open.
    let cases = [
        (2.0, 0.3, 0.35, 0.0, 0.2),
        (2.0, 0.125, 0.95, 0.6, 1.0),
        (0.5, 0.75, 0.6, 0.95, 0.5),
        (0.36, 0.0, 0.5, 0.3, 1.0),
        (1.2, 0.2, 0.0, 0.0, 0.7),
    ];
    for (n, &(max_time, time, feedback, filter, mix)) in cases.iter().enumerate() {
        let mut old = before::Delay::new(SAMPLE_RATE, max_time);
        let mut new = Delay::new(SAMPLE_RATE, max_time);
        new.set_wow(0.0);
        new.set_flutter(0.0);
        old.set_time(time, SAMPLE_RATE);
        new.set_time(time, SAMPLE_RATE);
        old.set_feedback(feedback);
        new.set_feedback(feedback);
        old.set_filter(filter);
        new.set_filter(filter);
        old.set_mix(mix);
        new.set_mix(mix);
        for i in 0..seconds(3.0) {
            // Halfway through, the time moves, as a tempo change moves it.
            if i == seconds(1.5) {
                old.set_time(time * 0.5, SAMPLE_RATE);
                new.set_time(time * 0.5, SAMPLE_RATE);
            }
            let (l, r) = plucks(i);
            let (a, b) = match i % 3 {
                0 => (old.process_stereo(l, r), new.process_stereo(l, r)),
                1 => (old.process_stereo_wet(l, r), new.process_stereo_wet(l, r)),
                _ => ((old.process(l), 0.0), (new.process(l), 0.0)),
            };
            assert_eq!(
                (a.0.to_bits(), a.1.to_bits()),
                (b.0.to_bits(), b.1.to_bits()),
                "case {n}: differs from the old delay at sample {i}"
            );
        }
    }
}

#[test]
fn a_synced_delay_at_zero_wow_matches_the_old_one_through_tempo_changes() {
    let mut old = before::Delay::new(SAMPLE_RATE, 1.0);
    let mut new = Delay::new(SAMPLE_RATE, 1.0);
    new.set_sync(DelaySync::DottedEighth, 124.0, SAMPLE_RATE);
    old.set_time(60.0 / 124.0 * 0.75, SAMPLE_RATE);
    new.set_feedback(0.45);
    new.set_filter(0.5);
    new.set_mix(1.0);
    old.set_feedback(0.45);
    old.set_filter(0.5);
    old.set_mix(1.0);
    for i in 0..seconds(2.0) {
        if i == seconds(1.0) {
            new.set_bpm(90.0, SAMPLE_RATE);
            old.set_time(60.0 / 90.0 * 0.75, SAMPLE_RATE);
        }
        let (l, r) = plucks(i);
        let (a, b) = (old.process_stereo_wet(l, r), new.process_stereo_wet(l, r));
        assert_eq!((a.0.to_bits(), a.1.to_bits()), (b.0.to_bits(), b.1.to_bits()), "sample {i}");
    }
}

/// Pitch of a steady sine after one pass through the line, cycle by cycle:
/// (time of each upward zero crossing, frequency over the cycle before it).
fn pitch_track(x: &[f32], skip: usize) -> Vec<(f32, f32)> {
    let mut crossings = Vec::new();
    for i in skip.max(1)..x.len() {
        let (a, b) = (x[i - 1], x[i]);
        if a < 0.0 && b >= 0.0 {
            crossings.push((i - 1) as f32 + a / (a - b));
        }
    }
    crossings.windows(2).map(|w| (w[1] / SAMPLE_RATE, SAMPLE_RATE / (w[1] - w[0]))).collect()
}

/// The strongest rate, in Hz, at which `track`'s pitch moves, searched over
/// 0.1..20 Hz, and the pitch's swing in cents either way of its mean.
fn wobble(track: &[(f32, f32)]) -> (f32, f32) {
    let mean = track.iter().map(|p| p.1).sum::<f32>() / track.len() as f32;
    let mut best = (0.0, 0.0);
    for k in 1..=400 {
        let f = k as f32 * 0.05;
        let (mut re, mut im) = (0.0f32, 0.0f32);
        for &(t, hz) in track {
            let w = core::f32::consts::TAU * f * t;
            re += (hz - mean) * w.cos();
            im += (hz - mean) * w.sin();
        }
        let mag = re * re + im * im;
        if mag > best.1 {
            best = (f, mag);
        }
    }
    let swing = track.iter().map(|p| (1200.0 * (p.1 / mean).log2()).abs()).fold(0.0f32, f32::max);
    (best.0, swing)
}

/// A 440 Hz sine through a 250 ms delay with no feedback, wet only.
fn sine_through(wow: f32, flutter: f32, secs: f32) -> (Vec<f32>, Vec<f32>) {
    let mut d = Delay::new(SAMPLE_RATE, 1.0);
    d.set_time(0.25, SAMPLE_RATE);
    d.set_feedback(0.0);
    d.set_wow(wow);
    d.set_flutter(flutter);
    let (mut l, mut r) = (Vec::new(), Vec::new());
    for i in 0..seconds(secs) {
        let x = (i as f64 / SAMPLE_RATE as f64 * 440.0 * core::f64::consts::TAU).sin() as f32 * 0.5;
        let (a, b) = d.process_stereo_wet(x, x);
        l.push(a);
        r.push(b);
    }
    (l, r)
}

#[test]
fn without_wow_or_flutter_the_repeats_hold_their_pitch() {
    let (l, _) = sine_through(0.0, 0.0, 4.0);
    let (_, swing) = wobble(&pitch_track(&l, seconds(0.3)));
    assert!(swing < 0.5, "a still delay moved the pitch {swing} cents");
}

#[test]
fn wow_bends_the_repeats_slowly() {
    let (l, r) = sine_through(1.0, 0.0, 20.0);
    for (side, x) in [("left", &l), ("right", &r)] {
        let (rate, swing) = wobble(&pitch_track(x, seconds(0.3)));
        assert!((0.3..=1.5).contains(&rate), "{side}: wow moves the pitch at {rate} Hz");
        assert!((15.0..=40.0).contains(&swing), "{side}: wow=1 bends the repeats {swing} cents");
    }
    // Half the wow is about half the bend.
    let (half, _) = sine_through(0.5, 0.0, 20.0);
    let (_, full_swing) = wobble(&pitch_track(&l, seconds(0.3)));
    let (_, half_swing) = wobble(&pitch_track(&half, seconds(0.3)));
    let ratio = half_swing / full_swing;
    assert!((0.4..=0.6).contains(&ratio), "wow=0.5 bends {half_swing} cents against {full_swing}");
}

#[test]
fn flutter_bends_the_repeats_quickly() {
    let (l, r) = sine_through(0.0, 1.0, 10.0);
    for (side, x) in [("left", &l), ("right", &r)] {
        let (rate, swing) = wobble(&pitch_track(x, seconds(0.3)));
        assert!((5.0..=12.0).contains(&rate), "{side}: flutter moves the pitch at {rate} Hz");
        assert!((10.0..=30.0).contains(&swing), "{side}: flutter=1 bends the repeats {swing} cents");
    }
}

#[test]
fn left_and_right_drift_apart() {
    for (wow, flutter) in [(1.0, 0.0), (0.0, 1.0), (0.5, 0.5)] {
        let (l, r) = sine_through(wow, flutter, 6.0);
        let (tl, tr) = (pitch_track(&l, seconds(0.3)), pitch_track(&r, seconds(0.3)));
        let n = tl.len().min(tr.len());
        let apart = (0..n).map(|i| (1200.0 * (tl[i].1 / tr[i].1).log2()).abs()).fold(0.0f32, f32::max);
        assert!(apart > 2.0, "wow={wow} flutter={flutter}: left and right never part by more than {apart} cents");
    }
}

/// Plucks through a delay at full wow and flutter, the most feedback, open.
fn worst_case() -> (Vec<f32>, Vec<f32>) {
    let mut d = Delay::new(SAMPLE_RATE, 2.0);
    d.set_time(0.3, SAMPLE_RATE);
    d.set_feedback(0.95);
    d.set_filter(0.0);
    d.set_mix(0.5);
    d.set_wow(1.0);
    d.set_flutter(1.0);
    let (mut l, mut r) = (Vec::new(), Vec::new());
    for i in 0..seconds(30.0) {
        // Plucks for five seconds, then the repeats ring out on their own.
        let (a, b) = if i < seconds(5.0) { plucks(i) } else { (0.0, 0.0) };
        let (x, y) = d.process_stereo(a, b);
        l.push(x);
        r.push(y);
    }
    (l, r)
}

#[test]
fn full_wow_and_flutter_with_the_most_feedback_stay_finite_and_bounded() {
    let (l, r) = worst_case();
    assert!(l.iter().chain(&r).all(|s| s.is_finite()), "the delay produced a NaN or infinity");
    let peak = l.iter().chain(&r).fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(peak < 1.0, "the delay clipped: peak {peak}");
    // The repeats die away: the last second is well under the first.
    let tail = l[l.len() - seconds(1.0)..].iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(tail < peak * 0.5, "repeats grew instead of dying: tail {tail}, peak {peak}");

    // And the read moves smoothly: no step from one sample to the next is
    // larger than what the dry delay's own plucks make.
    let mut still = Delay::new(SAMPLE_RATE, 2.0);
    still.set_time(0.3, SAMPLE_RATE);
    still.set_feedback(0.95);
    still.set_mix(0.5);
    let mut still_l = Vec::new();
    for i in 0..seconds(30.0) {
        let (a, b) = if i < seconds(5.0) { plucks(i) } else { (0.0, 0.0) };
        still_l.push(still.process_stereo(a, b).0);
    }
    let step = |x: &[f32]| x.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
    let (moving, steady) = (step(&l), step(&still_l));
    assert!(moving < steady * 1.5, "wow and flutter click: largest step {moving}, against {steady} without");
}

#[test]
fn a_render_is_the_same_every_time_and_after_a_reset() {
    let (l1, r1) = worst_case();
    let (l2, r2) = worst_case();
    assert!(l1.iter().zip(&l2).all(|(a, b)| a.to_bits() == b.to_bits()));
    assert!(r1.iter().zip(&r2).all(|(a, b)| a.to_bits() == b.to_bits()));

    let mut d = Delay::new(SAMPLE_RATE, 1.0);
    d.set_wow(0.7);
    d.set_flutter(0.4);
    d.set_feedback(0.6);
    let run =
        |d: &mut Delay| (0..seconds(2.0)).map(|i| d.process_stereo_wet(plucks(i).0, plucks(i).1)).collect::<Vec<_>>();
    let first = run(&mut d);
    d.reset();
    let again = run(&mut d);
    assert!(first.iter().zip(&again).all(|(a, b)| a.0.to_bits() == b.0.to_bits() && a.1.to_bits() == b.1.to_bits()));
}

#[test]
fn a_delay_set_at_the_end_of_its_line_has_room_to_swing() {
    // The time at the very end of the line, and the wow swinging past it:
    // the read must stay inside the buffer and keep bending the pitch.
    let mut d = Delay::new(SAMPLE_RATE, 0.25);
    d.set_time(10.0, SAMPLE_RATE);
    d.set_feedback(0.0);
    d.set_wow(1.0);
    d.set_flutter(1.0);
    let mut out = Vec::new();
    for i in 0..seconds(6.0) {
        let x = (i as f32 / SAMPLE_RATE * 440.0 * core::f32::consts::TAU).sin() * 0.5;
        out.push(d.process_stereo_wet(x, x).0);
    }
    let (_, swing) = wobble(&pitch_track(&out, seconds(0.3)));
    assert!(swing > 5.0, "the wow stopped at the end of the line: {swing} cents");
    // And at a zero time it still reads something sane.
    let mut z = Delay::new(SAMPLE_RATE, 0.25);
    z.set_time(0.0, SAMPLE_RATE);
    z.set_wow(1.0);
    z.set_flutter(1.0);
    for i in 0..seconds(1.0) {
        let (a, b) = z.process_stereo_wet(plucks(i).0, plucks(i).1);
        assert!(a.is_finite() && b.is_finite() && a.abs() < 2.0 && b.abs() < 2.0);
    }
}

const SONG: &str = r#"
tempo 120
scale A minor
delay sync=dotted_eighth feedback=0.5 filter=0.3 WOWLINE

module fm pluck { algorithm two_op decay 0.2 sustain 0.0 }

pattern hi { 1.4 - - - - - - - 1.5 - - - - - - - }

track lead { play hi using pluck level 0.8 delay_send 0.5 out > CHAIN > master }

scene a { track lead { play hi using pluck } }
arrange { a x2 }
"#;

fn song(wow_line: &str, chain: &str) -> String {
    SONG.replace("WOWLINE", wow_line).replace("CHAIN", chain)
}

fn chain_delay(src: &str) -> NodeSpec {
    let compiled = compiler::compile(&dsl::parse(src).unwrap_or_else(|e| panic!("{e:?}"))).expect("compile");
    compiled.tracks[0]
        .insert_fx
        .iter()
        .map(|s| s.spec)
        .find(|s| matches!(s, NodeSpec::Delay { .. }))
        .expect("a delay in the chain")
}

#[test]
fn the_global_delay_line_takes_wow_and_flutter_as_shares_or_percent() {
    let ast = dsl::parse(&song("wow=30% flutter=0.15", "gain(1)")).unwrap();
    assert_eq!(ast.globals.send_delay.wow, Some(0.3));
    assert_eq!(ast.globals.send_delay.flutter, Some(0.15));
    let ast = dsl::parse(&song("", "gain(1)")).unwrap();
    assert_eq!((ast.globals.send_delay.wow, ast.globals.send_delay.flutter), (None, None));

    let errs = dsl::parse(&song("wow=1.5", "gain(1)")).unwrap_err();
    assert!(errs[0].message.contains("delay wow 1.5 is outside 0..1"), "{}", errs[0].message);
    let errs = dsl::parse(&song("flutter=150%", "gain(1)")).unwrap_err();
    assert!(errs[0].message.contains("delay flutter 1.5 is outside"), "{}", errs[0].message);
    let errs = dsl::parse(&song("wow=3db", "gain(1)")).unwrap_err();
    assert!(errs[0].message.contains("plain number"), "{}", errs[0].message);
}

#[test]
fn the_chain_delay_takes_wow_and_flutter_as_shares_or_percent() {
    let NodeSpec::Delay { wow, flutter, .. } = chain_delay(&song("", "delay(1/8, 0.4, wow=30%, flutter=0.2)")) else {
        unreachable!()
    };
    assert!((wow - 0.3).abs() < 1e-6 && (flutter - 0.2).abs() < 1e-6, "wow {wow}, flutter {flutter}");
    let NodeSpec::Delay { wow, flutter, .. } = chain_delay(&song("", "delay(1/8, 0.4)")) else { unreachable!() };
    assert_eq!((wow, flutter), (0.0, 0.0));

    let ast = dsl::parse(&song("", "delay(1/8, 0.4, wow=2)")).expect("parse");
    let errs = compiler::compile(&ast).err().expect("wow=2 was accepted");
    assert!(errs[0].to_string().contains("wow = 2 is out of range (0..1)"), "{}", errs[0]);

    let errs = dsl::parse(&song("", "delay(1/8, 0.4, wow=3hz)")).unwrap_err();
    assert!(errs[0].message.contains("'wow' takes %"), "{}", errs[0].message);
}

fn render(src: &str) -> Vec<f32> {
    let mut engine = SongEngine::from_source(src).unwrap_or_else(|e| panic!("{e:?}"));
    engine.start();
    let (l, r) = engine.render_steps(32);
    l.into_iter().chain(r).collect()
}

#[test]
fn wow_and_flutter_reach_the_send_delay_and_the_chain_delay() {
    let plain = render(&song("", "gain(1)"));
    assert_eq!(plain, render(&song("wow=0 flutter=0%", "gain(1)")), "writing 0 is not the same as leaving it out");
    assert_ne!(plain, render(&song("wow=50%", "gain(1)")), "the send delay ignores wow");
    assert_ne!(plain, render(&song("flutter=50%", "gain(1)")), "the send delay ignores flutter");

    let chained = render(&song("", "delay(1/8, 0.4)"));
    assert_ne!(chained, render(&song("", "delay(1/8, 0.4, wow=0.5)")), "the chain delay ignores wow");
    assert_ne!(chained, render(&song("", "delay(1/8, 0.4, flutter=0.5)")), "the chain delay ignores flutter");
}
