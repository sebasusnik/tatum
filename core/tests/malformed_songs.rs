//! Songs that are wrong in ways the engine cannot play. Each has to come back
//! as an error at its line, from `check`, instead of a panic (which kills a
//! live session and the MCP server with it), a hang, or a render that is
//! quietly something else.

use tatum_core::dsl::{self, compiler};
use tatum_core::song_engine::SongEngine;

const TRACK: &str = "pattern p { 1.1 - - - }\ntrack tr { play p using xx out > master }\n";

fn parse_errors(source: &str) -> Vec<String> {
    match dsl::parse(source) {
        Ok(_) => Vec::new(),
        Err(errs) => errs.iter().map(|e| format!("line {}: {}", e.line, e.message)).collect(),
    }
}

fn compile_errors(source: &str) -> Vec<String> {
    let ast = dsl::parse(source).expect("should parse");
    match compiler::compile(&ast) {
        Ok(_) => Vec::new(),
        Err(errs) => errs.iter().map(|e| e.to_string()).collect(),
    }
}

fn one_error(errs: Vec<String>, line: usize, says: &str) {
    assert_eq!(errs.len(), 1, "{:?}", errs);
    assert!(errs[0].starts_with(&format!("line {}:", line)), "{}", errs[0]);
    assert!(errs[0].contains(says), "{}", errs[0]);
}

#[test]
fn globals_outside_what_the_engine_plays_are_errors() {
    let song = |globals: &str| format!("{}\nmodule bass xx {{ cutoff 1khz }}\n{}", globals, TRACK);
    one_error(parse_errors(&song("tempo 0")), 1, "tempo 0 is outside 20..999");
    one_error(parse_errors(&song("tempo 0.01")), 1, "outside 20..999");
    one_error(parse_errors(&song("tempo 1200")), 1, "outside 20..999");
    one_error(parse_errors(&song("tempo 120\nmeter 0/4")), 2, "meter 0 is outside 1..16");
    one_error(parse_errors(&song("tempo 120\nmeter 300/4")), 2, "outside 1..16");
    one_error(parse_errors(&song("tempo 120\nmeter 7/8")), 2, "only /4");
    one_error(parse_errors(&song("tempo 120\nswing 5")), 2, "swing 5 is outside 0.5..0.75");
    one_error(parse_errors(&song("tempo 120\nswing 0.3")), 2, "outside 0.5..0.75");
    one_error(parse_errors(&song("tempo 120\nhumanize 2")), 2, "humanize 2 is outside 0..1");
    // Both halves of the line are checked, and neither takes the rest down.
    let errs = parse_errors(&song("tempo 120\nhumanize 50 timing 9"));
    assert_eq!(errs.len(), 2, "{:?}", errs);
    assert!(errs[1].contains("humanize timing 9"), "{:?}", errs);
    // The edges are in.
    for ok in ["tempo 20", "tempo 999", "tempo 120\nmeter 1/4", "tempo 120\nmeter 16/4",
               "tempo 120\nswing 0.5", "tempo 120\nswing 0.75", "tempo 120\nhumanize 1 timing 1"] {
        assert!(parse_errors(&song(ok)).is_empty(), "{}: {:?}", ok, parse_errors(&song(ok)));
    }
}

#[test]
fn a_scene_tempo_is_held_to_the_same_span() {
    let src = format!("tempo 120\nmodule bass xx {{ cutoff 1khz }}\n{}scene a {{ tempo 0 }}\narrange {{ a x2 }}\n", TRACK);
    one_error(parse_errors(&src), 5, "tempo 0 is outside 20..999");
}

#[test]
fn an_instrument_that_feeds_itself_is_an_error_naming_the_loop() {
    let src = format!("tempo 120\ninstrument xx {{\n  osc saw(55) as osc1\n  osc1 > mix\n  mix > lowpass(800) as flt\n  flt > gain(0.5) as amp\n  amp > mix\n  amp > out\n}}\n{}", TRACK);
    let errs = compile_errors(&src);
    // One error: the track that plays the instrument does not add another.
    one_error(errs.clone(), 5, "loop back on themselves");
    assert!(errs[0].contains("(mix, flt, amp feed each other)"), "{}", errs[0]);
}

#[test]
fn an_instrument_past_the_graph_capacity_is_an_error_at_the_node() {
    let mut src = String::from("tempo 120\ninstrument xx {\n");
    for i in 0..40 {
        src += &format!("  osc saw(55) as osc{i}\n  osc{i} > mix\n");
    }
    src += "  mix > out\n}\n";
    src += TRACK;
    // mix is node 0, then osc0..osc30 fill the graph; osc31 is on line 65.
    one_error(compile_errors(&src), 65, "more than 32 nodes");
}

#[test]
fn a_node_with_too_many_inputs_is_an_error_at_the_connection() {
    let mut src = String::from("tempo 120\ninstrument xx {\n  osc saw(55) as osc0\n  osc0 > lowpass(800) as flt\n");
    for i in 1..9 {
        src += &format!("  osc saw(55) as osc{i}\n  osc{i} > flt\n");
    }
    src += "  flt > out\n}\n";
    src += TRACK;
    one_error(compile_errors(&src), 20, "more than 8 connections into 'flt'");
}

#[test]
fn instrument_errors_carry_the_line() {
    let src = format!("tempo 120\ninstrument xx {{\n  osc saw(55) as osc1\n  osc1 > flt\n  osc1 > out\n}}\n{}", TRACK);
    one_error(compile_errors(&src), 4, "unknown node 'flt'");
}

#[test]
fn an_instrument_that_moves_down_the_file_is_the_same_instrument() {
    // The live diff compares definitions; a line above them must not make
    // every instrument read as changed and force a full swap.
    let a = format!("tempo 120\ninstrument xx {{\n  osc saw(55) as osc1\n  osc1 > out\n}}\n{}", TRACK);
    let b = format!("tempo 120\n\n\n# a comment\ninstrument xx {{\n  osc saw(55) as osc1\n\n  osc1 > out\n}}\n{}", TRACK);
    assert_eq!(dsl::parse(&a).unwrap().instruments, dsl::parse(&b).unwrap().instruments);
}

#[test]
fn a_song_built_without_the_parser_is_clamped_by_the_engine() {
    let src = format!("tempo 120\nmodule bass xx {{ cutoff 1khz }}\n{}", TRACK);
    let mut ast = dsl::parse(&src).unwrap();
    ast.globals.tempo = 0.0;
    ast.globals.meter = (0, 4);
    ast.globals.swing = Some(5.0);
    let song = compiler::compile(&ast).unwrap();
    let mut engine = SongEngine::from_compiled(song);
    assert_eq!(engine.tempo(), 20.0);
    assert_eq!(engine.swing(), 0.75);
    let (l, _) = engine.render(1);
    assert!(!l.is_empty() && l.iter().all(|s| s.is_finite()));
}

#[test]
fn a_note_past_the_top_of_midi_is_an_error() {
    // `C999` used to play as C4.
    let src = "tempo 120\nmodule bass xx { cutoff 1khz }\npattern p { C999 - G9 - [C4 G#9] - <C4 D77> - }\ntrack tr { play p using xx out > master }\n";
    let errs = parse_errors(src);
    assert_eq!(errs.len(), 3, "{:?}", errs);
    assert!(errs.iter().all(|e| e.starts_with("line 3:") && e.contains("above what MIDI can play")), "{:?}", errs);
    assert!(errs[0].contains("C999") && errs[1].contains("G#9") && errs[2].contains("D77"), "{:?}", errs);
}

#[test]
fn a_setter_handed_nan_leaves_the_value_alone() {
    // Knobs, the browser and MIDI maps call these at run time; a NaN used to
    // pass `clamp` and poison the filter it reached.
    let src = format!("tempo 120\nmodule bass xx {{ cutoff 1khz }}\n{}", TRACK);
    let mut engine = SongEngine::from_source(&src).unwrap();
    engine.start();
    for v in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        engine.set_track_level(0, v);
        engine.set_track_pan(0, v);
        engine.set_track_velocity(0, v);
        engine.set_track_gate(0, v);
        engine.set_tempo(v);
        engine.set_swing(v);
        engine.set_humanize(v, v);
        engine.set_reverb_mix(v);
        engine.set_delay_mix(v);
        engine.set_pitch_bend(0, v);
        engine.set_output_gain(v);
        assert!(!engine.set_module_param(0, "cutoff", v));
    }
    assert_eq!(engine.tempo(), 120.0);
    let (l, r) = engine.render(2);
    assert!(l.iter().chain(&r).all(|s| s.is_finite()));
    assert!(l.iter().any(|s| s.abs() > 1e-3), "the track still plays");
}
