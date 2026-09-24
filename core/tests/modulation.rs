//! Movement: filter LFOs on insert chains, bar-synced module LFOs, autopan.

use tatum_core::dsl::{self, compiler};
use tatum_core::song_engine::SongEngine;

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
    let spec = tatum_core::params::lookup(tatum_core::params::ModuleKind::Keys, "lfo_sync").unwrap();
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

/// Energy above 5 kHz relative to the total, in dB. A filter LFO on a dark
/// sound must not add broadband grit.
fn hf_ratio_db(x: &[f32]) -> f32 {
    let c = (-2.0 * std::f32::consts::PI * 5000.0 / tatum_core::SAMPLE_RATE).exp();
    let (mut lp, mut hi, mut tot) = (0.0f32, 0.0f64, 0.0f64);
    for &v in x {
        lp = v * (1.0 - c) + lp * c;
        let h = v - lp;
        hi += (h * h) as f64;
        tot += (v * v) as f64;
    }
    20.0 * ((hi / tot.max(1e-12)).sqrt() as f32).log10()
}

#[test]
fn cutoff_lfo_is_relative_and_does_not_rasp() {
    use tatum_core::Module;
    use tatum_core::modules::bass::{BassModule, BassParam};

    let render = |depth: f32| {
        let mut m = BassModule::new();
        m.set_param(BassParam::Cutoff, 0.15);      // ~56 Hz: a dark drone
        m.set_param(BassParam::Resonance, 0.78);
        m.set_param(BassParam::CutoffEnv, 0.0);
        m.set_param(BassParam::Osc1Wave, 1.0);
        if depth > 0.0 {
            m.set_param(BassParam::LfoTarget, 0.0); // cutoff
            m.set_param(BassParam::LfoDepth, depth);
            m.set_param(BassParam::LfoSync, 1.0);   // fast enough to sweep within the render
        }
        m.note_on(28, 0.8);
        let mut out = vec![0.0f32; 44100 * 3];
        let mut buf = [0.0f32; 128];
        for ch in out.chunks_mut(128) {
            let n = ch.len();
            m.process_block(&mut buf[..n]);
            ch.copy_from_slice(&buf[..n]);
        }
        out
    };

    let dry = hf_ratio_db(&render(0.0)[22050..]);
    let wet = hf_ratio_db(&render(0.2)[22050..]);
    let deep = hf_ratio_db(&render(0.5)[22050..]);
    // An absolute Hz offset used to slam the 20 Hz floor and then sweep the
    // whole spectrum, adding >10 dB of hiss. Relative modulation stays close.
    assert!(wet - dry < 5.0, "cutoff LFO must not add grit: dry {:.1} dB, wet {:.1} dB", dry, wet);
    assert!(deep - dry < 6.0, "even a deep sweep stays clean: dry {:.1} dB, deep {:.1} dB", dry, deep);
    assert!(wet > dry, "the LFO still opens the filter: dry {:.1}, wet {:.1}", dry, wet);
}

/// An `auto` sweep used to be evaluated once per step, so a filter moved in
/// sixteenth-note stairs -- eight jumps a second, which on a resonant filter is
/// heard as stepping rather than as a sweep. It runs once per block now.
#[test]
fn an_auto_sweep_moves_smoothly_and_not_in_steps() {
    use tatum_core::song_engine::SongEngine;
    let src = "tempo 120\nscale C major\n\
        module keys v { voice_mode poly cutoff 300hz resonance 45% attack 5ms sustain 1.0 }\n\
        pattern p { 1.4:0.9 ..*63 }\n\
        track t { play p using v out > master }\n\
        master { in > out }\n\
        scene a { auto v cutoff 0.15 > 0.85  track t { play p using v } }\n\
        arrange { a x4 }\n";
    let mut engine = SongEngine::from_source(src).unwrap();
    engine.start();
    let (l, _) = engine.render(4);

    // The sweep raises the brightness monotonically. Sample the spectral
    // centroid in short windows: with per-step automation it climbs in a
    // staircase, so consecutive windows inside one step are identical.
    let sr = tatum_core::SAMPLE_RATE as usize;
    let win = 1024;
    let centroid = |x: &[f32]| -> f32 {
        // Zero-crossing rate stands in for brightness and needs no FFT.
        let n = x.windows(2).filter(|w| (w[0] <= 0.0) != (w[1] <= 0.0)).count();
        n as f32 / x.len() as f32
    };
    let step = sr * 60 / 120 / 4; // one sixteenth
    // Two windows inside the same step, away from note events.
    let a = sr + step / 4;
    let b = sr + step * 3 / 4;
    let (ca, cb) = (centroid(&l[a..a + win]), centroid(&l[b..b + win]));
    assert!(
        (ca - cb).abs() > 1e-6,
        "brightness is identical at two points inside one step ({} vs {}), so the sweep is still quantised to steps",
        ca, cb
    );
}
