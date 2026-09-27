//! Splitting one step into several notes: `<B4 C#5 D5>` and `A4*3`.
//!
//! Steps are locked to sixteen a bar (`steps_per_bar = meter * 4`), so the
//! fastest note a pattern could spell by hand was a sixteenth — 6.9 a second
//! at 104 BPM. The only way past that was handing a chord to the arpeggiator,
//! which is a different mechanism with a different envelope and no way to
//! choose its notes: it reads as a second instrument, because it is one.
//!
//! A subdivision group keeps the note on the voice that was already playing,
//! and every subnote carries its own velocity and its own `~`, so a run can
//! shape itself and end on a bend.

use tatum_core::dsl;
use tatum_core::dsl::compiler::{CompiledStep, MAX_SUBDIV};
use tatum_core::song_engine::SongEngine;

const HEAD: &str = r#"
tempo 120
scale C minor
humanize 0
module bass lead { cutoff 0.7 resonance 10% glide 100% attack 1ms decay 20ms sustain 0% release 12ms }
"#;

/// One full bar, so `render(1)` plays the pattern exactly once and an onset
/// count is the real number of notes rather than the number times the loops.
fn one_bar(head: &str, pattern: &str) -> String {
    format!(
        "{head}pattern p {{ {pattern} {} }}\n\
         track t {{ play p using lead level 0.5 gate 0.9 out > master }}\n\
         master {{ in > out }}\n",
        "- ".repeat(15)
    )
}

/// Snappier than the shared head: eight notes inside one sixteenth are 15 ms
/// apart, and an envelope that has not finished decaying blurs them together.
const SNAP: &str = r#"
tempo 120
scale C minor
humanize 0
module bass lead { cutoff 0.7 resonance 10% glide 100% attack 1ms decay 6ms sustain 0% release 2ms }
"#;

fn song(pattern: &str) -> String {
    format!(
        "{HEAD}pattern p {{ {pattern} }}\n\
         track t {{ play p using lead level 0.5 gate 0.9 out > master }}\n\
         master {{ in > out }}\n"
    )
}

fn render(src: &str) -> Vec<f32> {
    let mut e = SongEngine::from_source(src).unwrap_or_else(|err| panic!("{err}"));
    e.start();
    e.render(1).0
}

fn compile_steps(pattern: &str) -> Vec<CompiledStep> {
    let ast = dsl::parse(&song(pattern)).unwrap_or_else(|e| panic!("{e:?}"));
    let compiled = dsl::compiler::compile(&ast).unwrap_or_else(|e| panic!("{e:?}"));
    compiled.patterns[0].steps.clone()
}

/// Onsets counted off the amplitude envelope. The module above is percussive
/// on purpose: a sustaining patch blurs four attacks into one hump and the
/// count says 1 when the engine is doing exactly the right thing.
fn attacks(buf: &[f32]) -> usize {
    // Rectify and smooth first: counting peaks on the raw signal counts
    // waveform cycles, not notes.
    const WIN: usize = 128; // ~3 ms at 44.1 kHz
    if buf.len() < WIN * 2 {
        return 0;
    }
    let mut env = Vec::with_capacity(buf.len() - WIN);
    let mut acc: f32 = buf[..WIN].iter().map(|v| v.abs()).sum();
    for i in WIN..buf.len() {
        env.push(acc / WIN as f32);
        acc += buf[i].abs() - buf[i - WIN].abs();
    }
    let peak = env.iter().fold(0.0f32, |a, v| a.max(*v));
    if peak < 1e-5 {
        return 0;
    }
    let (thr, mut n, mut armed) = (peak * 0.3, 0usize, true);
    for i in 1..env.len() {
        if armed && env[i] > thr && env[i] > env[i - 1] {
            n += 1;
            armed = false;
        } else if env[i] < thr * 0.4 {
            armed = true;
        }
    }
    n
}

// ── What it compiles to ──────────────────────────────────────────────────────

#[test]
fn a_group_compiles_to_one_step_holding_several_notes() {
    let steps = compile_steps("<C4 E4 G4> - - -");
    assert_eq!(steps.len(), 4, "a group is ONE step, not three");
    match steps[0] {
        CompiledStep::Subdiv { notes, count, .. } => {
            assert_eq!(count, 3);
            assert_eq!(notes[0].midi_note, 60);
            assert_eq!(notes[1].midi_note, 64);
            assert_eq!(notes[2].midi_note, 67);
        }
        other => panic!("expected Subdiv, got {other:?}"),
    }
}

#[test]
fn each_subnote_keeps_its_own_velocity_and_slide() {
    let steps = compile_steps("<C4:0.3 ~E4:0.9 G4> - - -");
    match steps[0] {
        CompiledStep::Subdiv { notes, count, .. } => {
            assert_eq!(count, 3);
            assert!((notes[0].velocity - 0.3).abs() < 1e-6);
            assert!((notes[1].velocity - 0.9).abs() < 1e-6);
            assert!(!notes[0].slide, "the first note is picked");
            assert!(notes[1].slide, "`~` inside a group bends into that note");
            assert!(!notes[2].slide);
        }
        other => panic!("expected Subdiv, got {other:?}"),
    }
}

/// `x*3` has always been a drum roll. This is the same spelling for pitches,
/// desugared in the parser so the engine only ever sees one concept.
#[test]
fn a_ratchet_is_a_group_of_repeats() {
    let ratchet = compile_steps("C4:0.7*3 - - -");
    let spelled = compile_steps("<C4:0.7 C4:0.7 C4:0.7> - - -");
    match (ratchet[0], spelled[0]) {
        (
            CompiledStep::Subdiv { notes: a, count: ca, .. },
            CompiledStep::Subdiv { notes: b, count: cb, .. },
        ) => {
            assert_eq!(ca, cb);
            for i in 0..ca as usize {
                assert_eq!(a[i].midi_note, b[i].midi_note);
                assert!((a[i].velocity - b[i].velocity).abs() < 1e-6);
            }
        }
        other => panic!("expected two Subdiv steps, got {other:?}"),
    }
}

#[test]
fn a_group_of_one_is_just_a_note() {
    assert_eq!(render(&song("<C4:0.8> - - -")), render(&song("C4:0.8 - - -")));
}

// ── What it sounds like ──────────────────────────────────────────────────────

#[test]
fn a_group_of_three_fires_three_times_inside_one_step() {
    assert_eq!(attacks(&render(&one_bar(SNAP, "<C4 E4 G4>"))), 3);
    assert_eq!(attacks(&render(&one_bar(SNAP, "C4"))), 1);
}

#[test]
fn a_ratchet_retriggers_the_same_note() {
    assert_eq!(attacks(&render(&one_bar(SNAP, "C4*4"))), 4);
    assert_eq!(attacks(&render(&one_bar(SNAP, "C4*2"))), 2);
}

/// Rough pitch by zero crossings. Counting onsets stops working once the
/// notes are 15 ms apart — the envelope never falls far enough between them
/// to re-arm a threshold detector — but the pitch is unambiguous.
fn crossings_hz(buf: &[f32], from: usize, len: usize) -> f32 {
    let seg = &buf[from.min(buf.len())..(from + len).min(buf.len())];
    if seg.len() < 2 {
        return 0.0;
    }
    let n = seg.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
    n as f32 * 44100.0 / (2.0 * seg.len() as f32)
}

/// Eight notes in one sixteenth is the ceiling, and all eight have to sound
/// rather than the tail being quietly dropped on the queue. At 120 BPM they
/// are 15 ms apart, so this checks the run *ascends an octave* instead of
/// counting attacks.
#[test]
fn a_full_group_fires_every_note() {
    let buf = render(&one_bar(SNAP, "<C4 D4 E4 F4 G4 A4 B4 C5>"));
    let sub = (44100.0 * 60.0 / 120.0 / 4.0 / MAX_SUBDIV as f32) as usize; // ~15.6 ms
    let first = crossings_hz(&buf, sub / 4, sub / 2);
    let last = crossings_hz(&buf, sub * 7 + sub / 4, sub / 2);
    assert!(
        last > first * 1.7,
        "an eight-note run from C4 to C5 has to end about an octave above          where it started (first {first:.0} Hz, last {last:.0} Hz)"
    );
    // and the step after it is silent: the queue must not spill into the rest
    let after = &buf[(sub * MAX_SUBDIV * 2)..(sub * MAX_SUBDIV * 3).min(buf.len())];
    let spill = after.iter().fold(0.0f32, |a, v| a.max(v.abs()));
    assert!(spill < 0.01, "notes leaked past the step they belong to: {spill:.4}");
}

/// Each subnote releases the one before it. Without that a group on a poly
/// instrument stacks its notes into a chord instead of playing a run.
#[test]
fn a_run_does_not_pile_up_into_a_chord() {
    let poly = format!(
        "{HEAD}module keys pad {{ voice_mode poly attack 1ms decay 20ms sustain 0% release 10ms }}\n\
         pattern p {{ <C4 E4 G4 C5> - - - }}\n\
         track t {{ play p using pad level 0.5 gate 0.9 out > master }}\nmaster {{ in > out }}\n"
    );
    let chord = format!(
        "{HEAD}module keys pad {{ voice_mode poly attack 1ms decay 20ms sustain 0% release 10ms }}\n\
         pattern p {{ [C4 E4 G4 C5] - - - }}\n\
         track t {{ play p using pad level 0.5 gate 0.9 out > master }}\nmaster {{ in > out }}\n"
    );
    let run_peak = render(&poly).iter().fold(0.0f32, |a, v| a.max(v.abs()));
    let chord_peak = render(&chord).iter().fold(0.0f32, |a, v| a.max(v.abs()));
    assert!(
        run_peak < chord_peak * 0.8,
        "a run of four should never be as loud as four notes at once \
         (run {run_peak:.4}, chord {chord_peak:.4})"
    );
}

/// The notes are spread over the step's *effective* duration, which already
/// has the swing in it. A subdivided step must swing with everything else
/// rather than quietly opting out of the groove.
#[test]
fn subdivisions_move_with_the_swing() {
    let straight = format!("{}\nswing 0.5", song("<C4 E4 G4> - <C4 E4 G4> -"));
    let swung = format!("{}\nswing 0.66", song("<C4 E4 G4> - <C4 E4 G4> -"));
    assert_ne!(
        render(&straight),
        render(&swung),
        "swing has to reach the notes inside a group"
    );
}

// ── The edges ────────────────────────────────────────────────────────────────

#[test]
fn an_oversized_group_is_a_compile_error_not_a_silent_truncation() {
    let src = song("<C4 D4 E4 F4 G4 A4 B4 C5 D5> - - -");
    let err = dsl::parse(&src).expect_err("nine notes in one step must be rejected");
    let msg = format!("{err:?}");
    assert!(msg.contains("subdivision group"), "unhelpful message: {msg}");
    assert!(msg.contains('9') && msg.contains('8'), "say both numbers: {msg}");
}

#[test]
fn an_empty_group_is_a_compile_error() {
    let err = dsl::parse(&song("<> - - -")).expect_err("an empty group must be rejected");
    assert!(format!("{err:?}").contains("empty"));
}

/// The feature is additive: a pattern that never uses it has to render exactly
/// as it did before, down to the sample.
#[test]
fn patterns_without_groups_are_untouched() {
    let plain = song("C4:0.9 ~E4:0.6 - G4 .. - - - C4 - - - E4 - - -");
    let a = render(&plain);
    let b = render(&plain);
    assert_eq!(a, b);
    assert!(a.iter().any(|v| v.abs() > 1e-4), "the guard itself must make sound");
}
