//! 6.6: the `capture` node. A window of a track's own output, looped back
//! stretched or reversed. This is the one thing in the sound-design box that
//! no filter, delay or reverb can stand in for.

use synth_core::dsl::{self, compiler};
use synth_core::song_engine::SongEngine;

fn song(chain: &str) -> String {
    format!(r#"
tempo 120
scale C major

module bass low {{ cutoff 600hz sustain 0.9 }}

pattern line {{ 1.1 - 1.3 -  1.5 - 1.3 - }}
pattern quiet {{ - - - -  - - - -  - - - -  - - - - }}

track bass {{ play line using low out > {} master }}
master {{ in > limiter > out }}

scene main {{ track bass {{ play line using low }} }}
scene tail {{ track bass {{ play quiet using low }} }}
arrange {{ main x1 tail x1 }}
"#, chain)
}

/// Peak per bar of the whole render.
fn bars(source: &str, count: u32) -> Vec<f32> {
    let mut engine = SongEngine::from_source(source).unwrap_or_else(|e| panic!("{}", e));
    engine.start();
    let (l, r) = engine.render(count);
    let per_bar = l.len() / count as usize;
    (0..count as usize).map(|b| {
        let a = b * per_bar;
        l[a..a + per_bar].iter().zip(&r[a..a + per_bar])
            .fold(0.0f32, |m, (x, y)| m.max(x.abs()).max(y.abs()))
    }).collect()
}

#[test]
fn a_captured_bar_keeps_playing_after_the_track_goes_quiet() {
    // Bar 2 has nothing but rests, so anything audible there is the capture.
    let silent = bars(&song(""), 2);
    let looped = bars(&song("capture(1) >"), 2);

    assert!(silent[1] < silent[0] * 0.2, "the control should fall quiet in bar 2: {:?}", silent);
    assert!(
        looped[1] > silent[0] * 0.2,
        "the captured bar should still be playing in bar 2: control {:?}, capture {:?}",
        silent, looped
    );
}

#[test]
fn mix_zero_leaves_the_signal_alone() {
    let plain = bars(&song(""), 2);
    let muted = bars(&song("capture(1, mix=0) >"), 2);
    for b in 0..2 {
        assert!((plain[b] - muted[b]).abs() < 1e-4, "bar {}: {:?} vs {:?}", b, plain, muted);
    }
}

#[test]
fn the_window_is_sized_from_the_song_tempo() {
    let src = song("capture(2) >");
    let ast = dsl::parse(&src).unwrap();
    let compiled = compiler::compile(&ast).unwrap_or_else(|e| panic!("{:?}", e));
    let spec = compiled.tracks[0].insert_fx.iter().find_map(|s| match s.spec {
        synth_core::graph::node::NodeSpec::Capture { samples, .. } => Some(samples),
        _ => None,
    }).expect("a capture node");
    // Two bars of 4/4 at 120 BPM is four seconds.
    let expected = (synth_core::SAMPLE_RATE * 4.0) as u32;
    assert!(
        (spec as i64 - expected as i64).abs() < 100,
        "expected about {} samples, got {}", expected, spec
    );
}

#[test]
fn its_arguments_are_validated_like_every_other_node() {
    let src = song("capture(1, speeed=0.5) >");
    let errs = match compiler::compile(&dsl::parse(&src).unwrap()) {
        Ok(_) => panic!("should not compile"),
        Err(e) => e,
    };
    assert!(errs[0].message.contains("unknown option 'speeed'"), "{:?}", errs[0]);
    assert!(errs[0].message.contains("start, speed, reverse, mix"), "{:?}", errs[0]);

    let src = song("capture(99) >");
    let errs = match compiler::compile(&dsl::parse(&src).unwrap()) {
        Ok(_) => panic!("should not compile"),
        Err(e) => e,
    };
    assert!(errs[0].message.contains("bars = 99 is out of range"), "{:?}", errs[0]);
}
