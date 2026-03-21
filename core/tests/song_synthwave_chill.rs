//! Dreamy retro synthwave test song.
//! Run with: `cargo test --test song_synthwave_chill`

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

    // --- Arp on track 4: ArpProcessor drives Keys[1], UpDown pattern ---
    engine.tracks[4].kind = InstrumentKind::Keys;
    engine.tracks[4].instance_idx = 1;
    engine.tracks[4].level = 0.35;
    engine.keys2.set_param(KeysParam::Cutoff, 0.15);
    engine.keys2.set_param(KeysParam::Attack, 0.0);
    engine.keys2.set_param(KeysParam::Decay, 0.15);
    engine.keys2.set_param(KeysParam::Sustain, 0.3);
    engine.keys2.set_param(KeysParam::Release, 0.08);
    {
        let mut arp = ArpProcessor::new();
        arp.set_bpm(85.0);
        arp.set_gate(0.6);
        arp.set_pattern(1.0); // UpDown
        engine.tracks[4].arp = Some(arp);
    }

    // --- FM: glassy bell, longer sustain for dreamy feel ---
    engine.fm.set_param(FmParam::Algorithm, 1.0 / 7.0); // algo 1 — simple carrier+mod
    engine.fm.set_param(FmParam::ModIndex, 0.06);  // → mod_index ≈ 0.125, purer bell
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
    if let Some(ref mut arp) = engine.tracks[4].arp {
        arp.rebuild_notes_from_harmony(&HarmonyContext::new(52, Scale::Minor));
        arp.start(1.0);
    }

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
    engine.tracks[4].level = 0.5;

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
    engine.tracks[4].level = 0.2;

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
            engine.tracks[4].level = 0.2 * (1.0 - progress * 2.0);
        } else {
            engine.tracks[4].level = 0.0;
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
