//! CommandBus integration test — programs the funk pattern entirely via Commands.
//! This validates that the CommandBus protocol can fully drive the engine
//! with the same quality as direct API calls.

use synth_core::command::{Command, CommandBus};
use synth_core::engine::Engine;
use synth_core::{BLOCK_SIZE, SAMPLE_RATE};

mod test_helpers;
use test_helpers::{output_path, write_wav_stereo};

#[test]
fn test_funk_via_commands() {
    let mut engine = Engine::new();
    let mut bus = CommandBus::new();

    // ── Transport ──
    bus.push(Command::SetBpm(112.0));

    // ── Harmony: A minor ──
    bus.push(Command::SetHarmony {
        root: 57,
        scale: 1,
        degree: 0,
    });

    // ── Bass params: low cutoff + big wah sweep (matching songs.rs funk test) ──
    bus.push(Command::SetParam { id: 0, value: 0.08 }); // Cutoff (low resting ~80Hz)
    bus.push(Command::SetParam { id: 2, value: 0.75 }); // Reso
    bus.push(Command::SetParam { id: 1, value: 0.70 }); // EnvMod (big wah sweep)
    bus.push(Command::SetParam { id: 4, value: 0.00 }); // Attack
    bus.push(Command::SetParam { id: 3, value: 0.15 }); // Glide
    bus.push(Command::SetParam {
        id: 10,
        value: 0.25,
    }); // Osc2 sub octave
    bus.push(Command::SetParam {
        id: 12,
        value: 0.00,
    }); // Saw
    bus.push(Command::SetParam {
        id: 15,
        value: 0.50,
    }); // Keytrack

    // ── FM clav params ──
    bus.push(Command::SetParam {
        id: 64,
        value: 0.29,
    }); // Algorithm
    bus.push(Command::SetParam {
        id: 65,
        value: 0.63,
    }); // ModIndex
    bus.push(Command::SetParam {
        id: 71,
        value: 0.40,
    }); // Feedback
        // FM ADSR: Attack/Decay/Release are scaled *2.0 internally
        // Funk test wants: A=0.001, D=0.12, S=0.05, R=0.08
    bus.push(Command::SetParam {
        id: 76,
        value: 0.0005,
    }); // Attack: 0.001s / 2.0
    bus.push(Command::SetParam {
        id: 77,
        value: 0.06,
    }); // Decay: 0.12s / 2.0
    bus.push(Command::SetParam {
        id: 78,
        value: 0.05,
    }); // Sustain (no scaling)
    bus.push(Command::SetParam {
        id: 79,
        value: 0.04,
    }); // Release: 0.08s / 2.0

    // ── Mute arp ──
    bus.push(Command::SetParam {
        id: 131,
        value: 0.0,
    }); // Arp level = 0

    // ── Effects ──
    bus.push(Command::SetDelaySync(5)); // Sixteenth note
    bus.push(Command::SetDelayFeedback(0.25));
    bus.push(Command::SetDelayMix(0.10)); // Funk is dry
    bus.push(Command::SetDelayFilter(0.6));
    bus.push(Command::SetTiltEq(0.0)); // Neutral
    bus.push(Command::SetReverbSize(0.3));
    bus.push(Command::SetReverbDamping(0.6));
    bus.push(Command::SetReverbMix(0.10));
    bus.push(Command::SetEqLow(4.0));
    bus.push(Command::SetEqMid(2.0));
    bus.push(Command::SetEqHigh(1.0));
    bus.push(Command::SetCompThreshold(-9.0));
    bus.push(Command::SetCompRatio(5.0));
    bus.push(Command::SetCompAttack(8.0));
    bus.push(Command::SetCompRelease(80.0));
    bus.push(Command::SetCompMakeup(3.0));
    bus.push(Command::SetSidechain(0.4));

    // ── Bass steps (track 0) — syncopated funk ──
    // Gate ~0.85-0.90, env locks use exponential scaling
    let bass: [(u8, u8, f32, f32, f32, f32); 7] = [
        //  idx, note, vel,  gate, env,  res
        (0,  21, 0.90, 0.40, 0.75, 0.80),
        (3,  21, 0.55, 0.25, 0.50, 0.65),
        (5,  19, 0.70, 0.30, 0.65, 0.75),
        (8,  21, 0.80, 0.35, 0.70, 0.78),
        (10, 21, 0.40, 0.20, 0.40, 0.55),
        (12, 24, 0.50, 0.25, 0.55, 0.70),
        (14, 21, 0.65, 0.30, 0.60, 0.72),
    ];
    for &(idx, note, vel, gate, env, res) in &bass {
        bus.push(Command::SetStep {
            track: 0,
            idx,
            note,
            velocity: vel,
            gate,
        });
        bus.push(Command::SetStepLock {
            track: 0,
            idx,
            param_id: 1,
            value: env,
        });
        bus.push(Command::SetStepLock {
            track: 0,
            idx,
            param_id: 2,
            value: res,
        });
    }

    // ── FM steps (track 2) — clav stabs ──
    let fm: [(u8, u8, f32, f32); 7] = [
        (2, 60, 0.50, 0.30),
        (5, 64, 0.55, 0.30),
        (6, 57, 0.35, 0.20),
        (9, 67, 0.50, 0.30),
        (11, 64, 0.40, 0.25),
        (13, 60, 0.55, 0.30),
        (15, 57, 0.30, 0.20),
    ];
    for &(idx, note, vel, gate) in &fm {
        bus.push(Command::SetStep {
            track: 2,
            idx,
            note,
            velocity: vel,
            gate,
        });
    }

    // ── Drum lanes — matching funk test variation 0 ──
    // Kick
    for &(step, vel) in &[(0u8, 0.95f32), (7, 0.70), (8, 0.85)] {
        bus.push(Command::SetDrumHit { lane: 0, step, on: true, velocity: vel });
    }
    // Snare (no ghost snare in variation 0)
    for &(step, vel) in &[(4u8, 0.85f32), (12, 0.80)] {
        bus.push(Command::SetDrumHit { lane: 1, step, on: true, velocity: vel });
    }
    // Closed hats — only on steps WITHOUT kick/snare/open-hat
    for &(step, vel) in &[
        (1u8, 0.35f32), (2, 0.25), (3, 0.35), (5, 0.35),
        (9, 0.35), (10, 0.35), (11, 0.30), (13, 0.25), (15, 0.35),
    ] {
        bus.push(Command::SetDrumHit { lane: 2, step, on: true, velocity: vel });
    }
    // Open hats (MIDI 46) — use lane 4 repurposed
    bus.push(Command::SetDrumLaneNote { lane: 4, note: 46 });
    bus.push(Command::SetDrumHit { lane: 4, step: 6, on: true, velocity: 0.45 });
    bus.push(Command::SetDrumHit { lane: 4, step: 14, on: true, velocity: 0.40 });

    // ── Start ──
    bus.push(Command::Start);

    // Drain all commands into the engine
    bus.drain(&mut engine);
    assert!(bus.is_empty());

    // ── Render 8 bars ──
    let samples_per_bar = (SAMPLE_RATE * 60.0 / 112.0 * 4.0) as usize;
    let total = samples_per_bar * 8;
    let mut samples_l = vec![0.0f32; total];
    let mut samples_r = vec![0.0f32; total];

    let mut pos = 0;
    while pos < total {
        let bl = BLOCK_SIZE.min(total - pos);
        engine.process_block_stereo(&mut samples_l[pos..pos + bl], &mut samples_r[pos..pos + bl]);
        pos += bl;
    }

    let max_val = samples_l
        .iter()
        .chain(samples_r.iter())
        .fold(0.0f32, |a, &b| a.max(b.abs()));
    println!("CommandBus funk test — peak amplitude: {:.4}", max_val);
    assert!(
        max_val > 0.1,
        "Should produce audible output, peak={}",
        max_val
    );

    write_wav_stereo(
        &output_path("test_funk_command_bus.wav"),
        &samples_l,
        &samples_r,
        SAMPLE_RATE as u32,
    );
    println!("Wrote test_funk_command_bus.wav");
}

#[test]
fn test_command_flat_roundtrip() {
    // Verify that from_flat(to_flat(cmd)) == cmd for all variants
    let commands = [
        Command::Start,
        Command::Stop,
        Command::Reset,
        Command::SetBpm(120.0),
        Command::NoteOn {
            module: 0,
            note: 60,
            velocity: 0.8,
        },
        Command::NoteOff {
            module: 1,
            note: 64,
        },
        Command::AllNotesOff,
        Command::SetParam {
            id: 42,
            value: 0.75,
        },
        Command::SetStep {
            track: 0,
            idx: 3,
            note: 21,
            velocity: 0.9,
            gate: 0.4,
        },
        Command::ClearStep { track: 1, idx: 7 },
        Command::SetStepSlide {
            track: 0,
            idx: 5,
            slide: true,
        },
        Command::SetStepLock {
            track: 0,
            idx: 0,
            param_id: 1,
            value: 0.75,
        },
        Command::ClearStepLocks { track: 2, idx: 10 },
        Command::SetDrumHit {
            lane: 0,
            step: 4,
            on: true,
            velocity: 0.95,
        },
        Command::ClearDrumLane(2),
        Command::SetHarmony {
            root: 57,
            scale: 1,
            degree: 0,
        },
        Command::SetDelayTime(0.134),
        Command::SetCompMakeup(3.0),
        Command::SetMute {
            module: 2,
            muted: true,
        },
        Command::SetSolo {
            module: 0,
            solo: true,
        },
    ];

    for &cmd in &commands {
        let (tag, a, b, c, d, e) = cmd.to_flat();
        let decoded = Command::from_flat(tag, a, b, c, d, e);
        assert!(decoded.is_some(), "Failed to decode: {:?}", cmd);
        // For float commands, check approximate equality
        let decoded = decoded.unwrap();
        let (tag2, a2, b2, c2, d2, e2) = decoded.to_flat();
        assert_eq!(tag, tag2, "Tag mismatch for {:?}", cmd);
        assert!(
            (a - a2).abs() < 0.001,
            "a mismatch for {:?}: {} vs {}",
            cmd,
            a,
            a2
        );
        assert!(
            (b - b2).abs() < 0.001,
            "b mismatch for {:?}: {} vs {}",
            cmd,
            b,
            b2
        );
        assert!(
            (c - c2).abs() < 0.001,
            "c mismatch for {:?}: {} vs {}",
            cmd,
            c,
            c2
        );
        assert!(
            (d - d2).abs() < 0.001,
            "d mismatch for {:?}: {} vs {}",
            cmd,
            d,
            d2
        );
        assert!(
            (e - e2).abs() < 0.001,
            "e mismatch for {:?}: {} vs {}",
            cmd,
            e,
            e2
        );
    }
}

#[test]
fn test_command_bus_capacity() {
    let mut bus = CommandBus::new();

    // Fill the bus
    for i in 0..2000 {
        bus.push(Command::SetParam {
            id: (i % 256) as u8,
            value: 0.5,
        });
    }
    // Should have 2047 (BUS_SIZE - 1) commands, rest dropped
    assert!(bus.len() <= 2047);

    // Drain into a dummy engine
    let mut engine = Engine::new();
    bus.drain(&mut engine);
    assert!(bus.is_empty());
    assert_eq!(bus.len(), 0);
}

/// Replay commands recorded from the browser.
/// To use:
///   1. Press Play in the browser
///   2. Open DevTools console
///   3. Run: __dumpCommands()
///   4. Paste the output into the `cmds` array below
///   5. Run: cargo test test_replay_browser -- --nocapture
///
/// Compare test_replay_browser.wav with what you hear in the browser.
/// If they sound the same → problem is WASM vs native float differences.
/// If they sound different → the command list doesn't match.
#[test]
fn test_replay_browser() {
    // Recorded commands from browser session
    let cmds: Vec<Command> = vec![
        Command::from_flat(3, 112.0, 0.0, 0.0, 0.0, 0.0).unwrap(), // SetBpm
        Command::from_flat(8, 0.0, 0.0, 21.0, 0.9, 0.4).unwrap(), // SetStep
        Command::from_flat(11, 0.0, 0.0, 1.0, 0.75, 0.0).unwrap(), // SetStepLock
        Command::from_flat(11, 0.0, 0.0, 2.0, 0.8, 0.0).unwrap(), // SetStepLock
        Command::from_flat(9, 0.0, 1.0, 0.0, 0.0, 0.0).unwrap(),   // ClearStep
        Command::from_flat(9, 0.0, 2.0, 0.0, 0.0, 0.0).unwrap(),   // ClearStep
        Command::from_flat(8, 0.0, 3.0, 21.0, 0.55, 0.25).unwrap(), // SetStep
        Command::from_flat(11, 0.0, 3.0, 1.0, 0.5, 0.0).unwrap(), // SetStepLock
        Command::from_flat(11, 0.0, 3.0, 2.0, 0.65, 0.0).unwrap(), // SetStepLock
        Command::from_flat(9, 0.0, 4.0, 0.0, 0.0, 0.0).unwrap(),   // ClearStep
        Command::from_flat(8, 0.0, 5.0, 19.0, 0.7, 0.3).unwrap(), // SetStep
        Command::from_flat(11, 0.0, 5.0, 1.0, 0.65, 0.0).unwrap(), // SetStepLock
        Command::from_flat(11, 0.0, 5.0, 2.0, 0.75, 0.0).unwrap(), // SetStepLock
        Command::from_flat(9, 0.0, 6.0, 0.0, 0.0, 0.0).unwrap(),   // ClearStep
        Command::from_flat(9, 0.0, 7.0, 0.0, 0.0, 0.0).unwrap(),   // ClearStep
        Command::from_flat(8, 0.0, 8.0, 21.0, 0.8, 0.35).unwrap(), // SetStep
        Command::from_flat(11, 0.0, 8.0, 1.0, 0.7, 0.0).unwrap(), // SetStepLock
        Command::from_flat(11, 0.0, 8.0, 2.0, 0.78, 0.0).unwrap(), // SetStepLock
        Command::from_flat(9, 0.0, 9.0, 0.0, 0.0, 0.0).unwrap(),   // ClearStep
        Command::from_flat(8, 0.0, 10.0, 21.0, 0.4, 0.2).unwrap(), // SetStep
        Command::from_flat(11, 0.0, 10.0, 1.0, 0.4, 0.0).unwrap(), // SetStepLock
        Command::from_flat(11, 0.0, 10.0, 2.0, 0.55, 0.0).unwrap(), // SetStepLock
        Command::from_flat(9, 0.0, 11.0, 0.0, 0.0, 0.0).unwrap(),  // ClearStep
        Command::from_flat(8, 0.0, 12.0, 24.0, 0.5, 0.25).unwrap(), // SetStep
        Command::from_flat(11, 0.0, 12.0, 1.0, 0.55, 0.0).unwrap(), // SetStepLock
        Command::from_flat(11, 0.0, 12.0, 2.0, 0.7, 0.0).unwrap(), // SetStepLock
        Command::from_flat(9, 0.0, 13.0, 0.0, 0.0, 0.0).unwrap(),  // ClearStep
        Command::from_flat(8, 0.0, 14.0, 21.0, 0.65, 0.3).unwrap(), // SetStep
        Command::from_flat(11, 0.0, 14.0, 1.0, 0.6, 0.0).unwrap(), // SetStepLock
        Command::from_flat(11, 0.0, 14.0, 2.0, 0.72, 0.0).unwrap(), // SetStepLock
        Command::from_flat(9, 0.0, 15.0, 0.0, 0.0, 0.0).unwrap(),  // ClearStep
        Command::from_flat(9, 1.0, 0.0, 0.0, 0.0, 0.0).unwrap(),   // ClearStep
        Command::from_flat(9, 1.0, 1.0, 0.0, 0.0, 0.0).unwrap(),   // ClearStep
        Command::from_flat(9, 1.0, 2.0, 0.0, 0.0, 0.0).unwrap(),   // ClearStep
        Command::from_flat(9, 1.0, 3.0, 0.0, 0.0, 0.0).unwrap(),   // ClearStep
        Command::from_flat(9, 1.0, 4.0, 0.0, 0.0, 0.0).unwrap(),   // ClearStep
        Command::from_flat(9, 1.0, 5.0, 0.0, 0.0, 0.0).unwrap(),   // ClearStep
        Command::from_flat(9, 1.0, 6.0, 0.0, 0.0, 0.0).unwrap(),   // ClearStep
        Command::from_flat(9, 1.0, 7.0, 0.0, 0.0, 0.0).unwrap(),   // ClearStep
        Command::from_flat(9, 1.0, 8.0, 0.0, 0.0, 0.0).unwrap(),   // ClearStep
        Command::from_flat(9, 1.0, 9.0, 0.0, 0.0, 0.0).unwrap(),   // ClearStep
        Command::from_flat(9, 1.0, 10.0, 0.0, 0.0, 0.0).unwrap(),  // ClearStep
        Command::from_flat(9, 1.0, 11.0, 0.0, 0.0, 0.0).unwrap(),  // ClearStep
        Command::from_flat(9, 1.0, 12.0, 0.0, 0.0, 0.0).unwrap(),  // ClearStep
        Command::from_flat(9, 1.0, 13.0, 0.0, 0.0, 0.0).unwrap(),  // ClearStep
        Command::from_flat(9, 1.0, 14.0, 0.0, 0.0, 0.0).unwrap(),  // ClearStep
        Command::from_flat(9, 1.0, 15.0, 0.0, 0.0, 0.0).unwrap(),  // ClearStep
        Command::from_flat(9, 2.0, 0.0, 0.0, 0.0, 0.0).unwrap(),   // ClearStep
        Command::from_flat(9, 2.0, 1.0, 0.0, 0.0, 0.0).unwrap(),   // ClearStep
        Command::from_flat(8, 2.0, 2.0, 60.0, 0.5, 0.3).unwrap(), // SetStep
        Command::from_flat(9, 2.0, 3.0, 0.0, 0.0, 0.0).unwrap(),   // ClearStep
        Command::from_flat(9, 2.0, 4.0, 0.0, 0.0, 0.0).unwrap(),   // ClearStep
        Command::from_flat(8, 2.0, 5.0, 64.0, 0.55, 0.3).unwrap(), // SetStep
        Command::from_flat(8, 2.0, 6.0, 57.0, 0.35, 0.2).unwrap(), // SetStep
        Command::from_flat(9, 2.0, 7.0, 0.0, 0.0, 0.0).unwrap(),   // ClearStep
        Command::from_flat(9, 2.0, 8.0, 0.0, 0.0, 0.0).unwrap(),   // ClearStep
        Command::from_flat(8, 2.0, 9.0, 67.0, 0.5, 0.3).unwrap(), // SetStep
        Command::from_flat(9, 2.0, 10.0, 0.0, 0.0, 0.0).unwrap(),  // ClearStep
        Command::from_flat(8, 2.0, 11.0, 64.0, 0.4, 0.25).unwrap(), // SetStep
        Command::from_flat(9, 2.0, 12.0, 0.0, 0.0, 0.0).unwrap(),  // ClearStep
        Command::from_flat(8, 2.0, 13.0, 60.0, 0.55, 0.3).unwrap(), // SetStep
        Command::from_flat(9, 2.0, 14.0, 0.0, 0.0, 0.0).unwrap(),  // ClearStep
        Command::from_flat(8, 2.0, 15.0, 57.0, 0.3, 0.2).unwrap(), // SetStep
        Command::from_flat(16, 0.0, 0.0, 1.0, 0.95, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 0.0, 1.0, 0.0, 0.0, 0.0).unwrap(),  // SetDrumHit
        Command::from_flat(16, 0.0, 2.0, 0.0, 0.0, 0.0).unwrap(),  // SetDrumHit
        Command::from_flat(16, 0.0, 3.0, 0.0, 0.0, 0.0).unwrap(),  // SetDrumHit
        Command::from_flat(16, 0.0, 4.0, 0.0, 0.0, 0.0).unwrap(),  // SetDrumHit
        Command::from_flat(16, 0.0, 5.0, 0.0, 0.0, 0.0).unwrap(),  // SetDrumHit
        Command::from_flat(16, 0.0, 6.0, 0.0, 0.0, 0.0).unwrap(),  // SetDrumHit
        Command::from_flat(16, 0.0, 7.0, 1.0, 0.7, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 0.0, 8.0, 1.0, 0.85, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 0.0, 9.0, 0.0, 0.0, 0.0).unwrap(),  // SetDrumHit
        Command::from_flat(16, 0.0, 10.0, 0.0, 0.0, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 0.0, 11.0, 0.0, 0.0, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 0.0, 12.0, 0.0, 0.0, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 0.0, 13.0, 0.0, 0.0, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 0.0, 14.0, 0.0, 0.0, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 0.0, 15.0, 0.0, 0.0, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 1.0, 0.0, 0.0, 0.0, 0.0).unwrap(),  // SetDrumHit
        Command::from_flat(16, 1.0, 1.0, 0.0, 0.0, 0.0).unwrap(),  // SetDrumHit
        Command::from_flat(16, 1.0, 2.0, 0.0, 0.0, 0.0).unwrap(),  // SetDrumHit
        Command::from_flat(16, 1.0, 3.0, 0.0, 0.0, 0.0).unwrap(),  // SetDrumHit
        Command::from_flat(16, 1.0, 4.0, 1.0, 0.85, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 1.0, 5.0, 0.0, 0.0, 0.0).unwrap(),  // SetDrumHit
        Command::from_flat(16, 1.0, 6.0, 0.0, 0.0, 0.0).unwrap(),  // SetDrumHit
        Command::from_flat(16, 1.0, 7.0, 0.0, 0.0, 0.0).unwrap(),  // SetDrumHit
        Command::from_flat(16, 1.0, 8.0, 0.0, 0.0, 0.0).unwrap(),  // SetDrumHit
        Command::from_flat(16, 1.0, 9.0, 0.0, 0.0, 0.0).unwrap(),  // SetDrumHit
        Command::from_flat(16, 1.0, 10.0, 1.0, 0.3, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 1.0, 11.0, 0.0, 0.0, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 1.0, 12.0, 1.0, 0.8, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 1.0, 13.0, 0.0, 0.0, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 1.0, 14.0, 0.0, 0.0, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 1.0, 15.0, 0.0, 0.0, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 2.0, 0.0, 1.0, 0.35, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 2.0, 1.0, 1.0, 0.25, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 2.0, 2.0, 1.0, 0.35, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 2.0, 3.0, 1.0, 0.25, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 2.0, 4.0, 1.0, 0.35, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 2.0, 5.0, 1.0, 0.25, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 2.0, 6.0, 1.0, 0.45, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 2.0, 7.0, 1.0, 0.25, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 2.0, 8.0, 1.0, 0.35, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 2.0, 9.0, 1.0, 0.25, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 2.0, 10.0, 1.0, 0.35, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 2.0, 11.0, 1.0, 0.25, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 2.0, 12.0, 1.0, 0.35, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 2.0, 13.0, 1.0, 0.25, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 2.0, 14.0, 1.0, 0.4, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 2.0, 15.0, 1.0, 0.25, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 3.0, 0.0, 0.0, 0.0, 0.0).unwrap(),  // SetDrumHit
        Command::from_flat(16, 3.0, 1.0, 0.0, 0.0, 0.0).unwrap(),  // SetDrumHit
        Command::from_flat(16, 3.0, 2.0, 0.0, 0.0, 0.0).unwrap(),  // SetDrumHit
        Command::from_flat(16, 3.0, 3.0, 0.0, 0.0, 0.0).unwrap(),  // SetDrumHit
        Command::from_flat(16, 3.0, 4.0, 0.0, 0.0, 0.0).unwrap(),  // SetDrumHit
        Command::from_flat(16, 3.0, 5.0, 0.0, 0.0, 0.0).unwrap(),  // SetDrumHit
        Command::from_flat(16, 3.0, 6.0, 0.0, 0.0, 0.0).unwrap(),  // SetDrumHit
        Command::from_flat(16, 3.0, 7.0, 0.0, 0.0, 0.0).unwrap(),  // SetDrumHit
        Command::from_flat(16, 3.0, 8.0, 0.0, 0.0, 0.0).unwrap(),  // SetDrumHit
        Command::from_flat(16, 3.0, 9.0, 0.0, 0.0, 0.0).unwrap(),  // SetDrumHit
        Command::from_flat(16, 3.0, 10.0, 0.0, 0.0, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 3.0, 11.0, 1.0, 0.4, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 3.0, 12.0, 0.0, 0.0, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 3.0, 13.0, 0.0, 0.0, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 3.0, 14.0, 0.0, 0.0, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 3.0, 15.0, 0.0, 0.0, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 4.0, 0.0, 0.0, 0.6, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 4.0, 1.0, 0.0, 0.6, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 4.0, 2.0, 0.0, 0.6, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 4.0, 3.0, 0.0, 0.6, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 4.0, 4.0, 0.0, 0.6, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 4.0, 5.0, 0.0, 0.6, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 4.0, 6.0, 0.0, 0.6, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 4.0, 7.0, 0.0, 0.6, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 4.0, 8.0, 0.0, 0.6, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 4.0, 9.0, 0.0, 0.6, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 4.0, 10.0, 0.0, 0.6, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 4.0, 11.0, 0.0, 0.6, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 4.0, 12.0, 0.0, 0.6, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 4.0, 13.0, 0.0, 0.6, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 4.0, 14.0, 0.0, 0.6, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 4.0, 15.0, 0.0, 0.6, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 5.0, 0.0, 0.0, 0.7, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 5.0, 1.0, 0.0, 0.7, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 5.0, 2.0, 0.0, 0.7, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 5.0, 3.0, 0.0, 0.7, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 5.0, 4.0, 0.0, 0.7, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 5.0, 5.0, 0.0, 0.7, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 5.0, 6.0, 0.0, 0.7, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 5.0, 7.0, 0.0, 0.7, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 5.0, 8.0, 0.0, 0.7, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 5.0, 9.0, 0.0, 0.7, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 5.0, 10.0, 0.0, 0.7, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 5.0, 11.0, 0.0, 0.7, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 5.0, 12.0, 0.0, 0.7, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 5.0, 13.0, 0.0, 0.7, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 5.0, 14.0, 0.0, 0.7, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(16, 5.0, 15.0, 0.0, 0.7, 0.0).unwrap(), // SetDrumHit
        Command::from_flat(7, 0.0, 0.08, 0.0, 0.0, 0.0).unwrap(), // SetParam
        Command::from_flat(7, 2.0, 0.75, 0.0, 0.0, 0.0).unwrap(), // SetParam
        Command::from_flat(7, 1.0, 0.7, 0.0, 0.0, 0.0).unwrap(), // SetParam
        Command::from_flat(7, 3.0, 0.15, 0.0, 0.0, 0.0).unwrap(), // SetParam
        Command::from_flat(7, 4.0, 0.0, 0.0, 0.0, 0.0).unwrap(),   // SetParam
        Command::from_flat(7, 18.0, 0.1, 0.0, 0.0, 0.0).unwrap(), // SetParam
        Command::from_flat(7, 19.0, 0.8, 0.0, 0.0, 0.0).unwrap(), // SetParam
        Command::from_flat(7, 20.0, 0.08, 0.0, 0.0, 0.0).unwrap(), // SetParam
        Command::from_flat(7, 10.0, 0.25, 0.0, 0.0, 0.0).unwrap(), // SetParam
        Command::from_flat(7, 11.0, 0.5, 0.0, 0.0, 0.0).unwrap(), // SetParam
        Command::from_flat(7, 12.0, 0.0, 0.0, 0.0, 0.0).unwrap(),  // SetParam
        Command::from_flat(7, 15.0, 0.5, 0.0, 0.0, 0.0).unwrap(), // SetParam
        Command::from_flat(7, 5.0, 0.4, 0.0, 0.0, 0.0).unwrap(), // SetParam
        Command::from_flat(7, 6.0, 0.0, 0.0, 0.0, 0.0).unwrap(),   // SetParam
        Command::from_flat(7, 7.0, 0.0, 0.0, 0.0, 0.0).unwrap(),   // SetParam
        Command::from_flat(7, 8.0, 0.0, 0.0, 0.0, 0.0).unwrap(),   // SetParam
        Command::from_flat(7, 32.0, 0.15, 0.0, 0.0, 0.0).unwrap(), // SetParam
        Command::from_flat(7, 33.0, 0.1, 0.0, 0.0, 0.0).unwrap(), // SetParam
        Command::from_flat(7, 34.0, 0.2, 0.0, 0.0, 0.0).unwrap(), // SetParam
        Command::from_flat(7, 35.0, 0.7, 0.0, 0.0, 0.0).unwrap(), // SetParam
        Command::from_flat(7, 36.0, 0.3, 0.0, 0.0, 0.0).unwrap(), // SetParam
        Command::from_flat(7, 37.0, 0.2, 0.0, 0.0, 0.0).unwrap(), // SetParam
        Command::from_flat(7, 41.0, 0.25, 0.0, 0.0, 0.0).unwrap(), // SetParam
        Command::from_flat(7, 43.0, 0.1, 0.0, 0.0, 0.0).unwrap(), // SetParam
        Command::from_flat(7, 64.0, 0.29, 0.0, 0.0, 0.0).unwrap(), // SetParam
        Command::from_flat(7, 65.0, 0.63, 0.0, 0.0, 0.0).unwrap(), // SetParam
        Command::from_flat(7, 71.0, 0.4, 0.0, 0.0, 0.0).unwrap(), // SetParam
        Command::from_flat(7, 76.0, 0.01, 0.0, 0.0, 0.0).unwrap(), // SetParam
        Command::from_flat(7, 77.0, 0.12, 0.0, 0.0, 0.0).unwrap(), // SetParam
        Command::from_flat(7, 78.0, 0.05, 0.0, 0.0, 0.0).unwrap(), // SetParam
        Command::from_flat(7, 79.0, 0.08, 0.0, 0.0, 0.0).unwrap(), // SetParam
        Command::from_flat(7, 66.0, 0.35, 0.0, 0.0, 0.0).unwrap(), // SetParam
        Command::from_flat(7, 67.0, 0.15, 0.0, 0.0, 0.0).unwrap(), // SetParam
        Command::from_flat(7, 72.0, 0.0, 0.0, 0.0, 0.0).unwrap(),  // SetParam
        Command::from_flat(7, 73.0, 0.4, 0.0, 0.0, 0.0).unwrap(), // SetParam
        Command::from_flat(7, 97.0, 0.55, 0.0, 0.0, 0.0).unwrap(), // SetParam
        Command::from_flat(7, 98.0, 0.4, 0.0, 0.0, 0.0).unwrap(), // SetParam
        Command::from_flat(7, 106.0, 0.3, 0.0, 0.0, 0.0).unwrap(), // SetParam
        Command::from_flat(7, 107.0, 0.5, 0.0, 0.0, 0.0).unwrap(), // SetParam
        Command::from_flat(7, 108.0, 0.5, 0.0, 0.0, 0.0).unwrap(), // SetParam
        Command::from_flat(7, 109.0, 0.5, 0.0, 0.0, 0.0).unwrap(), // SetParam
        Command::from_flat(7, 110.0, 0.5, 0.0, 0.0, 0.0).unwrap(), // SetParam
        Command::from_flat(7, 103.0, 0.2, 0.0, 0.0, 0.0).unwrap(), // SetParam
        Command::from_flat(20, 0.585, 0.0, 0.0, 0.0, 0.0).unwrap(), // SetSwing
        Command::from_flat(7, 111.0, 0.0, 0.0, 0.0, 0.0).unwrap(), // SetParam
        Command::from_flat(21, 0.1, 0.0, 0.0, 0.0, 0.0).unwrap(), // SetHumanize
        Command::from_flat(23, 57.0, 1.0, 0.0, 0.0, 0.0).unwrap(), // SetHarmony
        Command::from_flat(7, 131.0, 0.0, 0.0, 0.0, 0.0).unwrap(), // SetParam
        Command::from_flat(43, 5.0, 0.0, 0.0, 0.0, 0.0).unwrap(),  // SetDelaySync
        Command::from_flat(25, 0.25, 0.0, 0.0, 0.0, 0.0).unwrap(), // SetDelayFeedback
        Command::from_flat(42, 0.1, 0.0, 0.0, 0.0, 0.0).unwrap(), // SetDelayMix
        Command::from_flat(26, 0.6, 0.0, 0.0, 0.0, 0.0).unwrap(), // SetDelayFilter
        Command::from_flat(44, 0.0, 0.0, 0.0, 0.0, 0.0).unwrap(),  // SetTiltEq
        Command::from_flat(27, 0.3, 0.0, 0.0, 0.0, 0.0).unwrap(), // SetReverbSize
        Command::from_flat(28, 0.6, 0.0, 0.0, 0.0, 0.0).unwrap(), // SetReverbDamping
        Command::from_flat(29, 0.1, 0.0, 0.0, 0.0, 0.0).unwrap(), // SetReverbMix
        Command::from_flat(30, 4.0, 0.0, 0.0, 0.0, 0.0).unwrap(),  // SetEqLow
        Command::from_flat(31, 2.0, 0.0, 0.0, 0.0, 0.0).unwrap(),  // SetEqMid
        Command::from_flat(32, 1.0, 0.0, 0.0, 0.0, 0.0).unwrap(),  // SetEqHigh
        Command::from_flat(33, -9.0, 0.0, 0.0, 0.0, 0.0).unwrap(), // SetCompThreshold
        Command::from_flat(34, 5.0, 0.0, 0.0, 0.0, 0.0).unwrap(),  // SetCompRatio
        Command::from_flat(35, 8.0, 0.0, 0.0, 0.0, 0.0).unwrap(),  // SetCompAttack
        Command::from_flat(36, 80.0, 0.0, 0.0, 0.0, 0.0).unwrap(), // SetCompRelease
        Command::from_flat(37, 3.0, 0.0, 0.0, 0.0, 0.0).unwrap(),  // SetCompMakeup
        Command::from_flat(38, 0.4, 0.0, 0.0, 0.0, 0.0).unwrap(), // SetSidechain
        Command::from_flat(3, 112.0, 0.0, 0.0, 0.0, 0.0).unwrap(), // SetBpm
        Command::from_flat(0, 0.0, 0.0, 0.0, 0.0, 0.0).unwrap(),   // Start
        // Stop removed — would silence everything before rendering
    ];

    if cmds.len() <= 3 {
        println!("No browser commands pasted yet. Paste the output of __dumpCommands() into the cmds array.");
        println!("Skipping test_replay_browser (no real data).");
        return;
    }

    let mut engine = Engine::new();
    let mut bus = CommandBus::new();

    for cmd in &cmds {
        bus.push(*cmd);
    }
    bus.drain(&mut engine);

    // Render 8 bars at whatever BPM was set
    let bpm = 112.0; // adjust if your dump uses a different BPM
    let samples_per_bar = (SAMPLE_RATE * 60.0 / bpm * 4.0) as usize;
    let total = samples_per_bar * 8;
    let mut samples_l = vec![0.0f32; total];
    let mut samples_r = vec![0.0f32; total];

    let mut pos = 0;
    while pos < total {
        let bl = BLOCK_SIZE.min(total - pos);
        engine.process_block_stereo(&mut samples_l[pos..pos + bl], &mut samples_r[pos..pos + bl]);
        pos += bl;
    }

    let max_val = samples_l
        .iter()
        .chain(samples_r.iter())
        .fold(0.0f32, |a, &b| a.max(b.abs()));
    println!("Browser replay — peak amplitude: {:.4}", max_val);

    write_wav_stereo(
        &output_path("test_replay_browser.wav"),
        &samples_l,
        &samples_r,
        SAMPLE_RATE as u32,
    );
    println!("Wrote test_replay_browser.wav — compare with browser audio!");
}
