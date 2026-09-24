//! 1.6 / 8.11: a parameter written with its unit. The DSL's whole premise is
//! that a human can read it, and `cutoff 0.433677` is not readable. A wrong
//! unit must be an error, not a number that happens to land in range —
//! `makeup=4` meaning +12 dB got past validation in all fourteen uses.

use tatum_core::dsl;
use tatum_core::params::{self, ModuleKind};

fn param(src: &str, module: &str, name: &str) -> f32 {
    let ast = dsl::parse(src).unwrap_or_else(|e| panic!("{:?}", e));
    let m = ast.module_defs.iter().find(|m| m.name == module).expect("module");
    m.params.iter().find(|p| p.name == name).unwrap_or_else(|| panic!("no '{}'", name)).value
}

fn song(body: &str) -> String {
    format!(
        "tempo 120\nscale A minor\n\nmodule bass acid {{\n{}}}\n\n\
         pattern p {{ 1.1 - 1.3 - }}\ntrack acid {{ play p using acid out > master }}\n\
         scene a {{ track acid {{ play p using acid }} }}\narrange {{ a x1 }}\n",
        body
    )
}

fn errors(src: &str) -> Vec<String> {
    dsl::parse(src).err().unwrap_or_default().into_iter().map(|e| e.message).collect()
}

#[test]
fn a_frequency_resolves_to_the_knob_position_the_module_wants() {
    let v = param(&song("    cutoff 800hz\n"), "acid", "cutoff");
    // The module turns the knob back into Hz with the same curve.
    let back = params::BASS_CUTOFF.to_real(v);
    assert!((back - 800.0).abs() < 1.0, "800hz came back as {} Hz (knob {})", back, v);

    let khz = param(&song("    cutoff 2khz\n"), "acid", "cutoff");
    assert!((params::BASS_CUTOFF.to_real(khz) - 2000.0).abs() < 3.0);
}

#[test]
fn envelope_times_read_in_milliseconds_and_seconds() {
    let a = param(&song("    attack 20ms\n"), "acid", "attack");
    assert!((params::ENV_TIME.to_real(a) - 20.0).abs() < 0.1, "got {} ms", params::ENV_TIME.to_real(a));
    let r = param(&song("    release 1.5s\n"), "acid", "release");
    assert!((params::ENV_TIME.to_real(r) - 1500.0).abs() < 5.0, "got {} ms", params::ENV_TIME.to_real(r));
}

#[test]
fn semitones_and_percentages() {
    let p = param(&song("    osc2_pitch -12st\n"), "acid", "osc2_pitch");
    assert!((params::OSC_PITCH.to_real(p) + 12.0).abs() < 0.2, "got {} st", params::OSC_PITCH.to_real(p));
    assert!((param(&song("    resonance 80%\n"), "acid", "resonance") - 0.8).abs() < 1e-5);
}

#[test]
fn the_wrong_unit_is_an_error_that_says_what_the_parameter_takes() {
    let e = errors(&song("    cutoff 20ms\n"));
    assert!(e[0].contains("'cutoff' has no unit 'ms'"), "{:?}", e);
    assert!(e[0].contains("hz (20hz..20khz)"), "{:?}", e);

    let e = errors(&song("    resonance 400hz\n"));
    assert!(e[0].contains("'resonance' has no unit 'hz'"), "{:?}", e);
    assert!(e[0].contains("80%"), "{:?}", e);
}

#[test]
fn a_frequency_outside_the_filter_range_is_an_error_in_its_own_unit() {
    let e = errors(&song("    cutoff 50khz\n"));
    assert!(e[0].contains("'cutoff' = 50khz is outside 20hz..20khz"), "{:?}", e);
}

#[test]
fn effect_arguments_take_units_too() {
    let src = song("    cutoff 0.4\n")
        .replace("out > master", "out > lowpass(2khz) > compressor(-8, makeup=6db) > master");
    let ast = dsl::parse(&src).unwrap_or_else(|e| panic!("{:?}", e));
    let track = &ast.tracks[0];
    let lowpass = track.routing.iter().find(|r| r.kind == "lowpass").expect("lowpass");
    match lowpass.params[0] {
        dsl::ast::Param::Float(v) => assert!((v - 2000.0).abs() < 1e-3, "2khz became {}", v),
        ref other => panic!("{:?}", other),
    }
    let comp = track.routing.iter().find(|r| r.kind == "compressor").expect("compressor");
    let makeup = comp.params.iter().find_map(|p| match p {
        dsl::ast::Param::Named(n, v) if n == "makeup" => Some(*v),
        _ => None,
    }).expect("makeup");
    // This is the whole point: 6 dB is a gain of 2, not of 6.
    assert!((makeup - 2.0).abs() < 0.01, "6db became {}", makeup);
}

#[test]
fn a_unit_on_an_argument_that_has_no_such_unit_is_an_error() {
    let src = song("    cutoff 0.4\n").replace("out > master", "out > lowpass(2ms) > master");
    let e = errors(&src);
    assert!(e[0].contains("'cutoff' takes hz or khz"), "{:?}", e);

    let src = song("    cutoff 0.4\n").replace("out > master", "out > saturate(3db) > master");
    let e = errors(&src);
    assert!(e[0].contains("takes no unit") || e[0].contains("not 'db'"), "{:?}", e);
}

#[test]
fn a_unit_where_no_parameter_gives_it_meaning_is_an_error() {
    // `tempo 120hz` is nonsense, and used to be accepted as 120.
    let src = song("    cutoff 0.4\n").replace("tempo 120", "tempo 120hz");
    let e = errors(&src);
    assert!(e[0].contains("has a unit"), "{:?}", e);
}

#[test]
fn plain_numbers_still_work_everywhere() {
    assert!(dsl::parse(&song("    cutoff 0.4\n    resonance 0.8\n    attack 0.2\n")).is_ok());
    let _ = ModuleKind::Bass;
}

/// A minus sign used to drop out of the unit path: `makeup=6db` worked and
/// `makeup=-6db` answered "this takes a plain number; units work on effect
/// arguments", which is what it was.
#[test]
fn a_negative_quantity_keeps_its_unit() {
    let src = song("    cutoff 0.4\n")
        .replace("out > master", "out > compressor(-8, makeup=-6db) > eq(mid=-3db) > master");
    let ast = dsl::parse(&src).unwrap_or_else(|e| panic!("{:?}", e));
    let track = &ast.tracks[0];
    let comp = track.routing.iter().find(|r| r.kind == "compressor").expect("compressor");
    let makeup = comp.params.iter().find_map(|p| match p {
        dsl::ast::Param::Named(n, v) if n == "makeup" => Some(*v),
        _ => None,
    }).expect("makeup");
    // -6 dB is a gain of one half.
    assert!((makeup - 0.5).abs() < 0.01, "-6db became {}", makeup);

    let eq = track.routing.iter().find(|r| r.kind == "eq").expect("eq");
    let mid = eq.params.iter().find_map(|p| match p {
        dsl::ast::Param::Named(n, v) if n == "mid" => Some(*v),
        _ => None,
    }).expect("mid");
    assert!((mid + 3.0).abs() < 0.01, "an eq band is already in dB, so -3db is -3: got {}", mid);
}
