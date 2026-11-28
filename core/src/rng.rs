/// Simple linear congruential generator for no_std environments.
/// Used for drift noise, percussion noise, etc.
pub struct Rng {
    state: u32,
}

impl Rng {
    pub fn new(seed: u32) -> Self {
        Self { state: if seed == 0 { 1 } else { seed } }
    }

    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        self.state = self.state.wrapping_mul(1664525).wrapping_add(1013904223);
        self.state
    }

    /// Returns a float in [0.0, 1.0).
    #[inline]
    pub fn next_f32(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / 16777216.0
    }

    /// Returns a float in [-1.0, 1.0).
    #[inline]
    pub fn next_bipolar(&mut self) -> f32 {
        self.next_f32() * 2.0 - 1.0
    }
}

/// Two numbers stirred into one, so that neighbours come out unrelated: the
/// seed of a node and the number of the note it plays, or of the stretch of
/// time it is in. The finalizer of MurmurHash3 over their combination.
#[inline]
pub fn stir(a: u32, b: u32) -> u32 {
    let mut h = a ^ b.wrapping_mul(0x9E37_79B9).rotate_left(13);
    h ^= h >> 16;
    h = h.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 13;
    h = h.wrapping_mul(0xC2B2_AE35);
    h ^= h >> 16;
    h
}

/// `stir` as a number in 0..1.
#[inline]
pub fn stir_unit(a: u32, b: u32) -> f32 {
    (stir(a, b) >> 8) as f32 / 16777216.0
}
