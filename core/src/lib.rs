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
pub mod sequencer;
pub mod effects;
pub mod engine;

#[cfg(test)]
pub mod test_utils;

pub const SAMPLE_RATE: f32 = 44100.0;
pub const BLOCK_SIZE: usize = 128;
pub const MAX_VOICES: usize = 8;

pub trait Module {
    fn process_block(&mut self, output: &mut [f32]);
    fn note_on(&mut self, note: u8, velocity: f32);
    fn note_off(&mut self, note: u8);
    fn reset(&mut self);
}
