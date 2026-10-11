//! Vowel (formant) filter: three parallel bandpasses at the formant
//! frequencies of a vowel, optionally morphing between two vowels with a
//! slow LFO. This is the "talking pad" sound.

use crate::math;
use crate::primitives::filter::{BiquadFilter, FilterType};
use crate::SAMPLE_RATE;

/// Formants F1..F3 in Hz for a, e, i, o, u.
pub const VOWELS: &[(&str, [f32; 3])] = &[
    ("a", [730.0, 1090.0, 2440.0]),
    ("e", [530.0, 1840.0, 2480.0]),
    ("i", [270.0, 2290.0, 3010.0]),
    ("o", [570.0, 840.0, 2410.0]),
    ("u", [300.0, 870.0, 2240.0]),
];
const GAINS: [f32; 3] = [1.0, 0.6, 0.35];

pub fn vowel_index(name: &str) -> Option<u8> {
    VOWELS.iter().position(|(n, _)| *n == name).map(|i| i as u8)
}

struct Bank {
    filters: [BiquadFilter; 3],
}

impl Bank {
    fn new() -> Self {
        let mut f = [BiquadFilter::new(SAMPLE_RATE), BiquadFilter::new(SAMPLE_RATE), BiquadFilter::new(SAMPLE_RATE)];
        for x in f.iter_mut() {
            x.set_params(FilterType::BandPass, 800.0, 0.9);
        }
        Self { filters: f }
    }
    fn tune(&mut self, freqs: [f32; 3]) {
        for (f, hz) in self.filters.iter_mut().zip(freqs) {
            f.set_cutoff(hz);
        }
    }
    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let mut out = 0.0;
        for (f, g) in self.filters.iter_mut().zip(GAINS) {
            out += f.process(x) * g;
        }
        out * 2.0
    }
    fn reset(&mut self) {
        for f in self.filters.iter_mut() {
            f.reset();
        }
    }
}

pub struct Formant {
    left: Bank,
    right: Bank,
    from: [f32; 3],
    to: [f32; 3],
    pub mix: f32,
    phase: f32,
    phase_inc: f32,
    bars: f32,
    tick: u32,
    /// `position`, set by a knob or the mod wheel: where along a-e-i-o-u
    /// the filter sits, 0..1, and where it has glided to so far. Set once,
    /// it takes over from the LFO for good.
    target: Option<f32>,
    pos: f32,
}

/// How fast the vowel follows a knob: about 25 ms to close most of the gap,
/// so a wheel thrown across its travel slides instead of stepping.
const GLIDE: f32 = 1.0 / (0.025 * SAMPLE_RATE);

/// The formants at `pos` along a-e-i-o-u, 0..1, each pair of neighbours
/// crossed geometrically, as the ear hears a formant move.
pub fn vowel_at(pos: f32) -> [f32; 3] {
    let x = math::clamp(pos, 0.0, 1.0) * (VOWELS.len() - 1) as f32;
    let i = (math::floor(x) as usize).min(VOWELS.len() - 2);
    let t = x - i as f32;
    let (a, b) = (VOWELS[i].1, VOWELS[i + 1].1);
    core::array::from_fn(|k| a[k] * math::pow(b[k] / a[k], t))
}

impl Formant {
    pub fn new(from: u8, to: u8, hz: f32, bars: f32, mix: f32) -> Self {
        let f = VOWELS[(from as usize).min(VOWELS.len() - 1)].1;
        let t = VOWELS[(to as usize).min(VOWELS.len() - 1)].1;
        let mut s = Self {
            left: Bank::new(),
            right: Bank::new(),
            from: f,
            to: t,
            mix: math::clamp(mix, 0.0, 1.0),
            phase: 0.0,
            phase_inc: 0.0,
            bars,
            tick: 0,
            target: None,
            pos: 0.0,
        };
        s.set_rate(if hz > 0.0 { hz } else { 0.25 });
        s.left.tune(f);
        s.right.tune(f);
        s
    }

    pub fn set_rate(&mut self, hz: f32) {
        self.phase_inc = hz / SAMPLE_RATE;
    }

    pub fn set_bpm(&mut self, bpm: f32) {
        if self.bars > 0.0 {
            self.set_rate(bpm / 60.0 / 4.0 / self.bars);
        }
    }

    fn morph(&mut self) {
        // Triangle 0..1 between the two vowels
        let t = if self.phase < 0.5 { self.phase * 2.0 } else { 2.0 - self.phase * 2.0 };
        let freqs: [f32; 3] = core::array::from_fn(|i| self.from[i] + (self.to[i] - self.from[i]) * t);
        self.left.tune(freqs);
        self.right.tune(freqs);
    }

    /// Put the filter at `pos` along a-e-i-o-u, gliding there. The first
    /// call starts the glide from where the LFO had it.
    pub fn set_position(&mut self, pos: f32) {
        if self.target.is_none() {
            self.pos = self.lfo_position();
        }
        self.target = Some(math::clamp(pos, 0.0, 1.0));
    }

    /// Where the LFO's morph sits on the a-e-i-o-u line, roughly: the
    /// nearer of its two vowels.
    fn lfo_position(&self) -> f32 {
        let t = if self.phase < 0.5 { self.phase * 2.0 } else { 2.0 - self.phase * 2.0 };
        let near = if t < 0.5 { self.from } else { self.to };
        let i = VOWELS.iter().position(|(_, f)| *f == near).unwrap_or(0);
        i as f32 / (VOWELS.len() - 1) as f32
    }

    pub fn process_stereo(&mut self, l: f32, r: f32) -> (f32, f32) {
        if let Some(target) = self.target {
            let d = target - self.pos;
            if d != 0.0 {
                self.pos = if math::abs(d) < 1e-4 { target } else { self.pos + d * GLIDE };
                if self.tick.is_multiple_of(16) || self.pos == target {
                    let f = vowel_at(self.pos);
                    self.left.tune(f);
                    self.right.tune(f);
                }
                self.tick = self.tick.wrapping_add(1);
            }
        } else if self.from != self.to {
            self.phase += self.phase_inc;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
            if self.tick.is_multiple_of(64) {
                self.morph();
            }
            self.tick = self.tick.wrapping_add(1);
        }
        let wl = self.left.process(l);
        let wr = self.right.process(r);
        (l * (1.0 - self.mix) + wl * self.mix, r * (1.0 - self.mix) + wr * self.mix)
    }

    pub fn process(&mut self, x: f32) -> f32 {
        self.process_stereo(x, x).0
    }

    pub fn reset(&mut self) {
        self.left.reset();
        self.right.reset();
        self.phase = 0.0;
        self.tick = 0;
        let f = match self.target {
            Some(t) => {
                self.pos = t;
                vowel_at(t)
            }
            None => self.from,
        };
        self.left.tune(f);
        self.right.tune(f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_line_passes_through_every_vowel() {
        for (i, (_, f)) in VOWELS.iter().enumerate() {
            let at = vowel_at(i as f32 / 4.0);
            for k in 0..3 {
                assert!((at[k] - f[k]).abs() < f[k] * 0.002, "vowel {i} formant {k}: {} vs {}", at[k], f[k]);
            }
        }
        let mid = vowel_at(0.125);
        assert!(mid[1] > 1090.0 && mid[1] < 1840.0, "between a and e: {mid:?}");
    }

    /// A wheel thrown from one end to the other: the output never jumps
    /// more than the input could explain, so nothing clicks.
    #[test]
    fn a_thrown_wheel_slides_without_a_click() {
        let mut f = Formant::new(0, 0, 0.25, 0.0, 1.0);
        let sine = |i: usize| (i as f32 * 220.0 / SAMPLE_RATE * core::f32::consts::TAU).sin() * 0.5;
        let mut last = 0.0f32;
        let mut worst = 0.0f32;
        for i in 0..44100 {
            if i == 10000 {
                f.set_position(1.0);
            }
            if i == 20000 {
                f.set_position(0.0);
            }
            let y = f.process(sine(i));
            if i > 2000 {
                worst = worst.max((y - last).abs());
            }
            last = y;
        }
        // A 220 Hz sine at 0.5 moves at most ~0.016 a sample; through the
        // bank's gain a smooth output stays within a few times that.
        assert!(worst < 0.2, "a step of {worst} in one sample");
    }
}
