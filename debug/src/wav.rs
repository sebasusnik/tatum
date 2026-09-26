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
