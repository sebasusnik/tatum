use crate::math;
use crate::rng::Rng;

#[derive(Clone, Copy, PartialEq)]
pub enum LfoWaveform {
    Sine,
    Triangle,
    Saw,
    Square,
    SampleHold,
}

#[derive(Clone, Copy, PartialEq)]
pub enum LfoSyncMode {
    FreeHz,
    Quarter,
    Eighth,
    Sixteenth,
    DottedEighth,
    TripletEighth,
    /// One cycle per bar (4/4), and multi-bar cycles for slow movement.
    Bar,
    Bars2,
    Bars4,
    Bars8,
    Bars12,
    Bars16,
}

#[derive(Clone, Copy, PartialEq)]
pub enum LfoTarget {
    Cutoff,
    Pitch,
    Amplitude,
    ModIndex,
    DelayTime,
}

pub struct Lfo {
    phase: f32,
    rate: f32,   // Hz (free-running rate)
    depth: f32,  // 0.0-1.0
    waveform: LfoWaveform,
    sample_rate: f32,
    phase_inc: f32,
    sync_mode: LfoSyncMode,
    bpm: f32,
    sh_value: f32,
    sh_rng: Rng,
}

impl Lfo {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            phase: 0.0,
            rate: 1.0,
            depth: 0.5,
            waveform: LfoWaveform::Sine,
            sample_rate,
            phase_inc: 1.0 / sample_rate,
            sync_mode: LfoSyncMode::FreeHz,
            bpm: 120.0,
            sh_value: 0.0,
            sh_rng: Rng::new(42),
        }
    }

    pub fn new_with_seed(sample_rate: f32, seed: u32) -> Self {
        Self {
            phase: 0.0,
            rate: 1.0,
            depth: 0.5,
            waveform: LfoWaveform::Sine,
            sample_rate,
            phase_inc: 1.0 / sample_rate,
            sync_mode: LfoSyncMode::FreeHz,
            bpm: 120.0,
            sh_value: 0.0,
            sh_rng: Rng::new(seed),
        }
    }

    pub fn set_rate(&mut self, rate: f32) {
        self.rate = rate;
        self.recalc_phase_inc();
    }

    pub fn set_depth(&mut self, depth: f32) {
        self.depth = math::clamp(depth, 0.0, 1.0);
    }

    pub fn set_waveform(&mut self, waveform: LfoWaveform) {
        self.waveform = waveform;
    }

    pub fn set_sync_mode(&mut self, mode: LfoSyncMode) {
        self.sync_mode = mode;
        self.recalc_phase_inc();
    }

    pub fn set_bpm(&mut self, bpm: f32) {
        self.bpm = bpm;
        self.recalc_phase_inc();
    }

    fn recalc_phase_inc(&mut self) {
        let freq = match self.sync_mode {
            LfoSyncMode::FreeHz => self.rate,
            LfoSyncMode::Quarter => self.bpm / 60.0,
            LfoSyncMode::Eighth => self.bpm / 60.0 * 2.0,
            LfoSyncMode::Sixteenth => self.bpm / 60.0 * 4.0,
            LfoSyncMode::DottedEighth => self.bpm / 60.0 * 4.0 / 3.0,
            LfoSyncMode::TripletEighth => self.bpm / 60.0 * 3.0,
            LfoSyncMode::Bar => self.bpm / 60.0 / 4.0,
            LfoSyncMode::Bars2 => self.bpm / 60.0 / 8.0,
            LfoSyncMode::Bars4 => self.bpm / 60.0 / 16.0,
            LfoSyncMode::Bars8 => self.bpm / 60.0 / 32.0,
            LfoSyncMode::Bars12 => self.bpm / 60.0 / 48.0,
            LfoSyncMode::Bars16 => self.bpm / 60.0 / 64.0,
        };
        self.phase_inc = freq / self.sample_rate;
    }

    /// Returns value in [-depth, +depth].
    pub fn next_sample(&mut self) -> f32 {
        let out = match self.waveform {
            LfoWaveform::Sine => math::sin(self.phase * math::TWO_PI),
            LfoWaveform::Triangle => {
                let p = self.phase;
                if p < 0.25 {
                    p * 4.0
                } else if p < 0.75 {
                    2.0 - p * 4.0
                } else {
                    p * 4.0 - 4.0
                }
            }
            LfoWaveform::Saw => self.phase * 2.0 - 1.0,
            LfoWaveform::Square => {
                if self.phase < 0.5 { 1.0 } else { -1.0 }
            }
            LfoWaveform::SampleHold => self.sh_value,
        };

        self.phase += self.phase_inc;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
            // Update S&H value on phase wrap
            if self.waveform == LfoWaveform::SampleHold {
                self.sh_value = self.sh_rng.next_bipolar();
            }
        }

        out * self.depth
    }

    pub fn reset(&mut self) {
        self.phase = 0.0;
        self.sh_value = 0.0;
    }
}

/// Routes an LFO to a modulatable parameter.
pub struct ModulationRouter {
    pub lfo: Lfo,
    pub target: LfoTarget,
    pub enabled: bool,
}

impl ModulationRouter {
    pub fn new(sample_rate: f32, seed: u32) -> Self {
        let mut lfo = Lfo::new_with_seed(sample_rate, seed);
        lfo.set_rate(2.0);
        lfo.set_depth(0.0);
        Self {
            lfo,
            target: LfoTarget::Cutoff,
            enabled: false,
        }
    }

    /// Returns LFO value, or 0.0 if disabled.
    pub fn next_sample(&mut self) -> f32 {
        if !self.enabled {
            return 0.0;
        }
        self.lfo.next_sample()
    }

    pub fn set_target(&mut self, target: LfoTarget) {
        self.target = target;
    }

    pub fn set_bpm(&mut self, bpm: f32) {
        self.lfo.set_bpm(bpm);
    }

    pub fn reset(&mut self) {
        self.lfo.reset();
    }
}
