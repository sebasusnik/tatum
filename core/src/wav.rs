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

/// Encode interleaved stereo 16-bit PCM. Samples are clamped to -1.0..1.0.
pub fn encode_stereo_16(left: &[f32], right: &[f32], sample_rate: u32) -> Vec<u8> {
    let frames = left.len().min(right.len());
    let data_len = (frames * 2 * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
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
    for i in 0..frames {
        for s in [left[i], right[i]] {
            let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
            out.extend_from_slice(&v.to_le_bytes());
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
}
