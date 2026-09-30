//! Arpeggiator and slide support in the DSL / SongEngine, plus the diff
//! engine treating every unhandled edit as structural.

use tatum_core::dsl::{self, compiler, diff};
use tatum_core::dsl::ast::Step;
use tatum_core::song_engine::SongEngine;

const SONG: &str = r#"
tempo 120
scale A minor

module fm pluck {
    algorithm two_op
    attack 0.0
    decay 0.3
    sustain 0.0
    release 0.1
}

module bass acid {
    cutoff 0.3
    glide 0.2
}

pattern chords { [1.3 3.3 5.3] .. .. .. .. .. .. .. - - - - - - - - }
pattern acid   { 1.2:0.9 ~1.2:0.8 ~5.2:0.8 - 1.2 - ~3.2 - }

track lead { play chords using pluck arp up rate=16 gate=0.5 octaves=2 out > master }
track acid { play acid using acid out > master }

scene a {
    track lead { play chords using pluck }
    track acid { play acid using acid }
}

arrange { a x2 }
"#;

fn compile_errors(src: &str) -> Vec<tatum_core::dsl::error::CompileError> {
    match compiler::compile(&dsl::parse(src).unwrap()) {
        Ok(_) => panic!("expected compile errors"),
        Err(errs) => errs,
    }
}

fn rms(l: &[f32], r: &[f32]) -> f32 {
    let n = l.len().max(1);
    let sum: f32 = l.iter().zip(r).map(|(a, b)| a * a + b * b).sum();
    (sum / (2 * n) as f32).sqrt()
}

#[test]
fn tilde_marks_a_slide_step() {
    let ast = dsl::parse(SONG).unwrap();
    let acid = ast.patterns.iter().find(|p| p.name == "acid").unwrap();
    let slides: Vec<bool> = acid.rows[0]
        .iter()
        .map(|s| match s {
            Step::Note(n) => n.slide,
            _ => false,
        })
        .collect();
    assert_eq!(slides, vec![false, true, true, false, false, false, true, false]);
}

#[test]
fn arp_clause_parses_and_compiles() {
    let ast = dsl::parse(SONG).unwrap();
    let lead = ast.tracks.iter().find(|t| t.name == "lead").unwrap();
    let arp = lead.arp.as_ref().expect("arp clause");
    assert_eq!(arp.mode, "up");
    assert_eq!(arp.rate, Some(16.0));
    assert_eq!(arp.gate, Some(0.5));
    assert_eq!(arp.octaves, Some(2.0));

    let compiled = compiler::compile(&ast).unwrap();
    let track = compiled.tracks.iter().find(|t| t.name == "lead").unwrap();
    let cfg = track.arp.expect("compiled arp");
    assert_eq!(cfg.octaves, 2);
    assert!((cfg.rate_mult - 1.0).abs() < 1e-6);
    // Scene tracks inherit the arp from the global track definition
    let scene_track = compiled.scenes[0].tracks.iter().find(|t| t.name == "lead").unwrap();
    assert_eq!(scene_track.arp, track.arp);
}

#[test]
fn arp_off_and_bad_values_are_reported() {
    let off = SONG.replace("arp up rate=16 gate=0.5 octaves=2", "arp off");
    let compiled = compiler::compile(&dsl::parse(&off).unwrap()).unwrap();
    assert!(compiled.tracks.iter().find(|t| t.name == "lead").unwrap().arp.is_none());

    let bad_rate = SONG.replace("rate=16", "rate=12");
    let errs = compile_errors(&bad_rate);
    assert!(errs[0].message.contains("arp rate 12"), "{}", errs[0].message);

    let bad_mode = SONG.replace("arp up", "arp random");
    let errs = compile_errors(&bad_mode);
    assert!(errs[0].message.contains("arp mode 'random'"), "{}", errs[0].message);
}

#[test]
fn arp_runs_while_notes_are_held_and_stops_on_rest() {
    let mut engine = SongEngine::from_source(SONG).unwrap();
    let lead = 0;
    engine.start();
    // Half a bar in: chord is held by ties, arp must be running
    let (l, r) = engine.render_steps(6);
    assert!(engine.track_arp_active(lead), "arp should run during the held chord");
    assert!(rms(&l, &r) > 0.01, "arp should produce sound, rms={}", rms(&l, &r));
    // Into the rests of the second half: arp stopped
    let (l2, r2) = engine.render_steps(6);
    assert!(!engine.track_arp_active(lead), "arp should stop on rests");
    assert!(rms(&l2[l2.len() / 2..], &r2[r2.len() / 2..]) < rms(&l, &r), "tail should be quieter than the arp");
}

#[test]
fn slide_song_renders_without_retrigger_clicks() {
    let mut engine = SongEngine::from_source(SONG).unwrap();
    let bars = engine.arrangement_bars();
    let (l, r) = engine.render(bars);
    let peak = l.iter().chain(r.iter()).fold(0.0f32, |a, &b| a.max(b.abs()));
    assert!(peak > 0.01 && peak <= 1.0, "peak={}", peak);
}

/// A slide after a tied note glides from it at any track gate. The last tie
/// used to apply the gate, so under 1.0 the note was let go an instant before
/// the slide arrived and the slide was struck as a new note: the pitch jumped
/// to it instead of gliding.
#[test]
fn a_slide_after_a_tie_glides_from_the_held_note() {
    let song = r#"
tempo 120
module bass b { attack 1ms sustain 1.0 cutoff 300hz cutoff_env 20hz resonance 0 glide 0 }
pattern p { A2:1.0 ..*7 ~A3 ..*7 }
track t { play p using b level 0.8 gate 0.85 }
scene s { track t { play p using b gate 0.85 } }
arrange { s x1 }
"#;
    let mut engine = SongEngine::from_source(song).unwrap();
    engine.start();
    engine.render_steps(8);
    let (l, _) = engine.render_steps(1);
    // Rising zero crossings over the first 40 ms of the slide step: 110 Hz
    // is A2, 220 Hz is A3. The slowest glide is still well short of A3 here;
    // a re-struck note is already there.
    let window = (0.040 * tatum_core::SAMPLE_RATE) as usize;
    let crossings = l[..window].windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
    let hz = crossings as f32 / 0.040;
    assert!(hz < 190.0, "the slide jumped to {hz} Hz instead of gliding up from 110");
}

#[test]
fn diff_treats_pattern_and_scene_edits_as_structural() {
    let old = dsl::parse(SONG).unwrap();

    let edited = SONG.replace("1.2:0.9 ~1.2:0.8", "1.2:0.9 ~3.2:0.8");
    let new = dsl::parse(&edited).unwrap();
    assert!(diff::has_structural_change(&diff::diff(&old, &new)), "editing pattern notes must hot-swap");

    let edited = SONG.replace("track lead { play chords using pluck }", "track lead { play acid using pluck }");
    let new = dsl::parse(&edited).unwrap();
    assert!(diff::has_structural_change(&diff::diff(&old, &new)), "editing a scene must hot-swap");

    let edited = SONG.replace("arp up rate=16", "arp down rate=16");
    let new = dsl::parse(&edited).unwrap();
    assert!(diff::has_structural_change(&diff::diff(&old, &new)), "changing the arp must hot-swap");
}

#[test]
fn diff_fast_paths_gate_and_removed_params() {
    let old = dsl::parse(SONG).unwrap();

    let edited = SONG.replace(
        "track acid { play acid using acid out > master }",
        "track acid { play acid using acid gate 0.5 out > master }",
    );
    let changes = diff::diff(&old, &dsl::parse(&edited).unwrap());
    assert!(!diff::has_structural_change(&changes));
    assert!(changes
        .iter()
        .any(|c| matches!(c, diff::DslChange::TrackGateChanged { gate, .. } if (*gate - 0.5).abs() < 1e-6)));

    // Deleting `glide 0.2` restores the registry default at runtime
    let edited = SONG.replace("    glide 0.2\n", "");
    let changes = diff::diff(&old, &dsl::parse(&edited).unwrap());
    assert!(!diff::has_structural_change(&changes));
    assert!(changes
        .iter()
        .any(|c| matches!(c, diff::DslChange::ModuleParamChanged { param_name, .. } if param_name == "glide")));
}
