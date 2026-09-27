//! Capture: record a stretch of a chain's output, then loop it back.
//!
//! This is the thing a sampler does that no filter or delay can approximate:
//! take two bars of what a track played, and play them back slowed, reversed,
//! or on repeat under the rest of the song. Delays repeat the last few hundred
//! milliseconds; this holds a musical phrase.
//!
//! The buffer is sized by the compiler from the song's slowest tempo and
//! allocated when the chain is built, never while audio is running.

extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    /// Before `start`: the signal passes untouched.
    Waiting,
    /// Recording the window, still passing the signal through.
    Recording,
    /// The window is full; the recording plays back.
    Playing,
}

pub struct Capture {
    left: Vec<f32>,
    right: Vec<f32>,
    /// Samples of silence before recording begins (`start` in bars).
    start_samples: u32,
    elapsed: u32,
    write_pos: usize,
    /// Fractional so `speed` can stretch the playback.
    read_pos: f32,
    speed: f32,
    reverse: bool,
    mix: f32,
    phase: Phase,
}

impl Capture {
    /// `samples` is the window length; the buffer is allocated here and never
    /// grows. `speed` below 1 stretches (and drops the pitch with it).
    pub fn new(samples: u32, start_samples: u32, speed: f32, reverse: bool, mix: f32) -> Self {
        let len = (samples.max(1)) as usize;
        Self {
            left: vec![0.0; len],
            right: vec![0.0; len],
            start_samples,
            elapsed: 0,
            write_pos: 0,
            read_pos: 0.0,
            speed: speed.max(0.01),
            reverse,
            mix: mix.clamp(0.0, 1.0),
            phase: if start_samples == 0 { Phase::Recording } else { Phase::Waiting },
        }
    }

    /// Linear interpolation into the captured window, wrapping at both ends so
    /// the loop is seamless at any speed.
    fn read(&self, pos: f32) -> (f32, f32) {
        let len = self.left.len();
        if len == 0 {
            return (0.0, 0.0);
        }
        let i = pos as usize % len;
        let j = (i + 1) % len;
        let frac = pos - crate::math::floor(pos);
        (self.left[i] + (self.left[j] - self.left[i]) * frac, self.right[i] + (self.right[j] - self.right[i]) * frac)
    }

    pub fn process_stereo(&mut self, l: f32, r: f32) -> (f32, f32) {
        match self.phase {
            Phase::Waiting => {
                self.elapsed += 1;
                if self.elapsed >= self.start_samples {
                    self.phase = Phase::Recording;
                }
                (l, r)
            }
            Phase::Recording => {
                self.left[self.write_pos] = l;
                self.right[self.write_pos] = r;
                self.write_pos += 1;
                if self.write_pos >= self.left.len() {
                    self.phase = Phase::Playing;
                    // Reverse playback starts at the end of the window.
                    self.read_pos = if self.reverse { self.left.len() as f32 - 1.0 } else { 0.0 };
                }
                (l, r)
            }
            Phase::Playing => {
                let (cl, cr) = self.read(self.read_pos);
                let len = self.left.len() as f32;
                if self.reverse {
                    self.read_pos -= self.speed;
                    if self.read_pos < 0.0 {
                        self.read_pos += len;
                    }
                } else {
                    self.read_pos += self.speed;
                    if self.read_pos >= len {
                        self.read_pos -= len;
                    }
                }
                let dry = 1.0 - self.mix;
                (l * dry + cl * self.mix, r * dry + cr * self.mix)
            }
        }
    }

    pub fn process(&mut self, x: f32) -> f32 {
        self.process_stereo(x, x).0
    }

    pub fn reset(&mut self) {
        for v in self.left.iter_mut() {
            *v = 0.0;
        }
        for v in self.right.iter_mut() {
            *v = 0.0;
        }
        self.elapsed = 0;
        self.write_pos = 0;
        self.read_pos = 0.0;
        self.phase = if self.start_samples == 0 { Phase::Recording } else { Phase::Waiting };
    }

    pub fn set_named(&mut self, name: &str, value: f32) -> bool {
        match name {
            "mix" => {
                self.mix = value.clamp(0.0, 1.0);
                true
            }
            "speed" => {
                self.speed = value.max(0.01);
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ramp in, the same ramp out on the next pass.
    #[test]
    fn the_window_comes_back_verbatim_at_speed_one() {
        let mut c = Capture::new(8, 0, 1.0, false, 1.0);
        let input: [f32; 8] = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8];
        for x in input {
            assert_eq!(c.process_stereo(x, x), (x, x), "recording passes the signal through");
        }
        for x in input {
            let (l, _) = c.process_stereo(0.0, 0.0);
            assert!((l - x).abs() < 1e-6, "expected {}, got {}", x, l);
        }
    }

    #[test]
    fn reverse_plays_the_window_backwards() {
        let mut c = Capture::new(4, 0, 1.0, true, 1.0);
        for x in [1.0f32, 2.0, 3.0, 4.0] {
            c.process_stereo(x, x);
        }
        let out: Vec<f32> = (0..4).map(|_| c.process_stereo(0.0, 0.0).0).collect();
        assert!((out[0] - 4.0).abs() < 1e-6, "{:?}", out);
        assert!((out[1] - 3.0).abs() < 1e-6, "{:?}", out);
    }

    #[test]
    fn half_speed_takes_twice_as_long_to_get_through_the_window() {
        let mut c = Capture::new(8, 0, 0.5, false, 1.0);
        for i in 0..8 {
            c.process_stereo(i as f32, i as f32);
        }
        let out: Vec<f32> = (0..4).map(|_| c.process_stereo(0.0, 0.0).0).collect();
        // Reading at half speed: 0, 0.5, 1.0, 1.5 into a 0..7 ramp.
        assert!((out[1] - 0.5).abs() < 1e-5, "{:?}", out);
        assert!((out[3] - 1.5).abs() < 1e-5, "{:?}", out);
    }

    #[test]
    fn start_delays_the_window_and_passes_the_signal_until_then() {
        let mut c = Capture::new(2, 3, 1.0, false, 1.0);
        for x in [9.0f32, 9.0, 9.0] {
            assert_eq!(c.process_stereo(x, x).0, x, "waiting passes through");
        }
        c.process_stereo(1.0, 1.0);
        c.process_stereo(2.0, 2.0);
        let out = c.process_stereo(0.0, 0.0).0;
        assert!((out - 1.0).abs() < 1e-6, "captured the window after start, got {}", out);
    }

    #[test]
    fn mix_blends_with_the_live_signal() {
        let mut c = Capture::new(2, 0, 1.0, false, 0.5);
        c.process_stereo(1.0, 1.0);
        c.process_stereo(1.0, 1.0);
        let out = c.process_stereo(3.0, 3.0).0;
        assert!((out - 2.0).abs() < 1e-6, "half of 3 live plus half of 1 captured, got {}", out);
    }
}
