extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;
use crate::math;

#[derive(Clone, Copy, PartialEq)]
pub enum DelaySync {
    Free,           // manual time in seconds
    Quarter,        // 1/4 note
    DottedEighth,   // 3/16 note
    Eighth,         // 1/8 note
    Sixteenth,      // 1/16 note
    TripletEighth,  // 1/8 triplet
}

impl DelaySync {
    /// Convert a normalized 0.0..1.0 value to a sync mode (for param locks).
    pub fn from_normalized(v: f32) -> Self {
        if v < 0.17 { DelaySync::Free }
        else if v < 0.33 { DelaySync::Quarter }
        else if v < 0.50 { DelaySync::DottedEighth }
        else if v < 0.67 { DelaySync::Eighth }
        else if v < 0.83 { DelaySync::Sixteenth }
        else { DelaySync::TripletEighth }
    }

    fn beat_multiplier(self) -> Option<f32> {
        match self {
            DelaySync::Free => None,
            DelaySync::Quarter => Some(1.0),
            DelaySync::DottedEighth => Some(0.75),
            DelaySync::Eighth => Some(0.5),
            DelaySync::Sixteenth => Some(0.25),
            DelaySync::TripletEighth => Some(1.0 / 3.0),
        }
    }
}

pub struct Delay {
    buffer_l: Vec<f32>,
    buffer_r: Vec<f32>,
    write_pos: usize,
    delay_samples: usize,
    feedback: f32,
    mix: f32,
    // Feedback filter (one-pole lowpass)
    fb_filter_l: f32,
    fb_filter_r: f32,
    fb_filter_coeff: f32, // 0.0 = no filtering, up to 0.95 = very dark
    // Tempo sync
    sync: DelaySync,
    bpm: f32,
}

impl Delay {
    pub fn new(sample_rate: f32, max_time: f32) -> Self {
        let max_samples = (sample_rate * max_time) as usize + 1;
        Self {
            buffer_l: vec![0.0; max_samples],
            buffer_r: vec![0.0; max_samples],
            write_pos: 0,
            delay_samples: (sample_rate * 0.3) as usize, // default 300ms
            feedback: 0.35,
            mix: 0.2,
            fb_filter_l: 0.0,
            fb_filter_r: 0.0,
            fb_filter_coeff: 0.0,
            sync: DelaySync::Free,
            bpm: 120.0,
        }
    }

    pub fn set_time(&mut self, time: f32, sample_rate: f32) {
        let samples = (sample_rate * time) as usize;
        self.delay_samples = if samples >= self.buffer_l.len() {
            self.buffer_l.len() - 1
        } else {
            samples
        };
    }

    pub fn set_feedback(&mut self, feedback: f32) {
        self.feedback = math::clamp(feedback, 0.0, 0.95);
    }

    pub fn set_mix(&mut self, mix: f32) {
        self.mix = math::clamp(mix, 0.0, 1.0);
    }

    pub fn set_filter(&mut self, amount: f32) {
        self.fb_filter_coeff = math::clamp(amount, 0.0, 0.95);
    }

    pub fn set_sync(&mut self, sync: DelaySync, bpm: f32, sample_rate: f32) {
        self.sync = sync;
        self.bpm = bpm;
        if let Some(mult) = sync.beat_multiplier() {
            let time = 60.0 / bpm * mult;
            self.set_time(time, sample_rate);
        }
    }

    pub fn set_bpm(&mut self, bpm: f32, sample_rate: f32) {
        self.bpm = bpm;
        if let Some(mult) = self.sync.beat_multiplier() {
            let time = 60.0 / bpm * mult;
            self.set_time(time, sample_rate);
        }
    }

    /// Stereo ping-pong delay: R feeds back into L, L feeds back into R.
    /// Feedback path includes a one-pole lowpass for analog-style darkening.
    pub fn process_stereo(&mut self, input_l: f32, input_r: f32) -> (f32, f32) {
        let buf_len = self.buffer_l.len();
        let read_pos = if self.write_pos >= self.delay_samples {
            self.write_pos - self.delay_samples
        } else {
            buf_len - (self.delay_samples - self.write_pos)
        };

        let delayed_l = self.buffer_l[read_pos % buf_len];
        let delayed_r = self.buffer_r[read_pos % buf_len];

        // Cross-feedback with one-pole lowpass filter (same pattern as CombFilter in reverb)
        let fb_l = delayed_r * self.feedback;
        let fb_r = delayed_l * self.feedback;
        self.fb_filter_l = fb_l + self.fb_filter_coeff * (self.fb_filter_l - fb_l);
        self.fb_filter_r = fb_r + self.fb_filter_coeff * (self.fb_filter_r - fb_r);

        self.buffer_l[self.write_pos % buf_len] = input_l + self.fb_filter_l;
        self.buffer_r[self.write_pos % buf_len] = input_r + self.fb_filter_r;

        self.write_pos += 1;
        if self.write_pos >= buf_len {
            self.write_pos = 0;
        }

        let dry = 1.0 - self.mix;
        let wet = self.mix;
        (input_l * dry + delayed_l * wet, input_r * dry + delayed_r * wet)
    }

    /// Backward-compatible mono process: delegates to stereo and sums.
    pub fn process(&mut self, input: f32) -> f32 {
        let (l, r) = self.process_stereo(input, input);
        (l + r) * 0.5
    }

    pub fn reset(&mut self) {
        for s in self.buffer_l.iter_mut() {
            *s = 0.0;
        }
        for s in self.buffer_r.iter_mut() {
            *s = 0.0;
        }
        self.write_pos = 0;
        self.fb_filter_l = 0.0;
        self.fb_filter_r = 0.0;
    }
}
