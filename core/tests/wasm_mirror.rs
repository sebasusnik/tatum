//! Mirror test: reproduces EXACTLY what the browser/WASM does.
//! If this sounds good → problem is in the AudioWorklet bridge.
//! If this sounds bad → problem is in how we use the multi-track sequencer.

use synth_core::engine::Engine;
use synth_core::sequencer::Step;
use synth_core::harmony::{HarmonyContext, Scale};
use synth_core::{SAMPLE_RATE, BLOCK_SIZE};

mod test_helpers;
use test_helpers::{write_wav_stereo, output_path};

#[test]
fn test_wasm_mirror() {
    let mut engine = Engine::new();

    // ── BPM (same as browser default) ──
    engine.set_bpm(112.0);

    // ── Harmony: A minor (same as browser) ──
    let ctx = HarmonyContext::new(57, Scale::Minor); // A3 root, minor
    engine.set_harmony(ctx);

    // ── Bass params (matching browser store defaults) ──
    // Page 0: Cutoff=8/100, Reso=75/100, EnvMod=70/100, Attack=0/100
    engine.apply_param_lock(0, 0.08);   // Cutoff
    engine.apply_param_lock(2, 0.75);   // Resonance
    engine.apply_param_lock(1, 0.70);   // CutoffEnv (Env Mod)
    engine.apply_param_lock(4, 0.00);   // Attack
    // Page 1: Glide=15/100, Osc2=25/100, Osc3=50/100, Drive=0/100
    engine.apply_param_lock(3, 0.15);   // Glide
    engine.apply_param_lock(10, 0.25);  // Osc2Pitch (sub octave)
    engine.apply_param_lock(11, 0.50);  // Osc3Pitch (center)
    engine.apply_param_lock(12, 0.00);  // Osc1Wave (saw)

    // ── FM params (clavinet) ──
    engine.apply_param_lock(64, 29.0 / 100.0); // Algorithm
    engine.apply_param_lock(65, 63.0 / 100.0); // ModIndex
    engine.apply_param_lock(71, 40.0 / 100.0); // Feedback

    // ── Keys params (pad, off by default) ──
    engine.apply_param_lock(32, 0.15);  // Cutoff
    engine.apply_param_lock(33, 0.10);  // Detune
    engine.apply_param_lock(34, 0.20);  // ChorusMix
    engine.apply_param_lock(35, 0.70);  // Level
    engine.apply_param_lock(41, 0.25);  // VoiceMode

    // ── Effects (matching syncEffectsToWasm) ──
    engine.set_delay_time(60.0 / 112.0 / 4.0); // 16th note at 112 BPM
    engine.set_delay_feedback(0.25);
    engine.set_delay_filter(0.6);
    engine.reverb.set_room_size(0.3);
    engine.reverb.set_damping(0.6);
    engine.reverb.set_mix(0.10);
    engine.set_eq_low(4.0);
    engine.set_eq_mid(2.0);
    engine.set_eq_high(1.0);
    engine.set_compressor_threshold(-9.0);
    engine.set_compressor_ratio(5.0);
    engine.set_sidechain_amount(0.4);

    // ── Bass track (track 0) — syncopated funk ──
    // Exactly what the browser sends via setTrackStep
    let bass_steps: [(u8, f32, f32); 16] = [
        (21, 0.90, 0.40), // 0: A0 strong downbeat
        (0,  0.0,  0.0),  // 1: rest
        (0,  0.0,  0.0),  // 2: rest
        (21, 0.55, 0.25), // 3: A0 ghost
        (0,  0.0,  0.0),  // 4: rest (pocket)
        (19, 0.70, 0.30), // 5: G0 chromatic
        (0,  0.0,  0.0),  // 6: rest
        (0,  0.0,  0.0),  // 7: rest
        (21, 0.80, 0.35), // 8: A0 beat 3
        (0,  0.0,  0.0),  // 9: rest
        (21, 0.40, 0.20), // 10: A0 subtle ghost
        (0,  0.0,  0.0),  // 11: rest
        (24, 0.50, 0.25), // 12: C1 minor third
        (0,  0.0,  0.0),  // 13: rest
        (21, 0.65, 0.30), // 14: A0 pickup
        (0,  0.0,  0.0),  // 15: rest
    ];

    // Per-step param locks for wah variation (CutoffEnv=1, Reso=2)
    let bass_locks: [(f32, f32); 16] = [
        (0.75, 0.80), // 0
        (0.0, 0.0),   // 1
        (0.0, 0.0),   // 2
        (0.50, 0.65), // 3
        (0.0, 0.0),   // 4
        (0.65, 0.75), // 5
        (0.0, 0.0),   // 6
        (0.0, 0.0),   // 7
        (0.70, 0.78), // 8
        (0.0, 0.0),   // 9
        (0.40, 0.55), // 10
        (0.0, 0.0),   // 11
        (0.55, 0.70), // 12
        (0.0, 0.0),   // 13
        (0.60, 0.72), // 14
        (0.0, 0.0),   // 15
    ];

    for i in 0..16 {
        let (note, vel, gate) = bass_steps[i];
        if vel > 0.0 {
            engine.sequencer.tracks[0].steps[i] = Step::new(note, vel, gate);
            let (env, res) = bass_locks[i];
            if env > 0.0 {
                // Add param locks
                let s = &mut engine.sequencer.tracks[0].steps[i];
                s.locks[0] = Some((1, env));  // CutoffEnv
                s.locks[1] = Some((2, res));  // Resonance
            }
        } else {
            engine.sequencer.tracks[0].steps[i] = Step::empty();
        }
    }

    // ── FM track (track 2) — clavinet stabs ──
    let fm_steps: [(u8, f32, f32); 16] = [
        (0,  0.0,  0.0),  // 0: rest
        (0,  0.0,  0.0),  // 1: rest
        (60, 0.50, 0.30), // 2: C4
        (0,  0.0,  0.0),  // 3: rest
        (0,  0.0,  0.0),  // 4: rest
        (64, 0.55, 0.30), // 5: E4
        (57, 0.35, 0.20), // 6: A3 ghost
        (0,  0.0,  0.0),  // 7: rest
        (0,  0.0,  0.0),  // 8: rest
        (67, 0.50, 0.30), // 9: G4
        (0,  0.0,  0.0),  // 10: rest
        (64, 0.40, 0.25), // 11: E4 ghost
        (0,  0.0,  0.0),  // 12: rest
        (60, 0.55, 0.30), // 13: C4
        (0,  0.0,  0.0),  // 14: rest
        (57, 0.30, 0.20), // 15: A3 pickup
    ];

    for i in 0..16 {
        let (note, vel, gate) = fm_steps[i];
        if vel > 0.0 {
            engine.sequencer.tracks[2].steps[i] = Step::new(note, vel, gate);
        } else {
            engine.sequencer.tracks[2].steps[i] = Step::empty();
        }
    }

    // ── Drum lanes (matching browser defaults) ──
    // Kick
    let kick_on =  [true,false,false,false,false,false,false,true,true,false,false,false,false,false,false,false];
    let kick_vel = [0.95, 0.0,  0.0,  0.0,  0.0,  0.0,  0.0, 0.70,0.85, 0.0,  0.0,  0.0,  0.0,  0.0,  0.0,  0.0];
    // Snare
    let snare_on =  [false,false,false,false,true,false,false,false,false,false,true,false,true,false,false,false];
    let snare_vel = [0.0,  0.0,  0.0,  0.0, 0.85, 0.0,  0.0,  0.0,  0.0,  0.0, 0.30, 0.0, 0.80, 0.0,  0.0,  0.0];
    // Hihat (all 16ths, alternating velocity)
    let hihat_vel = [0.35,0.25,0.35,0.25,0.35,0.25,0.45,0.25,0.35,0.25,0.35,0.25,0.35,0.25,0.40,0.25];
    // Clap ghost
    let clap_on =  [false,false,false,false,false,false,false,false,false,false,false,true,false,false,false,false];
    let clap_vel = [0.0,  0.0,  0.0,  0.0,  0.0,  0.0,  0.0,  0.0,  0.0,  0.0,  0.0, 0.40, 0.0,  0.0,  0.0,  0.0];

    for i in 0..16 {
        engine.sequencer.drum_track.lanes[0].on[i] = kick_on[i];
        engine.sequencer.drum_track.lanes[0].velocity[i] = kick_vel[i];
        engine.sequencer.drum_track.lanes[1].on[i] = snare_on[i];
        engine.sequencer.drum_track.lanes[1].velocity[i] = snare_vel[i];
        engine.sequencer.drum_track.lanes[2].on[i] = true; // hihat all 16ths
        engine.sequencer.drum_track.lanes[2].velocity[i] = hihat_vel[i];
        engine.sequencer.drum_track.lanes[3].on[i] = clap_on[i];
        engine.sequencer.drum_track.lanes[3].velocity[i] = clap_vel[i];
    }

    // ── Render 8 bars (enough to hear the groove) ──
    engine.sequencer.start();

    let samples_per_bar = (SAMPLE_RATE * 60.0 / 112.0 * 4.0) as usize;
    let total_bars = 8;
    let total = samples_per_bar * total_bars;
    let mut samples_l = vec![0.0f32; total];
    let mut samples_r = vec![0.0f32; total];

    let mut pos = 0;
    while pos < total {
        let bl = BLOCK_SIZE.min(total - pos);
        engine.process_block_stereo(
            &mut samples_l[pos..pos + bl],
            &mut samples_r[pos..pos + bl],
        );
        pos += bl;
    }

    // Check peak amplitude
    let max_val = samples_l.iter().chain(samples_r.iter())
        .fold(0.0f32, |a, &b| a.max(b.abs()));
    println!("WASM mirror test — peak amplitude: {:.4}", max_val);
    assert!(max_val > 0.05, "Should produce audible output, peak={}", max_val);

    write_wav_stereo(
        &output_path("test_wasm_mirror.wav"),
        &samples_l,
        &samples_r,
        SAMPLE_RATE as u32,
    );
    println!("Wrote test_wasm_mirror.wav — listen and compare with test_funk_envelope_filter.wav");
}
