//! The pitch strip and the mod wheel, scene by scene and by where the hands
//! are: a line with a zone answers while a key of that zone is the newest
//! held, `always` whatever the hands do, `idle` only with the hands off the
//! keys. A gesture ends on the lines it started on.

use tatum_core::dsl;
use tatum_core::live::{FastOp, LivePlanner, LivePlayer, Plan};
use tatum_core::song_engine::SongEngine;

const SONG: &str = r#"
tempo 120
scale E phrygian
module bass sub { cutoff 0.5 sustain 0.8 }
module fm lead { mod_index 0.3 }
module fm zap { mod_index 0.5 }
pattern rest { -*16 }
track bass { play rest using sub level 0.8 out > master }
track lead { play rest using lead level 0.6 out > lowpass(2khz, 0.2) as lp > vowel(a, wet=0) as talk > master }
track zap  { play rest using zap level 0.6 out > bitcrush(6, 0.3, wet=0) as crush > master }
master { in > lowpass(20khz, 0.05) as dj > out }
midi {
    zone bass 48..59 > bass
    zone lead 60..84 > lead
    cc 1 > lead lp cutoff 2khz..10khz
}
perform sing {
    lead > lead
    bend lead 2deg
    wheel lead > lead talk wet 0%..100%
    wheel lead > lead talk position 0..1
}
perform dive {
    lead > zap
    bend lead 24st
    bend bass 12st idle
    wheel lead > zap crush wet 0%..80%
    wheel idle > master dj cutoff 20khz..800hz
}
perform still {
    bend lead off
}
"#;

fn session() -> (LivePlanner, LivePlayer) {
    session_of(SONG)
}

fn session_of(src: &str) -> (LivePlanner, LivePlayer) {
    let mut planner = LivePlanner::new();
    planner.set_output_gain(1.0);
    let mut player = LivePlayer::new();
    player.apply(planner.plan(src, player.generation()).expect("compiles"));
    player.start();
    (planner, player)
}

/// The song with its scenes cut off: what the `midi` block alone does.
fn without_scenes() -> String {
    SONG.split("perform sing").next().unwrap().to_string()
}

#[test]
fn a_performance_starts_in_its_first_scene() {
    let (planner, _) = session();
    assert_eq!(planner.scene(), Some("sing"));
    let (planner, _) = session_of(&without_scenes());
    assert_eq!(planner.scene(), None);
}

fn ops(plans: Vec<Plan>) -> Vec<FastOp> {
    plans
        .into_iter()
        .filter_map(|p| match p {
            Plan::Control { op, .. } | Plan::Play { op, .. } => Some(op),
            _ => None,
        })
        .collect()
}

/// The semitones of the bend on `instrument`, from the ops.
fn bent(ops: &[FastOp], instrument: usize) -> Option<f32> {
    ops.iter().rev().find_map(|op| match op {
        FastOp::PitchBend { instrument: i, ratio } if *i == instrument => Some(12.0 * ratio.log2()),
        _ => None,
    })
}

const UP: u16 = 16383;
const DOWN: u16 = 0;

fn n(name: &str) -> u8 {
    tatum_core::dsl::compiler::note_name_to_midi(name)
}

#[test]
fn bend_and_wheel_lines_parse_and_check() {
    let song = dsl::parse(SONG).unwrap();
    let sing = &song.perform.scenes[0];
    assert_eq!(sing.bends[0].range, dsl::ast::BendRange::Degrees(2));
    assert_eq!(sing.knobs.len(), 2);
    assert!(sing.knobs.iter().all(|k| k.map.source == dsl::ast::MidiSource::Cc(1)));
    assert!(sing.knobs.iter().all(|k| k.hands == dsl::ast::Hands::Playing(dsl::ast::ZoneKind::Lead)));
    let dive = &song.perform.scenes[1];
    assert_eq!(dive.bends[0].range, dsl::ast::BendRange::Semitones(24.0));
    assert_eq!(dive.bends[0].hands, dsl::ast::Hands::Playing(dsl::ast::ZoneKind::Lead));
    assert_eq!(dive.bends[1].hands, dsl::ast::Hands::Idle);
    assert_eq!(dive.knobs[1].hands, dsl::ast::Hands::Idle);
    assert!(SongEngine::from_source(SONG).is_ok());
    for (extra, says) in [
        ("perform talky {\n  wheel > lead nothing wet\n}\n", "no node named 'nothing'"),
        ("midi {\n  zone triggers 36..47\n}\nperform talky {\n  wheel lead > lead lp cutoff\n}\n", ""),
        ("perform talky {\n  wheel > lead talk position 0..2\n}\n", "outside 0..1"),
        ("midi {\n  zone triggers 36..47\n}\nperform talky {\n  bend bass 2st\n}\n", ""),
    ] {
        let r = SongEngine::from_source(&format!("{SONG}{extra}"));
        if says.is_empty() {
            assert!(r.is_ok(), "{extra}: {:?}", r.err());
        } else {
            let err = r.err().unwrap_or_else(|| panic!("{extra} passed"));
            assert!(err.contains(says), "{extra}: {err}");
        }
    }
    for (extra, says) in [
        ("midi {\n  bend lead 60st\n}\n", "up to 48"),
        ("midi {\n  bend lead 9deg\n}\n", "from 1 to 7"),
        ("midi {\n  bend triggers 2st\n}\n", "bass or lead"),
    ] {
        let errs = dsl::parse(&format!("{SONG}{extra}")).err().unwrap_or_else(|| panic!("{extra} parsed"));
        assert!(errs.iter().any(|e| e.message.contains(says)), "{extra}: {errs:?}");
    }
}

fn press(planner: &mut LivePlanner, player: &mut LivePlayer, note: u8, velocity: u8) -> Vec<FastOp> {
    let g = player.generation();
    let plans = planner.key(note, velocity, g).unwrap_or_default();
    let out = plans
        .iter()
        .filter_map(|p| match p {
            Plan::Control { op, .. } | Plan::Play { op, .. } => Some(*op),
            _ => None,
        })
        .collect();
    for p in plans {
        player.apply(p);
    }
    out
}

#[test]
fn with_no_scene_the_lead_bends_two_semitones_while_played() {
    let (mut planner, mut player) = session_of(&without_scenes());
    let g = player.generation();
    // Hands off: nothing answers.
    let up = ops(planner.bend(UP, g));
    assert!(bent(&up, 1).unwrap().abs() < 1e-4, "{up:?}");
    planner.bend(8192, g);
    press(&mut planner, &mut player, n("E4"), 100);
    let up = ops(planner.bend(UP, g));
    assert!((bent(&up, 1).unwrap() - 2.0).abs() < 0.01, "{up:?}");
    assert!(bent(&up, 0).unwrap().abs() < 1e-4, "the bass zone is left alone: {up:?}");
    let rest = ops(planner.bend(8192, g));
    assert!(bent(&rest, 1).unwrap().abs() < 1e-4);
}

#[test]
fn played_the_strip_dives_the_sound_idle_it_drops_the_song() {
    let (mut planner, mut player) = session();
    let g = player.generation();
    planner.enter_scene("dive", g).unwrap();
    // A key of the lead zone held: the zap dives, the bass line stays.
    press(&mut planner, &mut player, n("E4"), 100);
    assert_eq!(planner.hands(), Some(tatum_core::dsl::ast::ZoneKind::Lead));
    let down = ops(planner.bend(DOWN, g));
    assert!((bent(&down, 2).unwrap() + 24.0).abs() < 0.01, "the zap: {down:?}");
    assert!(bent(&down, 0).unwrap().abs() < 1e-4, "the bass is not idle's yet: {down:?}");
    // Let go of the key mid-gesture: the gesture ends on what it started on.
    press(&mut planner, &mut player, n("E4"), 0);
    let still = ops(planner.bend(DOWN + 100, g));
    assert!(bent(&still, 0).unwrap().abs() < 1e-4, "the bass jumped mid-gesture: {still:?}");
    // Back to rest, then away again with the hands off: now the song.
    planner.bend(8192, g);
    let down = ops(planner.bend(DOWN, g));
    assert!((bent(&down, 0).unwrap() + 12.0).abs() < 0.01, "the bass line drops: {down:?}");
    assert!(bent(&down, 2).unwrap().abs() < 1e-4, "{down:?}");
    // A key pressed while the song is bent does not take the strip over.
    let pressed = press(&mut planner, &mut player, n("G4"), 100);
    assert!(bent(&pressed, 2).is_none_or(|st| st.abs() < 1e-4), "{pressed:?}");
    let more = ops(planner.bend(DOWN + 200, g));
    assert!(bent(&more, 0).unwrap() < -11.0 && bent(&more, 2).unwrap().abs() < 1e-4, "{more:?}");
    // `off`: the strip leaves the lead alone.
    planner.bend(8192, g);
    let plans = planner.enter_scene("still", g).unwrap();
    assert!(bent(&ops(plans), 1).is_none_or(|st| st.abs() < 1e-4));
}

#[test]
fn a_bend_in_degrees_lands_in_the_scale_from_the_note_held() {
    let (mut planner, mut player) = session();
    let g = player.generation();
    planner.enter_scene("sing", g).unwrap();
    press(&mut planner, &mut player, n("E4"), 100);
    // E phrygian: two degrees up from E is G, three semitones; down, C,
    // four below.
    let up = ops(planner.bend(UP, g));
    assert!((bent(&up, 1).unwrap() - 3.0).abs() < 0.01, "{up:?}");
    let down = ops(planner.bend(DOWN, g));
    assert!((bent(&down, 1).unwrap() + 4.0).abs() < 0.01, "{down:?}");
    // Half way is half the reach: it slides there, and only the end is in
    // the scale.
    let half = ops(planner.bend(8192 + 4096, g));
    assert!((bent(&half, 1).unwrap() - 1.5).abs() < 0.01, "{half:?}");
    // A new key while the strip is held re-aims the bend from its note:
    // from F, two degrees up is A, four semitones.
    planner.bend(UP, g);
    let moved = press(&mut planner, &mut player, n("F4"), 100);
    assert!((bent(&moved, 1).unwrap() - 4.0).abs() < 0.01, "{moved:?}");
}

fn wets(ops: &[FastOp], track: usize) -> Vec<f32> {
    ops.iter()
        .filter_map(|op| match op {
            FastOp::NodeWet { track: t, wet, .. } if *t == track => Some(*wet),
            _ => None,
        })
        .collect()
}

fn master_cutoff(ops: &[FastOp]) -> Option<f32> {
    ops.iter().find_map(|op| match op {
        FastOp::NodeParam { track: None, param: "cutoff", value, .. } => Some(*value),
        _ => None,
    })
}

#[test]
fn the_wheel_moves_what_the_scene_and_the_hands_say() {
    // No scene: the `midi` block's cc 1, the lead's low-pass, whatever the hands.
    let (mut planner, player) = session_of(&without_scenes());
    let turn = planner.knob(1, 127, player.generation());
    assert_eq!(turn.readings, ["lead lp cutoff 10khz"]);

    let (mut planner, mut player) = session();
    let g = player.generation();

    // `sing`, a key held: the wheel makes the lead talk, both lines at once.
    planner.enter_scene("sing", g).unwrap();
    press(&mut planner, &mut player, n("B4"), 100);
    let turn = planner.knob(1, 64, g);
    assert_eq!(turn.readings.len(), 2, "{:?}", turn.readings);
    assert!(turn.readings[0].starts_with("lead talk wet"), "{:?}", turn.readings);
    assert!(turn.readings[1].starts_with("lead talk position i"), "{:?}", turn.readings);
    assert!((wets(&ops(turn.plans), 1)[0] - 64.0 / 127.0).abs() < 1e-3);
    press(&mut planner, &mut player, n("B4"), 0);
    planner.knob(1, 0, g);

    // `dive`: a key held, the crusher; hands off, the song's filter.
    planner.enter_scene("dive", g).unwrap();
    press(&mut planner, &mut player, n("E4"), 100);
    let held = ops(planner.knob(1, 127, g).plans);
    assert!((wets(&held, 2)[0] - 0.8).abs() < 1e-3, "{held:?}");
    assert!(master_cutoff(&held).is_none(), "the song moved while a key was held: {held:?}");
    // Let go mid-gesture: the crusher stays the wheel's until it comes home.
    press(&mut planner, &mut player, n("E4"), 0);
    let still = ops(planner.knob(1, 100, g).plans);
    assert!(!wets(&still, 2).is_empty() && master_cutoff(&still).is_none(), "{still:?}");
    let home = ops(planner.knob(1, 0, g).plans);
    assert_eq!(wets(&home, 2), [0.0], "the crusher goes home: {home:?}");
    assert_eq!(master_cutoff(&home), Some(20000.0), "and the idle line is at rest: {home:?}");
    // Hands off from the start: the song's filter closes.
    let idle = planner.knob(1, 127, g);
    assert_eq!(idle.readings, ["master dj cutoff 800hz"], "{:?}", idle.readings);
    assert!(wets(&ops(idle.plans), 2).is_empty());

    // Into `sing` with the wheel up and no key: the idle filter goes back,
    // and sing's lines, which want a key, stay where they are.
    let plans = ops(planner.enter_scene("sing", g).unwrap());
    assert_eq!(master_cutoff(&plans), Some(20000.0), "{plans:?}");
    assert!(wets(&plans, 1).is_empty(), "{plans:?}");
    let waiting = planner.knob(1, 120, g);
    assert_eq!(waiting.readings, ["cc 1: this scene moves it with lead, lead"], "{:?}", waiting.readings);
}

/// The wheel walking the vowel in a real engine: the lead keeps sounding
/// and nothing clicks while the wheel is thrown across its travel.
#[test]
fn a_thrown_wheel_on_a_held_note_does_not_click() {
    let (mut planner, mut player) = session();
    let g = player.generation();
    planner.enter_scene("sing", g).unwrap();
    press(&mut planner, &mut player, n("B4"), 110);
    let (mut l, mut r) = ([0.0f32; 64], [0.0f32; 64]);
    let mut out = Vec::new();
    for block in 0..1400 {
        // Up over 64 samples, back down at once: as rough as a hand gets.
        if block == 400 {
            for p in planner.knob(1, 127, g).plans {
                player.apply(p);
            }
        }
        if block == 900 {
            for p in planner.knob(1, 0, g).plans {
                player.apply(p);
            }
        }
        player.process(&mut l, &mut r);
        out.extend_from_slice(&l);
    }
    let level = out[20000..].iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(level > 0.01, "the held note went quiet");
    let worst = out.windows(2).skip(20000).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
    assert!(worst < level * 0.6, "a jump of {worst} against a level of {level}");
}
