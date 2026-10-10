use crate::math;

/// ln(1000): an exponential covers 60 dB in this many time constants.
const STAGE: f32 = 6.907_755;

/// Under this a stage's time is its time constant, as it always was.
const FLOOR_S: f32 = 0.010;

/// From this up a stage is 60 dB down at the time written.
const KNEE_S: f32 = 0.100;

/// Between the two the time constant rises as a power of the time, from
/// 10 ms at the floor to 100 ms / ln(1000) at the knee:
/// ln(KNEE_S / (STAGE * FLOOR_S)) / ln(KNEE_S / FLOOR_S).
const BEND: f32 = 0.160_663;

/// The time constant of a stage `time` seconds long.
///
/// A stage takes the time it is given: the curve is exponential, and the
/// time is how long it takes to come within 60 dB of where it is going,
/// which is where each stage ends (attack at 0.999, release under 0.001), so
/// `release 1s` is a second of tail. That holds from 100 ms up.
///
/// At the short end the time is the time constant, as it always was: a
/// stage under 10 ms keeps the corner it had. Taken as 60 dB all the way
/// down, the 20-40 ms releases of hitech_psy's FM chirps clicked 32 times
/// in 32 bars, against once before; a 3 ms floor still left twenty. So from
/// 10 to 100 ms the time constant goes from 10 ms to 14.5 ms, rising with
/// the time on a log scale: no stage over 10 ms gets one under 10, every
/// time still lasts longer than the one under it, and the stages run from
/// 69 ms at 10 written to 100 ms at 100. Pinning it at 10 ms instead made
/// every time from 10 to 69 ms the same 69 ms; a smooth blend such as
/// sqrt((time / STAGE)^2 + 10 ms^2) keeps them apart but runs 21% long at
/// 100 ms and 5% at 200. With this the clicks `render` hears in 24 bars of
/// each example are what they were, give or take three on one pad that a
/// sidechain ducks.
///
/// Until `math::exp` was fixed every time constant came out capped at about
/// 30 ms, so no stage lasted much more than 200 ms whatever the song said.
fn stage_tau(time: f32) -> f32 {
    if time <= FLOOR_S {
        time
    } else if time >= KNEE_S {
        time / STAGE
    } else {
        FLOOR_S * math::pow(time / FLOOR_S, BEND)
    }
}

/// The per-sample coefficient for a stage `time` seconds long.
fn stage_coeff(time: f32, sample_rate: f32) -> f32 {
    math::exp(-1.0 / (stage_tau(time) * sample_rate))
}

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

    pub fn set_attack(&mut self, a: f32) {
        self.attack = math::clamp(a, 0.001, 10.0);
        self.recalc_rates();
    }
    pub fn set_decay(&mut self, d: f32) {
        self.decay = math::clamp(d, 0.001, 10.0);
        self.recalc_rates();
    }
    pub fn set_sustain(&mut self, s: f32) {
        self.sustain = math::clamp(s, 0.0, 1.0);
    }
    pub fn set_release(&mut self, r: f32) {
        self.release = math::clamp(r, 0.001, 10.0);
        self.recalc_rates();
    }

    fn recalc_rates(&mut self) {
        self.attack_coeff = stage_coeff(self.attack, self.sample_rate);
        self.decay_coeff = stage_coeff(self.decay, self.sample_rate);
        self.release_coeff = stage_coeff(self.release, self.sample_rate);
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
                self.level *= self.release_coeff;
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

#[cfg(test)]
mod stage_tests {
    use super::*;

    /// Samples from gate-off until the release falls under `level`.
    fn release_samples(release: f32, level: f32) -> usize {
        let mut env = Envelope::new(44100.0);
        env.set_adsr(0.001, 0.001, 1.0, release);
        env.gate_on();
        for _ in 0..4410 {
            env.next_sample();
        }
        env.gate_off();
        (1..441_000).find(|_| env.next_sample() < level).unwrap_or(usize::MAX)
    }

    #[test]
    fn a_long_stage_lasts_the_time_it_is_given() {
        // 60 dB down at the time written, within a percent.
        for release in [0.1f32, 0.5, 1.0, 2.0] {
            let n = release_samples(release, 0.001) as f32 / 44100.0;
            assert!((n / release - 1.0).abs() < 0.01, "release {release} s ended after {n} s");
        }
    }

    #[test]
    fn a_short_stage_keeps_the_corner_it_had() {
        // Under the floor the time is the time constant: 1/e at 1 ms.
        let n = release_samples(0.001, 0.367_879) as f32 / 44.1;
        assert!((n - 1.0).abs() < 0.05, "1 ms release reached 1/e after {n} ms");
        // Between the floor and the knee, no time constant under 10 ms, and
        // the longer the time the longer the constant: 30 ms gets 11.9.
        let n = release_samples(0.03, 0.367_879) as f32 / 44.1;
        assert!((n - 11.93).abs() < 0.3, "30 ms release reached 1/e after {n} ms");
        for ms in 11..100 {
            let tau = stage_tau(ms as f32 / 1000.0);
            assert!(tau >= FLOOR_S, "{ms} ms got a {} ms time constant", tau * 1000.0);
        }
    }

    #[test]
    fn every_time_lasts_longer_than_the_one_under_it() {
        // Millisecond by millisecond, the time constant grows from 1 ms to
        // 2 s, and the stage it plays gets longer up to a second. Past that a
        // millisecond is under what an f32 coefficient this close to 1 can
        // tell apart.
        let mut last_tau = 0.0;
        let mut last_len = 0;
        for ms in 1..=2000 {
            let time = ms as f32 / 1000.0;
            let tau = stage_tau(time);
            assert!(tau > last_tau, "{ms} ms got a time constant of {tau}, {last_tau} for the one under it");
            last_tau = tau;
            if ms <= 1000 {
                let n = release_samples(time, 0.001);
                assert!(n > last_len, "release {ms} ms ended after {n} samples, {last_len} for the one under it");
                last_len = n;
            }
        }
    }
}
