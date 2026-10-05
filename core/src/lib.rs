#![no_std]
#[allow(unused_imports)]
#[macro_use]
extern crate alloc;

#[cfg(test)]
extern crate std;

pub mod math;
pub mod rng;
pub mod primitives;
pub mod harmony;
pub mod modules;
pub mod effects;
pub mod graph;
pub mod dsl;
pub mod song_engine;
pub mod live;
pub mod midi;
pub mod params;
pub mod wav;
pub mod analysis;
pub mod nodes;
pub mod output;

/// The rate the engine renders at. 44.1 kHz unless the build sets
/// `TATUM_SAMPLE_RATE` (an experiment: a smaller machine can render fewer
/// samples a second).
pub const SAMPLE_RATE: f32 = match option_env!("TATUM_SAMPLE_RATE") {
    Some(s) => parse_rate(s),
    None => 44100.0,
};

const fn parse_rate(s: &str) -> f32 {
    let b = s.as_bytes();
    let mut n = 0u32;
    let mut i = 0;
    while i < b.len() {
        n = n * 10 + (b[i] - b'0') as u32;
        i += 1;
    }
    n as f32
}

/// A per-sample decay tuned at 44.1 kHz, at the engine's rate: the same time
/// constant. At 44.1 kHz it is the number as written, bit for bit.
pub const fn per_sample(decay_at_44k: f32) -> f32 {
    if SAMPLE_RATE == 44100.0 {
        return decay_at_44k;
    }
    // decay^k = exp(k ln decay), worked out at compile time: ln by the atanh
    // series (the decay is close to 1), exp by its Taylor series.
    let k = 44100.0 / SAMPLE_RATE;
    let z = (decay_at_44k - 1.0) / (decay_at_44k + 1.0);
    let (mut ln, mut term, mut n) = (0.0f64, z as f64, 1.0f64);
    while n < 60.0 {
        ln += term / n;
        term *= (z * z) as f64;
        n += 2.0;
    }
    let x = 2.0 * ln * k as f64;
    let (mut e, mut t, mut i) = (1.0f64, 1.0f64, 1.0f64);
    while i < 40.0 {
        t *= x / i;
        e += t;
        i += 1.0;
    }
    e as f32
}

/// A one-pole step (`x += (target - x) * step`) tuned at 44.1 kHz, at the
/// engine's rate: the same glide time. Bit for bit itself at 44.1 kHz.
pub fn one_pole_step(step_at_44k: f32) -> f32 {
    if SAMPLE_RATE == 44100.0 {
        step_at_44k
    } else {
        1.0 - math::pow(1.0 - step_at_44k, 44100.0 / SAMPLE_RATE)
    }
}

/// A length tuned in samples at 44.1 kHz, at the engine's rate: the same time.
pub const fn at_rate(samples_at_44k: usize) -> usize {
    if SAMPLE_RATE == 44100.0 {
        return samples_at_44k;
    }
    // f64: in f32, 1116 * 44100 / 44100 is 1115.99 and truncates to 1115.
    let n = (samples_at_44k as f64 * SAMPLE_RATE as f64 / 44100.0) as usize;
    if n == 0 {
        1
    } else {
        n
    }
}
pub const BLOCK_SIZE: usize = 128;
pub const MAX_VOICES: usize = 8;

pub trait Module {
    fn process_block(&mut self, output: &mut [f32]);
    fn note_on(&mut self, note: u8, velocity: f32);
    fn note_off(&mut self, note: u8);
    fn reset(&mut self);
}
