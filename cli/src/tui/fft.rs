//! A radix-2 FFT for the screen's spectrogram. Small and allocation-free once
//! built: the screen runs one a frame on a few thousand samples.

pub struct Fft {
    n: usize,
    cos: Vec<f32>,
    sin: Vec<f32>,
    window: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
}

impl Fft {
    /// `n` must be a power of two.
    pub fn new(n: usize) -> Self {
        assert!(n.is_power_of_two());
        let tau = std::f32::consts::TAU;
        Self {
            n,
            cos: (0..n / 2).map(|k| (tau * k as f32 / n as f32).cos()).collect(),
            sin: (0..n / 2).map(|k| -(tau * k as f32 / n as f32).sin()).collect(),
            // Hann: the leakage of a square window smears a bass note up
            // the whole screen.
            window: (0..n).map(|i| 0.5 - 0.5 * (tau * i as f32 / (n - 1) as f32).cos()).collect(),
            re: vec![0.0; n],
            im: vec![0.0; n],
        }
    }

    pub fn len(&self) -> usize {
        self.n
    }

    /// Power per bin, `n / 2` of them, in dB relative to a full-scale sine.
    pub fn power_db(&mut self, input: &[f32], out: &mut [f32]) {
        let n = self.n;
        for i in 0..n {
            self.re[i] = input.get(i).copied().unwrap_or(0.0) * self.window[i];
            self.im[i] = 0.0;
        }
        // Bit-reversal permutation.
        let mut j = 0;
        for i in 1..n {
            let mut bit = n >> 1;
            while j & bit != 0 {
                j ^= bit;
                bit >>= 1;
            }
            j |= bit;
            if i < j {
                self.re.swap(i, j);
                self.im.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= n {
            let step = n / len;
            for start in (0..n).step_by(len) {
                for k in 0..len / 2 {
                    let (wr, wi) = (self.cos[k * step], self.sin[k * step]);
                    let (a, b) = (start + k, start + k + len / 2);
                    let tr = self.re[b] * wr - self.im[b] * wi;
                    let ti = self.re[b] * wi + self.im[b] * wr;
                    self.re[b] = self.re[a] - tr;
                    self.im[b] = self.im[a] - ti;
                    self.re[a] += tr;
                    self.im[a] += ti;
                }
            }
            len <<= 1;
        }
        // A full-scale sine through a Hann window peaks at n/4.
        let reference = (n as f32 / 4.0).powi(2);
        for (k, o) in out.iter_mut().enumerate().take(n / 2) {
            let p = (self.re[k] * self.re[k] + self.im[k] * self.im[k]) / reference;
            *o = 10.0 * (p + 1e-12).log10();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_scale_sine_reads_zero_db_at_its_bin() {
        let n = 4096;
        let sr = 44_100.0;
        let bin = 93;
        let hz = bin as f32 * sr / n as f32;
        let x: Vec<f32> = (0..n).map(|i| (std::f32::consts::TAU * hz * i as f32 / sr).sin()).collect();
        let mut fft = Fft::new(n);
        let mut out = vec![0.0; n / 2];
        fft.power_db(&x, &mut out);
        assert!(out[bin].abs() < 0.5, "{} dB at the sine's bin", out[bin]);
        assert!(out[bin + 20] < -60.0, "{} dB twenty bins away", out[bin + 20]);
    }
}
