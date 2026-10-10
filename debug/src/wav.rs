//! A WAV file written as the render goes, so a part never has to be held in
//! memory whole. Same format as `tatum_core::wav`: 16-bit stereo PCM.

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use tatum_core::SAMPLE_RATE;

pub struct WavWriter {
    path: PathBuf,
    out: BufWriter<File>,
    frames: u32,
}

impl WavWriter {
    pub fn create(path: &Path) -> Result<WavWriter, String> {
        let file = File::create(path).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        let mut w = WavWriter { path: path.to_path_buf(), out: BufWriter::new(file), frames: 0 };
        w.header()?;
        Ok(w)
    }

    pub fn push(&mut self, l: &[f32], r: &[f32]) -> Result<(), String> {
        for (a, b) in l.iter().zip(r) {
            for s in [*a, *b] {
                let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
                self.out.write_all(&v.to_le_bytes()).map_err(|e| self.err(e))?;
            }
        }
        self.frames += l.len().min(r.len()) as u32;
        Ok(())
    }

    /// Go back and write the sizes the header could not know at the start.
    pub fn finish(mut self) -> Result<(), String> {
        self.out.seek(SeekFrom::Start(0)).map_err(|e| self.err(e))?;
        self.header()?;
        self.out.flush().map_err(|e| self.err(e))
    }

    fn header(&mut self) -> Result<(), String> {
        let rate = SAMPLE_RATE as u32;
        let data = self.frames * 4;
        let mut h = Vec::with_capacity(44);
        h.extend_from_slice(b"RIFF");
        h.extend_from_slice(&(36 + data).to_le_bytes());
        h.extend_from_slice(b"WAVEfmt ");
        h.extend_from_slice(&16u32.to_le_bytes());
        h.extend_from_slice(&1u16.to_le_bytes()); // PCM
        h.extend_from_slice(&2u16.to_le_bytes()); // channels
        h.extend_from_slice(&rate.to_le_bytes());
        h.extend_from_slice(&(rate * 4).to_le_bytes());
        h.extend_from_slice(&4u16.to_le_bytes()); // block align
        h.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
        h.extend_from_slice(b"data");
        h.extend_from_slice(&data.to_le_bytes());
        self.out.write_all(&h).map_err(|e| self.err(e))
    }

    fn err(&self, e: std::io::Error) -> String {
        format!("cannot write {}: {e}", self.path.display())
    }
}

/// A WAV file read whole, as `tatum analyze` takes a reference in. Mono comes
/// back as two equal channels; more than two keep the first two.
pub struct Audio {
    pub left: Vec<f32>,
    pub right: Vec<f32>,
    pub rate: u32,
}

pub fn read(path: &Path) -> Result<Audio, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    decode(&bytes).map_err(|e| format!("{}: {e}", path.display()))
}

/// PCM at 8, 16, 24 or 32 bits and float at 32 or 64, plain or extensible:
/// what a DAW, a sample pack or `ffmpeg -i song.mp3 song.wav` writes.
pub fn decode(bytes: &[u8]) -> Result<Audio, String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a WAV file; convert it first, e.g. `ffmpeg -i song.mp3 song.wav`".into());
    }
    let u16_at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
    let u32_at = |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
    // (format tag, channels, rate, block align, bits)
    let mut format = None;
    let mut data = None;
    let mut i = 12;
    while i + 8 <= bytes.len() {
        let size = u32_at(i + 4) as usize;
        let body = i + 8;
        match &bytes[i..i + 4] {
            b"fmt " if size >= 16 && body + 16 <= bytes.len() => {
                let mut tag = u16_at(body);
                // WAVE_FORMAT_EXTENSIBLE: the real tag opens the subformat GUID.
                if tag == 0xFFFE && size >= 26 && body + 26 <= bytes.len() {
                    tag = u16_at(body + 24);
                }
                format = Some((tag, u16_at(body + 2), u32_at(body + 4), u16_at(body + 12), u16_at(body + 14)));
            }
            // A stream written without going back for its size says 0, or more
            // than there is: the data runs to the end of the file.
            b"data" => {
                let end = if size == 0 { bytes.len() } else { body.saturating_add(size).min(bytes.len()) };
                data = Some(&bytes[body..end]);
                if size == 0 {
                    break;
                }
            }
            _ => {}
        }
        i = body.saturating_add(size).saturating_add(size & 1);
    }
    let (tag, channels, rate, block_align, bits) = format.ok_or("no `fmt ` chunk")?;
    let data = data.ok_or("no `data` chunk")?;
    if channels == 0 || rate == 0 {
        return Err("the header says no channels or no sample rate".into());
    }
    let width = (bits as usize).div_ceil(8);
    let sample: fn(&[u8]) -> f32 = match (tag, bits) {
        (1, 8) => |b| (b[0] as f32 - 128.0) / 128.0,
        (1, 16) => |b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0,
        (1, 24) => |b| (i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) as f32 / 8_388_608.0,
        (1, 32) => |b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0,
        (3, 32) => |b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        (3, 64) => |b| f64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]) as f32,
        _ => {
            return Err(format!(
                "unsupported WAV encoding (format {tag}, {bits} bits); convert it to 16- or 24-bit PCM"
            ))
        }
    };
    // A frame is `block_align` bytes, and each sample sits in a slot of its
    // share of it: 24 bits can come in a 4-byte slot.
    let frame = block_align as usize;
    let channels = channels as usize;
    if frame < width * channels {
        return Err(format!(
            "the header's block align ({frame} bytes) is less than {channels} channels of {bits} bits"
        ));
    }
    let slot = frame / channels;
    // An integer sample in a wider slot is read as the whole slot. Most files
    // put the bits at the top and pad below, which reads right as is; one that
    // puts them at the bottom leaves the padding bytes as sign, not zero, and
    // is read from the bottom.
    let padded = tag == 1 && bits > 8 && slot > width;
    if padded && slot > 8 {
        return Err(format!("unsupported WAV layout: {bits}-bit samples in {slot}-byte slots"));
    }
    let at_top = padded
        && data
            .chunks_exact(frame)
            .all(|f| (0..channels.min(2)).all(|c| f[c * slot..c * slot + slot - width].iter().all(|&b| b == 0)));
    let read = |b: &[u8]| -> f32 {
        if !padded {
            return sample(&b[..width]);
        }
        let mut v = 0i64;
        for (k, &byte) in b[..slot].iter().enumerate() {
            v |= (byte as i64) << (8 * k);
        }
        let top = 64 - 8 * slot as u32;
        let v = (v << top) >> top;
        let full = if at_top { 8 * slot as i32 - 1 } else { bits as i32 - 1 };
        (v as f64 / 2f64.powi(full)) as f32
    };
    let frames = data.len() / frame;
    let mut left = Vec::with_capacity(frames);
    let mut right = Vec::with_capacity(frames);
    for f in data.chunks_exact(frame) {
        let l = read(&f[..slot]);
        let r = if channels > 1 { read(&f[slot..2 * slot]) } else { l };
        // A float file can carry NaN; it would poison every sum after it.
        left.push(if l.is_finite() { l } else { 0.0 });
        right.push(if r.is_finite() { r } else { 0.0 });
    }
    Ok(Audio { left, right, rate })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(tag: u16, channels: u16, rate: u32, bits: u16, data: &[u8]) -> Vec<u8> {
        header_aligned(tag, channels, rate, bits, channels * bits / 8, data)
    }

    fn header_aligned(tag: u16, channels: u16, rate: u32, bits: u16, align: u16, data: &[u8]) -> Vec<u8> {
        let mut h = Vec::new();
        h.extend_from_slice(b"RIFF");
        h.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        h.extend_from_slice(b"WAVEfmt ");
        h.extend_from_slice(&16u32.to_le_bytes());
        h.extend_from_slice(&tag.to_le_bytes());
        h.extend_from_slice(&channels.to_le_bytes());
        h.extend_from_slice(&rate.to_le_bytes());
        h.extend_from_slice(&(rate * align as u32).to_le_bytes());
        h.extend_from_slice(&align.to_le_bytes());
        h.extend_from_slice(&bits.to_le_bytes());
        // A chunk the reader has to step over.
        h.extend_from_slice(b"LIST");
        h.extend_from_slice(&3u32.to_le_bytes());
        h.extend_from_slice(&[1, 2, 3, 0]);
        h.extend_from_slice(b"data");
        h.extend_from_slice(&(data.len() as u32).to_le_bytes());
        h.extend_from_slice(data);
        h
    }

    #[test]
    fn what_the_engine_writes_reads_back() {
        let l = [0.0f32, 0.5, -0.5];
        let r = [0.25f32, -0.25, 1.0];
        let a = decode(&tatum_core::wav::encode_stereo_16(&l, &r, 44100)).unwrap();
        assert_eq!(a.rate, 44100);
        for (x, y) in a.left.iter().zip(l).chain(a.right.iter().zip(r)) {
            assert!((x - y).abs() < 1e-4, "{x} {y}");
        }
    }

    #[test]
    fn twenty_four_bit_mono_and_float_stereo() {
        // -0.5 and +0.25 at 24 bits.
        let a = decode(&header(1, 1, 48000, 24, &[0x00, 0x00, 0xC0, 0x00, 0x00, 0x20])).unwrap();
        assert_eq!(a.rate, 48000);
        assert_eq!(a.left, vec![-0.5, 0.25]);
        assert_eq!(a.left, a.right);
        let mut data = Vec::new();
        for v in [0.75f32, -0.125] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        let b = decode(&header(3, 2, 44100, 32, &data)).unwrap();
        assert_eq!((b.left[0], b.right[0]), (0.75, -0.125));
    }

    #[test]
    fn twenty_four_bits_in_four_byte_slots() {
        // Stereo: left -0.5 then +0.25, right +0.25 then -0.5, each in a
        // 4-byte slot. Padded at the top (high bits) and at the bottom.
        let top = [0, 0x00, 0x00, 0xC0, 0, 0x00, 0x00, 0x20, 0, 0x00, 0x00, 0x20, 0, 0x00, 0x00, 0xC0];
        let a = decode(&header_aligned(1, 2, 48000, 24, 8, &top)).unwrap();
        assert_eq!((a.left.clone(), a.right.clone()), (vec![-0.5, 0.25], vec![0.25, -0.5]));
        // At the bottom the top byte is sign; the lowest bit of the first
        // sample is set, as real audio's are, which tells the two apart.
        let bottom = [0x01, 0x00, 0xC0, 0xFF, 0x00, 0x00, 0x20, 0, 0x00, 0x00, 0x20, 0, 0x00, 0x00, 0xC0, 0xFF];
        let b = decode(&header_aligned(1, 2, 48000, 24, 8, &bottom)).unwrap();
        for (x, y) in b.left.iter().chain(&b.right).zip(a.left.iter().chain(&a.right)) {
            assert!((x - y).abs() < 1e-6, "{x} {y}");
        }
        // A block align too small for the frame is refused, not misread.
        let e = decode(&header_aligned(1, 2, 48000, 24, 4, &top)).err().unwrap();
        assert!(e.contains("block align"), "{e}");
    }

    #[test]
    fn not_a_wav_says_how_to_make_one() {
        let e = decode(b"ID3\x04 not a wav at all").err().unwrap();
        assert!(e.contains("ffmpeg"));
    }
}
