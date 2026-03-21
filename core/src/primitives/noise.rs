use crate::rng::Rng;

/// White noise generator using LCG random number generator.
pub struct NoiseGen {
    rng: Rng,
}

impl NoiseGen {
    pub fn new(seed: u32) -> Self {
        Self { rng: Rng::new(seed) }
    }

    /// Returns a white noise sample in [-1.0, 1.0).
    #[inline]
    pub fn next_sample(&mut self) -> f32 {
        self.rng.next_bipolar()
    }

    pub fn reset(&mut self) {
        // Cannot truly reset RNG without storing original seed,
        // but noise is stateless from a musical perspective
    }
}
