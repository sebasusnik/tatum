//! `auto ... over N` outside any scene: the lanes of a song that loops. They
//! start on the first bar the text plays -- bar 0 for a render, the swap bar
//! for a text that takes over from another -- run over their bars and hold.
//! And a value one text automated does not stay where its lane left it when
//! the next text does not automate it.

use tatum_core::dsl;
use tatum_core::live::{Inherit, LivePlanner, LivePlayer, Plan};
use tatum_core::song_engine::SongEngine;
use tatum_core::{BLOCK_SIZE, SAMPLE_RATE};

const STEP: f32 = SAMPLE_RATE * 60.0 / 120.0 / 4.0;
const BAR: usize = (STEP * 16.0) as usize;

/// A kick, a bass and hats through a master lowpass with some resonance, so
/// that where its cutoff sits is plain in the top of the spectrum.
const RIG: &str = r#"
tempo 120
module beats kit { kick_level 1.0 }
module bass low { cutoff 0.5 }
pattern beat { kick: X - - - X - - - X - - - X - - - }
pattern hats { hat: x x x x x x x x x x x x x x x x }
pattern line { 1.1 - 1.3 - 1.5 - 1.3 - 1.1 - 1.3 - 1.5 - 1.3 - }
track kick { play beat using kit level 0.8 out > master }
track hat  { play hats using kit level 0.8 out > master }
track bass { play line using low level 0.6 out > master }
master { in > lowpass(20000, 0.7) > out }
"#;

fn with(lines: &str) -> String {
    format!("{RIG}\n{lines}\n")
}

fn parse_errors(src: &str) -> Vec<String> {
    match dsl::parse(src) {
        Ok(_) => Vec::new(),
        Err(errs) => errs.into_iter().map(|e| e.message).collect(),
    }
}

/// Energy of the first difference, in dB: a crude measure of the top end.
fn top_db(x: &[f32]) -> f32 {
    let e: f64 = x.windows(2).map(|w| ((w[1] - w[0]) as f64).powi(2)).sum::<f64>() / x.len().max(1) as f64;
    10.0 * e.max(1e-20).log10() as f32
}

fn bar(x: &[f32], b: usize) -> &[f32] {
    &x[b * BAR..((b + 1) * BAR).min(x.len())]
}

#[test]
fn a_top_level_lane_parses_with_its_length() {
    let song = dsl::parse(&with("auto master cutoff 800 > 20000 over 8\nauto bass.lp wet 0 > 1 > 0.5 > 1 over 16"))
        .expect("parses");
    assert_eq!(song.automations.len(), 2);
    assert_eq!(song.automations[0].target, "master.cutoff");
    assert_eq!(song.automations[0].keyframes, vec![800.0, 20000.0]);
    assert_eq!(song.automations[0].over, Some(8));
    assert_eq!(song.automations[1].target, "bass.lp.wet");
    assert_eq!(song.automations[1].keyframes.len(), 4);
    assert_eq!(song.automations[1].over, Some(16));
}

#[test]
fn a_top_level_lane_without_its_length_says_how_to_write_it() {
    let errs = parse_errors(&with("auto low cutoff 0.2 > 0.8"));
    assert_eq!(errs.len(), 1, "{errs:?}");
    assert!(errs[0].contains("auto low cutoff 0.2 > 0.8 over 8"), "{}", errs[0]);
}

#[test]
fn over_is_a_whole_number_of_bars() {
    for bad in ["over 0", "over 2.5", "over", "over 5000"] {
        let errs = parse_errors(&with(&format!("auto low cutoff 0.2 > 0.8 {bad}")));
        assert!(errs.iter().any(|e| e.contains("whole number of bars")), "{bad}: {errs:?}");
    }
}

#[test]
fn a_scene_lane_has_no_over_and_a_song_with_scenes_no_top_level_lane() {
    let scene = |body: &str| {
        format!("{RIG}\nscene a {{\n {body}\n track kick {{ play beat using kit }}\n}}\narrange {{ a x2 }}\n")
    };
    let errs = parse_errors(&scene("auto low cutoff 0.2 > 0.8 over 4"));
    assert!(errs.iter().any(|e| e.contains("a scene's lane already runs over the whole scene")), "{errs:?}");

    let errs = parse_errors(&format!("{}auto low cutoff 0.2 > 0.8 over 4\n", scene("")));
    assert!(errs.iter().any(|e| e.contains("put the lane inside the scene")), "{errs:?}");
}

#[test]
fn a_top_level_target_is_checked_like_a_scene_one() {
    let song = dsl::parse(&with("auto low cutof 0.2 > 0.8 over 4")).expect("parses");
    let errs = dsl::compiler::compile(&song).err().expect("an unknown parameter is an error");
    assert!(errs.iter().any(|e| e.message.contains("cutof")), "{errs:?}");

    let song = dsl::parse(&with("auto bass.nope wet 0 > 1 over 4")).expect("parses");
    assert!(dsl::compiler::compile(&song).is_err(), "a node the track does not have is an error");
}

/// A level lane over four bars: a quarter of the way at the end of the first
/// bar, all the way at the end of the fourth, and there it stays.
#[test]
fn a_level_lane_moves_over_its_bars_and_then_holds() {
    let mut e = SongEngine::from_source(&with("auto bass level 0 > 1 over 4")).unwrap();
    let bass = (0..e.track_count()).position(|i| e.track_name(i) == "bass").unwrap();
    e.start();
    let mut seen = Vec::new();
    for _ in 0..8 {
        e.render_steps(16);
        seen.push(e.track_level(bass));
    }
    for (b, want) in [0.25, 0.5, 0.75, 1.0, 1.0, 1.0, 1.0, 1.0].iter().enumerate() {
        assert!((seen[b] - want).abs() < 0.03, "end of bar {b}: level {} (want {want}); all {seen:?}", seen[b]);
    }
}

/// A master cutoff lane closes the top over two bars and holds it closed.
#[test]
fn a_master_cutoff_lane_moves_and_then_holds() {
    let render = |src: &str| {
        let mut e = SongEngine::from_source(src).unwrap();
        e.set_output_gain(1.0);
        e.start();
        e.render_steps(16 * 6).0
    };
    let closed = render(&RIG.replace("lowpass(20000", "lowpass(300"));
    let swept = render(&with("auto master cutoff 20000 > 300 over 2"));
    let (first, held, later) = (top_db(bar(&swept, 0)), top_db(bar(&swept, 3)), top_db(bar(&swept, 5)));
    assert!(first - held > 10.0, "the sweep closed the top by only {:.1} dB", first - held);
    assert!((held - later).abs() < 1.0, "after the lane the cutoff holds: {held:.1} then {later:.1} dB");
    let there = top_db(bar(&closed, 3));
    assert!((held - there).abs() < 1.0, "it holds where the lane ends: {held:.1} dB against {there:.1} written");
}

/// Plays `steps` as a set does, each for its bars, and returns the audio and
/// a reading taken at the end of every bar.
/// For each handover, the inherit map it carried, if it was a swap.
fn walk<T>(
    steps: &[(&str, usize)],
    mut read: impl FnMut(&SongEngine) -> T,
) -> (Vec<f32>, Vec<T>, Vec<Option<Inherit>>) {
    let mut planner = LivePlanner::new();
    planner.set_output_gain(1.0);
    planner.restart_lanes();
    let mut player = LivePlayer::new();
    player.apply(planner.plan(steps[0].0, player.generation()).unwrap());
    player.start();
    let (mut out, mut readings, mut plans) = (Vec::new(), Vec::new(), Vec::new());
    let (mut l, mut r) = ([0.0f32; BLOCK_SIZE], [0.0f32; BLOCK_SIZE]);
    let mut pos = 0usize;
    let mut next_at = steps[0].1 * BAR;
    let mut next = 1;
    let total: usize = steps.iter().map(|s| s.1).sum::<usize>() * BAR;
    while pos < total {
        // Hand the next step over one bar ahead: the swap lands on the line.
        if next < steps.len() && pos + BAR >= next_at {
            let plan = planner.plan(steps[next].0, player.generation()).unwrap();
            plans.push(match &plan {
                Plan::Swap { inherit, .. } => Some(inherit[0].1.clone()),
                _ => None,
            });
            player.apply(plan);
            next_at += steps[next].1 * BAR;
            next += 1;
        }
        let n = BLOCK_SIZE.min(total - pos);
        player.process(&mut l[..n], &mut r[..n]);
        out.extend_from_slice(&l[..n]);
        while player.take_retired().is_some() {}
        let before = pos / BAR;
        pos += n;
        // Taken in the block that crosses a bar line, just after it.
        if pos / BAR != before || pos == total {
            readings.push(read(player.engine().unwrap()));
        }
    }
    (out, readings, plans)
}

/// The lane of the second step starts on that step's first bar, not on bar 0
/// of the set: by then a lane counted from the top would be long over.
#[test]
fn a_steps_lane_starts_on_the_steps_first_bar() {
    let a = RIG.to_string();
    let b = with("auto bass level 0 > 1 over 2");
    let (_, levels, plans) = walk(&[(&a, 3), (&b, 4)], |e| {
        let i = (0..e.track_count()).position(|i| e.track_name(i) == "bass").unwrap();
        e.track_level(i)
    });
    assert!(plans[0].is_some(), "a step with a new lane takes over with a swap");
    // Read as each bar starts: bars 1-2 are the first step at its own level,
    // the lane starts on bar 3, is halfway on bar 4 and there from bar 5 on.
    for (b, want) in [(1, 0.6), (2, 0.6), (3, 0.0), (4, 0.5), (5, 1.0), (7, 1.0)] {
        let got = levels[b - 1];
        assert!((got - want).abs() < 0.03, "bar {b}: level {got} (want {want}); all {levels:?}");
    }
}

/// A step closes the master and a module's filter with lanes; the next step
/// does not automate either. The next step plays what its text says -- the
/// master open -- and not the closed filter the last step left behind.
#[test]
fn what_a_step_automated_returns_to_the_next_steps_text() {
    let a = with("auto master cutoff 20000 > 300 over 1\nauto low cutoff 0.5 > 0.05 over 1");
    let b = RIG.replace("level 0.6", "level 0.5");
    let (audio, _, plans) = walk(&[(&a, 3), (&b, 3)], |_| ());
    let map = plans[0].as_ref().expect("a swap");
    assert!(!map.master, "the master a lane moved is rebuilt from the text");
    let names: Vec<String> = {
        let e = SongEngine::from_source(&b).unwrap();
        (0..e.instrument_count()).map(|i| e.instrument_name(i).to_string()).collect()
    };
    for (i, n) in names.iter().enumerate() {
        match n.as_str() {
            "low" => assert_eq!(map.instruments[i], None, "the module a lane moved is rebuilt"),
            _ => assert!(map.instruments[i].is_some(), "{n} did not move and keeps its voices"),
        }
    }
    assert!(!map.lanes, "a set's steps never carry a lane's progress");

    // Heard: the second step as open as the same text played on its own.
    let mut alone = SongEngine::from_source(&b).unwrap();
    alone.set_output_gain(1.0);
    alone.start();
    let alone = alone.render_steps(16 * 3).0;
    let closed = top_db(bar(&audio, 2));
    let after = top_db(bar(&audio, 4));
    let reference = top_db(bar(&alone, 1));
    eprintln!("closed {closed:.1} after {after:.1} reference {reference:.1}");
    assert!(after - closed > 5.0, "the top did not come back: {closed:.1} then {after:.1} dB");
    assert!((after - reference).abs() < 1.5, "step two plays {after:.1} dB of top, on its own {reference:.1}");
}

/// In `tatum watch`, a save that leaves a lane alone keeps its progress; one
/// that changes it goes through the swap and starts it again.
#[test]
fn a_save_keeps_an_unchanged_lanes_progress() {
    let first = with("auto bass level 0 > 1 over 4");
    let level_at = |edit: &str, restart: bool| {
        let mut planner = LivePlanner::new();
        if restart {
            planner.restart_lanes();
        }
        let mut player = LivePlayer::new();
        player.apply(planner.plan(&first, player.generation()).unwrap());
        player.start();
        let (mut l, mut r) = ([0.0f32; BLOCK_SIZE], [0.0f32; BLOCK_SIZE]);
        let mut pos = 0;
        let mut kind = "";
        while pos < 3 * BAR {
            if kind.is_empty() && pos >= BAR + BAR / 2 {
                let plan = planner.plan(edit, player.generation()).unwrap();
                kind = plan.describe();
                player.apply(plan);
            }
            player.process(&mut l, &mut r);
            while player.take_retired().is_some() {}
            pos += BLOCK_SIZE;
        }
        let e = player.engine().unwrap();
        let i = (0..e.track_count()).position(|i| e.track_name(i) == "bass").unwrap();
        (kind, e.track_level(i))
    };
    // The edit lands on bar 2; the reading is taken just after bar 3 starts.
    let pattern_edit = first.replace("1.5 - 1.3 - 1.1 -", "1.5 - 1.5 - 1.1 -");
    let (kind, level) = level_at(&pattern_edit, false);
    assert_eq!(kind, "swap");
    assert!((level - 0.75).abs() < 0.05, "an unchanged lane kept going: {level}");

    let lane_edit = first.replace("0 > 1 over 4", "0 > 1 over 8");
    let (kind, level) = level_at(&lane_edit, false);
    assert_eq!(kind, "swap", "an `auto` edit is never a value edit");
    assert!((level - 1.0 / 8.0).abs() < 0.05, "a changed lane starts on the swap bar: {level}");

    // A value edit on the fast path leaves the lane where it is...
    let level_edit =
        first.replace("track kick { play beat using kit level 0.8", "track kick { play beat using kit level 0.7");
    let (kind, level) = level_at(&level_edit, false);
    assert_eq!(kind, "fast");
    assert!((level - 0.75).abs() < 0.05, "{level}");
    // ...but a set's next step starts its lanes over, so it swaps.
    let (kind, level) = level_at(&level_edit, true);
    assert_eq!(kind, "swap");
    assert!((level - 0.25).abs() < 0.05, "{level}");
}

/// Two filters on the master, DJ style: `auto master dj cutoff` reaches the
/// one named `dj`, as a knob mapped to `master dj cutoff` does, where a bare
/// `auto master cutoff` would take the first filter in the chain.
#[test]
fn a_lane_reaches_a_named_master_node() {
    let dj = RIG.replace("lowpass(20000, 0.7) > out", "highpass(20, 0.7) as hp > lowpass(20000, 0.7) as dj > out");
    let render = |src: &str| {
        let mut e = SongEngine::from_source(src).unwrap_or_else(|e| panic!("{e}"));
        e.set_output_gain(1.0);
        e.start();
        e.render_steps(16 * 4).0
    };
    let open = top_db(bar(&render(&dj), 3));
    let named = top_db(bar(&render(&format!("{dj}\nauto master dj cutoff 20000 > 300 over 2\n")), 3));
    let first = top_db(bar(&render(&format!("{dj}\nauto master cutoff 20000 > 300 over 2\n")), 3));
    let fixed = top_db(bar(&render(&dj.replace("lowpass(20000, 0.7) as dj", "lowpass(300, 0.7) as dj")), 3));
    assert!(open - named > 5.0, "the lane did not reach `dj`: {open:.1} then {named:.1} dB");
    assert!((named - fixed).abs() < 0.5, "`dj` ends where it is written at 300: {named:.1} against {fixed:.1} dB");
    assert!(first - named > 5.0, "a bare `master cutoff` moves the first filter, the highpass: {first:.1} dB");

    // In a scene too, and a name the chain does not have is an error.
    let scene = format!("{dj}\nscene a {{\n auto master dj cutoff 20000 > 300\n track kick {{ play beat using kit }}\n}}\narrange {{ a x2 }}\n");
    assert!(SongEngine::from_source(&scene).is_ok());
    let err = SongEngine::from_source(&format!("{dj}\nauto master deejay cutoff 20000 > 300 over 2\n")).err().unwrap();
    assert!(err.contains("no node named 'deejay'"), "{err}");
}
