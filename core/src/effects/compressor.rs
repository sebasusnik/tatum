use crate::math;

pub struct Compressor {
    gain: f32,
    threshold: f32,
    ratio: f32,
    attack_coeff: f32,
    release_coeff: f32,
    makeup_gain: f32,
    sample_rate: f32,
}

impl Compressor {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            gain: 1.0,
            threshold: math::db_to_linear(-3.0), // ~0.7
            ratio: 3.0,
            attack_coeff: math::exp(-1.0 / (0.020 * sample_rate)),
            release_coeff: math::exp(-1.0 / (0.1 * sample_rate)),
            makeup_gain: 1.0,
            sample_rate,
        }
    }

    pub fn set_threshold(&mut self, db: f32) {
        let db = math::clamp(db, -40.0, 0.0);
        self.threshold = math::db_to_linear(db);
    }

    pub fn set_ratio(&mut self, ratio: f32) {
        self.ratio = math::clamp(ratio, 1.0, 20.0);
    }

    pub fn set_attack(&mut self, ms: f32) {
        let ms = math::clamp(ms, 0.1, 100.0);
        self.attack_coeff = math::exp(-1.0 / (ms * 0.001 * self.sample_rate));
    }

    pub fn set_release(&mut self, ms: f32) {
        let ms = math::clamp(ms, 10.0, 500.0);
        self.release_coeff = math::exp(-1.0 / (ms * 0.001 * self.sample_rate));
    }

    pub fn set_makeup(&mut self, gain: f32) {
        self.makeup_gain = math::clamp(gain, 0.0, 6.0);
    }

    #[inline]
    pub fn process_stereo(&mut self, l: f32, r: f32) -> (f32, f32) {
        // Stereo-linked peak detection
        let abs_l = math::abs(l);
        let abs_r = math::abs(r);
        let peak = if abs_l > abs_r { abs_l } else { abs_r };

        // Gain computation in linear domain (equivalent to dB-domain ratio)
        let target_gain = if peak > self.threshold {
            // target = (threshold / peak)^(1 - 1/ratio)
            let reduction = self.threshold / peak;
            math::pow(reduction, 1.0 - 1.0 / self.ratio)
        } else {
            1.0
        };

        // Smoothing: attack when compressing more, release when compressing less
        if target_gain < self.gain {
            self.gain = self.attack_coeff * self.gain + (1.0 - self.attack_coeff) * target_gain;
        } else {
            self.gain = self.release_coeff * self.gain + (1.0 - self.release_coeff) * target_gain;
        }

        (l * self.gain * self.makeup_gain, r * self.gain * self.makeup_gain)
    }

    pub fn reset(&mut self) {
        self.gain = 1.0;
    }
}
