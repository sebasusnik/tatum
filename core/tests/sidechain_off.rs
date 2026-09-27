//! `sidechain 0` on a track turns its ducking off.
//!
//! It used to mean "take the song's amount", because the per-track override
//! was a float with zero standing for unset. A track written `sidechain 0.0`
//! to stay still under the kick was ducked at the global amount anyway;
//! `detroit`, `liquid_dnb` and `dub_techno` all wrote it that way.

use tatum_core::song_engine::SongEngine;

fn song(global: &str, pad: &str) -> String {
    format!(
        r#"
tempo 120
scale A minor
sidechain {global}
module beats kit {{ kick_level 1.0 }}
module keys pad {{ voice_mode poly attack 5ms release 300ms cutoff 3khz }}
pattern beat {{ kick: X - - - X - - - X - - - X - - - }}
pattern hold {{ [1.3 3.3 5.3]:0.8 ..*15 }}
track kick {{ play beat using kit level 0 out > master }}
track pad  {{ play hold using pad level 0.5 {pad} out > master }}
"#
    )
}

fn render(src: &str) -> Vec<f32> {
    SongEngine::from_source(src).unwrap().render(2).0
}

#[test]
fn a_track_at_sidechain_zero_is_not_ducked() {
    // The kick is at level 0: it is not heard, but it still drives the
    // sidechain, so all that is heard is the pad and how it is ducked.
    let still = render(&song("0.0", ""));
    let off = render(&song("0.8", "sidechain 0"));
    let ducked = render(&song("0.8", ""));
    assert_ne!(ducked, still, "the global amount should duck the pad");
    assert_eq!(off, still, "`sidechain 0` should leave the pad still");
}

#[test]
fn a_scene_can_turn_one_track_off() {
    let src = song("0.8", "") + "scene a { track kick { play beat using kit } track pad { play hold using pad sidechain 0 } }\narrange { a x2 }\n";
    let plain = song("0.0", "")
        + "scene a { track kick { play beat using kit } track pad { play hold using pad } }\narrange { a x2 }\n";
    assert_eq!(render(&src), render(&plain));
}
