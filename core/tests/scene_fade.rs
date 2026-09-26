//! A track the next scene drops fades out over 10 ms instead of stopping dead.
//!
//! Stopping dead cut the wave to zero wherever it was in its cycle, and that
//! step is a click: `tatum debug` found one on nearly every section change of
//! `hitech_psy`, on every pad, drone and lead that did not carry over.

use tatum_core::song_engine::SongEngine;

const SONG: &str = r#"
tempo 120
scale C major
gain_comp 0

module keys pad { cutoff 0.8 sustain 1.0 attack 1ms release 0.8 }
module beats kit { kick_level 1.0 }

pattern hold { 1.3 .. .. ..  .. .. .. ..  .. .. .. ..  .. .. .. .. }
pattern beat { kick: - - - -  - - - -  - - - -  - - - - }

track pad   { play hold using pad out > master }
track drums { play beat using kit out > master }

scene a {
    track pad   { play hold using pad }
}
scene b {
    track drums { play beat using kit }
}
arrange { a x1 b x1 }
"#;

#[test]
fn a_dropped_track_fades_instead_of_stopping_dead() {
    let mut e = SongEngine::from_source(SONG).unwrap();
    let (l, _) = e.render(2);
    let playing = l[..l.len() / 2].iter().fold(0.0f32, |a, v| a.max(v.abs()));
    assert!(playing > 0.05, "the pad should be playing, peak {playing}");
    // Scene b has nothing to play, so the last sound is where the pad ended.
    let end = l.iter().rposition(|v| v.abs() > 1e-6).expect("some sound");
    // Stopping dead leaves the wave at full height on its last sample.
    assert!(l[end].abs() < playing * 0.02,
        "the pad was cut at {:.3} of its level", l[end].abs() / playing);
    // Over about ten milliseconds, starting on the bar line.
    let bar = l.len() / 2;
    assert!(end > bar + 300 && end < bar + 600, "faded from {bar} to {end}");
}
