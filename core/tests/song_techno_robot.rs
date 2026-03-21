//! Sven Väth — "Robot" inspired test song.
//! Run with: `cargo test --test song_techno_robot`

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

    // --- Arp on track 4: ArpProcessor drives Keys[1] ---
    engine.tracks[4].kind = InstrumentKind::Keys;
    engine.tracks[4].instance_idx = 1;
    engine.tracks[4].level = 0.5;
    engine.keys2.set_param(KeysParam::Cutoff, 0.15);
    engine.keys2.set_param(KeysParam::Attack, 0.0);
    engine.keys2.set_param(KeysParam::Decay, 0.15);
    engine.keys2.set_param(KeysParam::Sustain, 0.3);
    engine.keys2.set_param(KeysParam::Release, 0.08);
    {
        let mut arp = ArpProcessor::new();
        arp.set_bpm(140.0);
        arp.set_gate(0.6);
        engine.tracks[4].arp = Some(arp);
    }

    // --- FM: metallic robotic percussive hits ---
    engine.fm.set_param(FmParam::Algorithm, 4.0 / 7.0); // serial chain = metallic
    engine.fm.set_param(FmParam::ModIndex, 0.34);  // → mod_index ≈ 0.35
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
    if let Some(ref mut arp) = engine.tracks[4].arp {
        arp.rebuild_notes_from_harmony(&HarmonyContext::new(49, Scale::Minor));
        arp.start(1.0);
    }

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
            engine.tracks[4].level = 0.0;
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
