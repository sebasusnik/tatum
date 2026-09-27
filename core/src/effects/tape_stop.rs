use alloc::vec;
use alloc::vec::Vec;
use crate::math;

const TAPE_BUF_SIZE: usize = 8192;

/// Tape Stop — simulates a turntable/tape machine slowing to a halt.
/// When triggered, playback speed ramps from 1.0 → 0.0 over `ramp_time` seconds,
/// causing the pitch to drop. When released, speed snaps back to 1.0.
pub struct TapeStop {
    buffer_l: Vec<f32>,
    buffer_r: Vec<f32>,
    write_pos: usize,
    read_pos: f32,       // fractional for pitch interpolation
    speed: f32,          // 1.0 = normal, ramps to 0.0
    ramp_time: f32,      // seconds to reach full stop
    ramp_decrement: f32, // per-sample speed decrease
    active: bool,
    mix: f32,
    sample_rate: f32,
}

impl TapeStop {
    pub fn new(sample_rate: f32) -> Self {
        let ramp_time = 0.5;
        Self {
            buffer_l: vec![0.0; TAPE_BUF_SIZE],
            buffer_r: vec![0.0; TAPE_BUF_SIZE],
            write_pos: 0,
            read_pos: 0.0,
            speed: 1.0,
            ramp_time,
            ramp_decrement: 1.0 / (ramp_time * sample_rate),
            active: false,
            mix: 1.0,
            sample_rate,
        }
    }

    pub fn set_ramp_time(&mut self, seconds: f32) {
        self.ramp_time = seconds.clamp(0.05, 3.0);
        self.ramp_decrement = 1.0 / (self.ramp_time * self.sample_rate);
    }

    pub fn set_mix(&mut self, mix: f32) {
        self.mix = mix.clamp(0.0, 1.0);
    }

    pub fn trigger(&mut self, on: bool) {
        self.active = on;
        if !on {
            // Snap back to normal speed
            self.speed = 1.0;
            self.read_pos = self.write_pos as f32;
        }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Linear interpolation read from ring buffer.
    #[inline]
    fn read_interp(buffer: &[f32], pos: f32) -> f32 {
        let len = buffer.len();
        let idx = pos as usize % len;
        let frac = pos - math::floor(pos);
        let next = (idx + 1) % len;
        buffer[idx] * (1.0 - frac) + buffer[next] * frac
    }

    #[inline]
    pub fn process_stereo(&mut self, l: f32, r: f32) -> (f32, f32) {
        let len = self.buffer_l.len();

        // Always write input into the buffer
        self.buffer_l[self.write_pos] = l;
        self.buffer_r[self.write_pos] = r;
        self.write_pos = (self.write_pos + 1) % len;

        if !self.active {
            // Pass through
            return (l, r);
        }

        // Ramp speed down
        if self.speed > 0.0 {
            self.speed -= self.ramp_decrement;
            if self.speed < 0.0 {
                self.speed = 0.0;
            }
        }

        // Advance read position at current speed
        self.read_pos += self.speed;
        // Wrap
        if self.read_pos >= len as f32 {
            self.read_pos -= len as f32;
        }

        let wet_l = Self::read_interp(&self.buffer_l, self.read_pos);
        let wet_r = Self::read_interp(&self.buffer_r, self.read_pos);

        (l * (1.0 - self.mix) + wet_l * self.mix, r * (1.0 - self.mix) + wet_r * self.mix)
    }

    pub fn reset(&mut self) {
        for s in self.buffer_l.iter_mut() {
            *s = 0.0;
        }
        for s in self.buffer_r.iter_mut() {
            *s = 0.0;
        }
        self.write_pos = 0;
        self.read_pos = 0.0;
        self.speed = 1.0;
        self.active = false;
    }
}
