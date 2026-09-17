//! Sample-rate conversion between the engine's 44.1 kHz and whatever the
//! output device runs at. The engine has one rate and no resampler by
//! design (`no_std`, every filter tuned once); Bluetooth outputs offer
//! 48 kHz or 24 kHz and nothing else, so the CLI converts on the way out.
//!
//! Windowed-sinc interpolation, 64 taps, from a table of 512 phases (plus
//! the phase-1.0 row, so the last phase interpolates towards a whole-sample
//! shift and not back to phase 0) with linear interpolation between them. Downsampling scales the sinc so its
//! cutoff sits at the output's Nyquist, which is what keeps a 20 kHz tone
//! from folding back at 24 kHz. Nothing here allocates after construction:
//! it runs inside the audio callback.

use synth_core::BLOCK_SIZE;

const TAPS: usize = 64;
const PHASES: usize = 512;
/// Input frames kept in the buffer before the read point, and how much of
/// the buffer must be free before the callback asks for more.
const CAPACITY: usize = 8192;

pub struct Resampler {
    /// Input frames per output frame.
    ratio: f64,
    table: Vec<f32>,
    left: Vec<f32>,
    right: Vec<f32>,
    /// Frames written into the buffers.
    len: usize,
    /// Fractional read position, in input frames from the buffer start.
    pos: f64,
}

impl Resampler {
    /// `input` is the engine's rate, `output` the device's.
    pub fn new(input: u32, output: u32) -> Self {
        let ratio = input as f64 / output as f64;
        // Sinc cutoff relative to the input rate: the whole band when
        // upsampling, the output's Nyquist when downsampling.
        let fc = (1.0 / ratio).min(1.0) as f32;
        let mut table = vec![0.0f32; (PHASES + 1) * TAPS];
        let half = TAPS as f32 / 2.0;
        for p in 0..=PHASES {
            let frac = p as f32 / PHASES as f32;
            let mut sum = 0.0f32;
            for k in 0..TAPS {
                // Tap k reads input frame floor(pos) - half + 1 + k; its
                // distance from the read point is that minus frac.
                let t = (k as f32 - half + 1.0) - frac;
                let x = std::f32::consts::PI * t;
                // sin(fc x)/x tends to fc at zero, not to 1: with 1 the
                // phase-0 row alone became a spike, and downsampling had
                // a -29 dB floor across the whole stopband.
                let sinc = if x.abs() < 1e-6 { fc } else { (fc * x).sin() / x };
                // Blackman window over the tap span.
                let w = 0.42 - 0.5 * (2.0 * std::f32::consts::PI * (t + half) / TAPS as f32).cos()
                    + 0.08 * (4.0 * std::f32::consts::PI * (t + half) / TAPS as f32).cos();
                let v = sinc * w.max(0.0);
                table[p * TAPS + k] = v;
                sum += v;
            }
            // Unity DC gain at every phase.
            for k in 0..TAPS { table[p * TAPS + k] /= sum; }
        }
        Self {
            ratio,
            table,
            left: vec![0.0; CAPACITY],
            right: vec![0.0; CAPACITY],
            len: TAPS,
            pos: TAPS as f64 / 2.0,
        }
    }

    /// Input frames the caller must supply before `out_frames` can be
    /// produced. Zero means go ahead.
    pub fn needed(&self, out_frames: usize) -> usize {
        let last_read = self.pos + (out_frames as f64) * self.ratio;
        let need = last_read.floor() as usize + TAPS / 2 + 1;
        need.saturating_sub(self.len)
    }

    /// Room for another input block, counting what compaction would free.
    /// False only if the caller stopped reading, which the callback never
    /// does.
    pub fn can_push(&self) -> bool {
        let freeable = (self.pos.floor() as usize).saturating_sub(TAPS);
        self.len - freeable + BLOCK_SIZE <= CAPACITY
    }

    /// Append rendered input. Compacts the buffer first when it is past
    /// half full, keeping the taps' history: a memmove, never an allocation.
    pub fn push(&mut self, l: &[f32], r: &[f32]) {
        if self.len + l.len() > CAPACITY {
            let keep_from = (self.pos.floor() as usize).saturating_sub(TAPS);
            self.left.copy_within(keep_from..self.len, 0);
            self.right.copy_within(keep_from..self.len, 0);
            self.len -= keep_from;
            self.pos -= keep_from as f64;
        }
        let n = l.len().min(CAPACITY - self.len);
        self.left[self.len..self.len + n].copy_from_slice(&l[..n]);
        self.right[self.len..self.len + n].copy_from_slice(&r[..n]);
        self.len += n;
    }

    /// Produce `out_l.len()` output frames. Call `needed` first and `push`
    /// until it returns zero.
    pub fn pull(&mut self, out_l: &mut [f32], out_r: &mut [f32]) {
        let half = TAPS / 2;
        if self.needed(out_l.len()) > 0 {
            // Underfed. A dropout is the honest failure; reading past the
            // buffer on the audio thread is not.
            out_l.fill(0.0);
            out_r.fill(0.0);
            return;
        }
        for i in 0..out_l.len() {
            let ip = self.pos.floor();
            let frac = self.pos - ip;
            let base = ip as usize + 1 - half;
            let ph = frac * PHASES as f64;
            let p0 = ph.floor() as usize;
            let pf = (ph - p0 as f64) as f32;
            let p1 = p0 + 1;
            let (mut al, mut ar) = (0.0f32, 0.0f32);
            let t0 = &self.table[p0 * TAPS..p0 * TAPS + TAPS];
            let t1 = &self.table[p1 * TAPS..p1 * TAPS + TAPS];
            for k in 0..TAPS {
                let c = t0[k] + (t1[k] - t0[k]) * pf;
                al += self.left[base + k] * c;
                ar += self.right[base + k] * c;
            }
            out_l[i] = al;
            out_r[i] = ar;
            self.pos += self.ratio;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run a sine at `hz` through a converter from `fin` to `fout`, return
    /// the output after the first quarter second (the filter settles).
    fn convert_sine(hz: f32, fin: u32, fout: u32, seconds: f32) -> Vec<f32> {
        let mut rs = Resampler::new(fin, fout);
        let total_out = (fout as f32 * seconds) as usize;
        let mut out = vec![0.0f32; total_out];
        let mut scratch = vec![0.0f32; total_out];
        let mut n_in = 0usize;
        let mut pos = 0;
        while pos < total_out {
            let chunk = BLOCK_SIZE.min(total_out - pos);
            while rs.needed(chunk) > 0 {
                let mut l = [0.0f32; BLOCK_SIZE];
                for v in l.iter_mut() {
                    *v = (2.0 * std::f64::consts::PI * hz as f64 * n_in as f64 / fin as f64).sin() as f32;
                    n_in += 1;
                }
                rs.push(&l, &l);
            }
            rs.pull(&mut out[pos..pos + chunk], &mut scratch[pos..pos + chunk]);
            pos += chunk;
        }
        out[(fout / 4) as usize..].to_vec()
    }

    fn rms(x: &[f32]) -> f32 { (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt() }

    fn zero_crossings(x: &[f32]) -> usize {
        x.windows(2).filter(|w| (w[0] >= 0.0) != (w[1] >= 0.0)).count()
    }

    #[test]
    fn a_tone_keeps_its_pitch_and_level_going_up_to_48k() {
        let out = convert_sine(1000.0, 44100, 48000, 1.25);
        // 1 kHz for one second: 2000 zero crossings, give or take the ends.
        let zc = zero_crossings(&out);
        assert!((zc as i64 - 2000).abs() <= 2, "{} zero crossings, expected 2000", zc);
        let db = 20.0 * (rms(&out) / (1.0 / 2f32.sqrt())).log10();
        assert!(db.abs() < 0.1, "level off by {:.2} dB", db);
    }

    #[test]
    fn a_tone_keeps_its_pitch_and_level_going_down_to_24k() {
        let out = convert_sine(1000.0, 44100, 24000, 1.25);
        let zc = zero_crossings(&out);
        assert!((zc as i64 - 2000).abs() <= 2, "{} zero crossings, expected 2000", zc);
        let db = 20.0 * (rms(&out) / (1.0 / 2f32.sqrt())).log10();
        assert!(db.abs() < 0.1, "level off by {:.2} dB", db);
    }

    #[test]
    fn downsampling_does_not_fold_the_top_octave_back() {
        // 20 kHz cannot exist at 24 kHz output; without the cutoff scaled to
        // the output's Nyquist it would come out as a 4 kHz alias at full
        // level. It has to be at least 40 dB down.
        let out = convert_sine(20000.0, 44100, 24000, 1.25);
        let db = 20.0 * (rms(&out) / (1.0 / 2f32.sqrt())).log10();
        assert!(db < -40.0, "20 kHz into 24 kHz output leaks at {:.1} dB", db);
    }

    #[test]
    fn upsampling_passes_the_audible_band_flat() {
        for hz in [100.0, 5000.0, 12000.0, 16000.0] {
            let out = convert_sine(hz, 44100, 48000, 1.25);
            let db = 20.0 * (rms(&out) / (1.0 / 2f32.sqrt())).log10();
            assert!(db.abs() < 0.5, "{} Hz off by {:.2} dB", hz, db);
        }
    }

    #[test]
    fn the_buffer_compacts_without_losing_continuity() {
        // Long enough that the buffer wraps many times: a discontinuity at
        // a compaction would show as a jump between consecutive samples far
        // larger than a 1 kHz sine can make (about 0.13 per sample at 48k).
        let out = convert_sine(1000.0, 44100, 48000, 10.0);
        let worst = out.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
        assert!(worst < 0.15, "a jump of {:.3} between consecutive samples", worst);
    }
}
#[cfg(test)]
mod no_alloc {
    use super::*;

    /// Everything the audio callback calls, counted by the crate's test
    /// allocator: needed, push and pull, across enough blocks to compact the
    /// buffer several times.
    #[test]
    fn the_callback_path_never_allocates() {
        let mut rs = Resampler::new(44100, 48000);
        let l = [0.1f32; BLOCK_SIZE];
        let mut ol = [0.0f32; 512];
        let mut or = [0.0f32; 512];
        crate::test_alloc::ALLOCS.with(|a| a.set(0));
        crate::test_alloc::COUNTING.with(|c| c.set(true));
        for _ in 0..2000 {
            while rs.needed(ol.len()) > 0 && rs.can_push() {
                rs.push(&l, &l);
            }
            rs.pull(&mut ol, &mut or);
        }
        crate::test_alloc::COUNTING.with(|c| c.set(false));
        let n = crate::test_alloc::ALLOCS.with(|a| a.get());
        assert_eq!(n, 0, "the resampler allocated {} times in the callback path", n);
    }
}
