//! Shared helpers for integration tests (WAV writing, output paths).
//! Mirrors the functionality in `src/test_utils.rs` but available to integration tests.

use std::fs::{self, File};
use std::io::Write;

/// Single-voice probes from the DSP tests. They live apart from the songs
/// because a raw bass with no mix on it is not something to listen to, and a
/// folder that mixes the two is a folder nobody can judge.
const OUTPUT_DIR: &str = "test_output/probes";

fn ensure_output_dir() {
    fs::create_dir_all(OUTPUT_DIR).expect("Failed to create test_output directory");
}

/// Build a path under the test_output/ directory.
pub fn output_path(filename: &str) -> String {
    format!("{}/{}", OUTPUT_DIR, filename)
}

/// Write a mono WAV file from f32 samples (range -1.0 to 1.0).
#[allow(dead_code)]
pub fn write_wav(filename: &str, samples: &[f32], sample_rate: u32) {
    ensure_output_dir();
    let num_frames = samples.len() as u32;
    let bits_per_sample: u16 = 16;
    let num_channels: u16 = 1;
    let byte_rate = sample_rate * (bits_per_sample as u32 / 8) * num_channels as u32;
    let block_align = num_channels * (bits_per_sample / 8);
    let data_size = num_frames * (bits_per_sample as u32 / 8) * num_channels as u32;
    let file_size = 36 + data_size;

    let mut file = File::create(filename).expect("Failed to create WAV file");

    file.write_all(b"RIFF").unwrap();
    file.write_all(&file_size.to_le_bytes()).unwrap();
    file.write_all(b"WAVE").unwrap();
    file.write_all(b"fmt ").unwrap();
    file.write_all(&16u32.to_le_bytes()).unwrap();
    file.write_all(&1u16.to_le_bytes()).unwrap();
    file.write_all(&num_channels.to_le_bytes()).unwrap();
    file.write_all(&sample_rate.to_le_bytes()).unwrap();
    file.write_all(&byte_rate.to_le_bytes()).unwrap();
    file.write_all(&block_align.to_le_bytes()).unwrap();
    file.write_all(&bits_per_sample.to_le_bytes()).unwrap();
    file.write_all(b"data").unwrap();
    file.write_all(&data_size.to_le_bytes()).unwrap();

    let clamp = |s: f32| -> i16 {
        let c = if s > 1.0 { 1.0 } else if s < -1.0 { -1.0 } else { s };
        (c * 32767.0) as i16
    };
    for &s in samples {
        file.write_all(&clamp(s).to_le_bytes()).unwrap();
    }
}

/// Write a stereo WAV file from f32 samples (range -1.0 to 1.0).
#[allow(dead_code)]
pub fn write_wav_stereo(filename: &str, samples_l: &[f32], samples_r: &[f32], sample_rate: u32) {
    ensure_output_dir();
    let num_frames = samples_l.len().min(samples_r.len()) as u32;
    let bits_per_sample: u16 = 16;
    let num_channels: u16 = 2;
    let byte_rate = sample_rate * (bits_per_sample as u32 / 8) * num_channels as u32;
    let block_align = num_channels * (bits_per_sample / 8);
    let data_size = num_frames * (bits_per_sample as u32 / 8) * num_channels as u32;
    let file_size = 36 + data_size;

    let mut file = File::create(filename).expect("Failed to create WAV file");

    file.write_all(b"RIFF").unwrap();
    file.write_all(&file_size.to_le_bytes()).unwrap();
    file.write_all(b"WAVE").unwrap();
    file.write_all(b"fmt ").unwrap();
    file.write_all(&16u32.to_le_bytes()).unwrap();
    file.write_all(&1u16.to_le_bytes()).unwrap();
    file.write_all(&num_channels.to_le_bytes()).unwrap();
    file.write_all(&sample_rate.to_le_bytes()).unwrap();
    file.write_all(&byte_rate.to_le_bytes()).unwrap();
    file.write_all(&block_align.to_le_bytes()).unwrap();
    file.write_all(&bits_per_sample.to_le_bytes()).unwrap();
    file.write_all(b"data").unwrap();
    file.write_all(&data_size.to_le_bytes()).unwrap();

    for i in 0..num_frames as usize {
        let clamp = |s: f32| -> i16 {
            let c = if s > 1.0 { 1.0 } else if s < -1.0 { -1.0 } else { s };
            (c * 32767.0) as i16
        };
        file.write_all(&clamp(samples_l[i]).to_le_bytes()).unwrap();
        file.write_all(&clamp(samples_r[i]).to_le_bytes()).unwrap();
    }
}
