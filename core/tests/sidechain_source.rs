//! 6.8b: `sidechain 0.5 from=<track>`. Until now everything ducked the kick,
//! which is the only thing house and techno need and the wrong answer for a
//! pad that should breathe with the bass.

use tatum_core::song_engine::SongEngine;
use tatum_core::dsl::{self, compiler};

/// A kick on beats 1 and 3, a bass on 2 and 4, and a sustained pad. The two
/// sources never hit together, so which one the pad ducks is visible in the
/// output: the pad dips where its source plays.
fn song(pad_sidechain: &str) -> String {
    format!(r#"
tempo 120
scale C major

module beats kit {{ kick_level 1.0 }}
module bass low {{ cutoff 0.4 sustain 0.9 }}
module keys pad {{ cutoff 0.6 sustain 1.0 attack 1ms }}

pattern beat {{ kick: X - - -  - - - -  X - - -  - - - - }}
pattern bassline {{ - - - -  1.1 .. .. ..  - - - -  1.1 .. .. .. }}
pattern hold {{ [1.3 3.3 5.3] .. .. ..  .. .. .. ..  .. .. .. ..  .. .. .. .. }}

track drums {{ play beat using kit out > master }}
track bass  {{ play bassline using low out > master }}
track pad   {{ play hold using pad {} out > master }}

master {{ in > limiter > out }}
scene a {{
    track drums {{ play beat using kit }}
    track bass  {{ play bassline using low }}
    track pad   {{ play hold using pad }}
}}
arrange {{ a x1 }}
"#, pad_sidechain)
}

/// Envelope of the pad track alone, in sixteen slots, one per step.
fn pad_envelope(source: &str) -> Vec<f32> {
    let src = song(source);
    let ast = dsl::parse(&src).unwrap_or_else(|e| panic!("{:?}", e));
    compiler::compile(&ast).unwrap_or_else(|e| panic!("{:?}", e));
    let mut engine = SongEngine::from_source(&src).unwrap();
    engine.start();
    // Mute everything but the pad so the measurement is the pad's own level.
    // After `start`, because applying the scene rewrites every track's level.
    let pad = (0..engine.track_count()).find(|i| engine.track_name(*i) == "pad").unwrap();
    for i in 0..engine.track_count() {
        if i != pad {
            engine.set_track_level(i, 0.0);
        }
    }
    let (l, _) = engine.render(1);
    let per_step = l.len() / 16;
    (0..16).map(|s| {
        l[s * per_step..(s + 1) * per_step].iter().fold(0.0f32, |a, v| a.max(v.abs()))
    }).collect()
}

#[test]
fn ducking_follows_the_named_source_and_not_the_kick() {
    let kicked = pad_envelope("sidechain 0.9");
    let bassed = pad_envelope("sidechain 0.9 from=bass");

    // The kick lands on steps 0 and 8, the bass on steps 4 and 12. Comparing the
    // two configurations at the same step keeps the pad's own envelope out of it.
    // Only the onsets: step 8 is the kick's, but the bass note from steps 4-7 is
    // still releasing there, so both configurations duck and neither wins.
    for step in [0usize] {
        assert!(
            kicked[step] < bassed[step] * 0.9,
            "step {}: the kick should duck harder than the bass does ({} vs {})\nkick {:?}\nbass {:?}",
            step, kicked[step], bassed[step], kicked, bassed
        );
    }
    for step in [4usize, 12] {
        assert!(
            bassed[step] < kicked[step] * 0.9,
            "step {}: from=bass should duck harder than the kick does ({} vs {})\nkick {:?}\nbass {:?}",
            step, bassed[step], kicked[step], kicked, bassed
        );
    }
}

#[test]
fn an_unknown_source_is_a_compile_error() {
    let src = song("sidechain 0.5 from=nobody");
    let ast = dsl::parse(&src).expect("parses");
    let errs = match compiler::compile(&ast) {
        Ok(_) => panic!("should not compile"),
        Err(e) => e,
    };
    assert!(errs[0].message.contains("names no track or module"), "{:?}", errs[0]);
}

#[test]
fn a_track_cannot_duck_against_itself() {
    let src = song("sidechain 0.5 from=pad");
    let ast = dsl::parse(&src).expect("parses");
    let errs = match compiler::compile(&ast) {
        Ok(_) => panic!("should not compile"),
        Err(e) => e,
    };
    assert!(errs[0].message.contains("against itself"), "{:?}", errs[0]);
}

#[test]
fn a_source_with_no_amount_is_a_compile_error() {
    let src = song("sidechain 0 from=bass");
    let ast = dsl::parse(&src).expect("parses");
    let errs = match compiler::compile(&ast) {
        Ok(_) => panic!("should not compile"),
        Err(e) => e,
    };
    assert!(errs[0].message.contains("has no amount"), "{:?}", errs[0]);
}

/// The shape of the duck, not just its depth. A release of a few milliseconds
/// lets the bass snap back inside the kick; a long one makes the pair read as
/// one instrument. Until this existed the DSL could not say which it wanted.
#[test]
fn the_release_time_changes_how_long_the_duck_lasts() {
    /// A kick on beat 1 and a pad that is already sounding, so the only thing
    /// shaping the pad's level is the duck.
    fn probe(shape: &str) -> Vec<f32> {
        let src = format!(r#"
tempo 120
scale C major
sidechain 0.9 {}
module beats kit {{ kick_level 1.0 }}
module keys pad {{ voice_mode unison cutoff 2khz attack 1ms sustain 1.0 release 1s }}
pattern beat {{ kick: X - - -  - - - -  - - - -  - - - - }}
pattern hold {{ [1.3 3.3 5.3] ..*15 }}
track drums {{ play beat using kit level 0.0 out > master }}
track pad {{ play hold using pad out > master }}
master {{ in > limiter > out }}
scene a {{ track drums {{ play beat using kit level 0.0 }} track pad {{ play hold using pad }} }}
arrange {{ a x1 }}
"#, shape);
        let mut engine = SongEngine::from_source(&src).unwrap_or_else(|e| panic!("{}", e));
        engine.start();
        let (l, _) = engine.render(1);
        let sr = tatum_core::SAMPLE_RATE as usize;
        let win = sr / 20; // 50 ms
        (0..4).map(|i| l[i * win..(i + 1) * win].iter().fold(0.0f32, |m, v| m.max(v.abs()))).collect()
    }

    let short = probe("");
    let long = probe("attack=1ms release=1500ms");
    assert!(
        long[0] < short[0] * 0.85,
        "a long release should hold the pad down through the first 50 ms:\n  default {:?}\n  long    {:?}",
        short, long
    );
    // And the two must agree once the kick is long gone, or the release is
    // doing something other than releasing.
    assert!((long[3] - short[3]).abs() < 1e-3, "{:?} vs {:?}", short, long);
}

#[test]
fn a_sidechain_time_needs_a_time() {
    let src = song("sidechain 0.5").replace("scale C major", "scale C major\nsidechain 0.4 release=loud");
    let errs = dsl::parse(&src).expect_err("should not parse");
    assert!(errs[0].message.contains("sidechain release= takes a time like 80ms"), "{}", errs[0].message);

    let src = song("sidechain 0.5").replace("scale C major", "scale C major\nsidechain 0.4 release=9s");
    let errs = dsl::parse(&src).expect_err("should not parse");
    assert!(errs[0].message.contains("outside 0.1ms..2000ms"), "{}", errs[0].message);
}
