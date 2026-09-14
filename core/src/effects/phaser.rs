//! Stereo phaser: a chain of first-order allpass stages swept by a slow LFO,
//! with feedback. The right channel runs a quarter cycle behind the left so
//! the sweep also moves across the stereo field.

use crate::math;
use crate::SAMPLE_RATE;

const MAX_STAGES: usize = 12;
const F_MIN: f32 = 200.0;
const F_MAX: f32 = 2400.0;

#[derive(Clone, Copy, Default)]
struct Allpass { x1: f32, y1: f32 }

impl Allpass {
    #[inline]
    fn process(&mut self, x: f32, a: f32) -> f32 {
        let y = a * x + self.x1 - a * self.y1;
        self.x1 = x;
        self.y1 = y;
        y
    }
}

struct Channel {
    stages: [Allpass; MAX_STAGES],
    fb: f32,
}

pub struct Phaser {
    left: Channel,
    right: Channel,
    stages: usize,
    feedback: f32,
    depth: f32,
    pub mix: f32,
    phase: f32,
    phase_inc: f32,
    bars: f32,
    tick: u32,
    coeff_l: f32,
    coeff_r: f32,
}

impl Phaser {
    pub fn new(mix: f32, hz: f32, bars: f32, stages: usize, feedback: f32, depth: f32) -> Self {
        let mut p = Self {
            left: Channel { stages: [Allpass::default(); MAX_STAGES], fb: 0.0 },
            right: Channel { stages: [Allpass::default(); MAX_STAGES], fb: 0.0 },
            stages: stages.clamp(2, MAX_STAGES),
            feedback: math::clamp(feedback, 0.0, 0.9),
            depth: math::clamp(depth, 0.0, 1.0),
            mix: math::clamp(mix, 0.0, 1.0),
            phase: 0.0,
            phase_inc: 0.0,
            bars,
            tick: 0,
            coeff_l: 0.0,
            coeff_r: 0.0,
        };
        p.set_rate(if hz > 0.0 { hz } else { 0.3 });
        p
    }

    pub fn set_rate(&mut self, hz: f32) {
        self.phase_inc = hz / SAMPLE_RATE;
    }

    pub fn set_bpm(&mut self, bpm: f32) {
        if self.bars > 0.0 {
            self.set_rate(bpm / 60.0 / 4.0 / self.bars);
        }
    }

    /// Allpass coefficient for a centre frequency.
    fn coeff(freq: f32) -> f32 {
        let w = math::PI * freq / SAMPLE_RATE;
        let t = math::sin(w) / math::cos(w).max(1e-4);
        (1.0 - t) / (1.0 + t)
    }

    fn sweep_freq(lfo: f32, depth: f32) -> f32 {
        // lfo in -1..1 → exponential sweep between F_MIN and F_MAX, scaled by depth
        let t = (lfo * depth + 1.0) * 0.5;
        F_MIN * math::pow(F_MAX / F_MIN, t)
    }

    fn update_coeffs(&mut self) {
        let l = math::sin(self.phase * math::TWO_PI);
        let r = math::sin((self.phase + 0.25) * math::TWO_PI);
        self.coeff_l = Self::coeff(Self::sweep_freq(l, self.depth));
        self.coeff_r = Self::coeff(Self::sweep_freq(r, self.depth));
    }

    #[inline]
    fn run(ch: &mut Channel, x: f32, a: f32, stages: usize, feedback: f32) -> f32 {
        let mut v = x + ch.fb * feedback;
        for s in 0..stages {
            v = ch.stages[s].process(v, a);
        }
        ch.fb = v;
        v
    }

    pub fn process_stereo(&mut self, l: f32, r: f32) -> (f32, f32) {
        self.phase += self.phase_inc;
        if self.phase >= 1.0 { self.phase -= 1.0; }
        if self.tick % 16 == 0 { self.update_coeffs(); }
        self.tick = self.tick.wrapping_add(1);
        let wl = Self::run(&mut self.left, l, self.coeff_l, self.stages, self.feedback);
        let wr = Self::run(&mut self.right, r, self.coeff_r, self.stages, self.feedback);
        (l * (1.0 - self.mix) + wl * self.mix, r * (1.0 - self.mix) + wr * self.mix)
    }

    pub fn process(&mut self, x: f32) -> f32 {
        self.process_stereo(x, x).0
    }

    pub fn reset(&mut self) {
        self.left = Channel { stages: [Allpass::default(); MAX_STAGES], fb: 0.0 };
        self.right = Channel { stages: [Allpass::default(); MAX_STAGES], fb: 0.0 };
        self.phase = 0.0;
        self.tick = 0;
    }
}
