//! A transient shaper: the hit and the tail of a sound turned up or down on
//! their own, whatever the level.
//!
//! A compressor works on level, so it cannot tell a kick's click from its
//! body if both are loud. This follows the signal twice and compares:
//!
//! - for the attack, a follower that rises in a millisecond against one that
//!   rises in twenty. While a hit is starting the fast one is ahead, and how
//!   far ahead is how much of the sound is attack right now;
//! - for the sustain, a follower that falls in twenty milliseconds against one
//!   that falls in five hundred. While a sound is dying the slow one is
//!   behind, and how far is how much of it is tail.
//!
//! Each share times its knob, in dB, is the gain. A steady tone is neither,
//! and passes at unity. Both channels are followed together, so a stereo
//! sound keeps its image.
//!
//! The followers do not read the wave itself but its peak, held for 12 ms:
//! a 50 Hz kick's wave goes through zero every 10 ms, and followers fast
//! enough to catch its hit would read each zero as the sound dying.

use crate::math;

pub struct Transient {
    attack_db: f32,
    sustain_db: f32,
    /// Coefficients: fast and slow rise, fast and slow fall.
    rise_fast: f32,
    rise_slow: f32,
    fall_fast: f32,
    fall_slow: f32,
    /// Followers: attack pair (fast rise, slow rise; both fall in 20 ms, so
    /// they meet again once the hit is over), sustain pair (fast fall, slow
    /// fall; both rise fast).
    att_fast: f32,
    att_slow: f32,
    sus_fast: f32,
    sus_slow: f32,
    /// The held peak, how long it holds, and how fast it lets go after.
    held: f32,
    hold_left: u32,
    hold: u32,
    let_go: f32,
}

/// A one-pole coefficient for `ms`.
fn coeff(ms: f32, sample_rate: f32) -> f32 {
    math::exp(-1.0 / (ms * 0.001 * sample_rate))
}

impl Transient {
    pub fn new(attack_db: f32, sustain_db: f32, sample_rate: f32) -> Self {
        Self {
            attack_db,
            sustain_db,
            rise_fast: coeff(1.0, sample_rate),
            rise_slow: coeff(20.0, sample_rate),
            fall_fast: coeff(20.0, sample_rate),
            fall_slow: coeff(500.0, sample_rate),
            att_fast: 0.0,
            att_slow: 0.0,
            sus_fast: 0.0,
            sus_slow: 0.0,
            held: 0.0,
            hold_left: 0,
            hold: (0.012 * sample_rate) as u32,
            let_go: coeff(5.0, sample_rate),
        }
    }

    #[inline]
    fn follow(env: &mut f32, x: f32, rise: f32, fall: f32) {
        let c = if x > *env { rise } else { fall };
        *env = x + (*env - x) * c;
    }

    /// The gain for a sample whose level (both channels) is `level`.
    #[inline]
    fn gain(&mut self, x: f32) -> f32 {
        if x >= self.held {
            self.held = x;
            self.hold_left = self.hold;
        } else if self.hold_left > 0 {
            self.hold_left -= 1;
        } else {
            self.held = x + (self.held - x) * self.let_go;
        }
        let level = self.held;
        Self::follow(&mut self.att_fast, level, self.rise_fast, self.fall_fast);
        Self::follow(&mut self.att_slow, level, self.rise_slow, self.fall_fast);
        Self::follow(&mut self.sus_fast, level, self.rise_fast, self.fall_fast);
        Self::follow(&mut self.sus_slow, level, self.rise_fast, self.fall_slow);
        let tiny = 1e-6;
        let attack = math::clamp((self.att_fast - self.att_slow) / (self.att_fast + tiny), 0.0, 1.0);
        let sustain = math::clamp((self.sus_slow - self.sus_fast) / (self.sus_slow + tiny), 0.0, 1.0);
        math::db_to_linear(self.attack_db * attack + self.sustain_db * sustain)
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        x * self.gain(math::abs(x))
    }

    #[inline]
    pub fn process_stereo(&mut self, l: f32, r: f32) -> (f32, f32) {
        let g = self.gain(math::abs(l).max(math::abs(r)));
        (l * g, r * g)
    }

    pub fn reset(&mut self) {
        self.att_fast = 0.0;
        self.att_slow = 0.0;
        self.sus_fast = 0.0;
        self.sus_slow = 0.0;
        self.held = 0.0;
        self.hold_left = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    /// A 60 Hz hit with a 150 ms decay, every half second.
    fn hits(n: usize) -> impl Iterator<Item = f32> {
        (0..n).map(|i| {
            let t = (i % 22050) as f32 / 44100.0;
            0.8 * math::exp(-t / 0.15) * math::sin(math::TWO_PI * 60.0 * t)
        })
    }

    /// Peak in `a..b` ms after each hit, over the last hits.
    fn peak(y: &[f32], a: f32, b: f32) -> f32 {
        let (a, b) = ((a * 44.1) as usize, (b * 44.1) as usize);
        y.chunks(22050).skip(2).map(|c| c[a..b].iter().fold(0.0f32, |m, v| m.max(v.abs()))).fold(0.0, f32::max)
    }

    #[test]
    fn attack_lifts_the_hit_and_sustain_the_tail() {
        let dry: Vec<f32> = hits(44100 * 3).collect();
        let shaped = |a: f32, s: f32| {
            let mut t = Transient::new(a, s, 44100.0);
            dry.iter().map(|&x| t.process(x)).collect::<Vec<f32>>()
        };
        let punch = shaped(6.0, 0.0);
        assert!(peak(&punch, 0.0, 10.0) > peak(&dry, 0.0, 10.0) * 1.3, "the hit");
        assert!(peak(&punch, 200.0, 300.0) < peak(&dry, 200.0, 300.0) * 1.1, "the tail stays");
        let short = shaped(0.0, -9.0);
        let (tail, dry_tail) = (peak(&short, 200.0, 300.0), peak(&dry, 200.0, 300.0));
        assert!(tail < dry_tail * 0.6, "the tail goes: {tail} against {dry_tail}");
        assert!(peak(&short, 0.0, 10.0) > peak(&dry, 0.0, 10.0) * 0.9, "the hit stays");
    }

    #[test]
    fn coefficients_are_exact() {
        // e^(-1/22050), to f32 precision: what 500 ms is at 44.1 kHz.
        assert!((coeff(500.0, 44100.0) - 0.999_954_66).abs() < 2e-8);
        assert!((coeff(1.0, 44100.0) - 0.977_579_4).abs() < 1e-7);
    }

    #[test]
    fn a_steady_tone_passes() {
        let mut t = Transient::new(12.0, -12.0, 44100.0);
        let mut out = 0.0f32;
        for i in 0..44100 {
            let x = 0.5 * math::sin(math::TWO_PI * 440.0 * i as f32 / 44100.0);
            let y = t.process(x);
            if i > 22050 {
                out = out.max(y.abs());
            }
        }
        assert!((out - 0.5).abs() < 0.05, "{out}");
    }
}
