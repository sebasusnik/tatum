use crate::math;

const LOOKAHEAD_SIZE: usize = 44; // ~1ms at 44100 Hz

pub struct Limiter {
    lookahead_l: [f32; LOOKAHEAD_SIZE],
    lookahead_r: [f32; LOOKAHEAD_SIZE],
    write_pos: usize,
    gain: f32,
    threshold: f32,
    release_coeff: f32,
    makeup_gain: f32,
}

impl Limiter {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            lookahead_l: [0.0; LOOKAHEAD_SIZE],
            lookahead_r: [0.0; LOOKAHEAD_SIZE],
            write_pos: 0,
            gain: 1.0,
            threshold: 0.95,
            release_coeff: math::exp(-1.0 / (0.03 * sample_rate)),
            makeup_gain: 1.0,
        }
    }

    pub fn set_threshold(&mut self, threshold: f32) {
        self.threshold = math::clamp(threshold, 0.1, 1.0);
    }

    pub fn set_release(&mut self, ms: f32, sample_rate: f32) {
        self.release_coeff = math::exp(-1.0 / (ms * 0.001 * sample_rate));
    }

    pub fn set_makeup_gain(&mut self, gain: f32) {
        self.makeup_gain = math::clamp(gain, 0.0, 6.0);
    }

    #[inline]
    pub fn process_stereo(&mut self, l: f32, r: f32) -> (f32, f32) {
        // Write new samples into the lookahead ring buffer
        self.lookahead_l[self.write_pos] = l;
        self.lookahead_r[self.write_pos] = r;

        // Read delayed samples (1ms behind)
        let read_pos = (self.write_pos + 1) % LOOKAHEAD_SIZE;
        let delayed_l = self.lookahead_l[read_pos];
        let delayed_r = self.lookahead_r[read_pos];

        // Scan the entire lookahead buffer for peak (stereo-linked)
        let mut peak = 0.0f32;
        for i in 0..LOOKAHEAD_SIZE {
            let abs_l = math::abs(self.lookahead_l[i]);
            let abs_r = math::abs(self.lookahead_r[i]);
            let sample_peak = if abs_l > abs_r { abs_l } else { abs_r };
            if sample_peak > peak {
                peak = sample_peak;
            }
        }

        // Calculate target gain
        let target = if peak > self.threshold { self.threshold / peak } else { 1.0 };

        // Smooth gain: instant attack, exponential release
        if target < self.gain {
            self.gain = target;
        } else {
            self.gain = self.release_coeff * self.gain + (1.0 - self.release_coeff) * target;
        }

        // Advance write position
        self.write_pos = (self.write_pos + 1) % LOOKAHEAD_SIZE;

        (delayed_l * self.gain * self.makeup_gain, delayed_r * self.gain * self.makeup_gain)
    }

    pub fn reset(&mut self) {
        self.lookahead_l = [0.0; LOOKAHEAD_SIZE];
        self.lookahead_r = [0.0; LOOKAHEAD_SIZE];
        self.write_pos = 0;
        self.gain = 1.0;
    }
}
