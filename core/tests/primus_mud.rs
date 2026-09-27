//! Primus-inspired slap-funk test — percussive bass, angular FM guitar, busy drums.
//!
//! Run with: cargo test -p tatum-core --test primus_mud -- --nocapture
//! Output:   core/test_output/primus_mud.wav

mod test_helpers;

use tatum_core::song_engine::SongEngine;
use test_helpers::{write_wav_stereo, output_path};

#[test]
#[ignore = "a whole song, rendered to listen to: cargo test --release -- --ignored"]
fn test_primus_mud() {
    const SOURCE: &str = include_str!("../../examples/primus_mud.synth");
    let mut engine = SongEngine::from_source(SOURCE).expect("should load primus_mud");

    let total_bars = engine.arrangement_bars();
    assert!(total_bars > 0, "arrangement should have bars");

    let (out_l, out_r) = engine.render(total_bars);

    let peak = out_l.iter().chain(out_r.iter()).fold(0.0f32, |a, &b| a.max(b.abs()));
    assert!(peak > 0.01, "Should produce audible output, peak={:.4}", peak);
    assert!(peak <= 1.0, "Should not clip, peak={:.4}", peak);

    // Bass should be prominent — check low-end energy exists
    let total_energy: f32 = out_l.iter().chain(out_r.iter()).map(|s| s * s).sum::<f32>();
    assert!(total_energy > 100.0, "Should have significant energy, got {:.1}", total_energy);

    write_wav_stereo(&output_path("primus_mud.wav"), &out_l, &out_r, 44100);

    let duration = out_l.len() as f32 / 44100.0;
    println!("Wrote test_output/primus_mud.wav ({:.1}s, {} bars at 108 BPM)", duration, total_bars);
    println!("  Peak: {:.3}", peak);
}
