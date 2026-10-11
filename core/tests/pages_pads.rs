//! The knobs' pages, soft takeover, the output effects on pads and keys,
//! and a track's name standing for the module it plays.

use tatum_core::dsl::ast::VoiceMove;
use tatum_core::live::{FastOp, LivePlanner, LivePlayer, Plan};
use tatum_core::perform::fx::OutFx;
use tatum_core::song_engine::SongEngine;
use tatum_core::{BLOCK_SIZE, SAMPLE_RATE};

const SONG: &str = r#"
tempo 120
scale E phrygian
module bass roller { cutoff 0.4 resonance 0.3 sustain 0.6 }
module beats kit { kick_level 1.0 kick_decay 0.3 }
module fm lead_v { mod_index 0.3 }
pattern line { - 1.1 1.1 1.1  - 1.1 1.1 1.1  - 1.1 1.1 1.1  - 1.1 1.1 1.1 }
pattern beat { kick: X - - -  X - - -  X - - -  X - - - }
pattern rest { -*16 }
track bass  { play line using roller level 0.8 out > master }
track drums { play beat using kit level 0.9 out > master }
track lead  { play rest using lead_v level 0.5 out > master }
master { in > lowpass(20khz, 0.05) as dj > gain(1.0) as vol > out }
midi {
    zone lead 60..84 > lead
    cc 74 > bass level 0..1.6
    cc 85 > master vol gain 0..1 guard
    takeover pickup
    page tema {
    }
    page bass {
        cc 74 > bass cutoff
        cc 71 > bass resonance
    }
    page drums {
        cc 74 > drums kick_decay
        cc 71 > nothing_here cutoff
    }
    cc 20 > voice step
    pad 39 > tapestop
    pad 40 > gate 1/16
    pad 41 > crush
    key 61 > cut
}
keyboard {
    tab > voice next
}
"#;

fn session() -> (LivePlanner, LivePlayer) {
    let mut planner = LivePlanner::new();
    planner.set_output_gain(1.0);
    let mut player = LivePlayer::new();
    player.apply(planner.plan(SONG, player.generation()).expect("compiles"));
    player.start();
    (planner, player)
}

fn ops(plans: &[Plan]) -> Vec<FastOp> {
    plans
        .iter()
        .filter_map(|p| match p {
            Plan::Control { op, .. } => Some(*op),
            _ => None,
        })
        .collect()
}

#[test]
fn a_track_name_reaches_the_module_it_plays() {
    // `bass cutoff`: no module is called bass; the track is, and plays `roller`.
    assert!(SongEngine::from_source(SONG).is_ok());
    let bad = SONG.replace("cc 74 > bass level 0..1.6", "cc 74 > bass level 0..1.6\n    cc 75 > bass nonsense");
    let err = SongEngine::from_source(&bad).err().unwrap();
    assert!(err.contains("nonsense"), "{err}");
}

#[test]
fn pages_change_what_the_knobs_move() {
    let (mut planner, player) = session();
    let g = player.generation();
    assert_eq!(planner.voice_name().as_deref(), Some("tema"));
    // On `tema` the `midi` block's line: the bass level. First touch at the
    // level the text writes (0.8 of 0..1.6, halfway): it takes at once.
    let turn = planner.knob(74, 64, g);
    assert_eq!(turn.readings, ["bass level -1.9 dB"], "{:?}", turn.readings);
    // Tab: the bass page, and K1 is its cutoff.
    let said = planner.voice(&VoiceMove::Next, g).unwrap();
    assert_eq!(said, "voice bass: cutoff · resonance");
    // The knob was left at 64 on `tema`; on `bass` the cutoff is where the
    // text has it, 0.4 (51 of 127): turning away from it does nothing.
    let turn = planner.knob(74, 70, g);
    assert!(turn.readings[0].starts_with("bass cutoff"), "{:?}", turn.readings);
    assert!(turn.plans.is_empty(), "jumped: {:?}", turn.readings);
    assert!(turn.readings[0].contains("turn ← to 51"), "{:?}", turn.readings);
    // Down through it, it takes over.
    let turn = planner.knob(74, 50, g);
    assert!(ops(&turn.plans).iter().any(|op| matches!(op, FastOp::ModuleParam { .. })));
    // A line naming what this track has not: shown, and does nothing.
    planner.voice(&VoiceMove::Next, g).unwrap();
    let turn = planner.knob(71, 90, g);
    assert_eq!(turn.readings, ["nothing_here cutoff (not in this track)"]);
    assert!(turn.plans.is_empty());
    // Round the pages to the start.
    assert!(planner.voice(&VoiceMove::Next, g).unwrap().starts_with("voice tema"));
    assert!(planner.voice(&VoiceMove::To("drums".into()), g).unwrap().starts_with("voice drums"));
    assert!(planner.voice(&VoiceMove::To("nope".into()), g).is_none());
}

#[test]
fn back_on_a_page_a_knob_waits_until_it_reaches_where_it_left_it() {
    let (mut planner, player) = session();
    let g = player.generation();
    planner.voice(&VoiceMove::To("bass".into()), g);
    // Taken at the text's cutoff (51), then left at 100.
    assert!(!planner.knob(74, 51, g).plans.is_empty());
    assert!(!planner.knob(74, 100, g).plans.is_empty());
    planner.voice(&VoiceMove::To("tema".into()), g);
    planner.knob(74, 20, g); // the level, from 20 on
    planner.knob(74, 30, g);
    planner.voice(&VoiceMove::To("bass".into()), g);
    // At 30, the cutoff is at 100: turning toward it does nothing yet.
    let turn = planner.knob(74, 60, g);
    assert!(turn.plans.is_empty(), "jumped: {:?}", turn.readings);
    assert!(turn.readings[0].contains("turn → to 100"), "{:?}", turn.readings);
    // Passing it takes over.
    let turn = planner.knob(74, 100, g);
    assert!(!turn.plans.is_empty(), "{:?}", turn.readings);
    let turn = planner.knob(74, 90, g);
    assert!(!turn.plans.is_empty());
}

#[test]
fn a_knob_never_touched_on_a_new_page_waits_for_the_text_value() {
    let (mut planner, player) = session();
    let g = player.generation();
    // Tab before touching anything: the knob's place is unknown, so its
    // first touch decides the side. The resonance is at 0.3 (38 of 127).
    planner.voice(&VoiceMove::To("bass".into()), g);
    let turn = planner.knob(71, 110, g);
    assert!(turn.plans.is_empty(), "jumped: {:?}", turn.readings);
    assert!(turn.readings[0].contains("turn ← to 38"), "{:?}", turn.readings);
    assert!(planner.knob(71, 60, g).plans.is_empty());
    assert!(!planner.knob(71, 38, g).plans.is_empty());
    // A first touch near the value takes at once.
    planner.voice(&VoiceMove::To("drums".into()), g);
    planner.voice(&VoiceMove::To("bass".into()), g);
    planner.reset_knobs(g);
    planner.voice(&VoiceMove::To("tema".into()), g);
    planner.voice(&VoiceMove::To("bass".into()), g);
    assert!(!planner.knob(74, 52, g).plans.is_empty());
}

#[test]
fn a_first_touch_away_from_the_text_waits_for_it() {
    let (mut planner, player) = session();
    let g = player.generation();
    // The master fader: the text has the gain at 1.0, the top. A fader
    // found at the bottom does not silence the room.
    let turn = planner.knob(85, 0, g);
    assert!(turn.plans.is_empty(), "the master jumped to silence");
    assert!(turn.readings[0].contains("turn → to 127"), "{:?}", turn.readings);
    let turn = planner.knob(85, 126, g);
    assert!(!turn.plans.is_empty(), "{:?}", turn.readings);
    // A knob without `guard` jumps on its first touch, as always: the bass
    // level, written at 0.8, taken from the top.
    let turn = planner.knob(74, 127, g);
    assert!(!turn.plans.is_empty(), "an unguarded first touch waited: {:?}", turn.readings);
}

#[test]
fn an_encoder_steps_through_the_pages() {
    let (mut planner, player) = session();
    let g = player.generation();
    let turn = planner.knob(20, 65, g);
    assert_eq!(turn.readings, ["voice bass: cutoff · resonance"]);
    planner.knob(20, 63, g);
    assert_eq!(planner.voice_name().as_deref(), Some("tema"));
}

#[test]
fn an_effect_pad_follows_the_strike_and_the_pressure() {
    let (mut planner, player) = session();
    let g = player.generation();
    let plans = planner.pad(39, 127, g).unwrap();
    assert!(matches!(plans[..], [Plan::Fx { fx: OutFx::TapeStop, on: true, amount }] if amount == 1.0));
    let plans = planner.pad(40, 64, g).unwrap();
    assert!(matches!(plans[..], [Plan::Fx { fx: OutFx::Gate(d), on: true, .. }] if d == 1.0));
    // Pressing harder raises it; pressing lighter never below the strike.
    let more = planner.pressure(true, Some(40), 127);
    assert!(matches!(more[..], [Plan::FxAmount { fx: OutFx::Gate(_), amount }] if amount == 1.0));
    let less = planner.pressure(true, Some(40), 10);
    assert!(matches!(less[..], [Plan::FxAmount { amount, .. }] if (amount - 64.0 / 127.0).abs() < 1e-4));
    // Channel aftertouch reaches every held pad; a key's does not.
    assert_eq!(planner.pressure(true, None, 100).len(), 2);
    assert!(planner.pressure(false, None, 100).is_empty());
    let up = planner.pad(39, 0, g).unwrap();
    assert!(matches!(up[..], [Plan::Fx { fx: OutFx::TapeStop, on: false, .. }]));
    assert_eq!(planner.pressure(true, None, 100).len(), 1);
    // A key of the trigger zone: the cut.
    let key = planner.key(61, 100, g).unwrap();
    assert!(matches!(key[..], [Plan::Fx { fx: OutFx::Cut, on: true, .. }]));
}

#[test]
fn a_held_tape_stop_silences_the_song_and_letting_go_brings_it_back() {
    let (mut planner, mut player) = session();
    let g = player.generation();
    let run = |player: &mut LivePlayer, n: usize| {
        let (mut l, mut r) = ([0.0f32; BLOCK_SIZE], [0.0f32; BLOCK_SIZE]);
        let mut out = Vec::new();
        while out.len() < n {
            player.process(&mut l, &mut r);
            out.extend_from_slice(&l);
        }
        out
    };
    let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
    let before = run(&mut player, SAMPLE_RATE as usize);
    for p in planner.pad(39, 127, g).unwrap() {
        player.apply(p);
    }
    let stopping = run(&mut player, SAMPLE_RATE as usize);
    for p in planner.pad(39, 0, g).unwrap() {
        player.apply(p);
    }
    let after = run(&mut player, SAMPLE_RATE as usize);
    let n = stopping.len();
    assert!(rms(&stopping[n / 2..]) < rms(&before) * 0.05, "the stop did not stop");
    assert!(rms(&after[4410..]) > rms(&before) * 0.5, "the song did not come back");
}

#[test]
fn reset_puts_every_touched_knob_back_to_the_text() {
    let (mut planner, player) = session();
    let g = player.generation();
    planner.knob(74, 127, g); // tema: the bass level, to the top
    planner.voice(&VoiceMove::To("bass".into()), g);
    planner.knob(74, 10, g); // bass: the cutoff, down
    let plans = planner.reset_knobs(g);
    let ops: Vec<FastOp> = plans
        .iter()
        .flat_map(|p| match p {
            Plan::Fast { ops, .. } => ops.clone(),
            _ => Vec::new(),
        })
        .collect();
    assert!(
        ops.iter().any(|op| matches!(op, FastOp::TrackLevel { track: 0, level } if (*level - 0.8).abs() < 1e-6)),
        "the bass level is not back at 0.8: {ops:?}"
    );
    assert!(
        ops.iter().any(|op| matches!(op, FastOp::ModuleParam { value, .. } if (*value - 0.4).abs() < 1e-6)),
        "the cutoff is not back at 0.4: {ops:?}"
    );
    // Untouched knobs are left alone: nothing on the master volume.
    assert!(!ops.iter().any(|op| matches!(op, FastOp::NodeParam { track: None, .. })), "{ops:?}");
    // Forgotten: back on the bass page the knob moves at once, no pickup.
    assert!(!planner.knob(74, 90, g).plans.is_empty());
}
