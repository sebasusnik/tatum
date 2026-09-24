//! Strict parameter validation: unknown names, out-of-range values, symbolic
//! choices and automation targets must fail loudly instead of being ignored.

use tatum_core::dsl::{self, compiler};
use tatum_core::song_engine::SongEngine;

const BASE: &str = r#"
tempo 120
scale A minor

module bass acid {
    cutoff 0.3
    resonance 0.8
}

module fm bell {
    algorithm two_op
    waveform half_sine
    lfo_target mod_index
}

module keys pad {
    voice_mode unison
}

pattern p { 1.1:0.9 - - - }

track acid { play p using acid out > master }
track bell { play p using bell out > master }
track pad  { play p using pad  out > master }

scene a {
    auto acid cutoff 0.1 > 0.5
    track acid { play p using acid }
}

arrange { a x1 }
"#;

fn compile_errors(source: &str) -> Vec<String> {
    let ast = dsl::parse(source).expect("should parse");
    match compiler::compile(&ast) {
        Ok(_) => Vec::new(),
        Err(errs) => errs.iter().map(|e| e.to_string()).collect(),
    }
}

#[test]
fn symbolic_choices_compile_and_encode_correctly() {
    assert!(compile_errors(BASE).is_empty(), "{:?}", compile_errors(BASE));
    let ast = dsl::parse(BASE).unwrap();
    let bell = ast.module_defs.iter().find(|m| m.name == "bell").unwrap();
    let algo = bell.params.iter().find(|p| p.name == "algorithm").unwrap();
    assert!((algo.value * 7.0 - 2.0).abs() < 1e-4, "two_op must be index 2, got {}", algo.value);
    let wave = bell.params.iter().find(|p| p.name == "waveform").unwrap();
    assert!((wave.value * 3.0 - 1.0).abs() < 1e-4, "half_sine must be index 1, got {}", wave.value);
    let pad = ast.module_defs.iter().find(|m| m.name == "pad").unwrap();
    let mode = pad.params.iter().find(|p| p.name == "voice_mode").unwrap();
    assert!((mode.value - 0.25).abs() < 1e-4, "unison must encode as 0.25");
}

#[test]
fn unknown_param_is_an_error_with_suggestion() {
    let src = BASE.replace("resonance 0.8", "resonanse 0.8");
    let errs = compile_errors(&src);
    assert_eq!(errs.len(), 1, "{:?}", errs);
    assert!(errs[0].contains("unknown parameter 'resonanse'"), "{}", errs[0]);
    assert!(errs[0].contains("Did you mean 'resonance'"), "{}", errs[0]);
    assert!(errs[0].starts_with("line 7:"), "should carry the source line: {}", errs[0]);
}

#[test]
fn param_from_another_module_is_explained() {
    let src = BASE.replace("resonance 0.8", "kick_level 0.8");
    let errs = compile_errors(&src);
    assert_eq!(errs.len(), 1, "{:?}", errs);
    assert!(errs[0].contains("It exists on: beats"), "{}", errs[0]);
}

#[test]
fn out_of_range_value_is_an_error() {
    let src = BASE.replace("cutoff 0.3", "cutoff 1.7");
    let errs = compile_errors(&src);
    assert_eq!(errs.len(), 1, "{:?}", errs);
    assert!(errs[0].contains("'cutoff' = 1.7 is out of range (0.0..1.0)"), "{}", errs[0]);
}

#[test]
fn integer_index_for_choice_param_is_rejected() {
    // `lfo_target 2` used to decode to whatever the module's fallback arm was.
    let src = BASE.replace("lfo_target mod_index", "lfo_target 2");
    let errs = compile_errors(&src);
    assert_eq!(errs.len(), 1, "{:?}", errs);
    assert!(errs[0].contains("pitch | amplitude | mod_index"), "{}", errs[0]);
}

#[test]
fn unknown_choice_name_is_a_parse_error() {
    let src = BASE.replace("waveform half_sine", "waveform saw");
    let errs = dsl::parse(&src).expect_err("should fail to parse");
    assert_eq!(errs.len(), 1, "{:?}", errs);
    assert!(errs[0].message.contains("'waveform' has no option 'saw'"), "{}", errs[0].message);
    assert!(errs[0].message.contains("sine | half_sine | abs_sine | quarter_sine"), "{}", errs[0].message);
}

#[test]
fn bad_module_does_not_cascade_into_track_errors() {
    let src = BASE.replace("cutoff 0.3", "cutof 0.3");
    let errs = compile_errors(&src);
    assert_eq!(errs.len(), 1, "one root cause, one error: {:?}", errs);
}

#[test]
fn automation_target_is_validated() {
    let src = BASE.replace("auto acid cutoff", "auto acid cutof");
    let errs = compile_errors(&src);
    assert_eq!(errs.len(), 1, "{:?}", errs);
    assert!(errs[0].contains("automation") && errs[0].contains("Did you mean 'cutoff'"), "{}", errs[0]);

    let src = BASE.replace("auto acid cutoff", "auto nobody cutoff");
    let errs = compile_errors(&src);
    assert_eq!(errs.len(), 1, "{:?}", errs);
    assert!(errs[0].contains("no module or track named 'nobody'"), "{}", errs[0]);
}

#[test]
fn runtime_param_changes_target_the_named_instrument() {
    let mut engine = SongEngine::from_source(BASE).unwrap();
    assert_eq!(engine.instrument_index("bell"), Some(1));
    assert_eq!(engine.instrument_index("nobody"), None);
    assert_eq!(engine.instrument_name(2), "pad");

    let bell = engine.instrument_index("bell").unwrap();
    assert!(engine.set_module_param(bell, "mod_index", 0.9));
    assert!(!engine.set_module_param(bell, "cutoff", 0.9), "fm has no cutoff");
    assert!(!engine.set_module_param(99, "mod_index", 0.9), "no such instrument");
}

#[test]
fn swing_and_humanize_apply_live() {
    let mut engine = SongEngine::from_source(BASE).unwrap();
    engine.set_swing(0.62);
    assert!((engine.swing() - 0.62).abs() < 1e-6);
    engine.set_swing(0.9);
    assert!((engine.swing() - 0.75).abs() < 1e-6, "clamped to the shuffle range");
    engine.set_humanize(0.2, 0.05);
    assert_eq!(engine.humanize(), (0.2, 0.05));
}

#[test]
fn short_names_that_look_like_drum_hits_are_valid_identifiers() {
    let src = BASE.replace("module bass acid {", "module bass x {")
        .replace("using acid", "using x")
        .replace("auto acid cutoff", "auto x cutoff");
    assert!(compile_errors(&src).is_empty(), "{:?}", compile_errors(&src));
    let src = BASE.replace("track pad  {", "track o  {");
    assert!(compile_errors(&src).is_empty(), "{:?}", compile_errors(&src));
}

#[test]
fn one_top_level_mistake_yields_one_error() {
    let src = "tempo 120\nmodul bass acid { cutoff 0.3 resonance 0.5 }\nscale A minor\n";
    let errs = dsl::parse(src).unwrap_err();
    assert_eq!(errs.len(), 1, "{:?}", errs);
    assert_eq!(errs[0].line, 2);
    assert!(errs[0].message.contains("unexpected 'modul'"), "{}", errs[0].message);
}

#[test]
fn unknown_drum_lane_is_an_error() {
    let src = BASE.replace("pattern p { 1.1:0.9 - - - }", "pattern p { kik: X - - - }")
        .replace("using bell", "using bell");
    let errs = compile_errors(&src);
    assert!(errs.iter().any(|e| e.contains("unknown drum lane 'kik'")), "{:?}", errs);
}

#[test]
fn effect_arguments_are_validated() {
    let bad = BASE.replace("track acid { play p using acid out > master }", "track acid { play p using acid out > saturat(0.3) > master }");
    let errs = compile_errors(&bad);
    assert!(errs[0].contains("unknown node 'saturat'") && errs[0].contains("Did you mean 'saturate'"), "{:?}", errs);

    let bad = BASE.replace("track acid { play p using acid out > master }", "track acid { play p using acid out > compressor(-8, ration=4) > master }");
    let errs = compile_errors(&bad);
    assert!(errs[0].contains("unknown option 'ration'") && errs[0].contains("ratio, attack, release, makeup"), "{:?}", errs);

    let bad = BASE.replace("track acid { play p using acid out > master }", "track acid { play p using acid out > lowpass(0.4) > master }");
    let errs = compile_errors(&bad);
    assert!(errs[0].contains("cutoff = 0.4 is out of range (20..20000)"), "{:?}", errs);

    let bad = BASE.replace("arrange { a x1 }", "master { in > eq(low=2, hi=1) > out }\narrange { a x1 }");
    let errs = compile_errors(&bad);
    assert!(errs[0].contains("master: eq: unknown option 'hi'"), "{:?}", errs);

    let bad = BASE.replace("track acid { play p using acid out > master }", "track acid { play p using acid out > saturate(warm) > master }");
    let errs = dsl::parse(&bad).unwrap_err();
    assert!(errs[0].message.contains("unexpected 'warm' in arguments"), "{}", errs[0].message);
}

#[test]
fn tie_and_rest_repeat_shorthand() {
    let src = BASE.replace("pattern p { 1.1:0.9 - - - }", "pattern p { 1.1:0.9 ..*7 -*4 [1.3 3.3]:0.5 ..*3 }");
    let ast = dsl::parse(&src).expect("parse");
    let p = ast.patterns.iter().find(|p| p.name == "p").unwrap();
    assert_eq!(p.rows[0].len(), 1 + 7 + 4 + 1 + 3);
    let bad = BASE.replace("pattern p { 1.1:0.9 - - - }", "pattern p { 1.1:0.9 ..*0 }");
    let errs = dsl::parse(&bad).unwrap_err();
    assert!(errs[0].message.contains("expected a count 1..256"), "{}", errs[0].message);
}

#[test]
fn unknown_step_parameter_is_an_error() {
    // A step lock used to drop names it did not recognise, so `(cutof=80)` set
    // nothing and the step just played normally.
    let src = BASE.replace("pattern p { 1.1:0.9 - - - }", "pattern p { 1.1:0.9(cutof=0.8) - - - }");
    let errs = dsl::parse(&src).expect_err("should fail to parse");
    assert!(errs[0].message.contains("unknown step parameter 'cutof'"), "{}", errs[0].message);
    assert!(errs[0].message.contains("cutoff, edepth, res or gate"), "{}", errs[0].message);

    let src = BASE.replace("pattern p { 1.1:0.9 - - - }", "pattern p { 1.1:0.9(cutoff=0.8, gate=0.4) - - - }");
    assert!(dsl::parse(&src).is_ok(), "the real names still parse");
}
