//! Design lints: warnings for songs that compile but will sound flat.

use synth_core::dsl::{self, compiler, lint};

const SONG: &str = r#"
tempo 120
scale A minor
sidechain 0.3
module keys pad { cutoff 0.3 }
module beats kit { kick_level 1.0 }
pattern hold { [1.3 3.3 5.3] .. .. .. .. .. .. .. .. .. .. .. .. .. .. .. }
pattern beat { kick: X - - - X - - - X - - - X - - - }
track pad   { play hold using pad reverb_send 0.3 out > master }
track drums { play beat using kit out > master }
master { in > limiter > out }
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
    let moving = SONG.replace("module keys pad { cutoff 0.3 }", "module keys pad { cutoff 0.3 lfo_target cutoff lfo_depth 0.2 }");
    assert!(pad_lints(&moving).is_empty());
    // Automating only scene b still leaves scene a static, and the message says so
    let half = SONG.replace("scene b { track pad { play hold using pad } }", "scene b { auto pad cutoff 0.2 > 0.6 track pad { play hold using pad } }");
    let ast = dsl::parse(&half).unwrap();
    let l: Vec<_> = lint::lint_song(&ast).into_iter().filter(|l| l.code == "static_pad").collect();
    assert_eq!(l.len(), 1);
    assert!(l[0].message.ends_with("in: a"), "{}", l[0].message);
    let automated = half.replace("scene a { track pad", "scene a { auto pad cutoff 0.2 > 0.6 track pad");
    assert!(pad_lints(&automated).is_empty());
    let arped = SONG.replace("track pad   { play hold using pad reverb_send 0.3 out > master }", "track pad   { play hold using pad arp up reverb_send 0.3 out > master }");
    assert!(pad_lints(&arped).is_empty());
}

#[test]
fn mix_and_arrangement_lints() {
    let dry = SONG.replace("reverb_send 0.3 ", "").replace("lfo", "lfo");
    assert!(lints(&dry).contains(&"dry_mix"));
    let no_sc = SONG.replace("sidechain 0.3\n", "");
    assert!(lints(&no_sc).contains(&"no_sidechain"));
    let no_lim = SONG.replace("master { in > limiter > out }\n", "");
    assert!(lints(&no_lim).contains(&"no_limiter"));
    let flat = SONG.replace("scene b { track pad { play hold using pad } }\n", "").replace("arrange { a x4 b x4 }", "arrange { a x16 }");
    assert!(lints(&flat).contains(&"single_scene"));
    let unused = SONG.replace("module beats kit { kick_level 1.0 }", "module beats kit { kick_level 1.0 }\nmodule fm spare { }\npattern spare_p { 1.1 - - - }");
    let l = lints(&unused);
    assert!(l.contains(&"unused_module") && l.contains(&"unused_pattern"), "{:?}", l);
}

#[test]
fn sidechain_without_kick_in_a_scene() {
    // scene b has no drums; the pad inherits the global sidechain
    let l = lints(&SONG.replace("module keys pad { cutoff 0.3 }", "module keys pad { cutoff 0.3 lfo_target cutoff lfo_depth 0.2 }"));
    assert_eq!(l, vec!["sidechain_without_kick"], "{:?}", l);
}

#[test]
fn riding_the_level_of_a_sustained_bed_is_flagged() {
    // This is the mistake that produced "the drone jumps in level between
    // sections": a continuous pad moved between 0.34 and 0.60 to hit a
    // loudness target per scene, which reads as someone touching the fader.
    let src = SONG
        .replace("scene a { track pad { play hold using pad }",
                 "scene a { track pad { play hold using pad level 0.34 }")
        .replace("scene b { track pad { play hold using pad } }",
                 "scene b { track pad { play hold using pad level 0.60 } }");
    let l: Vec<_> = lints(&src);
    assert!(l.contains(&"level_used_as_fader"), "{:?}", l);

    // Within 3 dB it is arrangement, not fader riding.
    let src = SONG
        .replace("scene a { track pad { play hold using pad }",
                 "scene a { track pad { play hold using pad level 0.50 }")
        .replace("scene b { track pad { play hold using pad } }",
                 "scene b { track pad { play hold using pad level 0.60 } }");
    assert!(!lints(&src).contains(&"level_used_as_fader"), "{:?}", lints(&src));

    // A staccato part may legitimately be a fader.
    let src = SONG
        .replace("pattern hold { [1.3 3.3 5.3] .. .. .. .. .. .. .. .. .. .. .. .. .. .. .. }",
                 "pattern hold { [1.3 3.3 5.3] - - - [1.3 3.3 5.3] - - - [1.3 3.3 5.3] - - - [1.3 3.3 5.3] - - - }")
        .replace("scene a { track pad { play hold using pad }",
                 "scene a { track pad { play hold using pad level 0.20 }")
        .replace("scene b { track pad { play hold using pad } }",
                 "scene b { track pad { play hold using pad level 0.80 } }");
    assert!(!lints(&src).contains(&"level_used_as_fader"), "{:?}", lints(&src));
}
