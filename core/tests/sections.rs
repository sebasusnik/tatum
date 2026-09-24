//! `sections()` is the arrangement as the mix report needs to see it.
//!
//! The tempo has to travel with each section. A scene can change it, and the
//! report turns bars into samples to slice the render -- put every section on
//! the song's opening tempo and every section after a tempo change lands in
//! the wrong place, which would make the arc a measurement of the wrong audio.

use tatum_core::song_engine::SongEngine;

const HEAD: &str = "\
scale C minor
module beats kit { }
pattern beat { kick: X - - - X - - - X - - - X - - - }
track drums { play beat using kit level 0.6 out > master }
";

fn sections(src: &str) -> Vec<(String, u32, f32)> {
    let e = SongEngine::from_source(src).unwrap_or_else(|err| panic!("{err}"));
    e.sections().into_iter().map(|(n, b, t)| (n.to_string(), b, t)).collect()
}

#[test]
fn the_arrangement_comes_back_in_order_with_its_repeats() {
    let src = format!(
        "tempo 120\n{HEAD}\
         scene a {{ track drums {{ play beat using kit }} }}\n\
         scene b {{ track drums {{ play beat using kit }} }}\n\
         arrange {{ a x4 b x8 a x2 }}\n\
         master {{ in > out }}\n"
    );
    let got = sections(&src);
    assert_eq!(
        got,
        vec![
            ("a".into(), 4, 120.0),
            ("b".into(), 8, 120.0),
            ("a".into(), 2, 120.0),
        ]
    );
}

#[test]
fn a_scene_that_changes_the_tempo_carries_it_forward() {
    let src = format!(
        "tempo 120\n{HEAD}\
         scene slow {{ track drums {{ play beat using kit }} }}\n\
         scene fast {{ tempo 160 track drums {{ play beat using kit }} }}\n\
         scene after {{ track drums {{ play beat using kit }} }}\n\
         arrange {{ slow x4 fast x4 after x4 }}\n\
         master {{ in > out }}\n"
    );
    let got = sections(&src);
    assert_eq!(got[0].2, 120.0);
    assert_eq!(got[1].2, 160.0, "the scene sets it");
    assert_eq!(got[2].2, 160.0, "and nothing sets it back, so it stays");
}

/// A song with no `arrange` has no sections, and the report says nothing
/// rather than inventing one.
#[test]
fn a_song_without_an_arrangement_has_no_sections() {
    let src = format!("tempo 120\n{HEAD}master {{ in > out }}\n");
    assert!(sections(&src).is_empty());
}
