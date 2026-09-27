//! Snare drum synthesis test — renders isolated snare hits to stereo WAV for audition.
//!
//! Run with: cargo test -p tatum-core --test snare_test -- --nocapture
//! Output:   core/test_output/test_snare_hits.wav

mod test_helpers;

use tatum_core::{Module, SAMPLE_RATE, BLOCK_SIZE};
use tatum_core::modules::beats::{BeatsModule, BeatsParam};
use test_helpers::{write_wav_stereo, output_path};

/// Render N stereo samples from a BeatsModule into L/R buffers.
fn render_block_stereo(beats: &mut BeatsModule, buf_l: &mut [f32], buf_r: &mut [f32]) {
    let len = buf_l.len();
    let mut pos = 0;
    while pos < len {
        let bl = BLOCK_SIZE.min(len - pos);
        beats.process_block_stereo(&mut buf_l[pos..pos + bl], &mut buf_r[pos..pos + bl]);
        pos += bl;
    }
}

#[test]
fn test_snare_hits() {
    let sr = SAMPLE_RATE as usize;

    // 4 seconds total: 8 snare hits spaced 0.5s apart
    let total_samples = sr * 4;
    let mut output_l = vec![0.0f32; total_samples];
    let mut output_r = vec![0.0f32; total_samples];

    let hit_spacing = sr / 2; // 0.5s between hits

    // ── Hit sequence ──
    // Hit 1: open hit (full velocity, default pitch)
    // Hit 2: closed hit (ghost-note velocity < 0.4, tight decay)
    // Hit 3: ghost note (very soft, vel 0.15)
    // Hits 4-8: velocity sweep 0.3, 0.5, 0.7, 0.85, 1.0

    struct Hit {
        velocity: f32,
        pitch: f32,
    }
    let hits = [
        Hit { velocity: 1.0, pitch: 0.5 },  // open hit
        Hit { velocity: 0.35, pitch: 0.5 }, // closed hit (vel < 0.4 = tight)
        Hit { velocity: 0.15, pitch: 0.5 }, // ghost note
        Hit { velocity: 0.3, pitch: 0.5 },  // velocity sweep
        Hit { velocity: 0.5, pitch: 0.5 },
        Hit { velocity: 0.7, pitch: 0.5 },
        Hit { velocity: 0.85, pitch: 0.5 },
        Hit { velocity: 1.0, pitch: 0.5 }, // full accent
    ];

    let mut beats = BeatsModule::new();
    beats.set_param(BeatsParam::SnareDecay, 0.30);
    beats.set_param(BeatsParam::SnareLevel, 1.0);

    for (i, hit) in hits.iter().enumerate() {
        let hit_start = i * hit_spacing;
        let hit_end = ((i + 1) * hit_spacing).min(total_samples);

        beats.set_param(BeatsParam::SnarePitch, hit.pitch);
        beats.note_on(38, hit.velocity);

        render_block_stereo(&mut beats, &mut output_l[hit_start..hit_end], &mut output_r[hit_start..hit_end]);
    }

    // Verify basic signal properties (check both channels)
    let peak_l = output_l.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
    let peak_r = output_r.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
    let peak = peak_l.max(peak_r);
    assert!(peak > 0.1, "Snare should produce audible output, got peak={:.4}", peak);
    assert!(peak <= 1.0, "Snare should not clip, got peak={:.4}", peak);

    // Verify stereo width: L and R should differ (tail has random pan jitter)
    let diff: f32 =
        output_l.iter().zip(output_r.iter()).map(|(l, r)| (l - r).abs()).sum::<f32>() / total_samples as f32;
    assert!(diff > 0.0001, "Stereo channels should differ (tail width), avg diff={:.6}", diff);

    // Verify the open hit (1) is louder than the ghost note (3)
    let rms = |slice_l: &[f32], slice_r: &[f32]| -> f32 {
        let n = slice_l.len().min(2000);
        let sum: f32 = slice_l[..n].iter().zip(slice_r[..n].iter()).map(|(l, r)| l * l + r * r).sum();
        (sum / (2 * n) as f32).sqrt()
    };
    let open_rms = rms(&output_l[0..hit_spacing], &output_r[0..hit_spacing]);
    let ghost_rms = rms(&output_l[2 * hit_spacing..3 * hit_spacing], &output_r[2 * hit_spacing..3 * hit_spacing]);
    assert!(
        open_rms > ghost_rms * 1.3,
        "Open hit RMS ({:.4}) should be louder than ghost RMS ({:.4})",
        open_rms,
        ghost_rms
    );

    // Verify each hit decays to near silence before the next hit
    for i in 0..hits.len() {
        let tail_start = i * hit_spacing + (hit_spacing * 3 / 4);
        let tail_end = ((i + 1) * hit_spacing).min(total_samples);
        if tail_end > tail_start {
            let tail_rms: f32 = {
                let sum: f32 = output_l[tail_start..tail_end]
                    .iter()
                    .zip(output_r[tail_start..tail_end].iter())
                    .map(|(l, r)| l * l + r * r)
                    .sum();
                (sum / (2 * (tail_end - tail_start)) as f32).sqrt()
            };
            assert!(tail_rms < 0.05, "Hit {} tail should decay to near silence, got RMS={:.4}", i + 1, tail_rms);
        }
    }

    // Write stereo WAV for manual audition
    write_wav_stereo(&output_path("test_snare_hits.wav"), &output_l, &output_r, SAMPLE_RATE as u32);
    println!("Wrote test_output/test_snare_hits.wav (stereo) — 8 snare hits:");
    println!("  1: open hit (vel 1.0)");
    println!("  2: closed hit (vel 0.35, tight decay)");
    println!("  3: ghost note (vel 0.15)");
    println!("  4-8: velocity sweep 0.3 → 1.0");
}
