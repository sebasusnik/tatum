//! A loop of bars caught by hand: the bars between the loop's in and out
//! play again, the bar count goes on, and letting go carries on from where
//! the music is.

use tatum_core::song_engine::SongEngine;
use tatum_core::BLOCK_SIZE;

// An eight-bar line, and its level rising over the eight bars.
const SONG: &str = r#"
tempo 120
module bass acid { cutoff 0.4 }
pattern line { 1.1 -*127 }
track bass { play line using acid out > master }
auto bass level 0 > 1 over 8
"#;

/// Plays until bar `bar` starts; returns, per bar crossed, the track's step
/// in its pattern on the bar's first block, and its level.
fn play_to(e: &mut SongEngine, bar: usize, seen: &mut Vec<(usize, usize)>, levels: &mut Vec<(usize, f32)>) {
    let (mut l, mut r) = ([0.0f32; BLOCK_SIZE], [0.0f32; BLOCK_SIZE]);
    while e.current_bar() < bar {
        let before = e.current_bar();
        e.process_block_stereo(&mut l, &mut r);
        if e.current_bar() != before {
            seen.push((e.current_bar(), e.track_step(0)));
            levels.push((e.current_bar(), e.track_level(0)));
        }
    }
}

fn step_of(seen: &[(usize, usize)], bar: usize) -> Option<usize> {
    seen.iter().find(|(b, _)| *b == bar).map(|(_, s)| *s / 16)
}

#[test]
fn the_bars_between_in_and_out_play_again() {
    // A pattern of 128 sixteenths: bar k starts on step 16k.
    let mut e = SongEngine::from_source(SONG).expect("compiles");
    e.start();
    let (mut seen, mut levels) = (Vec::new(), Vec::new());
    play_to(&mut e, 2, &mut seen, &mut levels);
    // In during bar 2, out during bar 3: bars 2 and 3 loop.
    e.loop_in();
    play_to(&mut e, 3, &mut seen, &mut levels);
    e.loop_out(2);
    play_to(&mut e, 8, &mut seen, &mut levels);
    // Bars 4 and 6 are bar 2 again, 5 and 7 bar 3.
    assert_eq!((2..8).map(|b| step_of(&seen, b).unwrap()).collect::<Vec<_>>(), [2, 3, 2, 3, 2, 3], "{seen:?}");
    // The `auto` goes back with it.
    let level = |bar: usize| levels.iter().find(|(b, _)| *b == bar).unwrap().1;
    assert!((level(4) - level(2)).abs() < 0.02 && (level(7) - level(3)).abs() < 0.02, "{levels:?}");
    assert!(level(3) > level(2) + 0.05, "{levels:?}");
    // Let go in bar 8 (bar 2 again): on to bar 3, then 4, of the music.
    e.loop_exit();
    play_to(&mut e, 10, &mut seen, &mut levels);
    assert_eq!(
        (step_of(&seen, 8).unwrap(), step_of(&seen, 9).unwrap(), step_of(&seen, 10).unwrap()),
        (2, 3, 4),
        "{seen:?}"
    );
    assert_eq!(e.current_bar(), 10, "the bar count goes on");
}
