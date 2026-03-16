use crate::math;

#[derive(Clone, Copy, PartialEq)]
pub enum EnvStage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

pub struct Envelope {
    attack: f32,  // seconds
    decay: f32,   // seconds
    sustain: f32, // level 0.0-1.0
    release: f32, // seconds
    stage: EnvStage,
    level: f32,
    sample_rate: f32,
    attack_coeff: f32,
    decay_coeff: f32,
    release_coeff: f32,
}

impl Envelope {
    pub fn new(sample_rate: f32) -> Self {
        let mut env = Self {
            attack: 0.01,
            decay: 0.1,
            sustain: 0.7,
            release: 0.3,
            stage: EnvStage::Idle,
            level: 0.0,
            sample_rate,
            attack_coeff: 0.0,
            decay_coeff: 0.0,
            release_coeff: 0.0,
        };
        env.recalc_rates();
        env
    }

    pub fn set_adsr(&mut self, a: f32, d: f32, s: f32, r: f32) {
        self.attack = math::clamp(a, 0.001, 10.0);
        self.decay = math::clamp(d, 0.001, 10.0);
        self.sustain = math::clamp(s, 0.0, 1.0);
        self.release = math::clamp(r, 0.001, 10.0);
        self.recalc_rates();
    }

    fn recalc_rates(&mut self) {
        // Exponential envelope coefficients
        // coeff = exp(-1 / (time * sample_rate))
        // This gives a smooth exponential curve that reaches ~63% per time constant
        self.attack_coeff = math::exp(-1.0 / (self.attack * self.sample_rate));
        self.decay_coeff = math::exp(-1.0 / (self.decay * self.sample_rate));
        self.release_coeff = math::exp(-1.0 / (self.release * self.sample_rate));
    }

    pub fn gate_on(&mut self) {
        self.stage = EnvStage::Attack;
    }

    pub fn gate_off(&mut self) {
        if self.stage != EnvStage::Idle {
            self.stage = EnvStage::Release;
        }
    }

    pub fn next_sample(&mut self) -> f32 {
        match self.stage {
            EnvStage::Idle => {
                self.level = 0.0;
            }
            EnvStage::Attack => {
                // Exponential approach to 1.0
                self.level = 1.0 - (1.0 - self.level) * self.attack_coeff;
                if self.level >= 0.999 {
                    self.level = 1.0;
                    self.stage = EnvStage::Decay;
                }
            }
            EnvStage::Decay => {
                // Exponential approach to sustain level
                self.level = self.sustain + (self.level - self.sustain) * self.decay_coeff;
                if self.level <= self.sustain + 0.0001 {
                    self.level = self.sustain;
                    self.stage = EnvStage::Sustain;
                }
            }
            EnvStage::Sustain => {
                self.level = self.sustain;
            }
            EnvStage::Release => {
                // Exponential decay to zero
                self.level = self.level * self.release_coeff;
                if self.level < 0.001 {
                    self.level = 0.0;
                    self.stage = EnvStage::Idle;
                }
            }
        }
        self.level
    }

    pub fn is_idle(&self) -> bool {
        self.stage == EnvStage::Idle
    }

    pub fn stage(&self) -> EnvStage {
        self.stage
    }

    pub fn reset(&mut self) {
        self.stage = EnvStage::Idle;
        self.level = 0.0;
    }
}
