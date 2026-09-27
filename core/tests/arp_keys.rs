//! DSL-driven counterpart to `arp_demo.rs::test_arp_keys_solo` — loads
//! `examples/arp_keys.synth` (a single Keys voice arpeggiating a held Cm7
//! chord, no drums/bass) through the DSL `SongEngine` instead of the direct
//! engine API, and checks it produces sane, audible output with the arp
//! actually running.
//!
//! Run with: cargo test -p tatum-core --test arp_keys -- --nocapture

mod test_helpers;

use tatum_core::song_engine::SongEngine;
use test_helpers::{write_wav_stereo, output_path};

#[test]
#[ignore = "a whole song, rendered to listen to: cargo test --release -- --ignored"]
fn test_arp_keys_dsl() {
    const SOURCE: &str = include_str!("../../examples/arp_keys.synth");
    let mut engine = SongEngine::from_source(SOURCE).expect("should load arp_keys");

    let total_bars = engine.arrangement_bars();
    assert!(total_bars > 0, "arrangement should have bars");

    let (out_l, out_r) = engine.render(total_bars);

    let peak = out_l.iter().chain(out_r.iter()).fold(0.0f32, |a, &b| a.max(b.abs()));
    assert!(peak > 0.01, "Arp-keys should produce audible output, peak={:.4}", peak);
    assert!(peak <= 1.0, "Arp-keys should not clip, peak={:.4}", peak);

    write_wav_stereo(&output_path("arp_keys.wav"), &out_l, &out_r, 44100);

    // The track arpeggiator should be running shortly after playback starts —
    // this is what drives the track's activity LED in the UI.
    engine.start();
    engine.render_steps(4);
    assert!(engine.track_arp_active(0), "Track 0's arpeggiator should be active after playback starts");

    let duration = out_l.len() as f32 / 44100.0;
    println!("Wrote test_output/arp_keys.wav ({:.1}s, {} bars at {} BPM)", duration, total_bars, engine.tempo());
    println!("  Peak: {:.3}", peak);
}
