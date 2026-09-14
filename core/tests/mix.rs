//! Mix-facing behaviour: multi-line chains, per-track metering, FM level,
//! gain compensation, and the choice-name round trip that keeps `lfo_sync`
//! honest when options are added.

use synth_core::dsl::{self, compiler};
use synth_core::params::{self, ModuleKind};
use synth_core::song_engine::SongEngine;

const SONG: &str = r#"
tempo 120
scale A minor
GAINCOMP
module bass sub { cutoff 0.3 }
module fm tone { algorithm two_op decay 1.0 sustain 1.0 }
pattern p { 1.1 ..*15 }
pattern q { 1.4 ..*15 }
track low { play p using sub level 0.6
            out > highpass(120, 0.2)
                > saturate(0.3)
                > master }
track hi  { play q using tone level 0.6 out > master }
scene a { track low { play p using sub } track hi { play q using tone } }
arrange { a x1 }
"#;

fn song(extra: &str) -> String { SONG.replace("GAINCOMP", extra) }

#[test]
fn routing_chains_may_wrap_across_lines() {
    let ast = dsl::parse(&song("")).expect("parse");
    let low = ast.tracks.iter().find(|t| t.name == "low").unwrap();
    let kinds: Vec<&str> = low.routing.iter().map(|r| r.kind.as_str()).collect();
    assert_eq!(kinds, vec!["highpass", "saturate", "master"], "the wrapped chain must survive");

    // A chain that simply ends still lets the track body continue
    let two = song("").replace("track hi  { play q using tone level 0.6 out > master }",
        "track hi  {\n    play q\n    using tone\n    out > master\n    level 0.6\n}");
    let ast = dsl::parse(&two).expect("parse");
    let hi = ast.tracks.iter().find(|t| t.name == "hi").unwrap();
    assert_eq!(hi.level, Some(0.6), "level after the chain is still read");

    let bad = song("").replace("> saturate(0.3)", "> 0.3");
    let errs = dsl::parse(&bad).unwrap_err();
    assert!(errs[0].message.contains("expected an effect or destination after '>'"), "{}", errs[0].message);
}

#[test]
fn per_track_meters_report_levels() {
    let mut e = SongEngine::from_source(&song("")).expect("load");
    e.reset_meters();
    let _ = e.render(1);
    let low = (0..e.track_count()).find(|&i| e.track_name(i) == "low").unwrap();
    let hi = (0..e.track_count()).find(|&i| e.track_name(i) == "hi").unwrap();
    assert!(e.track_rms(low) > 0.0 && e.track_peak(low) > 0.0, "a playing track meters");
    assert!(e.track_rms(hi) > 0.0);
    e.reset_meters();
    assert_eq!(e.track_peak(low), 0.0, "reset clears the meters");
}

#[test]
fn fm_has_a_level_parameter() {
    let spec = params::lookup(ModuleKind::Fm, "level").expect("fm level exists");
    assert_eq!(spec.default, 1.0, "registry default matches FmModule::new, so existing songs are unchanged");

    let loud = song("").replace("module fm tone { algorithm two_op", "module fm tone { level 4.0 algorithm two_op");
    let quiet = song("").replace("module fm tone { algorithm two_op", "module fm tone { level 0.5 algorithm two_op");
    let rms = |src: &str| {
        let mut e = SongEngine::from_source(src).unwrap();
        e.reset_meters();
        let _ = e.render(1);
        let hi = (0..e.track_count()).find(|&i| e.track_name(i) == "hi").unwrap();
        e.track_rms(hi)
    };
    assert!(rms(&loud) > rms(&quiet) * 3.0, "level must scale FM output: {} vs {}", rms(&loud), rms(&quiet));
}

#[test]
fn gain_comp_can_be_turned_off() {
    let on = SongEngine::from_source(&song("gain_comp 1")).unwrap();
    let off = SongEngine::from_source(&song("gain_comp 0")).unwrap();
    let mut on = on; let mut off = off;
    let _ = on.render(1);
    let _ = off.render(1);
    assert!(on.gain_compensation() < 0.9, "two tracks are compensated down: {}", on.gain_compensation());
    assert!((off.gain_compensation() - 1.0).abs() < 1e-3, "gain_comp 0 disables it: {}", off.gain_compensation());
}

#[test]
fn every_choice_name_decodes_to_its_own_index() {
    // Adding an option to a choice table changes the float encoding; this keeps
    // the module decode tables in step with the registry.
    for kind in ModuleKind::ALL {
        for spec in params::specs(kind) {
            if let synth_core::params::Range::Choice(names) = spec.range {
                for (i, name) in names.iter().enumerate() {
                    let v = spec.value_from_name(name).expect("resolves");
                    assert_eq!(spec.choice_name(v), Some(*name), "{:?}.{} option {} ({})", kind, spec.name, i, name);
                }
            }
        }
    }
    let sync = params::lookup(ModuleKind::Keys, "lfo_sync").unwrap();
    assert!(sync.value_from_name("bars_12").is_some(), "bars_12 exists");
    let v = sync.value_from_name("bars_12").unwrap();
    assert_eq!((v * 11.0).round() as usize, 10, "modules decode with * 11.0");
}

#[test]
fn a_silent_or_buried_track_is_visible_in_the_meters() {
    let quiet = song("").replace("track hi  { play q using tone level 0.6 out > master }",
        "track hi  { play q using tone level 0.6 out > gain(0.001) > master }");
    let mut e = SongEngine::from_source(&quiet).unwrap();
    e.reset_meters();
    let _ = e.render(1);
    let low = (0..e.track_count()).find(|&i| e.track_name(i) == "low").unwrap();
    let hi = (0..e.track_count()).find(|&i| e.track_name(i) == "hi").unwrap();
    let rel = 20.0 * (e.track_rms(hi).max(1e-6) / e.track_rms(low).max(1e-6)).log10();
    assert!(rel < -30.0, "a track trimmed to nothing reads as buried: {} dB", rel);
}

#[test]
fn compile_still_accepts_every_example() {
    for entry in std::fs::read_dir("../examples").unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("synth") { continue; }
        let src = std::fs::read_to_string(&path).unwrap();
        let ast = dsl::parse(&src).unwrap_or_else(|e| panic!("{}: {:?}", path.display(), e));
        compiler::compile(&ast).unwrap_or_else(|e| panic!("{}: {:?}", path.display(), e.first().map(|x| x.message.clone())));
    }
}

#[test]
fn registry_defaults_match_what_a_fresh_module_does() {
    // The diff engine restores the registry default when a param line is
    // deleted, so a registry default that differs from the module's own
    // constructor would change the sound on an unrelated edit.
    use synth_core::Module;
    use synth_core::modules::fm::{FmModule, FmParam};
    let spec = params::lookup(ModuleKind::Fm, "level").unwrap();
    let render = |set_default: bool| {
        let mut m = FmModule::new();
        if set_default { m.set_param(FmParam::Level, spec.default); }
        m.note_on(60, 1.0);
        let mut buf = [0.0f32; 128];
        let mut energy = 0.0f32;
        for _ in 0..16 {
            m.process_block(&mut buf);
            energy += buf.iter().map(|v| v.abs()).sum::<f32>();
        }
        energy
    };
    let untouched = render(false);
    let with_default = render(true);
    assert!(untouched > 0.0);
    assert!((untouched - with_default).abs() / untouched < 0.01,
        "applying the registry default must be a no-op on a fresh module: {} vs {}", untouched, with_default);
}

/// 7.6 / 8.8: no example may hand its dynamics to the master chain. This is the
/// corpus revalidation pass — examples age with the bugs of their era, and
/// every one of the thirteen read `makeup` as dB when it is a linear gain.
#[test]
fn no_example_lets_the_master_chain_eat_its_transients() {
    use synth_core::analysis;
    use synth_core::song_engine::SongEngine;

    let mut offenders = Vec::new();
    for entry in std::fs::read_dir("../examples").unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("synth") { continue; }
        let src = std::fs::read_to_string(&path).unwrap();
        let mut engine = SongEngine::from_source(&src).unwrap();
        engine.start();
        // Eight bars is enough to catch a chain that is crushing everything.
        let (l, r) = engine.render(8);
        let (out_peak, out_rms) = analysis::peak_rms(&l, &r);
        let (in_peak, in_rms) = engine.master_input_peak_rms();
        let (ci, co) = (analysis::crest(in_peak, in_rms), analysis::crest(out_peak, out_rms));
        if ci <= 0.0 || co <= 0.0 { continue; }
        let change_db = 20.0 * (co / ci).log10();
        if change_db < -3.0 {
            offenders.push(format!("{}: {:.1} dB of crest lost ({:.1} -> {:.1})",
                path.file_stem().unwrap().to_string_lossy(), -change_db, ci, co));
        }
    }
    assert!(offenders.is_empty(), "the master chain is doing the mixing in:\n  {}", offenders.join("\n  "));
}
