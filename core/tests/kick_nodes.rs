//! The nodes a modern kick is built with: a two-stage pitch sweep, a bell
//! and shelves at any frequency, an antialiased clipper and a transient
//! shaper. Each is checked where it is heard: in a song, through the
//! compiler, and against the same song without it.

use tatum_core::song_engine::SongEngine;

/// Eight beats of `instrument` (its graph lines) on a four-to-the-floor
/// pattern, through `chain`.
fn kick(graph: &str, chain: &str) -> String {
    format!(
        r#"
tempo 120
scale A minor
instrument boom {{
{graph}
}}
pattern four {{ C3 .. .. -  C3 .. .. -  C3 .. .. -  C3 .. .. - }}
track kick {{ play four using boom level 0.8 out > {chain}master }}
"#
    )
}

fn render(src: &str, bars: u32) -> Vec<f32> {
    let mut e = SongEngine::from_source(src).unwrap_or_else(|err| panic!("{}", err));
    let (l, _) = e.render(bars);
    l
}

fn errors(src: &str) -> String {
    match SongEngine::from_source(src) {
        Ok(_) => String::new(),
        Err(e) => e.to_string(),
    }
}

/// Frequency of the body between two times after the first hit (from where
/// the voice starts to sound), from its upward zero crossings.
fn pitch(x: &[f32], from_ms: f32, to_ms: f32) -> f32 {
    let t0 = x.iter().position(|v| v.abs() > 1e-4).expect("it sounds");
    let (a, b) = (t0 + (from_ms * 44.1) as usize, t0 + (to_ms * 44.1) as usize);
    let ups: Vec<f32> = (a + 1..b)
        .filter(|&i| x[i - 1] < 0.0 && x[i] >= 0.0)
        .map(|i| (i - 1) as f32 + x[i - 1] / (x[i - 1] - x[i]))
        .collect();
    assert!(ups.len() >= 2, "no cycles between {from_ms} and {to_ms} ms");
    44100.0 * (ups.len() - 1) as f32 / (ups[ups.len() - 1] - ups[0])
}

const BODY: &str = "  body > perc(0.001, 0.3) as amp\n  amp > out";

#[test]
fn one_stage_is_the_sweep_it_always_was() {
    let plain = render(&kick(&format!("  pitch_osc sine(300, 50, 0.999) as body\n{BODY}"), ""), 1);
    let unset = render(&kick(&format!("  pitch_osc sine(300, 50, 0.999, mid=300) as body\n{BODY}"), ""), 1);
    assert_eq!(plain, unset, "a mid at the start is one stage");
}

#[test]
fn two_stages_snap_to_mid_and_then_fall_to_the_end() {
    // A body that holds, so the sweep can be read to its end.
    let held = "  body > adsr(0.001, 0.1, 1.0, 0.01) as amp\n  amp > out";
    let one = render(&kick(&format!("  pitch_osc sine(1000, 50, 0.9998) as body\n{held}"), ""), 1);
    let two = render(&kick(&format!("  pitch_osc sine(1000, 50, 0.9998, mid=150, fast=0.98) as body\n{held}"), ""), 1);
    // 10 to 25 ms in, one stage is still high in its slow fall; two stages
    // have snapped down to mid and fall slowly from there.
    let (p1, p2) = (pitch(&one, 10.0, 25.0), pitch(&two, 10.0, 25.0));
    assert!(p1 > 600.0, "one stage at 10-25 ms: {p1}");
    assert!((110.0..160.0).contains(&p2), "two stages at 10-25 ms: {p2}");
    // And land on the end note: 50 Hz plus what is left of the slow stage,
    // 100 Hz * 0.9998^(44100 * 0.33) = 5.4 Hz.
    let end = pitch(&two, 300.0, 360.0);
    assert!((end - 55.4).abs() < 2.0, "{end}");
}

#[test]
fn a_mid_under_the_end_is_an_error() {
    let e = errors(&kick(&format!("  pitch_osc sine(300, 50, 0.999, mid=40) as body\n{BODY}"), ""));
    assert!(e.contains("mid=40"), "{e}");
}

/// A sweep that rises, written without a mid, is one stage as it always
/// was; a mid written over the start would have the fast stage run the
/// wrong way, and is an error rather than left out.
#[test]
fn a_rising_sweep_compiles_and_a_mid_over_the_start_does_not() {
    assert_eq!(errors(&kick(&format!("  pitch_osc sine(50, 300, 0.999) as body\n{BODY}"), "")), "");
    let e = errors(&kick(&format!("  pitch_osc sine(300, 50, 0.999, mid=400) as body\n{BODY}"), ""));
    assert!(e.contains("mid=400 is over the start frequency 300"), "{e}");
}

#[test]
fn eq_bands_take_units_and_check_their_ranges() {
    let graph = format!("  pitch_osc sine(300, 50, 0.999) as body\n{BODY}");
    assert_eq!(errors(&kick(&graph, "bell(300hz, -4db, 1.5) > lowshelf(60hz, 2db) > highshelf(6khz, -2db) > ")), "");
    let e = errors(&kick(&graph, "bell(300hz, -40db) > "));
    assert!(e.contains("gain"), "{e}");
    let e = errors(&kick(&graph, "bell(300ms, -4db) > "));
    assert!(e.contains("hz"), "{e}");
}

#[test]
fn a_bell_cut_takes_its_band_down() {
    let graph = "  osc sine as body\n  body > adsr(0.001, 0.1, 1.0, 0.01) as amp\n  amp > out";
    // A held C3 (131 Hz) with a bell cut right on it, and one far away.
    let on = render(&kick(graph, "bell(131hz, -12db, 2) > "), 1);
    let off = render(&kick(graph, "bell(3khz, -12db, 2) > "), 1);
    let peak = |x: &[f32]| x[4410..13230].iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let ratio = peak(&on) / peak(&off);
    assert!((0.2..0.3).contains(&ratio), "{ratio}");
}

#[test]
fn clip_holds_its_ceiling_and_makes_the_kick_denser() {
    let graph = format!("  pitch_osc sine(300, 50, 0.999) as body\n{BODY}");
    let dry = render(&kick(&graph, "gain(2) > "), 2);
    let clipped = render(&kick(&graph, "gain(2) > clip(1, ceiling=-6db) > "), 2);
    let peak = |x: &[f32]| x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
    // The track's level (0.8) is after its chain.
    assert!(peak(&clipped) <= 0.8 * 0.502, "{}", peak(&clipped));
    let crest = |x: &[f32]| peak(x) / rms(x);
    assert!(crest(&clipped) < crest(&dry) * 0.8, "{} against {}", crest(&clipped), crest(&dry));
}

#[test]
fn transient_shortens_a_tail_without_touching_the_hit() {
    let graph = format!("  pitch_osc sine(300, 50, 0.999) as body\n{BODY}");
    let dry = render(&kick(&graph, ""), 1);
    let short = render(&kick(&graph, "transient(0db, -12db) > "), 1);
    let peak = |x: &[f32], a: f32, b: f32| {
        x[(a * 44.1) as usize..(b * 44.1) as usize].iter().fold(0.0f32, |m, v| m.max(v.abs()))
    };
    assert!(peak(&short, 0.0, 15.0) > 0.85 * peak(&dry, 0.0, 15.0), "the hit");
    assert!(peak(&short, 250.0, 350.0) < 0.5 * peak(&dry, 250.0, 350.0), "the tail");
}

#[test]
fn a_knob_and_a_sweep_reach_a_clips_drive() {
    let src = r#"
tempo 120
scale A minor
module beats kit { }
pattern four { kick: X - - - X - - - X - - - X - - - }
track drums { play four using kit out > clip(1.5) as dense > master }
master { in > clip(1.2) > out }
scene a { track drums { play four using kit } auto master drive 1.0 > 3.0 }
arrange { a x2 }
midi { cc 20 > drums dense drive }
"#;
    assert_eq!(errors(src), "");
}
