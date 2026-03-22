//! FM Madness — extreme FM synthesis showcase test.
//! Renders metallic bells, screaming leads, glitch percussion, evolving pads,
//! acid stabs, dissonant drones, and FM sub-kicks.
//!
//! Run with: cargo test -p synth-core --test fm_madness -- --nocapture
//! Output:   core/test_output/fm_madness.wav

mod test_helpers;

use synth_core::song_engine::SongEngine;
use test_helpers::{write_wav_stereo, output_path};

#[test]
fn test_fm_madness() {
    const SOURCE: &str = include_str!("../../examples/fm_madness.synth");
    let mut engine = SongEngine::from_source(SOURCE).expect("should load fm_madness");

    let total_bars = engine.arrangement_bars();
    assert!(total_bars > 0, "arrangement should have bars");

    let (out_l, out_r) = engine.render(total_bars);

    // Verify output has audio content
    let peak = out_l.iter().chain(out_r.iter())
        .fold(0.0f32, |a, &b| a.max(b.abs()));
    assert!(peak > 0.01, "FM madness should produce audible output, peak={:.4}", peak);
    assert!(peak <= 1.0, "FM madness should not clip, peak={:.4}", peak);

    // Verify stereo content (chorus + delay + reverb should create L/R differences)
    let stereo_diff: f32 = out_l.iter().zip(out_r.iter())
        .map(|(l, r)| (l - r).abs())
        .sum::<f32>() / out_l.len() as f32;
    assert!(stereo_diff > 0.001, "Should have stereo width, avg diff={:.6}", stereo_diff);

    // Verify different sections have different energy (intro quieter than drops)
    let samples_per_bar = out_l.len() / total_bars as usize;
    let intro_rms = rms(&out_l[..samples_per_bar * 4], &out_r[..samples_per_bar * 4]);
    let drop_start = samples_per_bar * 8;
    let drop_end = samples_per_bar * 12;
    let drop_rms = rms(&out_l[drop_start..drop_end], &out_r[drop_start..drop_end]);
    assert!(
        drop_rms > intro_rms * 1.2,
        "Drop should be louder than intro: drop_rms={:.4}, intro_rms={:.4}",
        drop_rms, intro_rms
    );

    write_wav_stereo(
        &output_path("fm_madness.wav"),
        &out_l,
        &out_r,
        44100,
    );

    let duration = out_l.len() as f32 / 44100.0;
    println!("Wrote test_output/fm_madness.wav ({:.1}s, {} bars at 130 BPM)", duration, total_bars);
    println!("  Peak: {:.3}, Stereo diff: {:.5}, Intro RMS: {:.4}, Drop RMS: {:.4}",
        peak, stereo_diff, intro_rms, drop_rms);
}

fn rms(left: &[f32], right: &[f32]) -> f32 {
    let n = left.len();
    let sum: f32 = left.iter().zip(right.iter())
        .map(|(l, r)| l * l + r * r)
        .sum();
    (sum / (2 * n) as f32).sqrt()
}
