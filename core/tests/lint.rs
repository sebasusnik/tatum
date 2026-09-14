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

#[test]
fn static_pad_is_flagged_and_modulation_clears_it() {
    assert_eq!(lints(SONG), vec!["static_pad"]);
    let moving = SONG.replace("module keys pad { cutoff 0.3 }", "module keys pad { cutoff 0.3 lfo_target cutoff lfo_depth 0.2 }");
    assert!(lints(&moving).is_empty());
    // Automating only scene b still leaves scene a static, and the message says so
    let half = SONG.replace("scene b { track pad { play hold using pad } }", "scene b { auto pad cutoff 0.2 > 0.6 track pad { play hold using pad } }");
    let ast = dsl::parse(&half).unwrap();
    let l = lint::lint_song(&ast);
    assert_eq!(l.len(), 1);
    assert!(l[0].message.ends_with("in: a"), "{}", l[0].message);
    let automated = half.replace("scene a { track pad", "scene a { auto pad cutoff 0.2 > 0.6 track pad");
    assert!(lints(&automated).is_empty());
    let arped = SONG.replace("track pad   { play hold using pad reverb_send 0.3 out > master }", "track pad   { play hold using pad arp up reverb_send 0.3 out > master }");
    assert!(lints(&arped).is_empty());
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
