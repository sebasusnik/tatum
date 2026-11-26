//! Minimal WAV encoding (16-bit PCM) with no external dependencies, so the
//! CLI, the MCP server and tests all write identical files.

extern crate alloc;
use alloc::vec::Vec;

fn push_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn push_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// The most frames a stereo 16-bit file can hold: the RIFF size is 32 bits
/// and counts the 36 bytes of header after it. About 6 h 46 min at 44.1 kHz.
pub const MAX_STEREO_16_FRAMES: usize = ((u32::MAX - 36) / 4) as usize;

/// The 44-byte header of a stereo 16-bit PCM file of `frames` frames, at
/// most [`MAX_STEREO_16_FRAMES`]. A file written as it plays (`set play
/// --record`) writes it first with the length unknown and again over the top
/// once the length is known.
pub fn header_stereo_16(frames: usize, sample_rate: u32) -> [u8; 44] {
    let data_len = (frames.min(MAX_STEREO_16_FRAMES) * 4) as u32;
    let mut out = Vec::with_capacity(44);
    out.extend_from_slice(b"RIFF");
    push_u32(&mut out, 36 + data_len);
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    push_u32(&mut out, 16);
    push_u16(&mut out, 1); // PCM
    push_u16(&mut out, 2); // channels
    push_u32(&mut out, sample_rate);
    push_u32(&mut out, sample_rate * 4);
    push_u16(&mut out, 4); // block align
    push_u16(&mut out, 16); // bits per sample
    out.extend_from_slice(b"data");
    push_u32(&mut out, data_len);
    let mut header = [0u8; 44];
    header.copy_from_slice(&out);
    header
}

/// One sample as the file stores it: clamped to -1.0..1.0, then 16 bits.
pub fn sample_16(s: f32) -> i16 {
    (s.clamp(-1.0, 1.0) * 32767.0) as i16
}

/// Encode interleaved stereo 16-bit PCM. Samples are clamped to -1.0..1.0.
/// Past [`MAX_STEREO_16_FRAMES`] the file stops: no WAV can hold more.
pub fn encode_stereo_16(left: &[f32], right: &[f32], sample_rate: u32) -> Vec<u8> {
    let frames = left.len().min(right.len()).min(MAX_STEREO_16_FRAMES);
    let mut out = Vec::with_capacity(44 + frames * 4);
    out.extend_from_slice(&header_stereo_16(frames, sample_rate));
    for i in 0..frames {
        for s in [left[i], right[i]] {
            out.extend_from_slice(&sample_16(s).to_le_bytes());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_and_length() {
        let l = [0.0f32, 0.5, -1.0, 2.0];
        let r = [0.0f32, -0.5, 1.0, -2.0];
        let bytes = encode_stereo_16(&l, &r, 44100);
        assert_eq!(bytes.len(), 44 + 4 * 4);
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        // last frame clamps to full scale
        let last_l = i16::from_le_bytes([bytes[44 + 12], bytes[44 + 13]]);
        let last_r = i16::from_le_bytes([bytes[44 + 14], bytes[44 + 15]]);
        assert_eq!(last_l, 32767);
        assert_eq!(last_r, -32767);
    }

    /// Past what a RIFF size can count, the header says the most it can
    /// rather than wrap round to a short file.
    #[test]
    fn a_header_past_the_riff_limit_saturates() {
        let read = |h: &[u8; 44], at: usize| u32::from_le_bytes([h[at], h[at + 1], h[at + 2], h[at + 3]]);
        for frames in [MAX_STEREO_16_FRAMES, MAX_STEREO_16_FRAMES + 1, usize::MAX / 8] {
            let h = header_stereo_16(frames, 44100);
            assert_eq!(read(&h, 40) as usize, MAX_STEREO_16_FRAMES * 4, "{frames}");
            assert_eq!(read(&h, 4), read(&h, 40) + 36);
        }
        let h = header_stereo_16(10, 44100);
        assert_eq!((read(&h, 4), read(&h, 40)), (76, 40));
    }
}
