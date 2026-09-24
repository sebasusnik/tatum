//! Humanization draws from one random stream per track, not one per song.
//!
//! The shared stream was a quiet bug with a wide blast radius. Velocity
//! jitter and the drum probability gate are drawn inside `advance_step`,
//! which walks tracks in order, so every draw a track made shifted the
//! numbers handed to every track after it. Muting a voice, adding one, or
//! moving a line in the file changed the feel of the whole song -- and since
//! the change is a few percent of velocity spread over every hit, it does
//! not read as a bug. It reads as "the mix moved and I do not know why".
//!
//! Each track now seeds from its own name. The name, not the index, because
//! adding a track at the top of a file must not renumber everybody else's
//! stream. The consequence worth knowing: renaming a track rerolls its
//! jitter, the same way renaming it already restarts its voices on a hot
//! swap. A name is an identity here.

use tatum_core::song_engine::SongEngine;

const BARS: usize = 4;

fn render(src: &str) -> (Vec<f32>, Vec<f32>) {
    let mut e = SongEngine::from_source(src).unwrap_or_else(|err| panic!("{err:?}"));
    e.start();
    e.render_steps(BARS * 16)
}

fn first_difference(a: &(Vec<f32>, Vec<f32>), b: &(Vec<f32>, Vec<f32>)) -> Option<usize> {
    assert_eq!(a.0.len(), b.0.len(), "different lengths");
    (0..a.0.len()).find(|&i| a.0[i] != b.0[i] || a.1[i] != b.1[i])
}

/// `gain_comp 0` because it scales the master by the number of active tracks:
/// without it, adding a silent track changes the level of everything and the
/// test would be measuring that instead.
const HEAD: &str = "\
tempo 120
scale A minor
humanize 0.4 timing 0.2
gain_comp 0

module beats kit { kick_level 1.0 }
module keys pad { voice_mode poly attack 10ms release 300ms }
module bass low { cutoff 0.4 }

pattern beat { kick: X - - - X - - - X - - - X - - -
               hat:  - x?0.5 - x  - x?0.5 - x  - x?0.5 - x  - x?0.5 - x }
pattern hold { [1.3 3.3 5.3]:0.8 ..*15 }
pattern line { 1.1 - 1.3 -  1.5 - 1.3 -  1.1 - 1.3 -  1.5 - 1.3 - }

track kick { play beat using kit level 0.8 out > master }
track pad  { play hold using pad level 0.5 out > master }
";

const TAIL: &str = "master { in > limiter > out }\n";

fn song(extra: &str) -> String {
    format!("{HEAD}{extra}{TAIL}")
}

/// The property, stated the way it bites: a voice that makes no sound must
/// not be able to change the ones that do.
#[test]
fn a_track_at_level_zero_changes_nothing_about_the_others() {
    let without = render(&song(""));
    let with = render(&song("track bass { play line using low level 0.0 out > master }\n"));
    assert_eq!(
        first_difference(&without, &with),
        None,
        "a silent track moved the rest of the mix"
    );
}

/// The same, one step further: the silent track is also drawing from the
/// probability gate and from velocity jitter on every step.
#[test]
fn a_silent_drum_track_full_of_probability_gates_changes_nothing() {
    let extra = "pattern ghost { hat: x?0.5 x?0.5 x?0.5 x?0.5 x?0.5 x?0.5 x?0.5 x?0.5 \
                                      x?0.5 x?0.5 x?0.5 x?0.5 x?0.5 x?0.5 x?0.5 x?0.5 }\n\
                 module beats ghostkit { }\n\
                 track ghost { play ghost using ghostkit level 0.0 out > master }\n";
    assert_eq!(
        first_difference(&render(&song("")), &render(&song(extra))),
        None,
        "a silent track's coin flips reached the audible ones"
    );
}

/// Where the track sits in the file is not part of its identity.
#[test]
fn moving_a_track_up_the_file_does_not_reroll_it() {
    let normal = render(&song(""));
    let reordered = format!(
        "{}\ntrack pad  {{ play hold using pad level 0.5 out > master }}\n{TAIL}",
        HEAD.replace("track pad  { play hold using pad level 0.5 out > master }\n", "")
    );
    let moved = render(&reordered);
    // Summing is float addition, which is not associative, so swapping the
    // order two tracks are mixed in is allowed to move the last bit. What is
    // not allowed is the jitter changing, which is a percent, not an ulp.
    let worst = (0..normal.0.len())
        .map(|i| (normal.0[i] - moved.0[i]).abs().max((normal.1[i] - moved.1[i]).abs()))
        .fold(0.0f32, f32::max);
    assert!(worst < 1e-6, "reordering rerolled the jitter: worst sample differs by {worst}");
}

/// Two tracks that differ in one character must not jitter in lockstep. FNV
/// is in there for this: a seed that kept `hat` and `hats` adjacent would put
/// two hi-hats on the same random walk, which is the opposite of humanizing.
#[test]
fn one_character_of_name_is_a_different_stream() {
    let a = render(&song("track hat  { play line using low level 0.4 out > master }\n"));
    let b = render(&song("track hats { play line using low level 0.4 out > master }\n"));
    assert!(
        first_difference(&a, &b).is_some(),
        "`hat` and `hats` drew the same numbers"
    );
}

/// Nothing above means anything if the render is not reproducible.
#[test]
fn the_same_song_renders_the_same_twice() {
    assert_eq!(first_difference(&render(&song("")), &render(&song(""))), None);
}

// Not tested here: `reset()` followed by `start()` on one engine does not
// reproduce the first pass, and it does not with `humanize 0` either, so it
// is not the random streams -- something else in the engine survives a reset.
// Rendering a fresh engine twice, which is what the test above does, is
// reproducible.
