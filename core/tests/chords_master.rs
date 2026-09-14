//! Chord symbols in patterns and `auto master <param>` automation.

use synth_core::dsl::{self, compiler};
use synth_core::dsl::ast::{Step, NoteRef};
use synth_core::song_engine::SongEngine;

const SONG: &str = r#"
tempo 120
scale F minor
module keys pad { cutoff 0.4 lfo_target cutoff lfo_depth 0.1 }
module fm tone { algorithm two_op decay 1.0 sustain 1.0 }
pattern chords { Fm9 ..*7 Dbmaj7/2 ..*7 }
pattern more { C7:0.6 - Ab9 - F#m7b5/4 - Gsus4(gate=0.5) - }
pattern hold { 1.3 ..*15 }
track pad  { play chords using pad reverb_send 0.3 out > master }
track tone { play hold using tone out > master }
master { in > tilt(0.0) > eq(low=0, mid=0, high=0) > gain(1.0) > limiter > out }
scene a {
    auto master tilt 0.4 > -0.4
    auto master eq_high -6 > 3
    track pad { play chords using pad }
    track tone { play hold using tone }
}
scene b {
    auto master gain 1.0 > 0.0
    track tone { play hold using tone }
}
arrange { a x1 b x1 }
"#;

fn chord_midi(step: &Step) -> Vec<u8> {
    match step {
        Step::Chord(c) => c.notes.iter().map(|n| match n.note { NoteRef::Midi(m) => m, _ => 0 }).collect(),
        other => panic!("expected chord, got {:?}", other),
    }
}

#[test]
fn chord_symbols_expand_to_chords() {
    let ast = dsl::parse(SONG).expect("parse");
    let chords = ast.patterns.iter().find(|p| p.name == "chords").unwrap();
    assert_eq!(chords.rows[0].len(), 16);
    assert_eq!(chord_midi(&chords.rows[0][0]), vec![53, 56, 60, 63, 67], "Fm9 at octave 3");
    assert_eq!(chord_midi(&chords.rows[0][8]), vec![37, 41, 44, 48], "Dbmaj7 at octave 2");

    let more = ast.patterns.iter().find(|p| p.name == "more").unwrap();
    assert_eq!(chord_midi(&more.rows[0][0]), vec![48, 52, 55, 58], "C7 read as a chord, not C octave 7");
    if let Step::Chord(c) = &more.rows[0][0] { assert_eq!(c.velocity, Some(0.6)); }
    assert_eq!(chord_midi(&more.rows[0][2]), vec![56, 60, 63, 66, 70], "Ab9");
    assert_eq!(chord_midi(&more.rows[0][4]), vec![66, 69, 72, 76], "F#m7b5 at octave 4");
    if let Step::Chord(c) = &more.rows[0][6] { assert_eq!(c.plock.gate, Some(0.5)); }
    assert!(compiler::compile(&ast).is_ok());
}

#[test]
fn unknown_words_in_patterns_are_errors() {
    let bad = SONG.replace("Fm9 ..*7", "Fmx9 ..*7");
    let errs = dsl::parse(&bad).unwrap_err();
    assert!(errs[0].message.contains("unexpected 'Fmx9'"), "{}", errs[0].message);
}

#[test]
fn master_automation_is_validated() {
    let bad = SONG.replace("auto master tilt 0.4 > -0.4", "auto master tilte 0.4 > -0.4");
    let errs = match compiler::compile(&dsl::parse(&bad).unwrap()) { Err(e) => e, Ok(_) => panic!() };
    assert!(errs[0].message.contains("master has no parameter 'tilte'"), "{}", errs[0].message);

    let bad = SONG.replace("auto master tilt 0.4 > -0.4", "auto master drive 0.2 > 0.8");
    let errs = match compiler::compile(&dsl::parse(&bad).unwrap()) { Err(e) => e, Ok(_) => panic!() };
    assert!(errs[0].message.contains("needs a saturate or drive node"), "{}", errs[0].message);
}

#[test]
fn master_gain_automation_fades_the_mix() {
    let mut e = SongEngine::from_source(SONG).expect("load");
    e.start();
    let _ = e.render_steps(16);               // scene a
    let (first, _) = e.render_steps(3);       // start of scene b, gain ≈ 1
    let _ = e.render_steps(9);
    let (last, _) = e.render_steps(3);        // end of scene b, gain ≈ 0
    let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
    assert!(rms(&last) < rms(&first) * 0.3, "master gain must fade: start={} end={}", rms(&first), rms(&last));
}

#[test]
fn chord_symbol_with_velocity_at_row_start_is_not_a_drum_lane() {
    let src = SONG.replace("pattern chords { Fm9 ..*7 Dbmaj7/2 ..*7 }", "pattern chords {\n    Fm9:0.6 ..*15\n    Dbmaj7:0.6 ..*15\n}");
    let ast = dsl::parse(&src).expect("parse");
    let chords = ast.patterns.iter().find(|p| p.name == "chords").unwrap();
    assert!(chords.lane_labels.is_empty());
    assert_eq!(chords.rows.len(), 2);
    assert!(compiler::compile(&ast).is_ok());
}

#[test]
fn power_chord_needs_an_explicit_octave() {
    let src = SONG.replace("pattern chords { Fm9 ..*7 Dbmaj7/2 ..*7 }", "pattern chords { E5/3 ..*7 E5 ..*7 }");
    let ast = dsl::parse(&src).expect("parse");
    let chords = ast.patterns.iter().find(|p| p.name == "chords").unwrap();
    assert_eq!(chord_midi(&chords.rows[0][0]), vec![52, 59], "E5/3 is a power chord");
    assert!(matches!(chords.rows[0][8], Step::Note(_)), "bare E5 stays a note");
}
