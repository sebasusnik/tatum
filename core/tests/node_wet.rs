//! `wet=` on any chain node: the universal bypass.
//!
//! Changing an insert chain is structural -- the chain has to be rebuilt, which
//! means a swap, which means the voices restart. That made "switch the autowah
//! out for eight bars" impossible to do live, and it is the most ordinary
//! gesture there is. `wet=` is a fader on a node that already exists, so it
//! applies inside the bar like a level does.
//!
//! At 0 the node is not processed at all rather than processed and thrown away,
//! which is what makes it affordable to leave a full chain on every voice of a
//! rig and switch the parts in as you go.

use tatum_core::live::{FastOp, LivePlanner, LivePlayer, Plan};
use tatum_core::song_engine::SongEngine;

const SONG: &str = r#"
tempo 120
scale C minor
humanize 0
sidechain 0
module bass low { cutoff 0.75 sustain 0.9 }
pattern line { C2:0.9 - - -  C2:0.9 - - -  C2:0.9 - - -  C2:0.9 - - - }
track bass { play line using low level 0.5 out > lowpass(300, 0.1, wet=1.0) > master }
master { in > out }
"#;

fn render(src: &str, bars: u32) -> Vec<f32> {
    let mut e = SongEngine::from_source(src).unwrap_or_else(|err| panic!("{}", err));
    e.start();
    e.render(bars).0
}

/// Brightness, as the RMS of the sample-to-sample difference against the RMS
/// of the signal, in dB. Differencing is a high-pass with no coefficient to
/// get wrong, and it moves monotonically with how much top is left, which is
/// exactly what a lowpass takes away.
fn brightness_db(x: &[f32]) -> f32 {
    let energy = |v: &[f32]| v.iter().map(|s| (s * s) as f64).sum::<f64>() / v.len() as f64;
    let diff: Vec<f32> = x.windows(2).map(|w| w[1] - w[0]).collect();
    10.0 * (energy(&diff) / energy(x).max(1e-20)).max(1e-20).log10() as f32
}

#[test]
fn wet_zero_bypasses_the_node_and_wet_one_does_not() {
    let on = render(SONG, 2);
    let off = render(&SONG.replace("wet=1.0", "wet=0.0"), 2);
    let (a, b) = (brightness_db(&on), brightness_db(&off));
    assert!(b > a + 5.0, "bypassed should keep its top: filtered {a:.1} dB vs bypassed {b:.1} dB");
}

/// Not a switch: the values between have to blend, or an agent switching an
/// effect in has only one, very abrupt, gesture available.
#[test]
fn wet_between_zero_and_one_blends() {
    let e: Vec<f32> = ["0.0", "0.5", "1.0"]
        .iter()
        .map(|w| brightness_db(&render(&SONG.replace("wet=1.0", &format!("wet={w}")), 2)))
        .collect();
    assert!(e[0] > e[1] && e[1] > e[2], "wet should move monotonically, got {e:?}");
}

/// A node with no `wet=` is fully wet, so every existing song is unchanged.
#[test]
fn a_node_without_wet_is_unchanged() {
    let explicit = render(SONG, 2);
    let implicit = render(&SONG.replace(", wet=1.0", ""), 2);
    assert_eq!(explicit, implicit, "writing wet=1 must be the same as not writing it");
}

/// The reason the feature exists: this edit must not rebuild the chain.
#[test]
fn changing_only_wet_is_a_fast_op_not_a_swap() {
    let mut planner = LivePlanner::new();
    let mut player = LivePlayer::new();
    player.apply(planner.plan(SONG, player.generation()).unwrap());
    player.start();

    let edited = SONG.replace("wet=1.0", "wet=0.25");
    match planner.plan(&edited, player.generation()).unwrap() {
        Plan::Fast { ops, .. } => assert!(
            matches!(ops[..], [FastOp::NodeWet { track: 0, node: 0, wet }] if (wet - 0.25).abs() < 1e-6),
            "{ops:?}"
        ),
        other => panic!("expected a fast plan, got {}", other.describe()),
    }
}

/// And everything else about a chain stays structural: `wet` is the exception,
/// not a crack in the rule.
#[test]
fn any_other_chain_edit_is_still_a_swap() {
    let mut planner = LivePlanner::new();
    let mut player = LivePlayer::new();
    player.apply(planner.plan(SONG, player.generation()).unwrap());
    player.start();

    for edit in ["lowpass(500, 0.1", "highpass(300, 0.1"] {
        let edited = SONG.replace("lowpass(300, 0.1", edit);
        match planner.plan(&edited, player.generation()).unwrap() {
            Plan::Swap { .. } => {}
            other => panic!("`{edit}` should need a swap, got {}", other.describe()),
        }
    }
}

/// Adding a node is structural even if the new one is bypassed: the chain
/// really is different, and pretending otherwise would silently drop it.
#[test]
fn adding_a_bypassed_node_is_still_a_swap() {
    let mut planner = LivePlanner::new();
    let mut player = LivePlayer::new();
    player.apply(planner.plan(SONG, player.generation()).unwrap());
    player.start();

    let edited = SONG.replace("> master", "> chorus(0.3, wet=0.0) > master");
    match planner.plan(&edited, player.generation()).unwrap() {
        Plan::Swap { .. } => {}
        other => panic!("expected a swap, got {}", other.describe()),
    }
}

/// Switching a delay out and back in must not spit what it was holding. The
/// chain clears the node on the way into bypass.
#[test]
fn a_bypassed_node_does_not_come_back_stale() {
    let src = r#"
tempo 120
scale C minor
humanize 0
sidechain 0
module bass low { cutoff 0.75 sustain 0.9 }
pattern line { C2:0.9 - - -  - - - -  - - - -  - - - - }
track bass { play line using low level 0.5 out > delay(1/8, 0.7) > master }
master { in > out }
"#;
    let mut e = SongEngine::from_source(src).unwrap_or_else(|err| panic!("{}", err));
    e.start();
    let _ = e.render(1); // the delay fills up
    assert!(e.set_node_wet(0, 0, 0.0), "the delay should be addressable");
    let _ = e.render(2); // two bars bypassed: the note is long gone
    assert!(e.set_node_wet(0, 0, 1.0));
    // Bring it back one step before the next note and listen to the gap.
    let sr = tatum_core::SAMPLE_RATE as usize;
    let mut l = vec![0.0f32; sr / 8];
    let mut r = vec![0.0f32; sr / 8];
    e.process_block_stereo(&mut l, &mut r);
    // The first 10 ms are the bass's own release finishing, dry: the test
    // listens to the gap after it, where only a stale echo could be.
    let peak = l[sr / 100..].iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(peak < 0.01, "an old echo came back out of the bypass: peak {peak:.4}");
}

// ── Naming nodes ─────────────────────────────────────────────────────────────
//
// `wet=` alone is addressed by position, which is fine for a text edit the
// planner diffs but useless for an `auto` sweep and fragile for anything that
// has to point at a node from outside. `as <name>` gives the node an identity
// that survives someone inserting another node earlier in the chain.

use tatum_core::dsl::{self, compiler};

fn compile_err(src: &str) -> String {
    let ast = dsl::parse(src).expect("parses");
    match compiler::compile(&ast) {
        Ok(_) => panic!("expected a compile error"),
        Err(e) => e[0].message.clone(),
    }
}

const NAMED: &str = r#"
tempo 120
scale C minor
humanize 0
sidechain 0
module bass low { cutoff 0.75 sustain 0.9 }
pattern line { C2:0.9 - - -  C2:0.9 - - -  C2:0.9 - - -  C2:0.9 - - - }
track bass { play line using low level 0.5 out > lowpass(300, 0.1) as lp > master }
master { in > out }
scene a {
    auto bass.lp wet 0.0 > 1.0
    track bass { play line using low level 0.5 }
}
arrange { a x4 }
"#;

/// The gesture this is all for: sweep an effect in over a scene rather than
/// switching it. Brightness has to fall as the lowpass comes up.
#[test]
fn auto_can_sweep_a_named_nodes_wet_across_a_scene() {
    let l = render(NAMED, 4);
    let sr = tatum_core::SAMPLE_RATE as usize;
    let bar = sr * 2;
    let bars: Vec<f32> = (0..4).map(|i| brightness_db(&l[i * bar..(i + 1) * bar])).collect();
    assert!(bars.windows(2).all(|w| w[1] < w[0]), "the lowpass should fade in across the scene, got {bars:?}");
    assert!(bars[0] - bars[3] > 4.0, "and by an audible amount, got {bars:?}");
}

/// A misspelled node name has to be an error. Silently doing nothing is the
/// worst outcome for a live set, where nobody is reading the log.
#[test]
fn automating_a_node_that_does_not_exist_is_an_error() {
    let msg = compile_err(&NAMED.replace("bass.lp wet", "bass.lpp wet"));
    assert!(msg.contains("no node named 'lpp'"), "{msg}");
    assert!(msg.contains("it has lp"), "the error should list what is there: {msg}");
}

/// And so is naming two nodes the same thing, because then the name does not
/// pick out a node at all.
#[test]
fn two_nodes_with_one_name_is_an_error() {
    let src = NAMED.replace(
        "out > lowpass(300, 0.1) as lp > master",
        "out > lowpass(300, 0.1) as lp > highpass(80, 0.1) as lp > master",
    );
    let msg = compile_err(&src);
    assert!(msg.contains("both named 'lp'"), "{msg}");
}

/// Naming a node changes nothing about how it sounds.
#[test]
fn a_name_is_not_audible() {
    let plain = r#"
tempo 120
scale C minor
humanize 0
sidechain 0
module bass low { cutoff 0.75 sustain 0.9 }
pattern line { C2:0.9 - - -  C2:0.9 - - -  C2:0.9 - - -  C2:0.9 - - - }
track bass { play line using low level 0.5 out > lowpass(300, 0.1) > master }
master { in > out }
"#;
    let named = plain.replace("lowpass(300, 0.1) >", "lowpass(300, 0.1) as lp >");
    assert_eq!(render(plain, 2), render(&named, 2));
}
