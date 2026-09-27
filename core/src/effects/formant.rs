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

    pub fn process_stereo(&mut self, l: f32, r: f32) -> (f32, f32) {
        if self.from != self.to {
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
        self.left.tune(self.from);
        self.right.tune(self.from);
    }
}
