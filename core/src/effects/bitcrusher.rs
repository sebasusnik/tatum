use crate::math;

/// Bitcrusher — reduces bit depth and sample rate for lo-fi/glitch effects.
pub struct Bitcrusher {
    bit_depth: f32,     // 1.0..16.0 (lower = more crushed)
    rate_reduce: f32,   // 0.0..1.0 (0 = clean, 1 = extreme downsampling)
    mix: f32,           // dry/wet blend
    hold_l: f32,        // sample-and-hold state
    hold_r: f32,
    counter: f32,       // accumulator for rate reduction
}

impl Bitcrusher {
    pub fn new() -> Self {
        Self {
            bit_depth: 16.0,
            rate_reduce: 0.0,
            mix: 1.0,
            hold_l: 0.0,
            hold_r: 0.0,
            counter: 0.0,
        }
    }

    pub fn set_bit_depth(&mut self, bits: f32) {
        self.bit_depth = if bits < 1.0 { 1.0 } else if bits > 16.0 { 16.0 } else { bits };
    }

    pub fn set_rate_reduce(&mut self, rate: f32) {
        self.rate_reduce = if rate < 0.0 { 0.0 } else if rate > 1.0 { 1.0 } else { rate };
    }

    pub fn set_mix(&mut self, mix: f32) {
        self.mix = if mix < 0.0 { 0.0 } else if mix > 1.0 { 1.0 } else { mix };
    }

    /// Quantize a sample to N bits of depth.
    #[inline]
    fn crush(sample: f32, levels: f32) -> f32 {
        let v = sample * levels;
        math::floor(v + 0.5) / levels
    }

    #[inline]
    pub fn process_stereo(&mut self, l: f32, r: f32) -> (f32, f32) {
        let levels = math::pow(2.0, self.bit_depth) - 1.0;

        // Sample-and-hold for rate reduction
        // step = 1.0 means update every sample (clean), higher = skip more
        let step = 1.0 + self.rate_reduce * 49.0; // 1..50
        self.counter += 1.0;
        if self.counter >= step {
            self.counter -= step;
            self.hold_l = Self::crush(l, levels);
            self.hold_r = Self::crush(r, levels);
        }

        let wet_l = self.hold_l;
        let wet_r = self.hold_r;
        (
            l * (1.0 - self.mix) + wet_l * self.mix,
            r * (1.0 - self.mix) + wet_r * self.mix,
        )
    }

    pub fn reset(&mut self) {
        self.hold_l = 0.0;
        self.hold_r = 0.0;
        self.counter = 0.0;
    }
}
