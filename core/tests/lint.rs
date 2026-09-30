//! Design lints: warnings for songs that compile but will sound flat.

use tatum_core::dsl::{self, compiler, lint};

const SONG: &str = r#"
tempo 120
scale A minor
sidechain 0.3
module keys pad { cutoff 1khz }
module beats kit { kick_level 1.0 }
pattern hold { [1.3 3.3 5.3] .. .. .. .. .. .. .. .. .. .. .. .. .. .. .. }
pattern beat { kick: X - - - X - - - X - - - X - - - }
track pad   { play hold using pad reverb_send 0.3 out > master }
track drums { play beat using kit out > master }
master { in > eq(low=1.0) > out }
scene a { track pad { play hold using pad } track drums { play beat using kit } }
scene b { track pad { play hold using pad } }
arrange { a x4 b x4 }
"#;

fn lints(src: &str) -> Vec<&'static str> {
    let ast = dsl::parse(src).expect("parse");
    compiler::compile(&ast).expect("compile");
    lint::lint_song(&ast).into_iter().map(|l| l.code).collect()
}

/// Lints other than the sidechain one (scene b of SONG has no drums on purpose).
fn pad_lints(src: &str) -> Vec<&'static str> {
    lints(src).into_iter().filter(|c| *c != "sidechain_without_kick").collect()
}

#[test]
fn static_pad_is_flagged_and_modulation_clears_it() {
    assert_eq!(pad_lints(SONG), vec!["static_pad"]);
    let moving = SONG
        .replace("module keys pad { cutoff 1khz }", "module keys pad { cutoff 1khz lfo_target cutoff lfo_depth 0.2 }");
    assert!(pad_lints(&moving).is_empty());
    // Automating only scene b still leaves scene a static, and the message says so
    let half = SONG.replace(
        "scene b { track pad { play hold using pad } }",
        "scene b { auto pad cutoff 0.2 > 0.6 track pad { play hold using pad } }",
    );
    let ast = dsl::parse(&half).unwrap();
    let l: Vec<_> = lint::lint_song(&ast).into_iter().filter(|l| l.code == "static_pad").collect();
    assert_eq!(l.len(), 1);
    assert!(l[0].message.ends_with("in: a"), "{}", l[0].message);
    let automated = half.replace("scene a { track pad", "scene a { auto pad cutoff 0.2 > 0.6 track pad");
    assert!(pad_lints(&automated).is_empty());
    let arped = SONG.replace(
        "track pad   { play hold using pad reverb_send 0.3 out > master }",
        "track pad   { play hold using pad arp up reverb_send 0.3 out > master }",
    );
    assert!(pad_lints(&arped).is_empty());
}

#[test]
fn mix_and_arrangement_lints() {
    let dry = SONG.replace("reverb_send 0.3 ", "");
    assert!(lints(&dry).contains(&"dry_mix"));
    let no_sc = SONG.replace("sidechain 0.3\n", "");
    assert!(lints(&no_sc).contains(&"no_sidechain"));
    // The engine limits every song itself, so a master limiter is left out.
    let limited = SONG.replace("eq(low=1.0) > out", "eq(low=1.0) > limiter > out");
    assert!(lints(&limited).contains(&"master_limiter"));
    assert!(!lints(SONG).contains(&"master_limiter"));
    let flat = SONG
        .replace("scene b { track pad { play hold using pad } }\n", "")
        .replace("arrange { a x4 b x4 }", "arrange { a x16 }");
    assert!(lints(&flat).contains(&"single_scene"));
    let unused = SONG.replace(
        "module beats kit { kick_level 1.0 }",
        "module beats kit { kick_level 1.0 }\nmodule fm spare { }\npattern spare_p { 1.1 - - - }",
    );
    let l = lints(&unused);
    assert!(l.contains(&"unused_module") && l.contains(&"unused_pattern"), "{:?}", l);
}

#[test]
fn sidechain_without_kick_in_a_scene() {
    // scene b has no drums; the pad inherits the global sidechain
    let l =
        lints(&SONG.replace(
            "module keys pad { cutoff 1khz }",
            "module keys pad { cutoff 1khz lfo_target cutoff lfo_depth 0.2 }",
        ));
    assert_eq!(l, vec!["sidechain_without_kick"], "{:?}", l);
}

#[test]
fn riding_the_level_of_a_sustained_bed_is_flagged() {
    // This is the mistake that produced "the drone jumps in level between
    // sections": a continuous pad moved between 0.34 and 0.60 to hit a
    // loudness target per scene, which reads as someone touching the fader.
    let src = SONG
        .replace(
            "scene a { track pad { play hold using pad }",
            "scene a { track pad { play hold using pad level 0.34 }",
        )
        .replace(
            "scene b { track pad { play hold using pad } }",
            "scene b { track pad { play hold using pad level 0.60 } }",
        );
    let l: Vec<_> = lints(&src);
    assert!(l.contains(&"level_used_as_fader"), "{:?}", l);

    // Within 3 dB it is arrangement, not fader riding.
    let src = SONG
        .replace(
            "scene a { track pad { play hold using pad }",
            "scene a { track pad { play hold using pad level 0.50 }",
        )
        .replace(
            "scene b { track pad { play hold using pad } }",
            "scene b { track pad { play hold using pad level 0.60 } }",
        );
    assert!(!lints(&src).contains(&"level_used_as_fader"), "{:?}", lints(&src));

    // A staccato part may legitimately be a fader.
    let src = SONG
        .replace(
            "pattern hold { [1.3 3.3 5.3] .. .. .. .. .. .. .. .. .. .. .. .. .. .. .. }",
            "pattern hold { [1.3 3.3 5.3] - - - [1.3 3.3 5.3] - - - [1.3 3.3 5.3] - - - [1.3 3.3 5.3] - - - }",
        )
        .replace(
            "scene a { track pad { play hold using pad }",
            "scene a { track pad { play hold using pad level 0.20 }",
        )
        .replace(
            "scene b { track pad { play hold using pad } }",
            "scene b { track pad { play hold using pad level 0.80 } }",
        );
    assert!(!lints(&src).contains(&"level_used_as_fader"), "{:?}", lints(&src));
}

#[test]
fn a_knob_position_on_a_parameter_with_a_unit_is_flagged() {
    let bare = SONG.replace("cutoff 1khz", "cutoff 0.3");
    assert!(lints(&bare).contains(&"bare_number"));
    assert!(!lints(SONG).contains(&"bare_number"));
    // A parameter without a unit is fine as a number.
    assert!(!lints(&SONG.replace("cutoff 1khz", "cutoff 1khz resonance 0.3")).contains(&"bare_number"));
}

const LOCKS: &str = r#"
tempo 120
module MODTYPE lead { }
pattern line { C3:0.9(cutoff=0.4) - C3:0.8(res=0.6) - C3:0.9(gate=0.3) - - - }
track lead { play line using lead pan 0.3 reverb_send 0.2 out > master }
"#;

fn lock_lints(module_type: &str) -> Vec<String> {
    let src = LOCKS.replace("MODTYPE", module_type);
    let ast = dsl::parse(&src).expect("parse");
    compiler::compile(&ast).expect("compile");
    lint::lint_song(&ast).into_iter().filter(|l| l.code == "plock_ignored").map(|l| l.message).collect()
}

#[test]
fn filter_locks_on_a_module_that_drops_them_are_flagged() {
    let found = lock_lints("fm");
    assert_eq!(found.len(), 1, "{found:?}");
    // Two steps carry filter locks; the gate lock is not counted.
    assert!(found[0].contains("2 filter locks (cutoff, res) in 'line' (2)"), "{}", found[0]);
}

/// `keys` takes cutoff and resonance per voice; only `edepth` is lost on it.
#[test]
fn keys_is_flagged_only_for_the_envelope_it_does_not_have() {
    assert!(lock_lints("keys").is_empty());
    let src = LOCKS.replace("MODTYPE", "keys").replace("C3:0.8(res=0.6)", "C3:0.8(edepth=0.6)");
    let ast = dsl::parse(&src).expect("parse");
    compiler::compile(&ast).expect("compile");
    let found: Vec<_> = lint::lint_song(&ast).into_iter().filter(|l| l.code == "plock_ignored").collect();
    assert_eq!(found.len(), 1);
    assert!(found[0].message.contains("1 filter lock (edepth)"), "{}", found[0].message);
}

#[test]
fn filter_locks_on_a_bass_are_not_flagged() {
    assert!(lock_lints("bass").is_empty());
}

#[test]
fn a_gate_lock_alone_is_not_flagged() {
    let src =
        LOCKS.replace("MODTYPE", "fm").replace("C3:0.9(cutoff=0.4) - C3:0.8(res=0.6)", "C3:0.9(gate=0.5) - C3:0.8");
    let ast = dsl::parse(&src).expect("parse");
    compiler::compile(&ast).expect("compile");
    assert!(lint::lint_song(&ast).iter().all(|l| l.code != "plock_ignored"));
}
