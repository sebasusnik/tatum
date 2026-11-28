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
