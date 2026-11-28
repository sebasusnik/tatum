//! `glide=` on an oscillator: a graph instrument slides into a note written
//! `~note`, as a `bass` module does, instead of starting it again.

use tatum_core::song_engine::SongEngine;
use tatum_core::SAMPLE_RATE;

/// At 120 BPM a step is 125 ms: C3 for four steps, then a slide up a fifth.
fn song(osc: &str) -> String {
    format!(
        "tempo 120\n\
         instrument i {{\n  {osc} as src\n  src > adsr(0.3, 0.1, 1.0, 0.05) as amp\n  amp > out\n}}\n\
         pattern p {{ C3 .. .. .. ~G3 .. .. .. .. .. .. .. .. .. .. .. }}\n\
         track t {{ play p using i out > master }}\n"
    )
}

fn render(src: &str) -> Vec<f32> {
    SongEngine::from_source(src).unwrap_or_else(|e| panic!("{e}")).render(1).0
}

/// Hz around `seconds`, from the upward zero crossings of 30 ms.
fn pitch_at(buf: &[f32], seconds: f32) -> f32 {
    let (a, b) = (((seconds - 0.015) * SAMPLE_RATE) as usize, ((seconds + 0.015) * SAMPLE_RATE) as usize);
    let ups: Vec<f32> = (a + 1..b)
        .filter(|&i| buf[i - 1] < 0.0 && buf[i] >= 0.0)
        .map(|i| (i - 1) as f32 + buf[i - 1] / (buf[i - 1] - buf[i]))
        .collect();
    assert!(ups.len() >= 2, "no cycles at {seconds} s");
    (ups.len() - 1) as f32 * SAMPLE_RATE / (ups[ups.len() - 1] - ups[0])
}

fn level_at(buf: &[f32], seconds: f32) -> f32 {
    let (a, b) = (((seconds - 0.01) * SAMPLE_RATE) as usize, ((seconds + 0.01) * SAMPLE_RATE) as usize);
    buf[a..b].iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

const C3: f32 = 130.81;
const G3: f32 = 196.0;

/// The slide starts on the step (0.5 s), takes 200 ms, and half way through
/// it is half way in pitch: the note between the two, not the Hz between.
#[test]
fn a_slide_takes_its_glide_in_a_straight_line_in_pitch() {
    let out = render(&song("osc sine(55, glide=200ms)"));
    let cents = |hz: f32, of: f32| 1200.0 * (hz / of).log2();
    assert!(cents(pitch_at(&out, 0.45), C3).abs() < 15.0, "before: {} Hz", pitch_at(&out, 0.45));
    let half = pitch_at(&out, 0.6);
    assert!(cents(half, (C3 * G3).sqrt()).abs() < 25.0, "half way: {half} Hz");
    let quarter = pitch_at(&out, 0.55);
    assert!(cents(quarter, C3 * (G3 / C3).powf(0.25)).abs() < 25.0, "a quarter in: {quarter} Hz");
    assert!(cents(pitch_at(&out, 0.72), G3).abs() < 15.0, "after: {} Hz", pitch_at(&out, 0.72));
    assert!(cents(pitch_at(&out, 1.5), G3).abs() < 15.0, "it stays there");
}

/// Nothing starts again: the envelope, 300 ms of attack, is at the top
/// across the slide. Without `glide=` the note starts anew and is quiet there.
#[test]
fn a_slide_does_not_start_the_note_again() {
    let slid = render(&song("osc sine(55, glide=200ms)"));
    assert!(level_at(&slid, 0.53) > 0.97 * level_at(&slid, 0.45), "the level dipped at the slide");
    let struck = render(&song("osc sine(55)"));
    assert!(level_at(&struck, 0.53) < 0.75 * level_at(&struck, 0.45), "with no glide the note starts again");
    assert!(level_at(&slid, 0.58) > 0.9 * level_at(&slid, 0.45));
    assert!((pitch_at(&struck, 0.6) / G3 - 1.0).abs() < 0.01, "and is on its pitch at once");
}

/// A note that is not written `~` starts anew whatever the glide, and the
/// slide ends with the note it slid to.
#[test]
fn only_a_tilde_slides_and_the_note_still_ends() {
    let src = song("osc sine(55, glide=200ms)")
        .replace("~G3", "G3")
        .replace(".. .. .. .. .. .. .. .. .. .. ..", ".. .. .. - - - - - - - -");
    let out = render(&src);
    assert!((pitch_at(&out, 0.6) / G3 - 1.0).abs() < 0.01);
    let src = song("osc sine(55, glide=200ms)").replace(".. .. .. .. .. .. .. .. .. .. ..", ".. .. .. - - - - - - - -");
    let out = render(&src);
    assert!(level_at(&out, 0.8) > 0.9 * level_at(&out, 0.45));
    assert!(level_at(&out, 1.5) < 1e-3 * level_at(&out, 0.45), "the note it slid to never ended");
}

const D3: f32 = 146.83;

/// A chord, then a slide. `rest` is what follows the slid note.
fn chord_then_slide(rest: &str) -> String {
    format!(
        "tempo 120\n\
         instrument i {{\n  osc sine(55, glide=100ms) as src\n  src > adsr(0.01, 0.1, 0.8, 0.05) as amp\n  amp > out\n}}\n\
         pattern p {{ [C3 E3 G3] ~D3 {rest} }}\n\
         track t {{ play p using i out > master }}\n"
    )
}

fn peak_of_step(buf: &[f32], step: usize) -> f32 {
    let len = (SAMPLE_RATE * 60.0 / 120.0 / 4.0) as usize;
    buf[step * len..(step + 1) * len].iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

/// A slide is one line. After a chord the track used to slide one voice and
/// forget the other two, which nobody then released: they rang through the
/// rests and a new pair joined them on every loop.
#[test]
fn a_slide_from_a_chord_leaves_nothing_ringing() {
    let src = chord_then_slide("- - - - - - - - - - - - - -");
    let out = SongEngine::from_source(&src).unwrap_or_else(|e| panic!("{e}")).render(3).0;
    let loud = peak_of_step(&out, 0);
    assert!(loud > 0.01, "the chord sounds: {loud}");
    for bar in 0..3 {
        for step in 3..16 {
            let p = peak_of_step(&out, bar * 16 + step);
            assert!(p < 1e-3 * loud, "bar {bar} step {step} rings at {p} with the chord at {loud}");
        }
    }
    // And every loop is the first again: nothing piles up.
    let again = peak_of_step(&out, 32);
    assert!((again / loud - 1.0).abs() < 0.01, "the third chord peaks at {again}, the first at {loud}");
}

/// The note that slides is the last one written in the chord, and it is the
/// only one left: one clean pitch arriving on the note slid to, at the
/// level of one voice.
#[test]
fn the_last_note_of_a_chord_is_the_one_that_slides() {
    let src = chord_then_slide(".. .. .. .. .. .. .. - - - - - - -");
    let out = render(&src);
    // The slide starts at 0.125 s and takes 100 ms. Half way it is half
    // way from G3, the chord's last note, and not from C3 or E3.
    let cents = |hz: f32, of: f32| 1200.0 * (hz / of).log2();
    let half = pitch_at(&out, 0.175);
    assert!(cents(half, (G3 * D3).sqrt()).abs() < 40.0, "half way: {half} Hz");
    let landed = pitch_at(&out, 0.5);
    assert!(cents(landed, D3).abs() < 10.0, "it lands on D3: {landed} Hz");
    // One voice is left of three. A single note through the same patch,
    // held as long, is as loud.
    let single = render(&chord_then_slide(".. .. .. .. .. .. .. - - - - - - -").replace("[C3 E3 G3] ~D3", "D3 .."));
    assert!(
        (level_at(&out, 0.5) / level_at(&single, 0.5) - 1.0).abs() < 0.02,
        "after the slide {} sounds, one note alone {}",
        level_at(&out, 0.5),
        level_at(&single, 0.5)
    );
    assert!(level_at(&out, 1.6) < 1e-3 * level_at(&out, 0.5), "and it ends with its note");
}

/// One note, a slide, then rests: the slid note ends and stays ended on
/// every loop.
#[test]
fn a_slide_from_one_note_ends_with_its_note() {
    let src = chord_then_slide("- - - - - - - - - - - - - -").replace("[C3 E3 G3]", "G3");
    let out = SongEngine::from_source(&src).unwrap_or_else(|e| panic!("{e}")).render(2).0;
    let loud = peak_of_step(&out, 0);
    // It slid: the envelope carried on, so the step of the slide is at the
    // sustain and does not dip to an attack.
    assert!(peak_of_step(&out, 1) > 0.7 * loud, "the slid note sounds");
    for step in (3..16).chain(19..32) {
        let p = peak_of_step(&out, step);
        assert!(p < 1e-3 * loud, "step {step} rings at {p}");
    }
}

/// A slide inside a run takes the note before it in the run, and the run
/// ends with its last note.
#[test]
fn a_slide_inside_a_run_leaves_nothing_ringing() {
    let src = chord_then_slide("- - - - - - - - - - - - - -").replace("[C3 E3 G3] ~D3", "<C3 ~G3> -");
    let out = SongEngine::from_source(&src).unwrap_or_else(|e| panic!("{e}")).render(2).0;
    let loud = peak_of_step(&out, 0);
    assert!(loud > 0.01);
    for step in (3..16).chain(19..32) {
        let p = peak_of_step(&out, step);
        assert!(p < 1e-3 * loud, "step {step} rings at {p}");
    }
}
