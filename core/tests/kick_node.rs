//! The `kick` node: a whole kick written in the units a kick is thought in.
//! Each test renders it in a song, through the compiler, and checks what is
//! heard against what the file says: the note it lands on, how long its sweep
//! takes, its level held and then released, and that every hit is the same.

use tatum_core::song_engine::SongEngine;

/// One `kick(...)` straight to `out`, on `pattern`, at 120 BPM.
fn song(args: &str, pattern: &str) -> String {
    format!(
        r#"
tempo 120
scale A minor
instrument boom {{
  kick({args}) as k
  k > out
}}
pattern four {{ {pattern} }}
track kick {{ play four using boom level 0.8 out > master }}
"#
    )
}

const FOUR: &str = "C3 - - -  C3 - - -  C3 - - -  C3 - - -";

fn render(src: &str, bars: u32) -> Vec<f32> {
    let mut e = SongEngine::from_source(src).unwrap_or_else(|err| panic!("{}", err));
    let (l, _) = e.render(bars);
    l
}

fn errors(src: &str) -> String {
    match SongEngine::from_source(src) {
        Ok(_) => String::new(),
        Err(e) => e.to_string(),
    }
}

fn onset(x: &[f32]) -> usize {
    onsets(x)[0]
}

/// Where each hit starts: the first sound after a silence. The engine starts
/// a note on the block it falls in, so hits are not a fixed number of samples
/// apart; each is measured from its own start.
fn onsets(x: &[f32]) -> Vec<usize> {
    let mut on = Vec::new();
    let mut quiet = usize::MAX;
    for (i, v) in x.iter().enumerate() {
        if *v != 0.0 {
            if quiet > 1000 {
                on.push(i);
            }
            quiet = 0;
        } else {
            quiet = quiet.saturating_add(1);
        }
    }
    assert!(!on.is_empty(), "it sounds");
    on
}

/// Each hit, from its start, `len` samples long.
fn hits(x: &[f32], len: usize) -> Vec<&[f32]> {
    onsets(x).into_iter().filter(|&o| o + len <= x.len()).map(|o| &x[o..o + len]).collect()
}

/// Frequency between two times after the onset, from the upward zero crossings.
fn pitch(x: &[f32], from_ms: f32, to_ms: f32) -> f32 {
    let t0 = onset(x);
    let (a, b) = (t0 + (from_ms * 44.1) as usize, t0 + (to_ms * 44.1) as usize);
    let ups: Vec<f32> = (a + 1..b)
        .filter(|&i| x[i - 1] < 0.0 && x[i] >= 0.0)
        .map(|i| (i - 1) as f32 + x[i - 1] / (x[i - 1] - x[i]))
        .collect();
    assert!(ups.len() >= 2, "no cycles between {from_ms} and {to_ms} ms");
    44100.0 * (ups.len() - 1) as f32 / (ups[ups.len() - 1] - ups[0])
}

/// The peak in a 10 ms window starting `at_ms` after the onset.
fn peak(x: &[f32], at_ms: f32) -> f32 {
    let i = onset(x) + (at_ms * 44.1) as usize;
    x[i..i + 441].iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

fn db(ratio: f32) -> f32 {
    20.0 * ratio.max(1e-9).log10()
}

#[test]
fn it_lands_on_the_note_it_is_given() {
    let x = render(&song("A1, hold=300ms", FOUR), 1);
    let tail = pitch(&x, 200.0, 290.0);
    assert!((tail - 55.0).abs() < 0.5, "A1 is 55 Hz, the tail is {tail:.2}");
}

#[test]
fn a_note_name_is_its_frequency() {
    let named = render(&song("A1", FOUR), 1);
    let hz = render(&song("55hz", FOUR), 1);
    let worst = named.iter().zip(&hz).fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
    assert!(worst < 1e-3, "kick(A1) and kick(55hz) differ by {worst}");
}

#[test]
fn the_body_takes_as_long_as_it_says_to_come_within_ten_percent() {
    // What `tatum analyze` calls the sweep: from the hit until the pitch is
    // within 10% of the tail. Short snap, so the body is what is timed.
    let x = render(&song("50, body=80ms, snap=2ms, hold=300ms", FOUR), 1);
    let before = pitch(&x, 30.0, 70.0);
    let after = pitch(&x, 100.0, 140.0);
    assert!(before > 55.0 + 2.0, "half way through the body it is still falling: {before:.1} Hz");
    assert!(after < 55.0, "past the body it is within 10% of 50 Hz: {after:.1} Hz");
}

#[test]
fn it_holds_then_falls_sixty_db_over_the_release() {
    let x = render(&song("50, hold=100ms, release=200ms", FOUR), 1);
    let top = peak(&x, 5.0);
    assert!(db(peak(&x, 80.0) / top).abs() < 0.5, "still at full level inside the hold");
    // Half way through the release is half way down in dB.
    let half = db(peak(&x, 200.0) / top);
    assert!((half + 30.0).abs() < 4.0, "-30 dB half way through the release, got {half:.1}");
    // And then it is over: nothing until the next hit.
    let on = onsets(&x);
    let rest = x[on[0] + (320.0 * 44.1) as usize..on[1]].iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(rest == 0.0, "the voice ended with the release, yet {rest} is left");
}

#[test]
fn every_hit_is_the_same_samples() {
    // Hypnotic is the same hit, again: no drift, no noise that changes, no
    // phase left over from the one before.
    let x = render(&song("A1, click=-30db, drive=0.4", FOUR), 2);
    let all = hits(&x, 20_000);
    assert_eq!(all.len(), 8);
    for (n, other) in all.iter().enumerate().skip(1) {
        assert!(all[0] == *other, "hit {} differs from the first", n + 1);
    }
}

#[test]
fn the_length_of_the_note_does_not_cut_it() {
    // A drum step is a sixteenth long; the kick plays whole whatever it is.
    // Held three steps here (a rest before the next, which a tie into the
    // same note would not retrigger).
    let short = render(&song("50, hold=150ms, release=300ms", FOUR), 1);
    let tied = render(&song("50, hold=150ms, release=300ms", "C3 .. .. -  C3 .. .. -  C3 .. .. -  C3 .. .. -"), 1);
    let (a, b) = (hits(&short, 20_000), hits(&tied, 20_000));
    assert_eq!(a.len(), b.len(), "{:?} against {:?}", onsets(&short), onsets(&tied));
    for (n, (x, y)) in a.iter().zip(&b).enumerate() {
        assert!(x == y, "hit {} plays differently held", n + 1);
    }
}

#[test]
fn phase_ninety_starts_at_the_top() {
    let x = render(&song("50, phase=90", FOUR), 1);
    let first = x[onset(&x)].abs();
    assert!(first > 0.5 * peak(&x, 0.0), "at 90 degrees the first sample is near the peak: {first}");
    let zero = render(&song("50, phase=0", FOUR), 1);
    let i = onset(&zero);
    // The first sample of a hit at 0 is exactly 0, so the first one heard is
    // the next: at 800 Hz a sine from zero is a ninth of the way up by then.
    assert!(
        zero[i].abs() < 0.15 * peak(&zero, 0.0),
        "at 0 it rises from silence: {} against {}",
        zero[i],
        peak(&zero, 0.0)
    );
}

#[test]
fn drive_keeps_the_peak_and_fills_the_body() {
    let clean = render(&song("50, drive=0", FOUR), 1);
    let driven = render(&song("50, drive=1", FOUR), 1);
    let (pc, pd) = (peak(&clean, 0.0), peak(&driven, 0.0));
    assert!((db(pd / pc)).abs() < 0.5, "the peak stays: {pc} against {pd}");
    // A squarer wave has more energy at the same peak.
    let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
    let t0 = onset(&clean);
    let w = t0..t0 + 2205;
    assert!(rms(&driven[w.clone()]) > 1.1 * rms(&clean[w]), "drive fills the body");
}

#[test]
fn the_click_is_there_only_when_asked_for() {
    let none = render(&song("50", FOUR), 1);
    let some = render(&song("50, click=-20db", FOUR), 1);
    assert_ne!(none, some);
    // It is short: 20 ms in, the two are the same kick.
    let t0 = onset(&none);
    let at = t0 + (20.0 * 44.1) as usize;
    let worst = none[at..at + 441].iter().zip(&some[at..at + 441]).fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
    assert!(worst < 1e-3, "the click is over by 20 ms: {worst}");
}

#[test]
fn a_knee_out_of_place_says_where_it_goes() {
    let e = errors(&song("50, knee=52", FOUR));
    assert!(e.contains("within 10% of the note"), "{e}");
    let e = errors(&song("50, start=300, knee=400", FOUR));
    assert!(e.contains("over start=300"), "{e}");
    let e = errors(&song("50, start=52", FOUR));
    assert!(e.contains("start=52"), "a start under the note is about the start, not the knee: {e}");
    // Equal to the start is one stage, and fine.
    assert_eq!(errors(&song("50, start=300, knee=300", FOUR)), "");
}

#[test]
fn a_note_where_a_frequency_is_not_taken_is_an_error() {
    let e = errors(&song("50, hold=A1", FOUR));
    assert!(e.contains("is a note"), "{e}");
}

#[test]
fn a_node_named_kick_is_still_a_name() {
    let src = r#"
tempo 120
instrument boom {
  pitch_osc sine(300, 50, 0.999) as kick
  kick > perc(0.001, 0.3) as amp
  amp > out
}
pattern four { C3 - - -  C3 - - -  C3 - - -  C3 - - - }
track t { play four using boom }
"#;
    assert_eq!(errors(src), "");
}

#[test]
fn a_bare_kick_says_how_to_write_the_node() {
    let src = r#"
tempo 120
instrument boom {
  kick > out
}
pattern four { C3 - - -  C3 - - -  C3 - - -  C3 - - - }
track t { play four using boom }
"#;
    let e = errors(src);
    assert!(e.contains("`kick()`"), "{e}");
}

#[test]
fn a_chirp_is_a_faster_stage_over_the_start() {
    // Over the first millisecond the chirp is far above where the snap alone
    // starts; a few chirp times in, the two are the same sweep.
    let plain = render(&song("50, start=300, hold=200ms", FOUR), 1);
    let chirped = render(&song("50, start=300, chirp=3khz, chirp_time=0.5ms, hold=200ms", FOUR), 1);
    // Too few cycles there to time, so count the zero crossings instead.
    let crossings = |x: &[f32]| {
        let t0 = onset(x);
        x[t0..t0 + (3.0 * 44.1) as usize].windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count()
    };
    let (p, c) = (crossings(&plain), crossings(&chirped));
    assert!(c >= 2 * p, "the chirp is the tick on top: {c} crossings in 3 ms against {p}");
    let (p, c) = (pitch(&plain, 60.0, 150.0), pitch(&chirped, 60.0, 150.0));
    assert!((c - p).abs() < 0.5, "the body is untouched: {c:.2} against {p:.2}");
}

#[test]
fn a_chirp_under_the_start_says_where_it_goes() {
    let e = errors(&song("50, start=800, chirp=400", FOUR));
    assert!(e.contains("chirp=400 is under start=800"), "{e}");
}

#[test]
fn sag_leans_the_held_top_by_what_it_says() {
    let flat = render(&song("50, hold=200ms, release=100ms", FOUR), 1);
    let sagged = render(&song("50, hold=200ms, sag=-6db, release=100ms", FOUR), 1);
    let top = peak(&sagged, 2.0);
    assert!((db(peak(&flat, 180.0) / peak(&flat, 2.0))).abs() < 0.5, "unsagged, the top is flat");
    // Evenly in dB: half way, half the sag; at the end, all of it.
    let half = db(peak(&sagged, 100.0) / top);
    let end = db(peak(&sagged, 190.0) / top);
    assert!((half + 3.0).abs() < 0.7, "half way down the hold: {half:.2} dB");
    assert!((end + 5.7).abs() < 0.8, "at the end of the hold: {end:.2} dB");
}

fn heard_of(args: &str) -> (f32, f32) {
    let src = format!(
        "instrument x {{\n  kick({args}) as k\n  k > out\n}}\npattern p {{ C3 - - - }}\ntrack t {{ play p using x }}\n"
    );
    let song = tatum_core::dsl::parse(&src).expect("parses");
    let node = &song.instruments[0].nodes[0];
    let spec = tatum_core::dsl::compiler::kick_spec(&node.params).expect("a kick");
    let tatum_core::graph::node::NodeSpec::Kick { note, .. } = spec else { panic!("a kick") };
    (tatum_core::graph::node::kick_heard_hz(&spec), note)
}

fn st(a: f32, b: f32) -> f32 {
    12.0 * (a / b).log2()
}

#[test]
fn a_kick_that_lands_at_once_is_heard_on_its_note() {
    let (heard, note) = heard_of("A1, start=300hz, snap=1ms, body=8ms, hold=200ms, release=150ms");
    assert!(st(heard, note).abs() < 0.3, "heard {heard} on {note}");
}

#[test]
fn a_slow_sweep_is_heard_over_the_note_it_lands_on() {
    let (fast, _) = heard_of("A1, start=900hz, snap=3ms, body=20ms, hold=100ms, release=200ms");
    let (slow, note) = heard_of("A1, start=900hz, snap=3ms, body=90ms, hold=100ms, release=200ms");
    assert!(st(slow, note) > 1.0, "a slow body is heard over its note: {slow} on {note}");
    assert!(slow > fast, "slower is heard higher: {slow} against {fast}");
}

#[test]
fn heard_one_tunes_the_note_the_ear_takes() {
    let a1 = 55.0;
    let args = "A1, start=900hz, snap=3ms, body=90ms, hold=100ms, release=200ms, heard=1";
    let (heard, settles) = heard_of(args);
    assert!(st(heard, a1).abs() < 0.1, "heard {heard}, wanted A1");
    assert!(settles < a1 * 0.95, "it settles under what it is heard as: {settles}");
    // And the sound follows: the rendered tail is the note it settles on.
    let x = render(&song(args, FOUR), 1);
    let tail = pitch(&x, 180.0, 260.0);
    assert!(st(tail, settles).abs() < 0.3, "tail {tail} on {settles}");
}

#[test]
fn heard_zero_is_the_kick_as_written() {
    let (_, plain) = heard_of("A1, body=90ms");
    let (_, zero) = heard_of("A1, body=90ms, heard=0");
    assert_eq!(plain, zero);
    assert_eq!(plain, 55.0);
}
