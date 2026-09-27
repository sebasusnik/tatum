//! A spectrogram built as the audio streams through, one column at a time,
//! so a four-minute song with twenty parts never has to be held in memory.
//!
//! Frequency runs on a musical (logarithmic) scale from 20 Hz to 20 kHz: an
//! octave takes the same height at the bottom as at the top, which is how the
//! ear hears it and what makes a bass and a hat readable in one picture. The
//! loudness of each cell is absolute, in dB against full scale, so the parts
//! of a song can be compared by colour: a quiet track looks dark because it
//! is quiet, and a floor of hiss shows as the same purple haze wherever it is.

use crate::fft::fft;
use tatum_core::SAMPLE_RATE;

/// Rows per column. The sheet shrinks them; the part's own picture shows all.
pub const ROWS: usize = 256;
pub const LOW_HZ: f32 = 20.0;
pub const HIGH_HZ: f32 = 20_000.0;
/// The bottom of the colour scale. Below this is black.
pub const FLOOR_DB: f32 = -100.0;

pub struct Spectrogram {
    size: usize,
    hop: usize,
    ring: Vec<f32>,
    seen: usize,
    /// Sample count at which the next column is due. Columns are centred on
    /// `k * hop + hop / 2` of the range, so the picture lines up with the bar
    /// lines drawn over it rather than trailing them by half a window.
    next_at: usize,
    window: Vec<f64>,
    re: Vec<f64>,
    im: Vec<f64>,
    /// Inclusive bin range each row takes its value from.
    row_bins: Vec<(usize, usize)>,
    /// One `ROWS`-long run of loudness per column, row 0 the lowest frequency,
    /// 0 = `FLOOR_DB` or less, 255 = full scale.
    pub cells: Vec<u8>,
}

impl Spectrogram {
    /// `hop` samples per column. The window is several hops long so columns
    /// overlap, and never shorter than 1024 samples, which is where a bass
    /// note stops being resolvable at all.
    pub fn new(hop: usize) -> Spectrogram {
        let size = (hop * 4).next_power_of_two().clamp(1024, 4096);
        let window =
            (0..size).map(|i| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / size as f64).cos()).collect();
        let bin_hz = SAMPLE_RATE / size as f32;
        let row_bins = (0..ROWS)
            .map(|r| {
                let (f0, f1) = (row_hz(r as f32), row_hz(r as f32 + 1.0));
                let lo = ((f0 / bin_hz).round() as usize).max(1);
                let hi = ((f1 / bin_hz).round() as usize).max(lo).min(size / 2);
                (lo.min(hi), hi)
            })
            .collect();
        Spectrogram {
            size,
            hop,
            ring: vec![0.0; size],
            seen: 0,
            next_at: hop / 2 + size / 2,
            window,
            re: vec![0.0; size],
            im: vec![0.0; size],
            row_bins,
            cells: Vec::new(),
        }
    }

    pub fn push(&mut self, x: f32) {
        self.ring[self.seen % self.size] = x;
        self.seen += 1;
        if self.seen == self.next_at {
            self.column();
            self.next_at += self.hop;
        }
    }

    /// Run the columns out to `count`, feeding silence past the end, so every
    /// part's picture has the same width whatever its last window caught.
    pub fn finish(&mut self, count: usize) {
        while self.columns() < count {
            self.push(0.0);
        }
        self.cells.truncate(count * ROWS);
    }

    pub fn columns(&self) -> usize {
        self.cells.len() / ROWS
    }

    /// Cell of column `c`, row `r` (row 0 = 20 Hz).
    pub fn cell(&self, c: usize, r: usize) -> u8 {
        self.cells[c * ROWS + r]
    }

    fn column(&mut self) {
        let start = self.seen % self.size; // oldest sample
        for i in 0..self.size {
            self.re[i] = self.ring[(start + i) % self.size] as f64 * self.window[i];
            self.im[i] = 0.0;
        }
        fft(&mut self.re, &mut self.im);
        // A Hann window sums to size / 2, so a full-scale sine peaks at size / 4.
        let norm = (self.size as f64 / 4.0).powi(2);
        for &(lo, hi) in &self.row_bins {
            let mut p = 0.0f64;
            for b in lo..=hi {
                p = p.max(self.re[b] * self.re[b] + self.im[b] * self.im[b]);
            }
            let db = 10.0 * (p / norm).max(1e-20).log10() as f32;
            self.cells.push(to_cell(db));
        }
    }
}

/// Frequency at the bottom edge of row `r` (fractional rows allowed).
pub fn row_hz(r: f32) -> f32 {
    LOW_HZ * (HIGH_HZ / LOW_HZ).powf(r / ROWS as f32)
}

/// The row a frequency falls in.
pub fn hz_row(hz: f32) -> f32 {
    (hz / LOW_HZ).ln() / (HIGH_HZ / LOW_HZ).ln() * ROWS as f32
}

pub fn to_cell(db: f32) -> u8 {
    ((db - FLOOR_DB) / -FLOOR_DB * 255.0).clamp(0.0, 255.0) as u8
}

pub fn cell_db(c: u8) -> f32 {
    FLOOR_DB + c as f32 / 255.0 * -FLOOR_DB
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_scale_sine_lands_on_its_row_near_zero_db() {
        let mut s = Spectrogram::new(512);
        for i in 0..44100 {
            s.push((2.0 * std::f32::consts::PI * 1000.0 * i as f32 / SAMPLE_RATE).sin());
        }
        let c = s.columns() / 2;
        let (row, &top) = (0..ROWS)
            .map(|r| s.cell(c, r))
            .collect::<Vec<_>>()
            .iter()
            .enumerate()
            .max_by_key(|(_, v)| **v)
            .map(|(r, v)| (r, v))
            .unwrap();
        assert!((row_hz(row as f32) - 1000.0).abs() < 60.0, "row {row} = {} Hz", row_hz(row as f32));
        assert!(cell_db(top) > -2.0, "peak {} dB", cell_db(top));
        // and an octave away there is next to nothing
        let far = hz_row(4000.0) as usize;
        assert!(cell_db(s.cell(c, far)) < -60.0);
    }
}
