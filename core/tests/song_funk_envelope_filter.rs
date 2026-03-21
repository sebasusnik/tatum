//! Funk envelope filter test song.
//! Run with: `cargo test --test song_funk_envelope_filter`

use synth_core::engine::Engine;
use synth_core::harmony::{HarmonyContext, Scale};
use synth_core::modules::bass::BassParam;
use synth_core::modules::fm::FmParam;
use synth_core::modules::keys::KeysParam;
use synth_core::effects::delay::DelaySync;
use synth_core::sequencer::Step;
use synth_core::track::InstrumentKind;
use synth_core::primitives::arp_processor::ArpProcessor;
use synth_core::{Module, SAMPLE_RATE, BLOCK_SIZE};

mod test_helpers;
use test_helpers::{write_wav_stereo, output_path};

/// Funk jam — envelope-filter bass, tight drums, clavinet stabs, punchy compression.
/// 112 BPM, A minor, ~68 seconds (32 bars).
#[test]
fn test_funk_envelope_filter() {
    let mut engine = Engine::new();
    let bpm = 112.0;
    engine.set_bpm(bpm);

    // --- Harmony: A minor (root = A2 = MIDI 45 for harmony, but we use bass notes directly) ---
    let mut ctx = HarmonyContext::new(57, Scale::Minor); // A3 root
    ctx.set_chord_degree(0);
    engine.set_harmony(ctx);

    // --- Effects: tight, punchy, not too wet ---
    engine.set_delay_sync(DelaySync::Sixteenth);
    engine.delay.set_feedback(0.25);
    engine.delay.set_mix(0.10); // subtle delay — funk is dry
    engine.set_delay_filter(0.6);

    engine.reverb.set_room_size(0.3); // small room, tight
    engine.reverb.set_damping(0.6);
    engine.reverb.set_mix(0.10);

    engine.set_eq_low(4.0);   // pump up the bass
    engine.set_eq_mid(2.0);   // midrange presence
    engine.set_eq_high(1.0);  // crisp
    engine.set_tilt_eq(0.0);  // neutral tilt

    engine.set_sidechain_amount(0.4); // moderate pump

    // --- Compressor: punchy funk compression ---
    engine.set_compressor_threshold(-9.0);
    engine.set_compressor_ratio(5.0);
    engine.set_compressor_attack(8.0);    // let transients through
    engine.set_compressor_release(80.0);  // fast release for groove
    engine.set_compressor_makeup(3.0);

    // --- Bass: ENVELOPE FILTER / AUTO-WAH ---
    // High resonance + high cutoff env = classic quacky funk wah
    engine.bass.set_param(BassParam::Cutoff, 0.08);       // low resting cutoff
    engine.bass.set_param(BassParam::CutoffEnv, 0.7);     // BIG envelope sweep
    engine.bass.set_param(BassParam::Resonance, 0.75);    // resonant peak = quack!
    engine.bass.set_param(BassParam::Glide, 0.15);        // moderate glide for slides
    engine.bass.set_param(BassParam::Osc2Pitch, (-12.0 + 24.0) / 48.0); // sub octave
    engine.bass.set_param(BassParam::Osc1Wave, 0.0);      // saw — bright harmonics for filter
    engine.bass.set_param(BassParam::Keytrack, 0.5);       // filter follows pitch
    engine.bass.set_param(BassParam::Attack, 0.0);         // instant attack — snappy

    // --- FM: clavinet-like stabs (Stevie Wonder / Herbie Hancock) ---
    engine.fm.set_param(FmParam::Algorithm, 2.0 / 7.0); // algo 2: carrier+mod
    engine.fm.set_param(FmParam::ModIndex, 0.87);  // → mod_index ≈ 2.5, bright clav-like
    engine.fm.set_param(FmParam::Feedback, 0.4);         // adds bite
    engine.fm.set_op_envelope(0, 0.001, 0.12, 0.05, 0.08); // ultra-snappy: percussive clav

    // --- Arp on track 4: ArpProcessor drives Keys[1], staccato stabs ---
    engine.tracks[4].kind = InstrumentKind::Keys;
    engine.tracks[4].instance_idx = 1;
    engine.tracks[4].level = 0.0; // start silent, bring in later
    engine.keys2.set_param(KeysParam::Cutoff, 0.15);
    engine.keys2.set_param(KeysParam::Attack, 0.0);
    engine.keys2.set_param(KeysParam::Decay, 0.15);
    engine.keys2.set_param(KeysParam::Sustain, 0.3);
    engine.keys2.set_param(KeysParam::Release, 0.08);
    {
        let mut arp = ArpProcessor::new();
        arp.set_bpm(112.0);
        arp.set_gate(0.2); // short staccato stabs
        arp.set_pattern(0.0); // Up
        engine.tracks[4].arp = Some(arp);
    }

    // --- Keys: not used much, maybe a pad swell later ---
    engine.keys.set_param(KeysParam::VoiceMode, 0.25);
    engine.keys.set_param(KeysParam::Cutoff, 0.15);
    engine.keys.set_param(KeysParam::Detune, 0.10);
    engine.keys.set_param(KeysParam::ChorusMix, 0.2);

    // --- Buffers ---
    let samples_per_beat = (SAMPLE_RATE * 60.0 / bpm) as usize;
    let samples_per_bar = samples_per_beat * 4;
    let samples_per_16th = samples_per_beat / 4;

    let total_bars = 32;
    let total = samples_per_bar * total_bars;
    let mut samples_l = vec![0.0f32; total];
    let mut samples_r = vec![0.0f32; total];

    // Section boundaries
    let intro_end = samples_per_bar * 4;       // bars 1-4: drums + bass groove
    let verse1_end = samples_per_bar * 12;     // bars 5-12: full groove, clav enters
    let bridge_end = samples_per_bar * 16;     // bars 13-16: breakdown, filter sweep
    let verse2_end = samples_per_bar * 24;     // bars 17-24: peak funk, arp stabs
    let outro_end = samples_per_bar * 32;      // bars 25-32: strip down

    // =====================================================================
    // Bass line: syncopated 16th-note funk pattern with ghost notes
    // The groove! Rests create the pocket.
    //
    // Step:  0  1  2  3  4  5  6  7  8  9 10 11 12 13 14 15
    // Note:  A  .  .  A  .  G  .  .  A  .  .  .  C  .  A  .
    // Vel:  .9  -  - .6  - .7  -  - .8  -  -  - .5  - .7  -
    //
    // A2=33, G2=31, C3=36 — wait, these overlap with drums!
    // Bass module responds to MIDI 0-47, drums to 36-49.
    // Use bass notes below 36: A1=21, G1=19, C2=24, D2=26, E2=28
    // =====================================================================

    // Funk bass line using parameter locks for wah variation per step
    let bass_pattern: [(u8, f32, f32, f32, f32); 16] = [
        // (note, vel, duration, cutoff_env, resonance)
        (21, 0.90, 0.4,  0.75, 0.80), // 0: A1 — strong downbeat, deep wah
        (0,  0.0,  0.0,  0.0,  0.0),  // 1: rest
        (0,  0.0,  0.0,  0.0,  0.0),  // 2: rest
        (21, 0.55, 0.25, 0.50, 0.65), // 3: A1 ghost — lighter wah
        (0,  0.0,  0.0,  0.0,  0.0),  // 4: rest (the pocket!)
        (19, 0.70, 0.3,  0.65, 0.75), // 5: G1 — chromatic approach
        (0,  0.0,  0.0,  0.0,  0.0),  // 6: rest
        (0,  0.0,  0.0,  0.0,  0.0),  // 7: rest
        (21, 0.80, 0.35, 0.70, 0.78), // 8: A1 — beat 3
        (0,  0.0,  0.0,  0.0,  0.0),  // 9: rest
        (21, 0.40, 0.2,  0.40, 0.55), // 10: A1 ghost — subtle
        (0,  0.0,  0.0,  0.0,  0.0),  // 11: rest
        (24, 0.50, 0.25, 0.55, 0.70), // 12: C2 — minor third jump
        (0,  0.0,  0.0,  0.0,  0.0),  // 13: rest
        (21, 0.65, 0.3,  0.60, 0.72), // 14: A1 — pickup back to 1
        (0,  0.0,  0.0,  0.0,  0.0),  // 15: rest
    ];

    // Helper: program the funky drum pattern
    // Kick: 1, 3.5(7), 9 | Snare: 5, 13 | HH: 16ths | Clap: ghost on 11
    let program_funk_drums = |engine: &mut Engine, variation: u8| {
        for i in 0..16 {
            engine.sequencer.steps[i] = match i {
                // Kick on 1, "and" of 3 (step 7), beat 3 (step 8)
                0 => Step::new(36, 0.95, 0.4),
                7 => Step::new(36, 0.70, 0.35),
                8 => Step::new(36, 0.85, 0.4),
                // Snare on 2 and 4 (steps 4, 12)
                4 => Step::new(38, 0.85, 0.35),
                12 => Step::new(38, 0.80, 0.35),
                // Ghost snare
                10 => if variation >= 1 {
                    Step::new(38, 0.30, 0.15)  // ghost snare
                } else {
                    Step::new(42, 0.35, 0.15)  // closed hat
                },
                // Open hat on "and" of 2 and 4
                6 => Step::new(46, 0.45, 0.25),
                14 => Step::new(46, 0.40, 0.25),
                // Clap accent (variation 2+)
                11 => if variation >= 2 {
                    Step::new(39, 0.40, 0.2)
                } else {
                    Step::new(42, 0.30, 0.15)
                },
                // 16th closed hats everywhere else
                _ => Step::new(42, 0.35, 0.15),
            };
        }
    };

    // =====================================================================
    // SECTION 1: INTRO (bars 1-4) — Drums + bass groove, establishing pocket
    // =====================================================================
    program_funk_drums(&mut engine, 0);
    // Overlay bass notes onto the drum pattern via direct note-on/off in loop
    engine.sequencer.start();

    // Bass note on (will be retriggered by pattern)
    engine.bass.note_on(21, 0.9); // A1

    let mut pos = 0;
    while pos < intro_end {
        let bl = BLOCK_SIZE.min(intro_end - pos);
        let section_pos = pos;
        let bar_pos = section_pos % samples_per_bar;
        let step_in_bar = bar_pos / samples_per_16th;

        // Manual bass note triggering synced to 16th notes
        let step_boundary = bar_pos % samples_per_16th;
        if step_boundary < BLOCK_SIZE {
            let step = step_in_bar % 16;
            let (note, vel, _, env_val, res_val) = bass_pattern[step];
            if vel > 0.0 {
                engine.bass.note_off(21);
                engine.bass.note_off(19);
                engine.bass.note_off(24);
                engine.bass.set_param(BassParam::CutoffEnv, env_val);
                engine.bass.set_param(BassParam::Resonance, res_val);
                engine.bass.note_on(note, vel);
            } else {
                // Rest — release bass for staccato feel
                engine.bass.note_off(21);
                engine.bass.note_off(19);
                engine.bass.note_off(24);
            }
        }

        engine.process_block_stereo(
            &mut samples_l[pos..pos + bl],
            &mut samples_r[pos..pos + bl],
        );
        pos += bl;
    }

    // =====================================================================
    // SECTION 2: VERSE 1 (bars 5-12) — Add clav stabs, fuller groove
    // =====================================================================
    program_funk_drums(&mut engine, 1); // ghost snare variation

    // FM clav melody: rhythmic stabs on Am chord tones
    // A3(57), C4(60), E4(64), G4(67) — Am7 chord
    let clav_pattern: [(u8, f32); 16] = [
        (57, 0.0),  // 0: rest (bass has downbeat)
        (0,  0.0),  // 1: rest
        (60, 0.50), // 2: C4 stab
        (0,  0.0),  // 3: rest
        (0,  0.0),  // 4: rest (snare)
        (64, 0.55), // 5: E4
        (57, 0.35), // 6: A3 ghost
        (0,  0.0),  // 7: rest
        (0,  0.0),  // 8: rest
        (67, 0.50), // 9: G4
        (0,  0.0),  // 10: rest
        (64, 0.40), // 11: E4 ghost
        (0,  0.0),  // 12: rest (snare)
        (60, 0.55), // 13: C4
        (0,  0.0),  // 14: rest
        (57, 0.30), // 15: A3 pickup
    ];
    let mut clav_active: Option<u8> = None;

    let verse1_start = intro_end;
    let verse1_len = verse1_end - verse1_start;

    while pos < verse1_end {
        let bl = BLOCK_SIZE.min(verse1_end - pos);
        let section_pos = pos - verse1_start;
        let progress = section_pos as f32 / verse1_len as f32;
        let global_bar_pos = pos % samples_per_bar;
        let step_in_bar = global_bar_pos / samples_per_16th;

        // Bass pattern (same syncopated line)
        let step_boundary = global_bar_pos % samples_per_16th;
        if step_boundary < BLOCK_SIZE {
            let step = step_in_bar % 16;
            let (note, vel, _, env_val, res_val) = bass_pattern[step];
            if vel > 0.0 {
                engine.bass.note_off(21);
                engine.bass.note_off(19);
                engine.bass.note_off(24);
                engine.bass.set_param(BassParam::CutoffEnv, env_val);
                engine.bass.set_param(BassParam::Resonance, res_val);
                engine.bass.note_on(note, vel);
            } else {
                engine.bass.note_off(21);
                engine.bass.note_off(19);
                engine.bass.note_off(24);
            }

            // Clav stabs
            let (clav_note, clav_vel) = clav_pattern[step];
            if clav_vel > 0.0 {
                if let Some(prev) = clav_active {
                    engine.fm.note_off(prev);
                }
                engine.fm.note_on(clav_note, clav_vel);
                clav_active = Some(clav_note);
            } else if clav_active.is_some() {
                if let Some(prev) = clav_active {
                    engine.fm.note_off(prev);
                }
                clav_active = None;
            }
        }

        // Gradually open wah base cutoff for building energy
        engine.bass.set_param(BassParam::Cutoff, 0.08 + progress * 0.06);

        engine.process_block_stereo(
            &mut samples_l[pos..pos + bl],
            &mut samples_r[pos..pos + bl],
        );
        pos += bl;
    }
    if let Some(prev) = clav_active { engine.fm.note_off(prev); clav_active = None; }

    // =====================================================================
    // SECTION 3: BRIDGE (bars 13-16) — Breakdown, filter sweep, tension
    // =====================================================================
    // Strip to kick + hats only
    for i in 0..16 {
        engine.sequencer.steps[i] = match i {
            0 | 8 => Step::new(36, 0.75, 0.4),
            _ => Step::new(42, 0.25, 0.15),
        };
    }

    // Keys pad enters: Am7 — A3(57), C4(60), E4(64), G4(67)
    engine.keys.note_on(57, 0.30);
    engine.keys.note_on(60, 0.30);
    engine.keys.note_on(64, 0.30);

    // Bass holds a long note, filter sweeps dramatically
    engine.bass.note_off(21); engine.bass.note_off(19); engine.bass.note_off(24);
    engine.bass.note_on(21, 0.70);

    // Increase reverb/delay for bridge atmosphere
    engine.reverb.set_mix(0.20);
    engine.delay.set_mix(0.18);

    let bridge_start = verse1_end;
    let bridge_len = bridge_end - bridge_start;

    while pos < bridge_end {
        let bl = BLOCK_SIZE.min(bridge_end - pos);
        let section_pos = pos - bridge_start;
        let progress = section_pos as f32 / bridge_len as f32;

        // Dramatic filter sweep: closed → wide open → closed
        let sweep = if progress < 0.5 {
            progress * 2.0 // 0 → 1
        } else {
            (1.0 - progress) * 2.0 // 1 → 0
        };
        engine.bass.set_param(BassParam::Cutoff, 0.06 + sweep * 0.35);
        engine.bass.set_param(BassParam::CutoffEnv, 0.3 + sweep * 0.5);
        engine.bass.set_param(BassParam::Resonance, 0.5 + sweep * 0.3);

        // Pad filter opens
        engine.keys.set_param(KeysParam::Cutoff, 0.15 + progress * 0.10);

        engine.process_block_stereo(
            &mut samples_l[pos..pos + bl],
            &mut samples_r[pos..pos + bl],
        );
        pos += bl;
    }

    // Release pad
    engine.keys.note_off(57);
    engine.keys.note_off(60);
    engine.keys.note_off(64);

    // =====================================================================
    // SECTION 4: VERSE 2 (bars 17-24) — Peak funk! Arp stabs + full groove
    // =====================================================================
    program_funk_drums(&mut engine, 2); // full variation: ghost snare + clap

    // Reset effects to dry/punchy
    engine.reverb.set_mix(0.10);
    engine.delay.set_mix(0.10);

    // Reset bass envelope filter to funky settings
    engine.bass.set_param(BassParam::Cutoff, 0.10);
    engine.bass.set_param(BassParam::CutoffEnv, 0.75);
    engine.bass.set_param(BassParam::Resonance, 0.80);

    // Arp chord stabs — bring in!
    engine.tracks[4].level = 0.45;
    if let Some(ref mut arp) = engine.tracks[4].arp {
        arp.set_gate(0.15); // super short staccato
        arp.rebuild_notes_from_harmony(&HarmonyContext::new(57, Scale::Minor));
        arp.start(1.0);
    }

    // Alternate bass line: variation with D2 and E2 for chord movement
    // Am(21) → Dm(14) → G(19) → C(24) → Am → Dm → Em(16) → Am
    let chord_bass_notes: [u8; 8] = [21, 14, 19, 24, 21, 14, 16, 21];

    let verse2_start = bridge_end;
    let verse2_len = verse2_end - verse2_start;

    while pos < verse2_end {
        let bl = BLOCK_SIZE.min(verse2_end - pos);
        let section_pos = pos - verse2_start;
        let progress = section_pos as f32 / verse2_len as f32;
        let global_bar_pos = pos % samples_per_bar;
        let step_in_bar = global_bar_pos / samples_per_16th;
        let bar_in_section = section_pos / samples_per_bar;

        // Change root note every bar for chord progression
        let bar_boundary = section_pos % samples_per_bar;
        if bar_boundary < BLOCK_SIZE {
            let root = chord_bass_notes[bar_in_section % 8];
            // Update bass pattern root for this bar
            engine.bass.note_off(21); engine.bass.note_off(14);
            engine.bass.note_off(19); engine.bass.note_off(24);
            engine.bass.note_off(16);
            engine.bass.note_on(root, 0.85);
        }

        // Bass retriggering on syncopated pattern
        let step_boundary = global_bar_pos % samples_per_16th;
        if step_boundary < BLOCK_SIZE {
            let step = step_in_bar % 16;
            let (_, vel, _, env_val, res_val) = bass_pattern[step];
            if vel > 0.0 {
                let root = chord_bass_notes[bar_in_section % 8];
                // Use pattern note offsets relative to root
                let note = match bass_pattern[step].0 {
                    21 => root,       // root
                    19 => root - 2,   // minor 7th below root
                    24 => root + 3,   // minor 3rd above root
                    _ => root,
                };
                engine.bass.note_off(21); engine.bass.note_off(14);
                engine.bass.note_off(19); engine.bass.note_off(24);
                engine.bass.note_off(16);
                engine.bass.set_param(BassParam::CutoffEnv, env_val);
                engine.bass.set_param(BassParam::Resonance, res_val);
                engine.bass.note_on(note, vel);
            } else {
                engine.bass.note_off(21); engine.bass.note_off(14);
                engine.bass.note_off(19); engine.bass.note_off(24);
                engine.bass.note_off(16);
            }

            // Clav stabs continue
            let (clav_note, clav_vel) = clav_pattern[step];
            if clav_vel > 0.0 {
                if let Some(prev) = clav_active {
                    engine.fm.note_off(prev);
                }
                engine.fm.note_on(clav_note, clav_vel);
                clav_active = Some(clav_note);
            } else if clav_active.is_some() {
                if let Some(prev) = clav_active {
                    engine.fm.note_off(prev);
                }
                clav_active = None;
            }
        }

        // Wah intensity builds across verse
        let wah_boost = progress * 0.10;
        engine.bass.set_param(BassParam::Cutoff, 0.10 + wah_boost);

        engine.process_block_stereo(
            &mut samples_l[pos..pos + bl],
            &mut samples_r[pos..pos + bl],
        );
        pos += bl;
    }
    if let Some(prev) = clav_active { engine.fm.note_off(prev); }

    // =====================================================================
    // SECTION 5: OUTRO (bars 25-32) — Wind down, strip elements
    // =====================================================================
    // Simpler drums
    program_funk_drums(&mut engine, 0);

    // Arp fades
    engine.tracks[4].level = 0.3;

    // Bass back to Am root
    engine.bass.note_off(21); engine.bass.note_off(14);
    engine.bass.note_off(19); engine.bass.note_off(24);
    engine.bass.note_off(16);
    engine.bass.note_on(21, 0.75);

    let outro_start = verse2_end;
    let outro_len = outro_end - outro_start;

    while pos < outro_end {
        let bl = BLOCK_SIZE.min(outro_end - pos);
        let section_pos = pos - outro_start;
        let progress = section_pos as f32 / outro_len as f32;
        let global_bar_pos = pos % samples_per_bar;
        let step_in_bar = global_bar_pos / samples_per_16th;

        // Bass pattern continues but wah closes down
        let step_boundary = global_bar_pos % samples_per_16th;
        if step_boundary < BLOCK_SIZE {
            let step = step_in_bar % 16;
            let (note, vel, _, _, _) = bass_pattern[step];
            if vel > 0.0 {
                engine.bass.note_off(21);
                engine.bass.note_off(19);
                engine.bass.note_off(24);
                engine.bass.note_on(note, vel * (1.0 - progress * 0.4));
            } else {
                engine.bass.note_off(21);
                engine.bass.note_off(19);
                engine.bass.note_off(24);
            }
        }

        // Wah closes gradually
        engine.bass.set_param(BassParam::CutoffEnv, 0.75 - progress * 0.50);
        engine.bass.set_param(BassParam::Resonance, 0.80 - progress * 0.40);
        engine.bass.set_param(BassParam::Cutoff, 0.10 - progress * 0.05);

        // Arp fades to zero
        engine.tracks[4].level = 0.3 * (1.0 - progress);

        // EQ darkens
        engine.set_eq_high(1.0 - progress * 3.0);
        engine.set_eq_mid(2.0 - progress * 1.5);

        // Strip drums at bar 6 of outro
        if section_pos >= samples_per_bar * 6 && section_pos < samples_per_bar * 6 + BLOCK_SIZE {
            for i in 0..16 {
                engine.sequencer.steps[i] = match i {
                    0 => Step::new(36, 0.5, 0.4),
                    _ => Step::empty(),
                };
            }
        }

        // Volume fade last 2 bars
        let fade = if progress > 0.75 {
            1.0 - (progress - 0.75) / 0.25
        } else {
            1.0
        };

        engine.process_block_stereo(
            &mut samples_l[pos..pos + bl],
            &mut samples_r[pos..pos + bl],
        );

        if fade < 1.0 {
            for i in 0..bl {
                samples_l[pos + i] *= fade;
                samples_r[pos + i] *= fade;
            }
        }

        pos += bl;
    }

    // Cleanup
    engine.bass.note_off(21);
    engine.bass.note_off(19);
    engine.bass.note_off(24);

    // --- Verify ---
    let max_val = samples_l.iter().chain(samples_r.iter())
        .fold(0.0f32, |a, &b| if b.abs() > a { b.abs() } else { a });
    assert!(max_val > 0.01, "Funk Envelope Filter test should produce audible output, max={}", max_val);

    write_wav_stereo(
        &output_path("test_funk_envelope_filter.wav"),
        &samples_l,
        &samples_r,
        SAMPLE_RATE as u32,
    );
}
