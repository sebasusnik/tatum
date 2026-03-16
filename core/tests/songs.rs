//! Full song integration tests — renders complete tracks to WAV files.
//!
//! Moved out of `engine.rs` to keep the core module lean.
//! Run with: `cargo test --test songs`

use synth_core::engine::Engine;
use synth_core::harmony::{HarmonyContext, Scale};
use synth_core::modules::bass::BassParam;
use synth_core::modules::fm::FmParam;
use synth_core::modules::keys::KeysParam;
use synth_core::modules::arp::ArpParam;
use synth_core::effects::delay::DelaySync;
use synth_core::sequencer::Step;
use synth_core::{Module, SAMPLE_RATE, BLOCK_SIZE};

mod test_helpers;
use test_helpers::{write_wav_stereo, output_path};

#[test]
fn test_techno_robot() {
    // Sven Väth — "Robot" (1994, Cocoon) — dark, heavy 90s hard trance
    // 140 BPM, A# minor (C# tonal center). Co-produced with Ralf Hildenbeutel.
    //
    // Core elements (from analysis + description):
    //   1. TR-909 drums: unrelenting 4/4 kick, 16th closed hats, off-beat opens
    //   2. THE ARP: repetitive sequenced arp — the SOUL of the track.
    //      Notes don't change much; the FILTER CUTOFF automation does all the work.
    //      Filter starts closed (dark/muffled), slowly opens → acidic peak.
    //   3. Rolling syncopated bassline bouncing between kick hits
    //   4. Metallic/robotic percussive FX hits
    //   5. Heavy delay + reverb on breakdowns
    //
    // Structure: Intro → Build (filter opening) → Climax → Breakdown → Drop → Outro
    // Compressed from 7:46 into ~50s (29 bars)

    let mut engine = Engine::new();
    let bpm = 140.0;
    engine.set_bpm(bpm);

    // C# minor
    let mut ctx = HarmonyContext::new(49, Scale::Minor);
    ctx.set_chord_degree(0);
    engine.set_harmony(ctx);

    // --- Effects: dark, heavy, lots of delay ---
    engine.set_delay_sync(DelaySync::DottedEighth);
    engine.delay.set_feedback(0.45);       // heavy feedback — delays are key
    engine.delay.set_mix(0.20);
    engine.set_delay_filter(0.5);          // dark delay tails

    engine.reverb.set_room_size(0.6);
    engine.reverb.set_damping(0.5);
    engine.reverb.set_mix(0.18);

    engine.set_eq_low(5.0);                // heavy bottom
    engine.set_eq_mid(1.0);
    engine.set_eq_high(-4.0);              // dark top
    engine.set_tilt_eq(0.4);

    engine.set_sidechain_amount(0.7);      // pumping

    // --- Bass: rolling syncopated C#2 ---
    engine.bass.set_param(BassParam::Cutoff, 0.15);
    engine.bass.set_param(BassParam::CutoffEnv, 0.6);   // punchy envelope
    engine.bass.set_param(BassParam::Resonance, 0.5);
    engine.bass.set_param(BassParam::Glide, 0.15);
    engine.bass.set_param(BassParam::Osc2Pitch, (12.0 + 24.0) / 48.0); // sub -12st
    engine.bass.set_param(BassParam::Osc1Wave, 0.0);    // saw

    // --- Arp: THE main element — repetitive C# minor arp with filter sweep ---
    engine.arp.set_param(ArpParam::Rate, (140.0 - 60.0) / 180.0); // sync to BPM
    engine.arp.set_param(ArpParam::Gate, 0.6);           // tight gates
    engine.arp.set_param(ArpParam::Level, 0.5);          // prominent

    // --- FM: metallic robotic percussive hits ---
    engine.fm.set_param(FmParam::Algorithm, 4.0 / 7.0); // serial chain = metallic
    engine.fm.set_param(FmParam::ModIndex, 0.7 / 8.0);
    engine.fm.set_param(FmParam::Feedback, 0.25);
    engine.fm.set_op_envelope(0, 0.001, 0.08, 0.0, 0.05); // ultra short
    engine.fm.set_op_envelope(1, 0.001, 0.06, 0.0, 0.03);

    // --- Keys: dark pad for breakdown atmosphere ---
    engine.keys.set_param(KeysParam::VoiceMode, 0.2);   // Unison
    engine.keys.set_param(KeysParam::Cutoff, 0.08);
    engine.keys.set_param(KeysParam::Detune, 0.2);
    engine.keys.set_param(KeysParam::ChorusMix, 0.3);

    // --- Timing ---
    let samples_per_beat = (SAMPLE_RATE * 60.0 / bpm) as usize;
    let samples_per_bar = samples_per_beat * 4;
    let section_4bar = samples_per_bar * 4;

    // Sections: intro(4) + build(8) + climax(4) + breakdown(4) + drop(4) + outro(5)
    let total = samples_per_bar * 29;
    let mut samples_l = vec![0.0f32; total];
    let mut samples_r = vec![0.0f32; total];

    // ====================================================================
    // SECTION 1: INTRO — bars 1-4
    // Heavy kick + minimal percussion + bass skeleton.
    // "Stripped back. Mostly the heavy kick drum."
    // ====================================================================

    // TR-909: 4/4 kick + 16th closed hats (mechanical)
    for i in 0..16 {
        engine.sequencer.steps[i] = match i {
            0 | 4 | 8 | 12 => Step::new(36, 1.0, 0.5),       // kick
            _ => Step::new(42, 0.30, 0.25),                   // 16th hats
        };
    }
    // Off-beat open hats
    engine.sequencer.steps[2] = Step::new(46, 0.4, 0.3);
    engine.sequencer.steps[6] = Step::new(46, 0.35, 0.3);
    engine.sequencer.steps[10] = Step::new(46, 0.4, 0.3);
    engine.sequencer.steps[14] = Step::new(46, 0.35, 0.3);

    engine.sequencer.start();

    // Rolling bass — syncopated, bounces between kicks
    // Hits on off-16ths: steps 1, 3, 5, 9, 11
    engine.bass.note_on(37, 0.85); // C#2, sustained

    let sec1_end = section_4bar;
    let mut pos = 0;
    while pos < sec1_end {
        let bl = BLOCK_SIZE.min(sec1_end - pos);
        engine.process_block_stereo(
            &mut samples_l[pos..pos + bl],
            &mut samples_r[pos..pos + bl],
        );
        pos += bl;
    }

    // ====================================================================
    // SECTION 2: BUILD-UP — bars 5-12 (8 bars)
    // "The main arp is introduced with filter CLOSED (dark/muffled).
    //  As the track progresses, the filter slowly opens."
    // This is the heart of the track's tension.
    // ====================================================================

    // Arp starts — filter closed, will sweep open
    engine.arp.note_on(0, 1.0); // triggers rebuild from C# minor harmony

    // Metallic hits on beat 3 every 2 bars
    let sec2_start = sec1_end;
    let sec2_end = sec2_start + samples_per_bar * 8;
    let sec2_len = sec2_end - sec2_start;

    while pos < sec2_end {
        let bl = BLOCK_SIZE.min(sec2_end - pos);
        let section_pos = pos - sec2_start;

        // THE KEY AUTOMATION: arp filter cutoff sweep 0.04 → 0.40
        // This is what makes "Robot" sound like "Robot"
        let progress = section_pos as f32 / sec2_len as f32;
        // Bass filter also opens slightly
        engine.bass.set_param(BassParam::Cutoff, 0.15 + progress * 0.10);
        // EQ opens up
        engine.set_eq_high(-4.0 + progress * 2.0);
        engine.set_tilt_eq(0.4 - progress * 0.2);

        // Metallic FM hit every 2 bars at beat 3
        let two_bar = samples_per_bar * 2;
        let hit_pos = samples_per_beat * 2;
        let bar_pos = section_pos % two_bar;
        if bar_pos >= hit_pos && bar_pos < hit_pos + BLOCK_SIZE {
            engine.fm.note_on(61, 0.5); // C#4 metallic zap
        }
        if bar_pos >= hit_pos + BLOCK_SIZE * 3 && bar_pos < hit_pos + BLOCK_SIZE * 4 {
            engine.fm.note_off(61);
        }

        engine.process_block_stereo(
            &mut samples_l[pos..pos + bl],
            &mut samples_r[pos..pos + bl],
        );
        pos += bl;
    }

    // ====================================================================
    // SECTION 3: CLIMAX — bars 13-16
    // "Filter fully open. Ride cymbals. Peak hypnotic state."
    // Maximum energy, everything wide open.
    // ====================================================================

    // Add ride pattern — use open hats more aggressively
    for i in 0..16 {
        engine.sequencer.steps[i] = match i {
            0 | 4 | 8 | 12 => Step::new(36, 1.0, 0.5),     // kick
            2 | 6 | 10 | 14 => Step::new(46, 0.55, 0.35),  // open hats (rides)
            1 | 5 | 9 | 13 => Step::new(42, 0.45, 0.2),    // 16th hats
            _ => Step::new(42, 0.35, 0.2),                  // fill hats
        };
    }
    // Clap on 4 and 12
    engine.sequencer.steps[4] = Step::new(39, 0.8, 0.4);
    engine.sequencer.steps[12] = Step::new(39, 0.75, 0.4);

    engine.set_sidechain_amount(0.8);
    engine.bass.set_param(BassParam::Cutoff, 0.30);       // open bass filter

    let sec3_start = sec2_end;
    let sec3_end = sec3_start + section_4bar;

    while pos < sec3_end {
        let bl = BLOCK_SIZE.min(sec3_end - pos);
        let section_pos = pos - sec3_start;

        // FM hits every bar now — more intense
        let pos_in_bar = section_pos % samples_per_bar;
        let hit_pos = samples_per_beat * 2 + samples_per_beat / 2; // beat 2.5
        if pos_in_bar >= hit_pos && pos_in_bar < hit_pos + BLOCK_SIZE {
            engine.fm.note_on(61, 0.6);
        }
        if pos_in_bar >= hit_pos + BLOCK_SIZE * 2 && pos_in_bar < hit_pos + BLOCK_SIZE * 3 {
            engine.fm.note_off(61);
        }

        engine.process_block_stereo(
            &mut samples_l[pos..pos + bl],
            &mut samples_r[pos..pos + bl],
        );
        pos += bl;
    }

    // ====================================================================
    // SECTION 4: BREAKDOWN — bars 17-20
    // "Kick drops out. Synths alone with heavy delay + reverb.
    //  Tense moment of suspension."
    // ====================================================================

    // Kill the kick — only synths + effects
    for i in 0..16 {
        engine.sequencer.steps[i] = Step::empty();
    }

    engine.fm.note_off(61);

    // Increase delay/reverb for spacious breakdown
    engine.delay.set_feedback(0.55);
    engine.delay.set_mix(0.30);
    engine.reverb.set_mix(0.30);
    engine.reverb.set_room_size(0.8);

    // Keys pad enters — dark C#m chord (suspended atmosphere)
    engine.keys.note_on(49, 0.4);  // C#3
    engine.keys.note_on(52, 0.4);  // E3
    engine.keys.note_on(56, 0.4);  // G#3

    let sec4_start = sec3_end;
    let sec4_end = sec4_start + section_4bar;

    while pos < sec4_end {
        let bl = BLOCK_SIZE.min(sec4_end - pos);
        let section_pos = pos - sec4_start;
        let progress = section_pos as f32 / section_4bar as f32;

        // Slowly darken — filter closing builds tension for the drop
        engine.bass.set_param(BassParam::Cutoff, 0.30 - progress * 0.15);
        engine.keys.set_param(KeysParam::Cutoff, 0.08 + progress * 0.05);
        engine.set_tilt_eq(0.2 + progress * 0.3);

        engine.process_block_stereo(
            &mut samples_l[pos..pos + bl],
            &mut samples_r[pos..pos + bl],
        );
        pos += bl;
    }

    // ====================================================================
    // SECTION 5: THE DROP — bars 21-24
    // "Drums SLAM back in alongside fully opened synth lines."
    // Maximum impact moment.
    // ====================================================================

    // Kill pad, restore effects to normal
    engine.keys.note_off(49);
    engine.keys.note_off(52);
    engine.keys.note_off(56);
    engine.delay.set_feedback(0.45);
    engine.delay.set_mix(0.20);
    engine.reverb.set_mix(0.18);
    engine.reverb.set_room_size(0.6);

    // FULL drum pattern slams back
    for i in 0..16 {
        engine.sequencer.steps[i] = match i {
            0 | 4 | 8 | 12 => Step::new(36, 1.0, 0.5),     // kick HARD
            2 | 6 | 10 | 14 => Step::new(46, 0.55, 0.35),  // open hats
            _ => Step::new(42, 0.40, 0.25),                 // dense 16th hats
        };
    }
    engine.sequencer.steps[4] = Step::new(39, 0.85, 0.4); // clap
    engine.sequencer.steps[12] = Step::new(39, 0.80, 0.4);

    engine.set_sidechain_amount(0.8);
    engine.bass.set_param(BassParam::Cutoff, 0.30);
    engine.set_tilt_eq(0.15);
    engine.set_eq_high(-1.0); // bright — fully open

    let sec5_start = sec4_end;
    let sec5_end = sec5_start + section_4bar;

    while pos < sec5_end {
        let bl = BLOCK_SIZE.min(sec5_end - pos);
        let section_pos = pos - sec5_start;

        // Metallic hits every bar
        let pos_in_bar = section_pos % samples_per_bar;
        let hit_pos = samples_per_beat * 2;
        if pos_in_bar >= hit_pos && pos_in_bar < hit_pos + BLOCK_SIZE {
            engine.fm.note_on(61, 0.6);
        }
        if pos_in_bar >= hit_pos + BLOCK_SIZE * 2 && pos_in_bar < hit_pos + BLOCK_SIZE * 3 {
            engine.fm.note_off(61);
        }

        engine.process_block_stereo(
            &mut samples_l[pos..pos + bl],
            &mut samples_r[pos..pos + bl],
        );
        pos += bl;
    }

    // ====================================================================
    // SECTION 6: OUTRO — bars 25-29
    // "Elements muted one by one: synths fade, hats, bass, until only beat."
    // ====================================================================

    engine.fm.note_off(61);

    // Gradually strip elements
    let sec6_start = sec5_end;
    let sec6_bars = 5;
    let sec6_end = sec6_start + samples_per_bar * sec6_bars;
    let sec6_len = sec6_end - sec6_start;

    while pos < sec6_end {
        let bl = BLOCK_SIZE.min(sec6_end - pos);
        let section_pos = pos - sec6_start;
        let progress = section_pos as f32 / sec6_len as f32;

        // Bar 2 of outro: kill arp
        if section_pos >= samples_per_bar && section_pos < samples_per_bar + BLOCK_SIZE {
            engine.arp.set_param(ArpParam::Level, 0.0);
        }

        // Bar 3: remove hats, only kick
        if section_pos >= samples_per_bar * 2 && section_pos < samples_per_bar * 2 + BLOCK_SIZE {
            for i in 0..16 {
                engine.sequencer.steps[i] = match i {
                    0 | 4 | 8 | 12 => Step::new(36, 0.9, 0.5),
                    _ => Step::empty(),
                };
            }
        }

        // Filter closes back down
        engine.bass.set_param(BassParam::Cutoff, 0.30 - progress * 0.22);
        engine.set_eq_high(-1.0 - progress * 3.0);

        // Volume fade on last 2 bars
        let fade = if progress > 0.6 {
            1.0 - (progress - 0.6) / 0.4
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

    engine.bass.note_off(37);

    // --- Verify ---
    let max_val = samples_l.iter().chain(samples_r.iter())
        .fold(0.0f32, |a, &b| if b.abs() > a { b.abs() } else { a });
    assert!(max_val > 0.01, "Techno Robot test should produce audible output, max={}", max_val);

    write_wav_stereo(
        &output_path("test_techno_robot.wav"),
        &samples_l,
        &samples_r,
        SAMPLE_RATE as u32,
    );
}

/// Dreamy retro synthwave — lush pads, smooth bass, filtered arps, FM bells.
/// 85 BPM, E minor, ~79 seconds (28 bars).
#[test]
fn test_synthwave_chill() {
    let mut engine = Engine::new();
    let bpm = 85.0;
    engine.set_bpm(bpm);

    // --- Harmony: E minor (root = E3 = MIDI 52) ---
    let mut ctx = HarmonyContext::new(52, Scale::Minor);
    ctx.set_chord_degree(0);
    engine.set_harmony(ctx);

    // --- Effects: spacious, washed out, warm ---
    engine.set_delay_sync(DelaySync::DottedEighth);
    engine.delay.set_feedback(0.50);
    engine.delay.set_mix(0.25);
    engine.set_delay_filter(0.5);

    engine.reverb.set_room_size(0.75);
    engine.reverb.set_damping(0.4);
    engine.reverb.set_mix(0.30);

    engine.set_eq_low(3.0);
    engine.set_eq_mid(0.0);
    engine.set_eq_high(-1.0);
    engine.set_tilt_eq(0.15);

    engine.set_sidechain_amount(0.3);

    // --- Bass: smooth, sustained, sub-heavy (square wave = warmer retro tone) ---
    engine.bass.set_param(BassParam::Cutoff, 0.12);
    engine.bass.set_param(BassParam::CutoffEnv, 0.2);
    engine.bass.set_param(BassParam::Resonance, 0.25);
    engine.bass.set_param(BassParam::Glide, 0.3);  // longer glide for smooth slides
    engine.bass.set_param(BassParam::Osc2Pitch, (-12.0 + 24.0) / 48.0); // -12st sub
    engine.bass.set_param(BassParam::Osc1Wave, 1.0); // square — rounder, retro

    // --- Arp: filtered 16ths, UpDown pattern for melodic sweep ---
    engine.arp.set_param(ArpParam::Rate, (bpm - 60.0) / 180.0);
    engine.arp.set_param(ArpParam::Gate, 0.6);
    engine.arp.set_param(ArpParam::Level, 0.35);
    engine.arp.set_param(ArpParam::Pattern, 1.0); // UpDown

    // --- FM: glassy bell, longer sustain for dreamy feel ---
    engine.fm.set_param(FmParam::Algorithm, 1.0 / 7.0); // algo 1 — simple carrier+mod
    engine.fm.set_param(FmParam::ModIndex, 0.25 / 8.0); // lower mod = purer bell
    engine.fm.set_param(FmParam::Feedback, 0.05);
    engine.fm.set_param(FmParam::ChorusMix, 0.3); // chorus on bells
    engine.fm.set_op_envelope(0, 0.01, 0.4, 0.15, 0.5); // slow attack, long decay+release

    // --- Keys pad: thick unison, chorus, vibrato shimmer, very dark ---
    engine.keys.set_param(KeysParam::VoiceMode, 0.25); // Unison
    engine.keys.set_param(KeysParam::Cutoff, 0.10);
    engine.keys.set_param(KeysParam::Detune, 0.25);
    engine.keys.set_param(KeysParam::ChorusMix, 0.5);
    engine.keys.set_param(KeysParam::VibratoRate, 0.15); // slow shimmer
    engine.keys.set_param(KeysParam::VibratoDepth, 0.12); // subtle pitch wobble

    // --- Buffers ---
    let samples_per_beat = (SAMPLE_RATE * 60.0 / bpm) as usize;
    let samples_per_bar = samples_per_beat * 4;

    let total_bars = 28;
    let total = samples_per_bar * total_bars;
    let mut samples_l = vec![0.0f32; total];
    let mut samples_r = vec![0.0f32; total];

    // Section boundaries (in samples)
    let intro_end = samples_per_bar * 4;       // bars 1-4
    let verse_end = samples_per_bar * 12;      // bars 5-12
    let chorus_end = samples_per_bar * 20;     // bars 13-20
    let outro_end = samples_per_bar * 28;      // bars 21-28

    // =====================================================================
    // SECTION 1: INTRO (bars 1-4) — Lush pad only, no drums
    // =====================================================================
    // Sequencer off for intro — clear all steps
    for i in 0..16 {
        engine.sequencer.steps[i] = Step::empty();
    }

    // Keys pad: Em chord — E3(52), G3(55), B3(59)
    engine.keys.note_on(52, 0.4);
    engine.keys.note_on(55, 0.4);
    engine.keys.note_on(59, 0.4);

    let mut pos = 0;
    while pos < intro_end {
        let bl = BLOCK_SIZE.min(intro_end - pos);
        let progress = pos as f32 / intro_end as f32;

        // Pad filter opens gently during intro
        engine.keys.set_param(KeysParam::Cutoff, 0.10 + progress * 0.03);

        engine.process_block_stereo(
            &mut samples_l[pos..pos + bl],
            &mut samples_r[pos..pos + bl],
        );
        pos += bl;
    }

    // =====================================================================
    // SECTION 2: VERSE (bars 5-12) — Soft drums, bass slides in, arp opens
    // =====================================================================
    // Drum pattern: half-time feel
    // Kick: 1, 9 | Snare: 5, 13 | HH: 8th notes | Open: 7, 15
    for i in 0..16 {
        engine.sequencer.steps[i] = match i {
            0 | 8 => Step::new(36, 0.8, 0.5),                   // kick
            4 | 12 => Step::new(38, 0.6, 0.4),                  // snare
            6 | 14 => Step::new(46, 0.35, 0.3),                 // open hat
            1 | 3 | 5 | 7 | 9 | 11 | 13 | 15 => Step::new(42, 0.25, 0.2), // 8th hats
            _ => Step::empty(),
        };
    }
    engine.sequencer.start();

    // Bass: E2 (MIDI 28)
    engine.bass.note_on(28, 0.75);

    // Arp: start dark, will sweep open
    engine.arp.note_on(0, 1.0);

    let verse_start = intro_end;
    let verse_len = verse_end - verse_start;
    while pos < verse_end {
        let bl = BLOCK_SIZE.min(verse_end - pos);
        let section_pos = pos - verse_start;
        let progress = section_pos as f32 / verse_len as f32;

        // Bass filter opens gradually
        engine.bass.set_param(BassParam::Cutoff, 0.15 + progress * 0.10);

        // Pad filter continues opening
        engine.keys.set_param(KeysParam::Cutoff, 0.13 + progress * 0.05);

        // EQ warms up
        engine.set_tilt_eq(0.15 - progress * 0.05);

        // Bass note changes every 2 bars following Em progression
        let two_bars = samples_per_bar * 2;
        let bar_in_section = section_pos / two_bars;
        let bar_boundary = section_pos % two_bars;
        if bar_boundary < BLOCK_SIZE {
            let bass_note = match bar_in_section {
                0 => 28, // E2
                1 => 26, // D2
                2 => 24, // C2
                _ => 23, // B1
            };
            engine.bass.note_off(28);
            engine.bass.note_on(bass_note, 0.75);
        }

        engine.process_block_stereo(
            &mut samples_l[pos..pos + bl],
            &mut samples_r[pos..pos + bl],
        );
        pos += bl;
    }

    // =====================================================================
    // SECTION 3: CHORUS (bars 13-20) — Full energy, FM bell melody
    // =====================================================================
    // Upgraded drum pattern: add open hats on more beats
    for i in 0..16 {
        engine.sequencer.steps[i] = match i {
            0 | 8 => Step::new(36, 0.9, 0.5),                   // kick
            4 | 12 => Step::new(38, 0.7, 0.4),                  // snare
            2 | 6 | 10 | 14 => Step::new(46, 0.4, 0.3),        // open hats
            _ => Step::new(42, 0.30, 0.2),                      // 16th hats
        };
    }

    // Bright arp
    engine.arp.set_param(ArpParam::Level, 0.5);

    // Pad sustained, filter more open
    engine.keys.set_param(KeysParam::Cutoff, 0.20);

    // Bass back to root
    engine.bass.note_off(23);
    engine.bass.note_on(28, 0.80);
    engine.bass.set_param(BassParam::Cutoff, 0.25);

    // FM bell melody: arpeggiated across beats, not just downbeats
    // Pattern per bar: beat 1 + beat 2.5 (dotted quarter feel)
    let fm_melody = [64_u8, 67, 71, 74, 76, 71, 67, 64]; // E4 G4 B4 D5 E5 B4 G4 E4

    let chorus_start = verse_end;
    let chorus_len = chorus_end - chorus_start;
    let mut fm_note_active: Option<u8> = None;
    let mut fm_melody_idx = 0_usize;

    while pos < chorus_end {
        let bl = BLOCK_SIZE.min(chorus_end - pos);
        let section_pos = pos - chorus_start;
        let progress = section_pos as f32 / chorus_len as f32;

        // EQ opens up for chorus brightness
        engine.set_eq_high(-1.0 + progress * 1.5);

        // Vibrato deepens during chorus
        engine.keys.set_param(KeysParam::VibratoDepth, 0.12 + progress * 0.08);

        // FM bells on beat 1 and beat 3 of each bar (half-note rhythm)
        let half_bar = samples_per_bar / 2;
        let half_pos = section_pos % half_bar;
        if half_pos < BLOCK_SIZE {
            if let Some(prev) = fm_note_active {
                engine.fm.note_off(prev);
            }
            let note = fm_melody[fm_melody_idx % fm_melody.len()];
            engine.fm.note_on(note, 0.40);
            fm_note_active = Some(note);
            fm_melody_idx += 1;
        }

        // Chord progression: 2 bars each — update both bass AND pad voicing
        let two_bars = samples_per_bar * 2;
        let chord_idx = section_pos / two_bars;
        let chord_boundary = section_pos % two_bars;
        if chord_boundary < BLOCK_SIZE {
            match chord_idx {
                0 => {
                    // Em: E2 bass, E3-G3-B3 pad
                    engine.bass.note_off(23); engine.bass.note_off(24);
                    engine.bass.note_off(26); engine.bass.note_off(28);
                    engine.bass.note_on(28, 0.80);
                }
                1 => {
                    // Dm: D2 bass
                    engine.bass.note_off(28);
                    engine.bass.note_on(26, 0.80);
                    // Shift pad to D-F-A
                    engine.keys.note_off(52); engine.keys.note_off(55); engine.keys.note_off(59);
                    engine.keys.note_on(50, 0.4); // D3
                    engine.keys.note_on(53, 0.4); // F3
                    engine.keys.note_on(57, 0.4); // A3
                }
                2 => {
                    // C: C2 bass
                    engine.bass.note_off(26);
                    engine.bass.note_on(24, 0.80);
                    // Pad to C-E-G
                    engine.keys.note_off(50); engine.keys.note_off(53); engine.keys.note_off(57);
                    engine.keys.note_on(48, 0.4); // C3
                    engine.keys.note_on(52, 0.4); // E3
                    engine.keys.note_on(55, 0.4); // G3
                }
                _ => {
                    // Bm: B1 bass
                    engine.bass.note_off(24);
                    engine.bass.note_on(23, 0.80);
                    // Pad to B-D-F#
                    engine.keys.note_off(48); engine.keys.note_off(52); engine.keys.note_off(55);
                    engine.keys.note_on(47, 0.4); // B2
                    engine.keys.note_on(50, 0.4); // D3
                    engine.keys.note_on(54, 0.4); // F#3
                }
            }
        }

        engine.process_block_stereo(
            &mut samples_l[pos..pos + bl],
            &mut samples_r[pos..pos + bl],
        );
        pos += bl;
    }

    // Release FM note if active
    if let Some(prev) = fm_note_active {
        engine.fm.note_off(prev);
    }

    // =====================================================================
    // SECTION 4: OUTRO (bars 21-28) — Strip to pad + bass, fade out
    // =====================================================================
    // Strip drums: kick only, very soft
    for i in 0..16 {
        engine.sequencer.steps[i] = match i {
            0 | 8 => Step::new(36, 0.5, 0.5), // soft kick
            _ => Step::empty(),
        };
    }

    // Arp fades out
    engine.arp.set_param(ArpParam::Level, 0.2);

    // Release all possible bass/pad notes from chorus, return to Em
    for n in [23_u8, 24, 26, 28, 47, 48, 50, 52, 53, 54, 55, 57] {
        engine.bass.note_off(n);
        engine.keys.note_off(n);
    }
    // Bass back to root
    engine.bass.note_on(28, 0.65);
    // Pad back to Em
    engine.keys.note_on(52, 0.35);
    engine.keys.note_on(55, 0.35);
    engine.keys.note_on(59, 0.35);

    // Effects: more reverb/delay for tails
    engine.delay.set_feedback(0.55);
    engine.reverb.set_mix(0.35);

    let outro_start = chorus_end;
    let outro_len = outro_end - outro_start;

    while pos < outro_end {
        let bl = BLOCK_SIZE.min(outro_end - pos);
        let section_pos = pos - outro_start;
        let progress = section_pos as f32 / outro_len as f32;

        // Arp fades to zero by halfway
        if progress < 0.5 {
            engine.arp.set_param(ArpParam::Level, 0.2 * (1.0 - progress * 2.0));
        } else {
            engine.arp.set_param(ArpParam::Level, 0.0);
        }

        // Filter closes back down
        engine.bass.set_param(BassParam::Cutoff, 0.25 - progress * 0.15);
        engine.keys.set_param(KeysParam::Cutoff, 0.20 - progress * 0.12);

        // EQ darkens
        engine.set_eq_high(-1.0 - progress * 2.0);
        engine.set_tilt_eq(0.10 + progress * 0.2);

        // Strip drums entirely after bar 2 of outro
        if section_pos >= samples_per_bar * 2 && section_pos < samples_per_bar * 2 + BLOCK_SIZE {
            for i in 0..16 {
                engine.sequencer.steps[i] = Step::empty();
            }
        }

        // Volume fade on last 3 bars
        let fade = if progress > 0.625 {
            1.0 - (progress - 0.625) / 0.375
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

    // Cleanup — release everything
    engine.bass.note_off(28);
    engine.keys.note_off(52);
    engine.keys.note_off(55);
    engine.keys.note_off(59);

    // --- Verify ---
    let max_val = samples_l.iter().chain(samples_r.iter())
        .fold(0.0f32, |a, &b| if b.abs() > a { b.abs() } else { a });
    assert!(max_val > 0.01, "Synthwave Chill test should produce audible output, max={}", max_val);

    write_wav_stereo(
        &output_path("test_synthwave_chill.wav"),
        &samples_l,
        &samples_r,
        SAMPLE_RATE as u32,
    );
}

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
    engine.fm.set_param(FmParam::ModIndex, 5.0 / 8.0);  // high mod = bright, clav-like
    engine.fm.set_param(FmParam::Feedback, 0.4);         // adds bite
    engine.fm.set_op_envelope(0, 0.001, 0.12, 0.05, 0.08); // ultra-snappy: percussive clav

    // --- Arp: rhythmic chord stabs, short gate ---
    engine.arp.set_param(ArpParam::Rate, (bpm - 60.0) / 180.0);
    engine.arp.set_param(ArpParam::Gate, 0.2);  // short staccato stabs
    engine.arp.set_param(ArpParam::Level, 0.0);  // start silent, bring in later
    engine.arp.set_param(ArpParam::Pattern, 0.0); // Up

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
    engine.arp.set_param(ArpParam::Level, 0.45);
    engine.arp.set_param(ArpParam::Gate, 0.15); // super short staccato
    engine.arp.note_on(0, 1.0);

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
    engine.arp.set_param(ArpParam::Level, 0.3);

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
        engine.arp.set_param(ArpParam::Level, 0.3 * (1.0 - progress));

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
