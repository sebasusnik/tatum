//! Techno Robot — DSL replica of `song_techno_robot.rs`.
//!
//! Run with: cargo test -p synth-core --test techno_robot -- --nocapture
//! Output:   core/test_output/techno_robot.wav

mod test_helpers;

use synth_core::song_engine::SongEngine;
use test_helpers::{write_wav_stereo, output_path};

/// Section map of the arrangement, in bars: (name, start_bar, end_bar).
const SECTIONS: &[(&str, usize, usize)] = &[
    ("intro", 0, 4),
    ("build", 4, 12),
    ("climax", 12, 16),
    ("breakdown", 16, 20),
    ("drop", 20, 24),
    ("outro", 24, 29),
];

fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f32 = samples.iter().map(|s| s * s).sum();
    (sum / samples.len() as f32).sqrt()
}

fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |a, &b| a.max(b.abs()))
}

#[test]
fn test_techno_robot() {
    const SOURCE: &str = include_str!("../../examples/techno_robot.synth");
    let mut engine = SongEngine::from_source(SOURCE).expect("should load techno_robot");

    let total_bars = engine.arrangement_bars();
    assert_eq!(total_bars, 29, "arrangement should be 29 bars");

    let (out_l, out_r) = engine.render(total_bars);

    let peak_all = peak(&out_l).max(peak(&out_r));
    assert!(peak_all > 0.01, "Should produce audible output, peak={:.4}", peak_all);
    assert!(peak_all <= 1.0, "Should not clip, peak={:.4}", peak_all);

    // 140 BPM, 4/4 → one bar is four beats.
    let samples_per_bar = (44100.0 * 60.0 / 140.0 * 4.0) as usize;

    let section_rms = |start_bar: usize, end_bar: usize| -> f32 {
        let a = (start_bar * samples_per_bar).min(out_l.len());
        let b = (end_bar * samples_per_bar).min(out_l.len());
        rms(&out_l[a..b])
    };
    let section_peak = |start_bar: usize, end_bar: usize| -> f32 {
        let a = (start_bar * samples_per_bar).min(out_l.len());
        let b = (end_bar * samples_per_bar).min(out_l.len());
        peak(&out_l[a..b])
    };

    for (name, a, b) in SECTIONS {
        println!(
            "  {:<10} bars {:>2}-{:<2}  rms {:.4}  peak {:.4}",
            name, a + 1, b, section_rms(*a, *b), section_peak(*a, *b)
        );
    }

    let intro_rms = section_rms(0, 4);
    let climax_rms = section_rms(12, 16);
    let intro_peak = section_peak(0, 4);
    let drop_peak = section_peak(20, 24);

    // The climax (filter open, rides + claps, arp on top) must be louder than
    // the stripped-back intro, and the drop must hit harder than the intro.
    assert!(
        climax_rms > intro_rms,
        "climax should be louder than intro: climax={:.4} intro={:.4}",
        climax_rms, intro_rms
    );
    assert!(
        drop_peak > intro_peak,
        "drop should hit harder than intro: drop={:.4} intro={:.4}",
        drop_peak, intro_peak
    );

    write_wav_stereo(&output_path("techno_robot.wav"), &out_l, &out_r, 44100);

    let duration = out_l.len() as f32 / 44100.0;
    println!(
        "Wrote test_output/techno_robot.wav ({:.1}s, {} bars at 140 BPM)",
        duration, total_bars
    );
    println!("  Peak: {:.3}", peak_all);
}
