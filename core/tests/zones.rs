//! The keyboard split into zones: trigger keys that act like pads, a bass
//! zone that rolls, a lead zone kept to the scale, and scenes that change
//! all of it on a bar line.

use tatum_core::dsl;
use tatum_core::live::{FastOp, LivePlanner, LivePlayer, Plan};
use tatum_core::song_engine::SongEngine;
use tatum_core::{BLOCK_SIZE, SAMPLE_RATE};

/// 120 BPM: a sixteenth is 5512.5 samples, a bar 88200.
const STEP: f64 = SAMPLE_RATE as f64 * 60.0 / 120.0 / 4.0;
const BAR: f64 = STEP * 16.0;

const SONG: &str = r#"
tempo 120
scale E phrygian
module bass pluck { cutoff 0.5 decay 0.08 sustain 0.0 release 0.01 }
module bass drone { cutoff 0.6 sustain 1.0 }
module fm lead { mod_index 0.3 }
module beats kit { kick_level 1.0 }
pattern rest { -*16 }
pattern beat { kick: X - - - X - - - X - - - X - - - }
track bass  { play rest using pluck level 0.8 gate 0.4 out > master }
track drone { play rest using drone level 0.6 out > master }
track lead  { play rest using lead level 0.6 out > master }
track drums { play beat using kit level 0.8 out > master }
midi {
    zone triggers 36..47
    zone bass 48..59 > bass roll
    zone lead 60..96 > lead
    lock snap
    key 36 > mute drums
    key 37 > hold drone q=bar
}
perform drop {
    bass > bass roll kick=drums
    lead > lead
    set drone level 0.2
}
perform raga {
    scale E phrygian_dominant
    lock white
    lead > drone
}
keyboard {
    f1 > perform drop
    f2 > perform raga
    space > next
    quantize bar
}
"#;

fn session(src: &str) -> (LivePlanner, LivePlayer) {
    let mut planner = LivePlanner::new();
    planner.set_output_gain(1.0);
    let mut player = LivePlayer::new();
    player.apply(planner.plan(src, player.generation()).expect("compiles"));
    player.start();
    (planner, player)
}

fn run(player: &mut LivePlayer, samples: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(samples);
    let (mut l, mut r) = ([0.0f32; BLOCK_SIZE], [0.0f32; BLOCK_SIZE]);
    while out.len() < samples {
        let n = BLOCK_SIZE.min(samples - out.len());
        player.process(&mut l[..n], &mut r[..n]);
        out.extend_from_slice(&l[..n]);
        while player.take_retired().is_some() {}
    }
    out
}

fn key(planner: &mut LivePlanner, player: &mut LivePlayer, note: u8, velocity: u8) -> Vec<FastOp> {
    let plans = planner.key(note, velocity, player.generation()).unwrap_or_default();
    let mut ops = Vec::new();
    for p in plans {
        match &p {
            Plan::Play { op, .. } | Plan::Control { op, .. } => ops.push(*op),
            _ => {}
        }
        player.apply(p);
    }
    ops
}

fn notes_on(ops: &[FastOp]) -> Vec<u8> {
    ops.iter().filter_map(|op| if let FastOp::NoteOn { note, .. } = op { Some(*note) } else { None }).collect()
}

fn n(name: &str) -> u8 {
    tatum_core::dsl::compiler::note_name_to_midi(name)
}

#[test]
fn the_blocks_parse_and_redefine() {
    let song = dsl::parse(SONG).unwrap();
    let p = &song.perform;
    assert_eq!(p.zones.len(), 3);
    assert_eq!((p.zones[1].low, p.zones[1].high), (48, 59));
    assert!(p.zones[1].play.as_ref().unwrap().roll);
    assert_eq!(p.scenes.len(), 2);
    assert_eq!(p.scenes[0].bass.as_ref().unwrap().kick.as_deref(), Some("drums"));
    assert_eq!(p.scenes[1].scale.as_ref().unwrap().kind, "phrygian_dominant");
    assert_eq!(p.keyboard.len(), 3);
    assert_eq!(p.scene_bars, Some(1));
    assert!(SongEngine::from_source(SONG).is_ok());

    // A later block replaces the zone of the same kind, the scene of the same
    // name and the binding of the same key, and leaves the rest.
    let later = format!(
        "{SONG}\nmidi {{\n  zone lead C5..C7 > drone\n}}\nperform drop {{\n  lead > drone\n}}\nkeyboard {{\n  f1 > perform raga\n  quantize phrase\n}}\n"
    );
    let song = dsl::parse(&later).unwrap();
    let p = &song.perform;
    assert_eq!(p.zones.len(), 3);
    let lead = p.zones.iter().find(|z| z.kind == dsl::ast::ZoneKind::Lead).unwrap();
    assert_eq!((lead.low, lead.high, lead.play.as_ref().unwrap().track.as_str()), (72, 96, "drone"));
    let drop = p.scenes.iter().find(|s| s.name == "drop").unwrap();
    assert!(drop.bass.is_none() && drop.sets.is_empty(), "a scene is replaced whole");
    assert_eq!(p.keyboard.iter().find(|b| b.key == "f1").unwrap().action, dsl::ast::KeyAction::Perform("raga".into()));
    assert_eq!(p.scene_bars, Some(8));
}

#[test]
fn check_catches_what_would_fail_on_stage() {
    for (extra, says) in [
        ("midi {\n  zone lead 55..70 > lead\n}\n", "overlaps zone bass"),
        ("midi {\n  zone bass 48..59 > nobody\n}\n", "no track named 'nobody'"),
        ("midi {\n  zone bass 48..59 > drums\n}\n", "plays drums"),
        ("midi {\n  key 40 > next\n}\n", "computer's keys"),
        ("midi {\n  key 60 > mute drums\n}\n", "outside the trigger zone"),
        ("perform nope {\n  bass > bass roll kick=drone\n}\n", "not a drum kit"),
        ("perform nope {\n  scale E dorian_flat9\n}\n", "one of major"),
        ("perform nope {\n  set drone level 9\n}\n", "a level runs from 0"),
        ("keyboard {\n  f5 > perform nowhere\n}\n", "no such scene"),
        ("keyboard {\n  q > perform drop\n}\n", "q quits"),
        ("scale E dorian_flat9\n", "is not a scale"),
    ] {
        let err = SongEngine::from_source(&format!("{SONG}{extra}")).err().unwrap_or_else(|| panic!("{extra} passed"));
        assert!(err.contains(says), "{extra}: {err}");
    }
    for (extra, says) in [
        ("midi {\n  zone bass 48..40\n}\n", "runs downwards"),
        ("midi {\n  zone lead 60..96 > lead roll\n}\n", "`roll` is for the bass zone"),
        ("midi {\n  lock chromatic\n}\n", "lock: snap"),
        ("keyboard {\n  f13 > next\n}\n", "a key is a letter"),
    ] {
        let errs = dsl::parse(&format!("{SONG}{extra}")).err().unwrap_or_else(|| panic!("{extra} parsed"));
        assert!(errs.iter().any(|e| e.message.contains(says)), "{extra}: {errs:?}");
    }
}

#[test]
fn the_lead_zone_keeps_to_the_scale() {
    let (mut planner, mut player) = session(SONG);
    // E phrygian: F# is out and snaps down to F; G is in.
    assert_eq!(notes_on(&key(&mut planner, &mut player, n("F#4"), 100)), [n("F4")]);
    assert_eq!(notes_on(&key(&mut planner, &mut player, n("G4"), 100)), [n("G4")]);
    // F and F# both sound F: it stops when the last of them lets go.
    assert_eq!(notes_on(&key(&mut planner, &mut player, n("F4"), 100)), [n("F4")]);
    let up = key(&mut planner, &mut player, n("F#4"), 0);
    assert!(up.is_empty(), "F still held: {up:?}");
    let up = key(&mut planner, &mut player, n("F4"), 0);
    assert!(matches!(up[..], [FastOp::NoteOff { note, .. }] if note == n("F4")), "{up:?}");
}

#[test]
fn a_scene_brings_its_scale_and_its_lock() {
    let (mut planner, mut player) = session(SONG);
    let g = player.generation();
    planner.enter_scene("raga", g).unwrap();
    assert_eq!(planner.scene(), Some("raga"));
    // White keys are the degrees of E phrygian dominant, on the drone now.
    let ops = key(&mut planner, &mut player, n("C4"), 100);
    assert!(matches!(ops[..], [FastOp::NoteOn { note, track, .. }] if note == n("E4") && track == 1), "{ops:?}");
    assert_eq!(notes_on(&key(&mut planner, &mut player, n("E4"), 100)), [n("G#4")], "the major third");
    assert!(key(&mut planner, &mut player, n("C#4"), 100).is_empty(), "a black key plays nothing");
    // A key held through a scene change sounds on where it started, and
    // lets go there: the drone, not the drop's lead.
    let plans = planner.enter_scene("drop", player.generation()).unwrap();
    assert!(plans.is_empty(), "nothing is cut on the line");
    // The drone is a `bass` module, one note at a time: letting go of the
    // newest falls back to the key under it, and the last one lets go.
    let up = key(&mut planner, &mut player, n("E4"), 0);
    assert!(matches!(up[..], [FastOp::NoteOn { note, track: 1, .. }] if note == n("E4")), "{up:?}");
    let up = key(&mut planner, &mut player, n("C4"), 0);
    assert!(matches!(up[..], [FastOp::NoteOff { note, track: 1 }] if note == n("E4")), "{up:?}");
    // The next key plays the drop's lead, in E phrygian again, snapped.
    let ops = key(&mut planner, &mut player, n("G#4"), 100);
    assert!(matches!(ops[..], [FastOp::NoteOn { note, track: 2, .. }] if note == n("G4")), "{ops:?}");
    assert!(planner.enter_scene("nowhere", player.generation()).is_none());
}

#[test]
fn a_trigger_key_does_what_a_pad_does() {
    let (mut planner, mut player) = session(SONG);
    run(&mut player, 1000);
    key(&mut planner, &mut player, 36, 100);
    assert!(player.engine().unwrap().track_muted(3), "the drums are out while the key is held");
    key(&mut planner, &mut player, 36, 0);
    assert!(!player.engine().unwrap().track_muted(3));
    // A key the trigger zone has nothing on plays nothing.
    assert!(planner.key(40, 100, player.generation()).is_none());
    // The drone, held out until its key is: `hold` with `q=bar`.
    assert!(player.engine().unwrap().track_muted(1));
}

/// Where the rolled bass sounds: the loudest stretch of each sixteenth.
fn per_step(out: &[f32], from: f64, steps: usize) -> Vec<f32> {
    (0..steps)
        .map(|k| {
            let a = (from + k as f64 * STEP) as usize;
            let b = (a + (STEP * 0.5) as usize).min(out.len());
            out[a..b].iter().fold(0.0f32, |m, v| m.max(v.abs()))
        })
        .collect()
}

#[test]
fn a_held_bass_key_rolls_on_the_offbeat_sixteenths_and_stops_when_let_go() {
    let quiet =
        SONG.replace("track drums { play beat using kit level 0.8", "track drums { play beat using kit level 0");
    let (mut planner, mut player) = session(&quiet);
    let mut out = run(&mut player, (BAR * 0.5 + STEP * 0.3) as usize);
    // Held a third of the way into a sixteenth: the roll starts on the next.
    let ops = key(&mut planner, &mut player, n("F#3"), 110);
    assert!(
        matches!(ops[..], [FastOp::Roll { note, velocity, track: 0, kick: None }] if note == n("F3") && velocity > 0.8),
        "{ops:?}"
    );
    assert_eq!(player.engine().unwrap().track_roll(0), Some(n("F3")));
    out.extend(run(&mut player, (BAR * 1.5) as usize));
    let from = BAR * 0.5 + STEP; // the first sixteenth after the press, a beat's second
    let levels = per_step(&out, from, 15);
    let loudest = levels.iter().cloned().fold(0.0f32, f32::max);
    assert!(loudest > 0.01, "the roll is silent: {levels:?}");
    for (k, v) in levels.iter().enumerate() {
        // `from` is the second sixteenth of a beat: every fourth after it
        // (k = 3, 7, 11) is a beat, left to the kick.
        if k % 4 == 3 {
            assert!(*v < loudest * 0.35, "sixteenth {k} is on the beat and sounded {v} ({levels:?})");
        } else {
            assert!(*v > loudest * 0.5, "sixteenth {k} should roll ({levels:?})");
        }
    }
    // Let go: the pattern, all rests, plays on.
    key(&mut planner, &mut player, n("F#3"), 0);
    assert_eq!(player.engine().unwrap().track_roll(0), None);
    run(&mut player, (STEP * 2.0) as usize);
    let tail = run(&mut player, BAR as usize);
    assert!(tail.iter().all(|v| v.abs() < 1e-3), "still rolling after the key let go");
}

#[test]
fn a_roll_in_a_scene_with_a_kick_strikes_it_on_the_beat() {
    let quiet = SONG.replace("pattern beat { kick: X - - - X - - - X - - - X - - - }", "pattern beat { kick: -*16 }");
    let (mut planner, mut player) = session(&quiet);
    planner.enter_scene("drop", player.generation()).unwrap();
    let ops = key(&mut planner, &mut player, n("E3"), 100);
    assert!(matches!(ops[..], [FastOp::Roll { kick: Some((3, 36)), .. }]), "{ops:?}");
    let out = run(&mut player, (BAR * 1.25) as usize);
    // The drums' own pattern is all rests: whatever sounds on the downbeats
    // is the roll's kick.
    let beats = per_step(&out, BAR, 4);
    assert!(beats[0] > 0.05, "no kick on the downbeat: {beats:?}");
}

#[test]
fn a_scene_from_the_keys_lands_its_values_on_the_bar() {
    let (mut planner, mut player) = session(SONG);
    run(&mut player, (BAR * 0.4) as usize);
    let drone = 1;
    let before = player.engine().unwrap().track_level(drone);
    let plans = planner.scene_values("drop", Some(1), player.generation()).unwrap();
    assert!(plans.iter().all(|p| matches!(p, Plan::AtBar { bar: 1, .. })));
    for p in plans {
        player.apply(p);
    }
    run(&mut player, (BAR * 0.59) as usize);
    assert_eq!(player.engine().unwrap().track_level(drone), before, "the level moved before the bar");
    let mut moved_at = None;
    let (mut l, mut r) = ([0.0f32; 16], [0.0f32; 16]);
    let mut at = (BAR * 0.99) as usize;
    while at < (BAR * 1.1) as usize {
        player.process(&mut l, &mut r);
        at += 16;
        if moved_at.is_none() && (player.engine().unwrap().track_level(drone) - 0.2).abs() < 1e-4 {
            moved_at = Some(at);
        }
    }
    let moved_at = moved_at.expect("the scene's level never landed") as f64;
    assert!((moved_at - BAR).abs() <= 32.0, "landed at {moved_at}, the bar is {BAR}");
}

#[test]
fn a_rebuilt_engine_keeps_rolling() {
    let (mut planner, mut player) = session(SONG);
    run(&mut player, 2000);
    key(&mut planner, &mut player, n("A3"), 100);
    let edited = SONG.replace("module fm lead { mod_index 0.3 }", "module fm lead { mod_index 0.4 feedback 0.1 }\nmodule fm extra { mod_index 0.2 }\ntrack extra { play rest using extra level 0.3 out > master }");
    let plan = planner.plan(&edited, player.generation()).unwrap();
    assert!(matches!(plan, Plan::Swap { .. }));
    player.apply(plan);
    run(&mut player, (BAR * 1.2) as usize);
    assert!(player.swaps() >= 1);
    let e = player.engine().unwrap();
    // The new track comes first in the text: the bass is no longer track 0.
    let bass = (0..e.track_count()).find(|&t| e.track_name(t) == "bass").unwrap();
    assert_eq!(bass, 1);
    assert_eq!(e.track_roll(bass), Some(n("A3")));
}
