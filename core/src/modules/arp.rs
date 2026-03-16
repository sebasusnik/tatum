use crate::math;
use crate::primitives::oscillator::{Oscillator, Waveform};
use crate::primitives::envelope::Envelope;
use crate::primitives::filter::{BiquadFilter, FilterType};
use crate::harmony::HarmonyContext;
use crate::{Module, SAMPLE_RATE};

pub enum ArpParam {
    Rate,
    Gate,
    Pattern,
    Level,
}

#[derive(Clone, Copy, PartialEq)]
pub enum ArpPattern {
    Up,
    Down,
    UpDown,
}

pub struct ArpModule {
    pub harmony: Option<HarmonyContext>,
    osc: Oscillator,
    env: Envelope,
    filter: BiquadFilter,
    // Arp state
    pattern: ArpPattern,
    current_step: usize,
    direction: i8, // 1 = up, -1 = down (for UpDown)
    arp_notes: [u8; 8],
    num_notes: usize,
    // Timing
    samples_per_step: f32,
    sample_counter: f32,
    gate_length: f32, // 0-1, fraction of step that note is on
    // State
    level: f32,
    octave_range: u8, // number of octaves to span
}

impl ArpModule {
    pub fn new() -> Self {
        let mut osc = Oscillator::new(Waveform::Saw, SAMPLE_RATE);
        osc.set_drift_seed(7777);

        let mut env = Envelope::new(SAMPLE_RATE);
        env.set_adsr(0.005, 0.1, 0.3, 0.08);

        let mut filter = BiquadFilter::new(SAMPLE_RATE);
        filter.set_params(FilterType::LowPass, 5000.0, 0.3);

        // Default 120 BPM, 16th notes
        let samples_per_step = SAMPLE_RATE * 60.0 / 120.0 / 4.0;

        Self {
            harmony: Some(HarmonyContext::new(57, crate::harmony::Scale::Minor)),
            osc,
            env,
            filter,
            pattern: ArpPattern::Up,
            current_step: 0,
            direction: 1,
            arp_notes: [0; 8],
            num_notes: 0,
            samples_per_step,
            sample_counter: 0.0,
            gate_length: 0.7,
            level: 0.5,
            octave_range: 2,
        }
    }

    /// Rebuild arp note list from harmony context.
    pub fn rebuild_notes(&mut self) {
        if let Some(ref harmony) = self.harmony {
            let chord = harmony.chord_notes();
            let mut idx = 0;
            for octave in 0..self.octave_range {
                for note_opt in &chord {
                    if let Some(note) = note_opt {
                        if idx < 8 {
                            self.arp_notes[idx] = note + octave * 12;
                            idx += 1;
                        }
                    }
                }
            }
            self.num_notes = idx;
        }
    }

    pub fn set_param(&mut self, param: ArpParam, value: f32) {
        match param {
            ArpParam::Rate => self.set_bpm(60.0 + value * 180.0),
            ArpParam::Gate => self.gate_length = math::clamp(value, 0.1, 0.95),
            ArpParam::Pattern => {
                self.pattern = match (value * 2.0) as u8 {
                    0 => ArpPattern::Up,
                    1 => ArpPattern::Down,
                    _ => ArpPattern::UpDown,
                };
            }
            ArpParam::Level => self.level = value,
        }
    }

    pub fn set_bpm(&mut self, bpm: f32) {
        // 16th notes
        self.samples_per_step = SAMPLE_RATE * 60.0 / bpm / 4.0;
    }

    fn advance_step(&mut self) {
        if self.num_notes == 0 {
            return;
        }

        match self.pattern {
            ArpPattern::Up => {
                self.current_step = (self.current_step + 1) % self.num_notes;
            }
            ArpPattern::Down => {
                if self.current_step == 0 {
                    self.current_step = self.num_notes - 1;
                } else {
                    self.current_step -= 1;
                }
            }
            ArpPattern::UpDown => {
                let next = self.current_step as i8 + self.direction;
                if next >= self.num_notes as i8 {
                    self.direction = -1;
                    self.current_step = if self.num_notes > 1 {
                        self.num_notes - 2
                    } else {
                        0
                    };
                } else if next < 0 {
                    self.direction = 1;
                    self.current_step = if self.num_notes > 1 { 1 } else { 0 };
                } else {
                    self.current_step = next as usize;
                }
            }
        }
    }
}

impl Module for ArpModule {
    fn process_block(&mut self, output: &mut [f32]) {
        if self.num_notes == 0 {
            for s in output.iter_mut() {
                *s = 0.0;
            }
            return;
        }

        for sample in output.iter_mut() {
            self.sample_counter += 1.0;

            if self.sample_counter >= self.samples_per_step {
                self.sample_counter = 0.0;
                self.advance_step();
                // Trigger note
                let note = self.arp_notes[self.current_step % self.num_notes];
                let freq = math::midi_to_freq(note);
                self.osc.set_frequency(freq);
                self.env.gate_on();
            }

            // Gate off at gate_length fraction of step
            let gate_off_point = self.samples_per_step * self.gate_length;
            if self.sample_counter >= gate_off_point && self.sample_counter < gate_off_point + 1.0 {
                self.env.gate_off();
            }

            let env = self.env.next_sample();
            let raw = self.osc.next_sample();
            let filtered = self.filter.process(raw);
            *sample = filtered * env * self.level;
        }
    }

    fn note_on(&mut self, _note: u8, _velocity: f32) {
        // Arp uses harmony context, not individual notes
        self.rebuild_notes();
        self.current_step = 0;
        self.sample_counter = self.samples_per_step; // trigger immediately
    }

    fn note_off(&mut self, _note: u8) {
        // Arp runs continuously; could stop here if desired
    }

    fn reset(&mut self) {
        self.osc.reset();
        self.env.reset();
        self.filter.reset();
        self.current_step = 0;
        self.sample_counter = 0.0;
        self.direction = 1;
    }
}
