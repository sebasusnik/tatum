//! Scene binding by name, level automation, strict scene overrides and the
//! global send-effect settings.

use synth_core::dsl::{self, compiler};
use synth_core::song_engine::SongEngine;

const SONG: &str = r#"
tempo 120
scale A minor
delay sync=dotted_eighth feedback=0.45 filter=0.5
reverb size=0.7 damp=0.4 predelay=20

module fm pluck { algorithm two_op decay 0.3 sustain 0.0 }
module bass sub { cutoff 0.3 }

pattern hi  { 1.4 1.4 1.4 1.4 1.4 1.4 1.4 1.4 1.4 1.4 1.4 1.4 1.4 1.4 1.4 1.4 }
pattern low { 1.1 1.1 1.1 1.1 1.1 1.1 1.1 1.1 1.1 1.1 1.1 1.1 1.1 1.1 1.1 1.1 }

track lead { play hi using pluck level 0.8 delay_send 0.3 out > master }
track bass { play low using sub level 0.8 out > master }

scene a {
    track lead { play hi using pluck }
    track bass { play low using sub }
}

# Same tracks, listed in the opposite order
scene b {
    reverb_mix = 0.2
    auto lead level 0.8 > 0.0
    track bass { play low using sub }
    track lead { play hi using pluck }
}

arrange { a x1 b x1 }
"#;

fn compile_error(src: &str) -> String {
    match compiler::compile(&dsl::parse(src).expect("parse")) {
        Ok(_) => panic!("expected a compile error"),
        Err(errs) => errs[0].to_string(),
    }
}

fn rms(l: &[f32], r: &[f32]) -> f32 {
    let n = l.len().max(1);
    (l.iter().zip(r).map(|(a, b)| a * a + b * b).sum::<f32>() / (2 * n) as f32).sqrt()
}

#[test]
fn send_fx_globals_parse() {
    let ast = dsl::parse(SONG).unwrap();
    assert_eq!(ast.globals.send_delay.sync.as_deref(), Some("dotted_eighth"));
    assert_eq!(ast.globals.send_delay.feedback, Some(0.45));
    assert_eq!(ast.globals.send_reverb.size, Some(0.7));
    assert_eq!(ast.globals.send_reverb.predelay, Some(20.0));

    let bad = SONG.replace("delay sync=dotted_eighth", "delay sync=swung");
    let errs = dsl::parse(&bad).unwrap_err();
    assert!(errs[0].message.contains("delay sync"), "{}", errs[0].message);

    let bad = SONG.replace("reverb size=0.7", "reverb room=0.7");
    let errs = dsl::parse(&bad).unwrap_err();
    assert!(errs[0].message.contains("unknown option 'room'"), "{}", errs[0].message);
}

#[test]
fn scene_tracks_bind_by_name_not_position() {
    let mut engine = SongEngine::from_source(SONG).unwrap();
    let lead = engine.instrument_index("pluck").unwrap();
    let sub = engine.instrument_index("sub").unwrap();
    engine.start();
    // Scene b starts at bar 2; render past it
    engine.render_steps(16 + 2);
    assert_eq!(engine.track_name(0), "lead");
    assert_eq!(engine.track_kind(0), "fm", "lead slot must still drive the fm instrument");
    assert_eq!(engine.track_kind(1), "bass");
    assert_eq!(engine.track_pattern(0), 0, "lead keeps pattern 'hi'");
    assert_eq!(engine.track_pattern(1), 1, "bass keeps pattern 'low'");
    let _ = (lead, sub);
}

#[test]
fn track_level_automation_applies() {
    let mut engine = SongEngine::from_source(SONG).unwrap();
    engine.start();
    engine.render_steps(16); // scene a
    let (l1, r1) = engine.render_steps(4); // scene b start: lead at full level
    let _ = engine.render_steps(8);
    let (l2, r2) = engine.render_steps(4); // scene b end: lead faded to 0
    assert!(rms(&l2, &r2) < rms(&l1, &r1) * 0.8,
        "lead fade should lower the mix: start={} end={}", rms(&l1, &r1), rms(&l2, &r2));
}

#[test]
fn unknown_scene_override_is_an_error() {
    let bad = SONG.replace("reverb_mix = 0.2", "reverb_mixx = 0.2");
    let msg = compile_error(&bad);
    assert!(msg.contains("unknown override 'reverb_mixx'"), "{}", msg);

    let bare = SONG.replace("reverb_mix = 0.2", "reverb_mix");
    let errs = dsl::parse(&bare).unwrap_err();
    assert!(errs[0].message.contains("unexpected 'reverb_mix'"), "{}", errs[0].message);
}

#[test]
fn scene_track_must_exist_at_top_level() {
    let bad = SONG.replace("track bass { play low using sub }\n    track lead", "track ghost { play low using sub }\n    track lead");
    let msg = compile_error(&bad);
    assert!(msg.contains("track 'ghost' is not declared at top level"), "{}", msg);
}
