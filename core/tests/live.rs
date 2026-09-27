//! The live session: what did not change in the text keeps its state in the
//! engine. Each test renders a song straight through and again with an edit
//! applied mid-way through a `LivePlayer`, then compares samples. Where the
//! edit does not touch what is audible, the two must be identical to the bit:
//! that is the only assertion that catches a swap landing a sample late, a
//! downbeat firing twice, a reverb tail starting over or a random sequence
//! rewinding.

use tatum_core::live::{Applied, LivePlanner, LivePlayer, Plan, FastOp};
use tatum_core::song_engine::{DslError, SongEngine};
use tatum_core::{BLOCK_SIZE, SAMPLE_RATE};

const STEP: f32 = SAMPLE_RATE * 60.0 / 120.0 / 4.0;
const BAR: usize = (STEP * 16.0) as usize;

/// Kick, a held pad with both sends and a bus with a compressor: the state
/// that has to survive an edit (voices, reverb, delay, bus chain, master).
const LIVE: &str = r#"
tempo 120
scale A minor
swing 0.56
humanize 0.3
reverb size=1.0 damp=0.3 predelay=10
delay sync=dotted_eighth feedback=0.6

module beats kit { kick_level 1.0 }
module keys pad { voice_mode poly attack 10ms release 400ms cutoff 2khz }
module bass low { cutoff 0.4 }

pattern beat { kick: X - - - X - - - X - - - X - - - }
pattern hold { [1.3 3.3 5.3]:0.8 ..*15 }
pattern line { 1.1 - 1.3 -  1.5 - 1.3 -  1.1 - 1.3 -  1.5 - 1.3 - }

bus drums
drums { in > compressor(-8, ratio=4) > master }

track kick { play beat using kit level 0.8 out > drums }
track bass { play line using low level 0.6 delay_send 0.3 out > master }
track pad  { play hold using pad level 0.5 reverb_send 0.7 delay_send 0.5 out > master }
"#;

/// The same material arranged: two scenes, an automation sweep, a swap that
/// may land on a scene boundary or inside one.
fn arranged() -> String {
    format!("{}\n\
        scene a {{\n  auto low cutoff 0.2 > 0.8\n  track kick {{ play beat using kit level 0.8 }}\n  track bass {{ play line using low level 0.6 delay_send 0.3 }}\n  track pad {{ play hold using pad level 0.5 reverb_send 0.7 delay_send 0.5 }}\n}}\n\
        scene b {{ track pad {{ play hold using pad level 0.5 reverb_send 0.7 }} track kick {{ play beat using kit }} }}\n\
        arrange {{ a x4 b x4 }}\n", LIVE)
}

/// Structurally different, audibly identical: an unused pattern.
fn with_unused_pattern(src: &str) -> String {
    format!("{}\npattern unused {{ 1.1 - - - }}\n", src)
}

/// A live session levels the song once, when it loads it; straight through,
/// the same song gets the same gain.
fn render_straight(src: &str, bars: usize) -> (Vec<f32>, Vec<f32>) {
    let mut e = SongEngine::from_source(src).unwrap();
    e.set_output_gain(tatum_core::dsl::isolate::output_gain(src).unwrap());
    e.start();
    e.render_steps(bars * 16)
}

/// Render `bars` through the player, loading `edits` (sample offset, source)
/// as the render passes each offset. Returns the audio and the bars swaps
/// landed on.
fn render_live(first: &str, edits: &[(usize, &str)], bars: usize) -> (Vec<f32>, Vec<f32>, Vec<usize>) {
    let mut planner = LivePlanner::new();
    let mut player = LivePlayer::new();
    let plan = planner.plan(first, player.generation()).unwrap_or_else(|e| panic!("{}", e.to_json()));
    assert_eq!(player.apply(plan), Applied::Loaded);
    player.start();
    let total = bars * BAR;
    let mut l = vec![0.0f32; total];
    let mut r = vec![0.0f32; total];
    let mut swaps = Vec::new();
    let mut next_edit = 0;
    let mut pos = 0;
    while pos < total {
        while next_edit < edits.len() && edits[next_edit].0 <= pos {
            let plan =
                planner.plan(edits[next_edit].1, player.generation()).unwrap_or_else(|e| panic!("{}", e.to_json()));
            let applied = player.apply(plan);
            assert_ne!(applied, Applied::Stale, "a plan went stale");
            next_edit += 1;
        }
        let chunk = BLOCK_SIZE.min(total - pos);
        if let Some(bar) = player.process(&mut l[pos..pos + chunk], &mut r[pos..pos + chunk]) {
            swaps.push(bar);
        }
        while player.take_retired().is_some() {}
        pos += chunk;
    }
    (l, r, swaps)
}

fn first_difference(a: &[f32], b: &[f32]) -> Option<usize> {
    a.iter().zip(b).position(|(x, y)| x.to_bits() != y.to_bits())
}

fn assert_identical(reference: &(Vec<f32>, Vec<f32>), live: &(Vec<f32>, Vec<f32>), what: &str) {
    for (ch, (r, l)) in [(&reference.0, &live.0), (&reference.1, &live.1)].iter().enumerate() {
        if let Some(i) = first_difference(r, l) {
            panic!(
                "{}: channel {} differs from sample {} (bar {}, step {:.2}): {} vs {}",
                what,
                ch,
                i,
                i / BAR,
                (i % BAR) as f32 / STEP,
                r[i],
                l[i]
            );
        }
    }
}

#[test]
fn swap_to_the_same_song_is_bit_identical() {
    // An unused pattern is a structural change, so this goes through a full
    // swap at bar 4 with everything inherited. Swing and humanize are on, so
    // the random sequence and the fractional step clock must carry over too.
    let reference = render_straight(LIVE, 8);
    let edited = with_unused_pattern(LIVE);
    let (l, r, swaps) = render_live(LIVE, &[(3 * BAR + 1000, &edited)], 8);
    assert_eq!(swaps, vec![4], "swap landed on {:?}", swaps);
    assert_identical(&reference, &(l, r), "live mode");
}

#[test]
fn swap_inside_an_arranged_scene_is_bit_identical() {
    // Bar 2 is inside scene a, three bars into its automation sweep. The
    // sweep must continue from where it was, not restart.
    let src = arranged();
    let reference = render_straight(&src, 8);
    let edited = with_unused_pattern(&src);
    let (l, r, swaps) = render_live(&src, &[(BAR + 500, &edited)], 8);
    assert_eq!(swaps, vec![2]);
    assert_identical(&reference, &(l, r), "arranged, mid-scene");
}

#[test]
fn swap_on_a_scene_boundary_is_bit_identical() {
    // Bar 4 opens scene b: the straight run releases everything and applies
    // the scene there. The swap must do exactly that and no more.
    let src = arranged();
    let reference = render_straight(&src, 8);
    let edited = with_unused_pattern(&src);
    let (l, r, swaps) = render_live(&src, &[(3 * BAR + 2000, &edited)], 8);
    assert_eq!(swaps, vec![4]);
    assert_identical(&reference, &(l, r), "arranged, scene boundary");
}

#[test]
fn changing_one_track_leaves_every_other_sample_untouched() {
    // The bass pattern changes at bar 4. Kick, pad, reverb and delay tails
    // did not change in the text, so with the bass silenced the render is
    // identical to the reference: nothing retriggered, no tail restarted,
    // no downbeat fired twice.
    //
    // Humanize stays on, and that is the point. It used to have to come out:
    // with one random stream for the whole song, a bass pattern with one more
    // note in it consumed one more draw and every kick and pad velocity after
    // bar 4 shifted. Each track draws from its own stream now, so an edit to
    // the bass cannot reach them.
    let quiet_bass = LIVE.replace("level 0.6 delay_send 0.3", "level 0.0");
    assert!(quiet_bass.contains("humanize 0.3"));
    let reference = render_straight(&quiet_bass, 8);
    let edited = quiet_bass.replace(
        "pattern line { 1.1 - 1.3 -  1.5 - 1.3 -  1.1 - 1.3 -  1.5 - 1.3 - }",
        "pattern line { 1.1 1.1 1.3 -  1.5 - 1.3 -  1.1 - 1.3 -  1.5 - 1.3 - }",
    );
    assert_ne!(edited, quiet_bass);
    let (l, r, swaps) = render_live(&quiet_bass, &[(3 * BAR + 100, &edited)], 8);
    assert_eq!(swaps, vec![4]);
    assert_identical(&reference, &(l, r), "bass pattern edit");
}

#[test]
fn a_changed_track_is_the_only_thing_that_changes() {
    // The pad track gets an insert at bar 4: the track cannot be inherited,
    // so its held chord is released on the (inherited) pad and retriggered
    // at the bar. The kick is inherited, so it must fire exactly where the
    // reference does, and only once.
    let reference = render_straight(LIVE, 8);
    let edited = LIVE.replace("delay_send 0.5 out > master", "delay_send 0.5 out > highpass(300) > master");
    assert_ne!(edited, LIVE);
    let (l, _, swaps) = render_live(LIVE, &[(3 * BAR + 100, &edited)], 8);
    assert_eq!(swaps, vec![4]);
    // Before the swap: identical.
    assert_eq!(first_difference(&reference.0[..4 * BAR], &l[..4 * BAR]), None);
    // The downbeat kick: same sample in both. The pad and its reverb never go
    // quiet, so the kick shows up as a sample-to-sample jump far bigger than
    // anything a pad makes. Take the biggest jump in the window as the onset
    // rather than everything over a fixed threshold -- the threshold used to
    // be 0.08, which is the kick's height at one particular humanized
    // velocity and stopped being true the moment the random stream changed.
    // The engine fires a step at the start of the block it falls in, so look
    // from a block before the bar.
    let from = 4 * BAR - 256;
    let onset = |x: &[f32]| -> (usize, f32) {
        let mut jumps: Vec<(usize, f32)> = (from..from + 1323).map(|i| (i, (x[i + 1] - x[i]).abs())).collect();
        jumps.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        (jumps[0].0, jumps[jumps.len() / 2].1)
    };
    let (ref_at, ref_median) = onset(&reference.0);
    let (live_at, _) = onset(&l);
    // It is a transient, not the loudest moment of a smooth signal.
    let ref_peak = (reference.0[ref_at + 1] - reference.0[ref_at]).abs();
    assert!(
        ref_peak > ref_median * 5.0,
        "no transient in the window: biggest jump {ref_peak:.4} against a median of {ref_median:.4}"
    );
    assert_eq!(live_at, ref_at, "the downbeat moved");
}

#[test]
fn the_fast_path_validates_like_check() {
    let mut planner = LivePlanner::new();
    let mut player = LivePlayer::new();
    let plan = planner.plan(LIVE, player.generation()).unwrap();
    player.apply(plan);
    player.start();
    // In range: a fast op with the value resolved to an id, no swap.
    let edited = LIVE.replace("cutoff 0.4", "cutoff 0.5");
    match planner.plan(&edited, player.generation()).unwrap() {
        Plan::Fast { ops, .. } => {
            assert!(
                matches!(ops[..], [FastOp::ModuleParam { instrument: 2, value, .. }] if (value - 0.5).abs() < 1e-6),
                "{:?}",
                ops
            );
        }
        other => panic!("expected a fast plan, got {}", other.describe()),
    }
    // Out of range: the same error `tatum check` gives, not a silent clamp.
    let bad = LIVE.replace("cutoff 0.4", "cutoff 7.0");
    match planner.plan(&bad, player.generation()) {
        Err(DslError::Compile(errs)) => assert!(errs[0].message.contains("cutoff"), "{}", errs[0].message),
        Err(DslError::Parse(_)) => panic!("parse error, expected a compile error"),
        Ok(p) => panic!("out-of-range value produced a {} plan", p.describe()),
    }
}

#[test]
fn a_fast_edit_after_a_queued_swap_lands_on_the_new_engine() {
    // Two saves inside one bar: a structural edit (queued), then a level
    // change. The level belongs to the queued song, so it must be on the
    // engine that takes over, not on the one about to retire.
    let mut planner = LivePlanner::new();
    let mut player = LivePlayer::new();
    let plan = planner.plan(LIVE, player.generation()).unwrap();
    player.apply(plan);
    player.start();
    let mut l = [0.0f32; BLOCK_SIZE];
    let mut r = [0.0f32; BLOCK_SIZE];
    player.process(&mut l, &mut r); // past bar 0's line
    let structural = with_unused_pattern(LIVE);
    assert_eq!(player.apply(planner.plan(&structural, player.generation()).unwrap()), Applied::Queued);
    let level = structural.replace("level 0.8 out > drums", "level 0.3 out > drums");
    assert_eq!(player.apply(planner.plan(&level, player.generation()).unwrap()), Applied::Fast);
    assert!((player.engine().unwrap().track_level(0) - 0.8).abs() < 1e-6, "applied to the old engine");
    let mut swapped = None;
    for _ in 0..(BAR / BLOCK_SIZE + 2) {
        if let Some(b) = player.process(&mut l, &mut r) {
            swapped = Some(b);
        }
    }
    assert_eq!(swapped, Some(1));
    assert!((player.engine().unwrap().track_level(0) - 0.3).abs() < 1e-6);
    // And the planner now knows the swap landed: the next edit is fast on it.
    let again = level.replace("level 0.3 out > drums", "level 0.4 out > drums");
    assert_eq!(player.apply(planner.plan(&again, player.generation()).unwrap()), Applied::Fast);
    assert!((player.engine().unwrap().track_level(0) - 0.4).abs() < 1e-6);
    assert_eq!(player.blind_swaps(), 0);
}

#[test]
fn switching_which_pattern_a_track_plays_waits_for_the_bar() {
    // docs/DSL.md: only values apply instantly. `play` is what is played.
    let mut planner = LivePlanner::new();
    let mut player = LivePlayer::new();
    player.apply(planner.plan(LIVE, player.generation()).unwrap());
    player.start();
    let edited = LIVE.replace("play line using low", "play hold using low");
    match planner.plan(&edited, player.generation()).unwrap() {
        Plan::Swap { .. } => {}
        other => panic!("expected a swap, got {}", other.describe()),
    }
}

#[test]
fn identical_text_is_reported_as_unchanged() {
    let mut planner = LivePlanner::new();
    let mut player = LivePlayer::new();
    player.apply(planner.plan(LIVE, player.generation()).unwrap());
    let spaced = LIVE.replace("level 0.8", "level  0.8");
    assert!(matches!(planner.plan(&spaced, player.generation()).unwrap(), Plan::Unchanged));
}

#[test]
fn stopping_takes_the_queued_engine_at_once() {
    let mut planner = LivePlanner::new();
    let mut player = LivePlayer::new();
    player.apply(planner.plan(LIVE, player.generation()).unwrap());
    player.start();
    let edited = with_unused_pattern(LIVE);
    assert_eq!(player.apply(planner.plan(&edited, player.generation()).unwrap()), Applied::Queued);
    player.stop();
    assert!(!player.running());
    assert!(!player.has_pending());
    assert_eq!(player.engine().unwrap().pattern_count(), 4);
    assert!(player.take_retired().is_some());
}

#[test]
fn two_tracks_on_one_module_both_keep_their_voices_across_a_swap() {
    // The engine gives every track that names a module its own copy, under
    // the same name. Inheriting by name alone paired both copies with the
    // first old one, so the second track retriggered at every swap. Copies
    // pair by occurrence now, and the render stays identical.
    let src = "tempo 120\nscale A minor\nmodule keys pad { voice_mode poly attack 10ms release 400ms cutoff 2khz }\npattern hold { [1.3 3.3 5.3]:0.8 ..*15 }\npattern high { [1.5 3.5]:0.6 ..*15 }\ntrack a { play hold using pad level 0.5 out > master }\ntrack b { play high using pad level 0.5 out > master }\nmaster { in > out }\n";
    let reference = render_straight(src, 6);
    let edited = with_unused_pattern(src);
    let (l, r, swaps) = render_live(src, &[(2 * BAR + 100, &edited)], 6);
    assert_eq!(swaps, vec![3]);
    assert_identical(&reference, &(l, r), "two tracks on one module");

    // And a value edit on the module reaches every copy: the live render
    // after the edit equals a straight render of the edited song.
    let cut = src.replace("cutoff 2khz", "cutoff 500hz");
    let mut planner = LivePlanner::new();
    let mut player = LivePlayer::new();
    player.apply(planner.plan(src, player.generation()).unwrap());
    player.start();
    assert_eq!(player.apply(planner.plan(&cut, player.generation()).unwrap()), Applied::Fast);
    // At the gain the session measured when it loaded `src`: edits keep it.
    let mut e = SongEngine::from_source(&cut).unwrap();
    e.set_output_gain(tatum_core::dsl::isolate::output_gain(src).unwrap());
    e.start();
    let (expected, _) = e.render_steps(16);
    let mut got = vec![0.0f32; BAR];
    let mut scratch = vec![0.0f32; BAR];
    let mut pos = 0;
    while pos < BAR {
        let chunk = BLOCK_SIZE.min(BAR - pos);
        player.process(&mut got[pos..pos + chunk], &mut scratch[pos..pos + chunk]);
        pos += chunk;
    }
    let first = first_difference(&expected, &got);
    assert_eq!(first, None, "the cutoff edit did not reach every copy (differs at {:?})", first);
}
