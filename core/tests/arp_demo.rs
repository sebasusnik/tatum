//! Minimal arp-on-keys demo — ArpProcessor drives Keys[0] alone.
//! No drums, no bass, just the arp through keys to hear it clearly.

use synth_core::engine::Engine;
use synth_core::harmony::{HarmonyContext, Scale};
use synth_core::modules::bass::BassParam;
use synth_core::modules::keys::KeysParam;
use synth_core::effects::delay::DelaySync;
use synth_core::sequencer::Step;
use synth_core::track::InstrumentKind;
use synth_core::primitives::arp_processor::ArpProcessor;
use synth_core::{Module, SAMPLE_RATE, BLOCK_SIZE};

mod test_helpers;
use test_helpers::{write_wav_stereo, output_path};

#[test]
fn test_arp_keys_solo() {
    let mut engine = Engine::new();
    engine.set_bpm(125.0);
    engine.set_harmony(HarmonyContext::new(60, Scale::Minor)); // C minor

    // Mute everything except track 1 (keys)
    engine.set_module_mute(0, true); // bass
    engine.set_module_mute(2, true); // fm
    engine.set_module_mute(3, true); // beats
    engine.set_module_mute(4, true); // legacy arp

    // Configure keys for a bright pad sound
    engine.keys.set_param(KeysParam::Cutoff, 0.6);
    engine.keys.set_param(KeysParam::Resonance, 0.3);
    engine.keys.set_param(KeysParam::ChorusMix, 0.4);
    engine.keys.set_param(KeysParam::Attack, 0.01);
    engine.keys.set_param(KeysParam::Decay, 0.2);
    engine.keys.set_param(KeysParam::Sustain, 0.4);
    engine.keys.set_param(KeysParam::Release, 0.15);
    engine.tracks[1].level = 0.6;

    // Attach ArpProcessor to track 1 (keys)
    let mut arp = ArpProcessor::new();
    arp.set_bpm(125.0);
    arp.set_gate(0.5);
    arp.set_pattern(1.0); // UpDown
    arp.set_octave_range(0.66); // 3 octaves
    arp.rebuild_notes_from_harmony(&HarmonyContext::new(60, Scale::Minor));
    arp.start(0.8);
    engine.tracks[1].arp = Some(arp);

    // Some delay/reverb for space
    engine.set_module_delay_send(1, 0.25);
    engine.set_module_reverb_send(1, 0.30);
    engine.delay.set_feedback(0.45);
    engine.delay.set_mix(0.3);
    engine.reverb.set_room_size(0.7);
    engine.reverb.set_mix(0.25);

    // Render 8 bars
    let samples_per_bar = (SAMPLE_RATE * 60.0 / 125.0 * 4.0) as usize;
    let total = 8 * samples_per_bar;
    let mut out_l = vec![0.0f32; total];
    let mut out_r = vec![0.0f32; total];

    let mut pos = 0;
    while pos < total {
        let chunk = BLOCK_SIZE.min(total - pos);
        engine.process_block_stereo(&mut out_l[pos..pos + chunk], &mut out_r[pos..pos + chunk]);
        pos += chunk;
    }

    let peak = out_l.iter().chain(out_r.iter())
        .fold(0.0f32, |a, &b| a.max(b.abs()));
    assert!(peak > 0.05, "Arp-keys solo should produce audio, peak={}", peak);

    write_wav_stereo(
        &output_path("test_arp_keys_solo.wav"),
        &out_l, &out_r,
        SAMPLE_RATE as u32,
    );
}

#[test]
fn test_arp_on_bass_acid() {
    // Acid arp: ArpProcessor drives Bass[1] on track 4 — the arp's notes go
    // through the bass module's ladder filter for that classic acid sound.
    // 138 BPM, A minor.

    let mut engine = Engine::new();
    engine.set_bpm(138.0);
    engine.set_harmony(HarmonyContext::new(57, Scale::Minor)); // A minor

    // Assign Bass[1] to track 4 (was Arp/legacy)
    engine.tracks[4].kind = InstrumentKind::Bass;
    engine.tracks[4].instance_idx = 1;
    engine.tracks[4].level = 0.5;
    engine.tracks[4].pan = 0.0;

    // Configure bass2 for acid sound
    engine.bass2.set_param(BassParam::Cutoff, 0.08);       // closed filter
    engine.bass2.set_param(BassParam::CutoffEnv, 0.70);    // big envelope sweep
    engine.bass2.set_param(BassParam::Resonance, 0.75);    // resonant peak
    engine.bass2.set_param(BassParam::Attack, 0.0);
    engine.bass2.set_param(BassParam::Decay, 0.15);
    engine.bass2.set_param(BassParam::Sustain, 0.1);
    engine.bass2.set_param(BassParam::Release, 0.05);

    // Attach ArpProcessor to track 4
    let mut arp = ArpProcessor::new();
    arp.set_bpm(138.0);
    arp.set_gate(0.6);
    arp.set_pattern(0.0); // Up
    arp.set_octave_range(0.33); // 2 octaves
    // Build notes from harmony
    arp.rebuild_notes_from_harmony(&HarmonyContext::new(57, Scale::Minor));
    arp.start(0.9);
    engine.tracks[4].arp = Some(arp);

    // Also set up basic drums on track 3 for rhythm context
    for i in 0..16 {
        engine.sequencer.drum_track.lanes[0].on[i] = i % 4 == 0; // kick on 1,5,9,13
        engine.sequencer.drum_track.lanes[0].velocity[i] = 1.0;
        engine.sequencer.drum_track.lanes[2].on[i] = true; // 16th hats
        engine.sequencer.drum_track.lanes[2].velocity[i] = 0.35;
    }

    // Set bass on track 0 for a sub-bass root note
    engine.bass.set_param(BassParam::Cutoff, 0.04);
    engine.bass.set_param(BassParam::Resonance, 0.2);
    engine.sequencer.tracks[0].steps[0] = Step::new(45, 0.8, 0.9); // A1

    // Effects
    engine.set_module_delay_send(4, 0.15);
    engine.set_module_reverb_send(4, 0.10);
    engine.delay.set_feedback(0.4);
    engine.delay.set_sync(DelaySync::Sixteenth, 138.0, SAMPLE_RATE);
    engine.reverb.set_room_size(0.5);
    engine.reverb.set_mix(0.2);
    engine.set_sidechain_amount(0.5);

    engine.sequencer.start();

    // Render 8 bars
    let bars = 8;
    let samples_per_bar = (SAMPLE_RATE * 60.0 / 138.0 * 4.0) as usize;
    let total_samples = bars * samples_per_bar;
    let mut samples_l = vec![0.0f32; total_samples];
    let mut samples_r = vec![0.0f32; total_samples];

    // Automate filter cutoff sweep on bass2 (track 4) over the 8 bars
    for bar in 0..bars {
        let bar_start = bar * samples_per_bar;
        let bar_end = (bar + 1) * samples_per_bar;
        let cutoff = 0.08 + (bar as f32 / bars as f32) * 0.35; // sweep open
        engine.bass2.set_param(BassParam::Cutoff, cutoff);

        let mut pos = bar_start;
        while pos < bar_end {
            let chunk = BLOCK_SIZE.min(bar_end - pos);
            engine.process_block_stereo(
                &mut samples_l[pos..pos + chunk],
                &mut samples_r[pos..pos + chunk],
            );
            pos += chunk;
        }
    }

    let max_val = samples_l.iter().chain(samples_r.iter())
        .fold(0.0f32, |a, &b| if b.abs() > a { b.abs() } else { a });
    assert!(max_val > 0.05, "Arp-on-bass acid test should produce audible output, max={}", max_val);

    write_wav_stereo(
        &output_path("test_arp_on_bass_acid.wav"),
        &samples_l,
        &samples_r,
        SAMPLE_RATE as u32,
    );
}
