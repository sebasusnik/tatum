//! A later definition of the same name replaces the earlier one.
//!
//! This exists for one reason: a live set is twenty states of one rig, and
//! before this every state had to carry a copy of the whole rig. Measured on a
//! twenty-step set with a ten-voice rig: 4100 lines total, of which 11 changed
//! between one step and the next. Everything else was the same 202 lines,
//! twenty times, and editing the rig orphaned all twenty.
//!
//! With this, a step is `use "rig.synth"` and the handful of tracks it moves.
//! The same set became 416 lines and renders byte-identical.

use tatum_core::dsl;
use tatum_core::song_engine::SongEngine;

const RIG: &str = r#"
tempo 120
scale C minor
humanize 0
module bass low { cutoff 0.5 sustain 0.8 }
pattern sparse { C2:0.9 - - - }
pattern busy   { C2:0.9 - C2:0.6 - }
track bass { play sparse using low level 0.5 out > lowpass(600, 0.1, wet=0.0) as lp > master }
master { in > out }
"#;

fn parse(src: &str) -> dsl::ast::Song {
    dsl::parse(src).unwrap_or_else(|e| panic!("{e:?}"))
}

/// The bug this fixes: a second `track bass` used to add a second track called
/// `bass`, and both played. Two voices where the author wrote one.
#[test]
fn redefining_a_track_replaces_it_rather_than_adding_another() {
    let song = parse(&format!("{RIG}\ntrack bass {{ play busy using low level 0.2 out > master }}"));
    assert_eq!(song.tracks.len(), 1, "a redefinition must not add a track");
    assert_eq!(song.tracks[0].play, "busy");
    assert_eq!(song.tracks[0].level, Some(0.2));
}

#[test]
fn redefining_a_module_or_a_pattern_replaces_it_too() {
    let song = parse(&format!("{RIG}\nmodule bass low {{ cutoff 0.9 }}\npattern sparse {{ E2:0.5 - - - }}"));
    assert_eq!(song.module_defs.len(), 1);
    assert_eq!(song.patterns.len(), 2, "sparse replaced, busy untouched");
    let low = &song.module_defs[0];
    assert_eq!(low.params.iter().find(|p| p.name == "cutoff").map(|p| p.value), Some(0.9));
}

/// Position matters as much as identity. The engine builds and mixes in
/// declaration order, and a live swap inherits voice state by position as well
/// as by name -- so a redefinition that moved a track to the end of the list
/// would hand its state to whatever used to be there.
#[test]
fn a_redefinition_keeps_its_place_in_the_order() {
    let src = r#"
tempo 120
scale C minor
module bass low { cutoff 0.5 }
pattern p { C2:0.9 - - - }
track first  { play p using low level 0.5 out > master }
track second { play p using low level 0.5 out > master }
track third  { play p using low level 0.5 out > master }
track first  { play p using low level 0.1 out > master }
"#;
    let song = parse(src);
    let names: Vec<&str> = song.tracks.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["first", "second", "third"], "order must survive a redefinition");
    assert_eq!(song.tracks[0].level, Some(0.1), "and the redefinition must be the one that won");
}

/// What it is for: the same song written twice, once whole and once as
/// overrides, has to render identically. Not approximately -- identically.
#[test]
fn a_rig_plus_overrides_is_the_same_song_as_writing_it_out() {
    let whole = r#"
tempo 120
scale C minor
humanize 0
module bass low { cutoff 0.5 sustain 0.8 }
pattern sparse { C2:0.9 - - - }
pattern busy   { C2:0.9 - C2:0.6 - }
track bass { play busy using low level 0.2 out > lowpass(600, 0.1, wet=1.0) as lp > master }
master { in > out }
"#;
    let overridden = format!(
        "{RIG}\ntrack bass {{ play busy using low level 0.2 out > lowpass(600, 0.1, wet=1.0) as lp > master }}"
    );
    let render = |s: &str| {
        let mut e = SongEngine::from_source(s).unwrap_or_else(|err| panic!("{err}"));
        e.start();
        e.render(2).0
    };
    assert_eq!(render(whole), render(&overridden));
}

// ── Partial redefinition ─────────────────────────────────────────────────────
//
// Replacing a whole definition was not enough. A step of a live set that wants
// to move one fader still had to restate the track's entire chain, and then it
// owned that chain: a progressive house set had a pad whose filter was fixed in
// the rig and the fix reached none of the twenty steps, because every step had
// copied the old chain. So a redefinition now says only what it changes.

const RIG2: &str = r#"
tempo 120
scale C minor
humanize 0
module bass low { cutoff 0.5 sustain 0.8 resonance 0.2 }
pattern p1 { C2:0.9 - - - }
pattern p2 { C2:0.9 - C2:0.6 - }
track bass { play p1 using low level 0.5 pan -0.3 reverb_send 0.2 out > lowpass(600, 0.1, wet=0.0) as lp > master }
master { in > out }
"#;

#[test]
fn a_redefinition_inherits_everything_it_does_not_mention() {
    let song = parse(&format!("{RIG2}\ntrack bass {{ level 0.2 play p2 }}"));
    let t = &song.tracks[0];
    assert_eq!(song.tracks.len(), 1);
    assert_eq!(t.level, Some(0.2), "what it said");
    assert_eq!(t.play, "p2", "and that");
    assert_eq!(t.using_instrument, "low", "inherited");
    assert_eq!(t.pan, Some(-0.3), "inherited");
    assert_eq!(t.reverb_send, Some(0.2), "inherited");
    assert_eq!(t.routing.len(), 2, "the chain survives: lowpass then master");
}

/// The same for a module: naming one parameter must not reset the others to
/// the registry default, which is what made this worth doing at all.
#[test]
fn a_module_redefinition_keeps_the_parameters_it_does_not_name() {
    let song = parse(&format!("{RIG2}\nmodule bass low {{ cutoff 0.8 }}"));
    let m = &song.module_defs[0];
    let get = |n: &str| m.params.iter().find(|p| p.name == n).map(|p| p.value);
    assert_eq!(get("cutoff"), Some(0.8), "changed");
    assert_eq!(get("sustain"), Some(0.8), "kept");
    assert_eq!(get("resonance"), Some(0.2), "kept");
}

/// And the whole point, end to end: the rig plus a two-word override renders
/// exactly the same as the song written out in full.
#[test]
fn a_partial_override_sounds_like_the_song_written_out() {
    let whole = r#"
tempo 120
scale C minor
humanize 0
module bass low { cutoff 0.8 sustain 0.8 resonance 0.2 }
pattern p1 { C2:0.9 - - - }
pattern p2 { C2:0.9 - C2:0.6 - }
track bass { play p2 using low level 0.2 pan -0.3 reverb_send 0.2 out > lowpass(600, 0.1, wet=0.0) as lp > master }
master { in > out }
"#;
    let overridden = format!("{RIG2}\ntrack bass {{ level 0.2 play p2 }}\nmodule bass low {{ cutoff 0.8 }}");
    let render = |s: &str| {
        let mut e = SongEngine::from_source(s).unwrap_or_else(|err| panic!("{err}"));
        e.start();
        e.render(2).0
    };
    assert_eq!(render(whole), render(&overridden));
}

/// A fix to the rig's chain has to reach a step that never mentioned it.
/// This is the bug that started it: it used to not.
#[test]
fn a_fix_to_the_rigs_chain_reaches_a_step_that_only_moved_a_fader() {
    let fixed_rig = RIG2.replace("lowpass(600, 0.1, wet=0.0) as lp", "lowpass(600, 0.1, wet=1.0) as lp");
    let step = format!("{fixed_rig}\ntrack bass {{ level 0.2 play p2 }}");
    let song = parse(&step);
    let wet = song.tracks[0].routing.iter().flat_map(|n| n.params.iter()).find_map(|p| match p {
        tatum_core::dsl::ast::Param::Named(n, v) if n == "wet" => Some(*v),
        _ => None,
    });
    assert_eq!(wet, Some(1.0), "the step inherited the rig's repaired chain");
}
