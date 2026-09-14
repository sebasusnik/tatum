//! Movement: filter LFOs on insert chains, bar-synced module LFOs, autopan.

use synth_core::dsl::{self, compiler};
use synth_core::song_engine::SongEngine;

const BASE: &str = r#"
tempo 120
scale A minor
module fm tone { algorithm two_op mod_index 0.4 attack 0.0 decay 1.0 sustain 1.0 release 0.5 }
pattern hold { 1.3 .. .. .. .. .. .. .. .. .. .. .. .. .. .. .. }
track t { play hold using tone level 0.6 out > CHAIN > master }
scene a { track t { play hold using tone } }
arrange { a x2 }
"#;

fn render(chain: &str) -> (Vec<f32>, Vec<f32>) {
    let src = BASE.replace("CHAIN", chain);
    let mut e = SongEngine::from_source(&src).expect("load");
    e.render(2)
}

fn rms(x: &[f32]) -> f32 { (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt() }

#[test]
fn filter_lfo_moves_the_cutoff() {
    let (static_l, _) = render("lowpass(600, 0.7)");
    let (moving_l, _) = render("lowpass(600, 0.7, lfo_bars=1, lfo_depth=2000)");
    assert_eq!(static_l.len(), moving_l.len());
    let diff: f32 = static_l.iter().zip(&moving_l).map(|(a, b)| (a - b).abs()).sum::<f32>() / static_l.len() as f32;
    assert!(diff > 1e-3, "an LFO on the cutoff must change the output, avg diff = {}", diff);
    // Half a bar in, the sweep is near its top: brighter (more energy) than at the start
    let n = static_l.len();
    let bar = n / 2;
    let early = rms(&moving_l[bar / 8..bar / 4]);
    let peak = rms(&moving_l[bar * 3 / 8..bar / 2]);
    assert!(peak != early, "energy should vary along the LFO cycle");
}

#[test]
fn filter_lfo_arguments_are_validated() {
    let src = BASE.replace("CHAIN", "lowpass(600, 0.7, lfo_bar=1)");
    let errs = match compiler::compile(&dsl::parse(&src).unwrap()) { Err(e) => e, Ok(_) => panic!("expected error") };
    assert!(errs[0].message.contains("unknown option 'lfo_bar'"), "{}", errs[0].message);
}

#[test]
fn autopan_creates_stereo_movement() {
    let (l0, r0) = render("gain(1.0)");
    let (l, r) = render("autopan(1.0, bars=1)");
    let width0: f32 = l0.iter().zip(&r0).map(|(a, b)| (a - b).abs()).sum::<f32>() / l0.len() as f32;
    let width: f32 = l.iter().zip(&r).map(|(a, b)| (a - b).abs()).sum::<f32>() / l.len() as f32;
    assert!(width0 < 1e-6, "a mono source through gain is centered");
    assert!(width > 1e-3, "autopan must separate L and R, avg |L-R| = {}", width);
    // Over one bar the balance flips: quarter-bar windows favour opposite sides
    let n = l.len();
    let q = n / 8; // bar = n/2, quarter bar = n/8
    let left_first = rms(&l[0..q]) - rms(&r[0..q]);
    let right_later = rms(&l[q * 2..q * 3]) - rms(&r[q * 2..q * 3]);
    assert!(left_first * right_later < 0.0, "balance should swing: {} vs {}", left_first, right_later);
}

#[test]
fn module_lfo_can_sync_to_bars() {
    let src = "tempo 120\nscale A minor\nmodule keys pad { lfo_target cutoff lfo_depth 0.2 lfo_sync bars_2 }\npattern p { 1.3 .. .. .. }\ntrack t { play p using pad out > master }\nscene a { track t { play p using pad } }\narrange { a x1 }\n";
    let ast = dsl::parse(src).unwrap();
    let pad = ast.module_defs.iter().find(|m| m.name == "pad").unwrap();
    // Don't hardcode the encoding: adding an option to the table shifts it.
    // core/tests/mix.rs asserts the whole table round-trips.
    let sync = pad.params.iter().find(|p| p.name == "lfo_sync").unwrap();
    let spec = synth_core::params::lookup(synth_core::params::ModuleKind::Keys, "lfo_sync").unwrap();
    assert_eq!(spec.choice_name(sync.value), Some("bars_2"), "encoded value must decode back to bars_2");
    assert!(compiler::compile(&ast).is_ok());
    let mut e = SongEngine::from_source(src).unwrap();
    let (l, _) = e.render(1);
    assert!(rms(&l) > 0.001);
}

#[test]
fn phaser_sweeps_and_widens() {
    let (dry_l, _) = render("gain(1.0)");
    let (l, r) = render("phaser(0.5, bars=1, stages=6, feedback=0.5)");
    let diff: f32 = dry_l.iter().zip(&l).map(|(a, b)| (a - b).abs()).sum::<f32>() / l.len() as f32;
    assert!(diff > 1e-3, "phaser must change the signal, avg diff = {}", diff);
    let width: f32 = l.iter().zip(&r).map(|(a, b)| (a - b).abs()).sum::<f32>() / l.len() as f32;
    assert!(width > 1e-4, "right channel runs a quarter cycle behind, avg |L-R| = {}", width);
    assert!(rms(&l) > 0.001 && rms(&l).is_finite());
}

#[test]
fn vowel_filter_shapes_and_morphs() {
    let (dry_l, _) = render("gain(1.0)");
    let (a_l, _) = render("vowel(a)");
    let (ao_l, _) = render("vowel(a, o, bars=1)");
    let d1: f32 = dry_l.iter().zip(&a_l).map(|(x, y)| (x - y).abs()).sum::<f32>() / a_l.len() as f32;
    let d2: f32 = a_l.iter().zip(&ao_l).map(|(x, y)| (x - y).abs()).sum::<f32>() / a_l.len() as f32;
    assert!(d1 > 1e-3, "a static vowel must filter the signal");
    assert!(d2 > 1e-4, "morphing a→o must differ from a static a");
    assert!(rms(&ao_l).is_finite() && rms(&ao_l) > 0.0005);

    let bad = BASE.replace("CHAIN", "vowel(q)");
    let errs = dsl::parse(&bad).unwrap_err();
    assert!(errs[0].message.contains("unexpected 'q' in arguments"), "{}", errs[0].message);
    let bad = BASE.replace("CHAIN", "vowel(x)");
    let errs = dsl::parse(&bad).unwrap_err();
    assert!(errs[0].message.contains("unexpected 'x' in arguments"), "{}", errs[0].message);
    // `mix` is a keyword elsewhere but a valid option here
    assert!(SongEngine::from_source(&BASE.replace("CHAIN", "vowel(a, o, bars=2, mix=0.5)")).is_ok());
    let bad = BASE.replace("CHAIN", "vowel(bars=2)");
    let errs = match compiler::compile(&dsl::parse(&bad).unwrap()) { Err(e) => e, Ok(_) => panic!() };
    assert!(errs[0].message.contains("needs one or two vowels"), "{}", errs[0].message);
}
