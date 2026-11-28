//! A note struck again while its last strike is still releasing.
//!
//! On an instrument graph the note-off was looked up by note number and took
//! the first voice on that note, which was the old one, already releasing. The
//! new voice never got it and rang through every rest after it, until it was
//! stolen. A note-off goes to the latest strike still held; the old voice keeps
//! releasing, as a key struck again does.

use tatum_core::graph::node::NodeSpec;
use tatum_core::graph::voice::Instrument;
use tatum_core::graph::GraphBuilder;
use tatum_core::primitives::oscillator::Waveform;
use tatum_core::song_engine::SongEngine;
use tatum_core::{BLOCK_SIZE, MAX_VOICES, SAMPLE_RATE};

const TEMPO: f32 = 120.0;

fn step() -> usize {
    (SAMPLE_RATE * 60.0 / TEMPO / 4.0) as usize
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| (v * v) as f64).sum::<f64>() / x.len().max(1) as f64).sqrt() as f32
}

fn db(a: f32, b: f32) -> f32 {
    20.0 * (a.max(1e-12) / b.max(1e-12)).log10()
}

/// `bars` of one track playing `pattern` on `voice` (a whole `instrument` or
/// `module` declaration named `v`).
fn song(voice: &str, pattern: &str, bars: u32) -> Vec<f32> {
    let src = format!(
        "tempo {TEMPO}\nscale C major\n{voice}\n\
         pattern p {{ {pattern} }}\n\
         track t {{ play p using v out > master }}\n\
         master {{ in > out }}\n\
         scene a {{ track t {{ play p using v }} }}\narrange {{ a x{bars} }}\n"
    );
    let mut engine = SongEngine::from_source(&src).unwrap_or_else(|e| panic!("{}\n{}", e, src));
    engine.render(bars).0
}

/// A saw through an envelope with a release of a second.
const GRAPH: &str = "instrument v { osc saw > adsr(0.005, 0.1, 0.8, 1.0) as amp\n amp > out }";

/// Steps `from..to` of bar `bar` (16 steps a bar).
fn steps(x: &[f32], bar: usize, from: usize, to: usize) -> &[f32] {
    let s = step();
    &x[(bar * 16 + from) * s..((bar * 16 + to) * s).min(x.len())]
}

/// The level of the last two steps of each bar against the notes at its top.
fn rest_under_notes(x: &[f32], bars: usize) -> f32 {
    (0..bars).map(|b| db(rms(steps(x, b, 14, 16)), rms(steps(x, 0, 0, 4)))).fold(f32::MIN, f32::max)
}

#[test]
fn a_restruck_note_is_silent_through_the_rests() {
    // Four strikes an eighth of a second apart, each inside the last one's
    // release of a second, then a second and a half of rests: twice over, so
    // nothing rings into the next loop either.
    let x = song(GRAPH, "C3 C3 C3 C3 -*12", 3);
    let under = rest_under_notes(&x, 3);
    assert!(under < -50.0, "the rests are {under:.1} dB under the notes: a strike was never released");
}

#[test]
fn restruck_chords_are_silent_through_the_rests() {
    let x = song(GRAPH, "[C3 E3 G3] [C3 E3 G3] [C3 E3 G3] -*13", 3);
    let under = rest_under_notes(&x, 3);
    assert!(under < -50.0, "the rests are {under:.1} dB under the chords");
    // Two chords that share a note, struck in turn.
    let x = song(GRAPH, "[C3 E3 G3] [C3 F3 A3] [C3 E3 G3] [C3 F3 A3] -*12", 3);
    let under = rest_under_notes(&x, 3);
    assert!(under < -50.0, "the rests are {under:.1} dB under the chords that share a note");
}

#[test]
fn a_tie_across_a_restrike_still_holds_and_then_ends() {
    // Struck, struck again and tied for four steps: the tied note is at its
    // sustain through the ties, and gone by the end of the bar.
    let x = song(GRAPH, "C3 C3 .. .. .. .. -*10", 3);
    let held = db(rms(steps(&x, 0, 4, 6)), rms(steps(&x, 0, 0, 2)));
    assert!(held > -6.0, "the tied note is {held:.1} dB under the strikes: it was released early");
    let under = rest_under_notes(&x, 3);
    assert!(under < -50.0, "the rests are {under:.1} dB under the notes");
}

#[test]
fn a_slide_across_a_restrike_still_plays_and_then_ends() {
    // A graph has no glide, so `~` retriggers: the strikes land on one note,
    // from another and back, each inside the last one's release.
    let x = song(GRAPH, "C3 ~C3 C3 ~E3 ~C3 C3 -*10", 3);
    let last = db(rms(steps(&x, 0, 5, 6)), rms(steps(&x, 0, 0, 1)));
    assert!(last > -6.0, "the last strike is {last:.1} dB under the first");
    let under = rest_under_notes(&x, 3);
    assert!(under < -50.0, "the rests are {under:.1} dB under the notes");
}

#[test]
fn the_modules_end_a_restruck_note_too() {
    // `keys` and `fm` release every voice on the note that is not already
    // releasing, `bass` has one voice: none of them shared the flaw.
    for voice in [
        "module keys v { voice_mode poly attack 5ms release 1s }",
        "module fm v { op0_envelope 0.005 0.1 0.8 1.0 op1_envelope 0.005 0.1 0.8 1.0 }",
        "module bass v { release 1s }",
    ] {
        let x = song(voice, "C3 C3 C3 C3 -*12", 3);
        // These tails are not straight lines to -60 dB in a second, so the
        // mark is lower: a voice left held sits at its sustain, near 0 dB.
        let under = rest_under_notes(&x, 3);
        assert!(under < -30.0, "{voice}: the rests are {under:.1} dB under the notes");
    }
}

/// A sine through an envelope with a release of `r` seconds.
fn instrument(r: f32) -> Instrument {
    let mut b = GraphBuilder::new();
    let osc = b.add_node(NodeSpec::Osc {
        waveform: Waveform::Sine,
        freq: 220.0,
        drift_seed: 1,
        fixed: false,
        pitch_semitones: 0.0,
        pitch_env: 0.0,
        pitch_decay: 0.005,
    });
    let env = b.add_node(NodeSpec::Env { a: 0.002, d: 0.05, s: 0.8, r });
    let vca = b.add_node(NodeSpec::Vca);
    let out = b.add_node(NodeSpec::Output);
    b.connect(osc, vca);
    b.connect(env, vca);
    b.connect(vca, out);
    Instrument::new(b.build())
}

/// Run `seconds` and return the peak of the last block.
fn run(inst: &mut Instrument, seconds: f32) -> f32 {
    let mut buf = [0.0f32; BLOCK_SIZE];
    for _ in 0..((seconds * SAMPLE_RATE) as usize / BLOCK_SIZE).max(1) {
        inst.process_block(&mut buf);
    }
    buf.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

#[test]
fn a_note_off_takes_the_latest_strike_still_held() {
    let mut inst = instrument(0.2);
    // Held twice, as two keys on one note: each note-off ends one of them.
    inst.note_on(60, 1.0);
    run(&mut inst, 0.1);
    inst.note_on(60, 1.0);
    run(&mut inst, 0.1);
    inst.note_off(60);
    assert!(run(&mut inst, 0.6) > 0.1, "one strike is still held");
    assert!(!inst.is_idle());
    inst.note_off(60);
    run(&mut inst, 0.6);
    assert!(inst.is_idle(), "both strikes were released");

    // Struck, released, struck inside the release: the note-off is the new
    // strike's, whichever voice the old one sits in.
    inst.note_on(60, 1.0);
    run(&mut inst, 0.05);
    inst.note_off(60);
    run(&mut inst, 0.02);
    inst.note_on(60, 1.0);
    run(&mut inst, 0.05);
    inst.note_off(60);
    run(&mut inst, 0.6);
    assert!(inst.is_idle(), "the second strike got its note-off");

    // A note-off with nothing held does nothing: a voice releasing is left to.
    inst.note_on(60, 1.0);
    run(&mut inst, 0.05);
    inst.note_off(60);
    let before = run(&mut inst, 0.05);
    inst.note_off(60);
    let after = run(&mut inst, 0.02);
    assert!(after < before && after > 0.0, "the release went on: {before} then {after}");
    run(&mut inst, 0.6);
    assert!(inst.is_idle());
}

#[test]
fn stealing_at_the_voice_limit_leaves_nothing_held() {
    // More strikes of one note than there are voices, each inside the last
    // one's release: the oldest voice is taken, and every strike still ends.
    let mut inst = instrument(2.0);
    for _ in 0..MAX_VOICES * 2 + 3 {
        inst.note_on(48, 1.0);
        run(&mut inst, 0.05);
        inst.note_off(48);
        run(&mut inst, 0.05);
    }
    run(&mut inst, 2.5);
    assert!(inst.is_idle(), "a strike of the run was never released");

    // More notes held at once than there are voices: the first ones are
    // stolen, their note-offs find nothing and release nobody else's voice.
    let mut inst = instrument(0.1);
    let n = MAX_VOICES as u8 + 2;
    for k in 0..n {
        inst.note_on(40 + k, 1.0);
        run(&mut inst, 0.01);
    }
    for k in 0..2 {
        inst.note_off(40 + k);
    }
    assert!(run(&mut inst, 0.5) > 0.1, "the notes still held went on sounding");
    for k in 2..n {
        inst.note_off(40 + k);
    }
    run(&mut inst, 0.5);
    assert!(inst.is_idle(), "a held note was left after the note-offs");
}
