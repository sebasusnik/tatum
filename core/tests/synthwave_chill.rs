//! Synthwave Chill — DSL replica of song_synthwave_chill.rs.
//!
//! Run with: cargo test -p tatum-core --test synthwave_chill -- --nocapture
//! Output:   core/test_output/synthwave_chill.wav

mod test_helpers;

use tatum_core::song_engine::SongEngine;
use test_helpers::{write_wav_stereo, output_path};

#[test]
#[ignore = "a whole song, rendered to listen to: cargo test --release -- --ignored"]
fn test_synthwave_chill_dsl() {
    const SOURCE: &str = include_str!("../../examples/synthwave_chill.synth");
    let mut engine = SongEngine::from_source(SOURCE).expect("should load synthwave_chill");

    let total_bars = engine.arrangement_bars();
    assert_eq!(total_bars, 28, "arrangement should be 28 bars");

    let (out_l, out_r) = engine.render(total_bars);

    let peak = out_l.iter().chain(out_r.iter())
        .fold(0.0f32, |a, &b| a.max(b.abs()));
    assert!(peak > 0.01, "Should produce audible output, peak={:.4}", peak);
    assert!(peak <= 1.0, "Should not clip, peak={:.4}", peak);

    // Stereo width: panned drums, the panned FM bells and the reverb return
    // should decorrelate the two channels.
    let width: f32 = out_l.iter().zip(out_r.iter())
        .map(|(l, r)| (l - r).abs())
        .sum::<f32>() / out_l.len() as f32;
    assert!(width > 0.001, "Should be stereo, avg |L-R|={:.5}", width);

    write_wav_stereo(
        &output_path("synthwave_chill.wav"),
        &out_l,
        &out_r,
        44100,
    );

    let duration = out_l.len() as f32 / 44100.0;
    println!("Wrote test_output/synthwave_chill.wav ({:.1}s, {} bars at 85 BPM)", duration, total_bars);
    println!("  Peak: {:.3}  avg |L-R|: {:.5}", peak, width);
}
