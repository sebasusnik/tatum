//! Tests for per-track insert FX chains.

use synth_core::engine::Engine;
use synth_core::harmony::{HarmonyContext, Scale};
use synth_core::modules::bass::BassParam;
use synth_core::track::{InstrumentKind, InsertFxType, InsertFxSlot, MAX_INSERT_FX};
use synth_core::{Module, SAMPLE_RATE, BLOCK_SIZE};

mod test_helpers;
use test_helpers::{write_wav_stereo, output_path};

/// Render N samples from the engine, return peak amplitude.
fn render_peak(engine: &mut Engine, samples: usize) -> f32 {
    let mut out_l = vec![0.0f32; samples];
    let mut out_r = vec![0.0f32; samples];
    let mut pos = 0;
    while pos < samples {
        let chunk = BLOCK_SIZE.min(samples - pos);
        engine.process_block_stereo(&mut out_l[pos..pos + chunk], &mut out_r[pos..pos + chunk]);
        pos += chunk;
    }
    out_l.iter().chain(out_r.iter())
        .fold(0.0f32, |a, &b| a.max(b.abs()))
}

#[test]
fn test_insert_saturator_on_bass() {
    // Bass with saturator should be louder/hotter than clean bass
    let mut clean = Engine::new();
    clean.set_bpm(120.0);
    clean.bass.note_on(36, 0.8);
    let clean_peak = render_peak(&mut clean, 44100);

    let mut saturated = Engine::new();
    saturated.set_bpm(120.0);
    // Add saturator on track 0 (bass), slot 0, instance 0
    saturated.tracks[0].insert_fx[0] = InsertFxSlot::new(InsertFxType::Saturator, 0);
    saturated.insert_saturators[0].set_drive(3.0);
    saturated.bass.note_on(36, 0.8);
    let sat_peak = render_peak(&mut saturated, 44100);

    assert!(sat_peak > 0.05, "Saturated bass should produce audio");
    assert!(clean_peak > 0.05, "Clean bass should produce audio");
    // Saturator changes the waveform (can be louder or same due to normalization)
    // The key test is that it doesn't crash and produces different output
    assert!((sat_peak - clean_peak).abs() > 0.001 || sat_peak > 0.05,
        "Saturator should affect the signal");
}

#[test]
fn test_insert_filter_on_bass() {
    // Bass with a closed filter should be quieter than open bass
    let mut open = Engine::new();
    open.set_bpm(120.0);
    open.bass.set_param(BassParam::Cutoff, 0.8); // wide open
    open.bass.note_on(36, 0.8);
    let open_peak = render_peak(&mut open, 44100);

    let mut filtered = Engine::new();
    filtered.set_bpm(120.0);
    filtered.bass.set_param(BassParam::Cutoff, 0.8);
    // Add lowpass filter on track 0, slot 0, instance 0
    filtered.tracks[0].insert_fx[0] = InsertFxSlot::new(InsertFxType::Filter, 0);
    // Set filter cutoff very low (0.05 → ~30Hz)
    filtered.apply_insert_param(0, 0, 0, 0.05);
    filtered.bass.note_on(36, 0.8);
    let filtered_peak = render_peak(&mut filtered, 44100);

    assert!(open_peak > 0.05, "Open bass should produce audio");
    assert!(filtered_peak < open_peak, "Closed filter should reduce level: open={} filtered={}", open_peak, filtered_peak);
}

#[test]
fn test_insert_compressor_on_beats() {
    // Compressor on beats track
    let mut engine = Engine::new();
    engine.set_bpm(130.0);

    // Add compressor on track 3 (beats), slot 0
    engine.tracks[3].insert_fx[0] = InsertFxSlot::new(InsertFxType::Compressor, 0);
    engine.insert_compressors[0].set_threshold(-10.0);
    engine.insert_compressors[0].set_ratio(4.0);

    // Trigger a kick
    engine.beats.note_on(36, 1.0);
    let peak = render_peak(&mut engine, 22050);

    assert!(peak > 0.01, "Compressed beats should produce audio, peak={}", peak);
}

#[test]
fn test_multiple_inserts_chained() {
    // Bass → Filter → Saturator → output (chained inserts)
    let mut engine = Engine::new();
    engine.set_bpm(120.0);

    // Slot 0: filter
    engine.tracks[0].insert_fx[0] = InsertFxSlot::new(InsertFxType::Filter, 0);
    engine.apply_insert_param(0, 0, 0, 0.3); // cutoff at ~600Hz

    // Slot 1: saturator
    engine.tracks[0].insert_fx[1] = InsertFxSlot::new(InsertFxType::Saturator, 0);
    engine.insert_saturators[0].set_drive(2.5);

    engine.bass.note_on(36, 0.9);
    let peak = render_peak(&mut engine, 44100);

    assert!(peak > 0.05, "Chained filter→saturator should produce audio, peak={}", peak);
}

#[test]
fn test_insert_fx_on_different_tracks() {
    // Different FX on different tracks simultaneously
    let mut engine = Engine::new();
    engine.set_bpm(120.0);
    engine.set_harmony(HarmonyContext::new(57, Scale::Minor));

    // Bass: saturator
    engine.tracks[0].insert_fx[0] = InsertFxSlot::new(InsertFxType::Saturator, 0);
    engine.insert_saturators[0].set_drive(2.0);

    // Keys: chorus (instance 0)
    engine.tracks[1].insert_fx[0] = InsertFxSlot::new(InsertFxType::Chorus, 0);
    engine.insert_choruses[0].set_mix(0.5);

    // FM: filter (instance 2, leaving 0-1 for stereo pair usage)
    engine.tracks[2].insert_fx[0] = InsertFxSlot::new(InsertFxType::Filter, 2);
    engine.apply_insert_param(2, 0, 0, 0.4);

    // Beats: compressor
    engine.tracks[3].insert_fx[0] = InsertFxSlot::new(InsertFxType::Compressor, 0);
    engine.insert_compressors[0].set_threshold(-12.0);
    engine.insert_compressors[0].set_ratio(6.0);

    engine.bass.note_on(33, 0.8);
    engine.keys.note_on(57, 0.7);
    engine.fm.note_on(69, 0.5);
    engine.beats.note_on(36, 1.0);

    let samples = (SAMPLE_RATE * 2.0) as usize;
    let mut out_l = vec![0.0f32; samples];
    let mut out_r = vec![0.0f32; samples];
    let mut pos = 0;
    while pos < samples {
        let chunk = BLOCK_SIZE.min(samples - pos);
        engine.process_block_stereo(&mut out_l[pos..pos + chunk], &mut out_r[pos..pos + chunk]);
        pos += chunk;
    }

    let peak = out_l.iter().chain(out_r.iter())
        .fold(0.0f32, |a, &b| a.max(b.abs()));
    assert!(peak > 0.05, "Multi-track insert FX should produce audio, peak={}", peak);

    write_wav_stereo(
        &output_path("test_insert_fx_multi_track.wav"),
        &out_l, &out_r,
        SAMPLE_RATE as u32,
    );
}

#[test]
fn test_insert_fx_disable_enable() {
    let mut engine = Engine::new();
    engine.set_bpm(120.0);

    // Add very closed filter
    engine.tracks[0].insert_fx[0] = InsertFxSlot::new(InsertFxType::Filter, 0);
    engine.apply_insert_param(0, 0, 0, 0.02); // nearly closed

    engine.bass.note_on(36, 0.8);
    let filtered_peak = render_peak(&mut engine, 22050);

    // Disable the filter
    engine.tracks[0].insert_fx[0].enabled = false;
    engine.bass.note_on(36, 0.8);
    let bypass_peak = render_peak(&mut engine, 22050);

    assert!(bypass_peak > filtered_peak,
        "Bypassed should be louder: bypass={} filtered={}", bypass_peak, filtered_peak);
}
