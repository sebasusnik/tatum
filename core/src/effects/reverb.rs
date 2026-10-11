extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;
use crate::math;

// ─── Freeverb Implementation (Stereo) ───
// Pre-delay + Early reflections + 8 parallel modulated comb filters + 4 series allpass filters per channel

const NUM_COMBS: usize = 8;
const NUM_ALLPASSES: usize = 4;

// Comb filter delay lengths (in samples at 44100 Hz, Freeverb standard)
const COMB_LENGTHS: [usize; NUM_COMBS] = [
    crate::at_rate(1116),
    crate::at_rate(1188),
    crate::at_rate(1277),
    crate::at_rate(1356),
    crate::at_rate(1422),
    crate::at_rate(1491),
    crate::at_rate(1557),
    crate::at_rate(1617),
];

// Allpass filter delay lengths
const ALLPASS_LENGTHS: [usize; NUM_ALLPASSES] =
    [crate::at_rate(556), crate::at_rate(441), crate::at_rate(341), crate::at_rate(225)];

// Stereo spread: R channel delays offset by 23 samples (Freeverb standard)
const STEREO_SPREAD: usize = crate::at_rate(23);

// ─── Pre-Delay ───

const PRE_DELAY_MAX_SAMPLES: usize = crate::at_rate(4411); // 100ms at 44100Hz

struct PreDelay {
    buffer: [f32; PRE_DELAY_MAX_SAMPLES],
    pos: usize,
    delay_samples: usize,
}

impl PreDelay {
    fn new() -> Self {
        Self { buffer: [0.0; PRE_DELAY_MAX_SAMPLES], pos: 0, delay_samples: 0 }
    }

    fn set_delay_ms(&mut self, ms: f32) {
        let samples = (ms * (crate::SAMPLE_RATE / 1000.0)) as usize;
        self.delay_samples = if samples >= PRE_DELAY_MAX_SAMPLES { PRE_DELAY_MAX_SAMPLES - 1 } else { samples };
    }

    fn process(&mut self, input: f32) -> f32 {
        if self.delay_samples == 0 {
            return input;
        }

        self.buffer[self.pos] = input;

        let read_pos = if self.pos >= self.delay_samples {
            self.pos - self.delay_samples
        } else {
            PRE_DELAY_MAX_SAMPLES - (self.delay_samples - self.pos)
        };
        let output = self.buffer[read_pos];

        self.pos += 1;
        if self.pos >= PRE_DELAY_MAX_SAMPLES {
            self.pos = 0;
        }

        output
    }

    fn reset(&mut self) {
        self.buffer = [0.0; PRE_DELAY_MAX_SAMPLES];
        self.pos = 0;
    }
}

// ─── Early Reflections ───

const ER_BUFFER_SIZE: usize = crate::at_rate(6500);
const ER_NUM_TAPS: usize = 7;

// Tap delays in samples (prime numbers, roughly 50ms-139ms at 44100Hz)
const ER_DELAYS: [usize; ER_NUM_TAPS] = [
    crate::at_rate(2207),
    crate::at_rate(2953),
    crate::at_rate(3571),
    crate::at_rate(4201),
    crate::at_rate(4831),
    crate::at_rate(5501),
    crate::at_rate(6133),
];

// Decaying gains for each tap (normalized so sum ≈ 1.0)
const ER_GAINS: [f32; ER_NUM_TAPS] = [0.228, 0.195, 0.163, 0.137, 0.114, 0.091, 0.072];

struct EarlyReflections {
    buffer: [f32; ER_BUFFER_SIZE],
    pos: usize,
}

impl EarlyReflections {
    fn new() -> Self {
        Self { buffer: [0.0; ER_BUFFER_SIZE], pos: 0 }
    }

    fn process(&mut self, input: f32) -> f32 {
        self.buffer[self.pos] = input;

        let mut out = 0.0;
        for i in 0..ER_NUM_TAPS {
            let read_pos = if self.pos >= ER_DELAYS[i] {
                self.pos - ER_DELAYS[i]
            } else {
                ER_BUFFER_SIZE - (ER_DELAYS[i] - self.pos)
            };
            out += self.buffer[read_pos] * ER_GAINS[i];
        }

        self.pos += 1;
        if self.pos >= ER_BUFFER_SIZE {
            self.pos = 0;
        }

        out
    }

    fn reset(&mut self) {
        self.buffer = [0.0; ER_BUFFER_SIZE];
        self.pos = 0;
    }
}

// ─── Modulated Comb Filter ───

const COMB_MOD_DEPTH: f32 = 3.0;
const COMB_MOD_HEADROOM: usize = 8;
const COMB_LFO_RATES: [f32; 8] = [0.50, 0.64, 0.79, 0.93, 1.07, 1.21, 1.36, 1.50];

struct ModCombFilter {
    buffer: Vec<f32>,
    pos: usize,
    nominal_len: usize,
    feedback: f32,
    damp1: f32,
    damp2: f32,
    filterstore: f32,
    lfo_phase: f32,
    lfo_phase_inc: f32,
}

impl ModCombFilter {
    fn new(size: usize, lfo_rate: f32, initial_phase: f32) -> Self {
        let buf_len = size + COMB_MOD_HEADROOM;
        Self {
            buffer: vec![0.0; buf_len],
            pos: 0,
            nominal_len: size,
            feedback: 0.5,
            damp1: 0.5,
            damp2: 0.5,
            filterstore: 0.0,
            lfo_phase: initial_phase,
            lfo_phase_inc: lfo_rate / crate::SAMPLE_RATE,
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        // LFO modulation of read position
        let mod_offset = math::sin(self.lfo_phase * math::TWO_PI) * COMB_MOD_DEPTH;
        let delay_f = self.nominal_len as f32 + mod_offset;
        let delay_int = delay_f as usize;
        let frac = delay_f - delay_int as f32;

        let buf_len = self.buffer.len();

        // Read with linear interpolation
        let read_pos_0 = if self.pos >= delay_int { self.pos - delay_int } else { buf_len - (delay_int - self.pos) };
        let read_pos_1 = if read_pos_0 == 0 { buf_len - 1 } else { read_pos_0 - 1 };

        let output = math::lerp(self.buffer[read_pos_0], self.buffer[read_pos_1], frac);

        // Damping filter + write
        self.filterstore = output * self.damp2 + self.filterstore * self.damp1;
        self.buffer[self.pos] = input + self.filterstore * self.feedback;

        self.pos += 1;
        if self.pos >= buf_len {
            self.pos = 0;
        }

        // Advance LFO
        self.lfo_phase += self.lfo_phase_inc;
        if self.lfo_phase >= 1.0 {
            self.lfo_phase -= 1.0;
        }

        output
    }

    fn set_feedback(&mut self, feedback: f32) {
        self.feedback = feedback;
    }

    fn set_damp(&mut self, damp: f32) {
        self.damp1 = damp;
        self.damp2 = 1.0 - damp;
    }

    fn reset(&mut self) {
        for s in self.buffer.iter_mut() {
            *s = 0.0;
        }
        self.filterstore = 0.0;
        self.pos = 0;
    }
}

// ─── Allpass Filter ───

struct AllpassFilter {
    buffer: Vec<f32>,
    pos: usize,
}

impl AllpassFilter {
    fn new(size: usize) -> Self {
        Self { buffer: vec![0.0; size], pos: 0 }
    }

    fn process(&mut self, input: f32) -> f32 {
        let buffered = self.buffer[self.pos];
        let output = buffered - input;
        self.buffer[self.pos] = input + buffered * 0.5;
        self.pos += 1;
        if self.pos >= self.buffer.len() {
            self.pos = 0;
        }
        output
    }

    fn reset(&mut self) {
        for s in self.buffer.iter_mut() {
            *s = 0.0;
        }
        self.pos = 0;
    }
}

// ─── Reverb (Improved Freeverb) ───

pub struct Reverb {
    pre_delay: PreDelay,
    early: EarlyReflections,
    combs_l: [ModCombFilter; NUM_COMBS],
    combs_r: [ModCombFilter; NUM_COMBS],
    allpasses_l: [AllpassFilter; NUM_ALLPASSES],
    allpasses_r: [AllpassFilter; NUM_ALLPASSES],
    room_size: f32,
    damping: f32,
    mix: f32,
    /// Freeverb-style freeze: the tank recirculates forever and ignores new input.
    frozen: bool,
}

impl Reverb {
    pub fn new(_sample_rate: f32) -> Self {
        let combs_l = core::array::from_fn(|i| ModCombFilter::new(COMB_LENGTHS[i], COMB_LFO_RATES[i], i as f32 / 8.0));
        let combs_r = core::array::from_fn(|i| {
            ModCombFilter::new(COMB_LENGTHS[i] + STEREO_SPREAD, COMB_LFO_RATES[i], (i as f32 / 8.0 + 0.5) % 1.0)
        });
        let allpasses_l = core::array::from_fn(|i| AllpassFilter::new(ALLPASS_LENGTHS[i]));
        let allpasses_r = core::array::from_fn(|i| AllpassFilter::new(ALLPASS_LENGTHS[i] + STEREO_SPREAD));

        let mut rev = Self {
            pre_delay: PreDelay::new(),
            early: EarlyReflections::new(),
            combs_l,
            combs_r,
            allpasses_l,
            allpasses_r,
            room_size: 0.6,
            damping: 0.5,
            mix: 0.2,
            frozen: false,
        };
        rev.update_params();
        rev
    }

    pub fn set_room_size(&mut self, size: f32) {
        self.room_size = math::clamp(size, 0.0, 1.0);
        self.update_params();
    }

    pub fn set_damping(&mut self, damp: f32) {
        self.damping = math::clamp(damp, 0.0, 1.0);
        self.update_params();
    }

    pub fn set_mix(&mut self, mix: f32) {
        self.mix = math::clamp(mix, 0.0, 1.0);
    }

    pub fn set_pre_delay(&mut self, ms: f32) {
        self.pre_delay.set_delay_ms(ms);
    }

    /// Hold the current tail indefinitely (no decay, no new input).
    pub fn set_freeze(&mut self, frozen: bool) {
        if self.frozen != frozen {
            self.frozen = frozen;
            self.update_params();
        }
    }

    pub fn is_frozen(&self) -> bool {
        self.frozen
    }

    fn update_params(&mut self) {
        // size 0..1 → feedback 0.7..0.985. At 0.985 the tail is ~15 s; beyond
        // that the combs ring and pile up into a roar under sustained input.
        let (feedback, damp) =
            if self.frozen { (1.0, 0.0) } else { (self.room_size * 0.285 + 0.7, self.damping * 0.4 + 0.1) };
        for comb in &mut self.combs_l {
            comb.set_feedback(feedback);
            comb.set_damp(damp);
        }
        for comb in &mut self.combs_r {
            comb.set_feedback(feedback);
            comb.set_damp(damp);
        }
    }

    /// Process stereo: returns (left, right).
    pub fn process_stereo(&mut self, input: f32) -> (f32, f32) {
        let pd = self.pre_delay.process(input);

        // Early reflections (mono)
        let er = self.early.process(pd);

        // Parallel comb filters per channel
        let mut out_l = 0.0;
        let mut out_r = 0.0;
        for comb in &mut self.combs_l {
            out_l += comb.process(er);
        }
        for comb in &mut self.combs_r {
            out_r += comb.process(er);
        }
        out_l /= NUM_COMBS as f32;
        out_r /= NUM_COMBS as f32;

        // Series allpass filters per channel
        for ap in &mut self.allpasses_l {
            out_l = ap.process(out_l);
        }
        for ap in &mut self.allpasses_r {
            out_r = ap.process(out_r);
        }

        let dry = 1.0 - self.mix;
        let wet = self.mix;
        (input * dry + out_l * wet, input * dry + out_r * wet)
    }

    /// Core stereo processing: advances state and returns raw reverb wet signal.
    fn process_stereo_in_core(&mut self, input_l: f32, input_r: f32) -> (f32, f32) {
        let mono = if self.frozen { 0.0 } else { (input_l + input_r) * 0.5 };
        let pd = self.pre_delay.process(mono);

        // Early reflections (mono)
        let er = self.early.process(pd);

        // Parallel comb filters per channel
        let mut out_l = 0.0;
        let mut out_r = 0.0;
        for comb in &mut self.combs_l {
            out_l += comb.process(er);
        }
        for comb in &mut self.combs_r {
            out_r += comb.process(er);
        }
        out_l /= NUM_COMBS as f32;
        out_r /= NUM_COMBS as f32;

        // Series allpass filters per channel
        for ap in &mut self.allpasses_l {
            out_l = ap.process(out_l);
        }
        for ap in &mut self.allpasses_r {
            out_r = ap.process(out_r);
        }

        (out_l, out_r)
    }

    /// Process stereo input: mono-sums into the reverb tank, preserves dry stereo.
    pub fn process_stereo_in(&mut self, input_l: f32, input_r: f32) -> (f32, f32) {
        let (out_l, out_r) = self.process_stereo_in_core(input_l, input_r);
        let dry = 1.0 - self.mix;
        let wet = self.mix;
        (input_l * dry + out_l * wet, input_r * dry + out_r * wet)
    }

    /// Wet-only stereo reverb for send/return routing.
    /// Returns only the reverb signal without dry blend.
    pub fn process_stereo_in_wet(&mut self, input_l: f32, input_r: f32) -> (f32, f32) {
        self.process_stereo_in_core(input_l, input_r)
    }

    /// Backward-compatible mono process: delegates to stereo and sums.
    pub fn process(&mut self, input: f32) -> f32 {
        let (l, r) = self.process_stereo(input);
        (l + r) * 0.5
    }

    pub fn reset(&mut self) {
        self.pre_delay.reset();
        self.early.reset();
        for comb in &mut self.combs_l {
            comb.reset();
        }
        for comb in &mut self.combs_r {
            comb.reset();
        }
        for ap in &mut self.allpasses_l {
            ap.reset();
        }
        for ap in &mut self.allpasses_r {
            ap.reset();
        }
    }
}

// ─── Dattorro Plate Reverb ───
// Based on Dattorro (1997) "Effect Design Part 1: Reverberator and Other Filters"
// Delay lengths scaled from 29761Hz → 44100Hz (×1.4817)

// ─── Helper: FractionalDelay ───

struct FractionalDelay {
    buffer: Vec<f32>,
    pos: usize,
}

impl FractionalDelay {
    fn new(len: usize) -> Self {
        Self { buffer: vec![0.0; len], pos: 0 }
    }

    fn write(&mut self, input: f32) {
        self.buffer[self.pos] = input;
        self.pos += 1;
        if self.pos >= self.buffer.len() {
            self.pos = 0;
        }
    }

    fn read(&self, delay: f32) -> f32 {
        let len = self.buffer.len();
        let delay_int = delay as usize;
        let frac = delay - delay_int as f32;

        let read_pos_0 = if self.pos > delay_int { self.pos - delay_int - 1 } else { len - 1 - (delay_int - self.pos) };
        let read_pos_1 = if read_pos_0 == 0 { len - 1 } else { read_pos_0 - 1 };

        math::lerp(self.buffer[read_pos_0], self.buffer[read_pos_1], frac)
    }

    fn read_at(&self, index: usize) -> f32 {
        let len = self.buffer.len();
        let read_pos = if self.pos > index { self.pos - index - 1 } else { len - 1 - (index - self.pos) };
        self.buffer[read_pos]
    }

    fn reset(&mut self) {
        for s in self.buffer.iter_mut() {
            *s = 0.0;
        }
        self.pos = 0;
    }
}

// ─── Helper: StaticAllpass ───

struct StaticAllpass {
    buffer: Vec<f32>,
    pos: usize,
    gain: f32,
}

impl StaticAllpass {
    fn new(size: usize, gain: f32) -> Self {
        Self { buffer: vec![0.0; size], pos: 0, gain }
    }

    fn process(&mut self, input: f32) -> f32 {
        let buffered = self.buffer[self.pos];
        // Schroeder allpass: v = x + g*v_d ; y = -g*v + v_d = v_d*(1 - g²) - g*x
        let output = buffered * (1.0 - self.gain * self.gain) - input * self.gain;
        self.buffer[self.pos] = input + buffered * self.gain;
        self.pos += 1;
        if self.pos >= self.buffer.len() {
            self.pos = 0;
        }
        output
    }

    fn reset(&mut self) {
        for s in self.buffer.iter_mut() {
            *s = 0.0;
        }
        self.pos = 0;
    }
}

// ─── Helper: ModAllpass ───

const MOD_AP_HEADROOM: usize = 16;

struct ModAllpass {
    buffer: Vec<f32>,
    pos: usize,
    nominal_len: usize,
    gain: f32,
    lfo_phase: f32,
    lfo_phase_inc: f32,
    mod_depth: f32,
}

impl ModAllpass {
    fn new(nominal_len: usize, lfo_rate: f32, mod_depth: f32) -> Self {
        let buf_len = nominal_len + MOD_AP_HEADROOM;
        Self {
            buffer: vec![0.0; buf_len],
            pos: 0,
            nominal_len,
            gain: 0.7,
            lfo_phase: 0.0,
            lfo_phase_inc: lfo_rate / crate::SAMPLE_RATE,
            mod_depth,
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        let mod_offset = math::sin(self.lfo_phase * math::TWO_PI) * self.mod_depth;
        let delay_f = self.nominal_len as f32 + mod_offset;
        let delay_int = delay_f as usize;
        let frac = delay_f - delay_int as f32;

        let buf_len = self.buffer.len();

        let read_pos_0 = if self.pos >= delay_int { self.pos - delay_int } else { buf_len - (delay_int - self.pos) };
        let read_pos_1 = if read_pos_0 == 0 { buf_len - 1 } else { read_pos_0 - 1 };

        let buffered = math::lerp(self.buffer[read_pos_0], self.buffer[read_pos_1], frac);
        let output = buffered * (1.0 - self.gain * self.gain) - input * self.gain;
        self.buffer[self.pos] = input + buffered * self.gain;

        self.pos += 1;
        if self.pos >= buf_len {
            self.pos = 0;
        }

        self.lfo_phase += self.lfo_phase_inc;
        if self.lfo_phase >= 1.0 {
            self.lfo_phase -= 1.0;
        }

        output
    }

    fn reset(&mut self) {
        for s in self.buffer.iter_mut() {
            *s = 0.0;
        }
        self.pos = 0;
    }
}

// ─── Dattorro Plate Reverb ───

// Input diffusion allpass lengths (scaled from 29761Hz to 44100Hz)
const DATTORRO_INPUT_AP: [usize; 4] =
    [crate::at_rate(311), crate::at_rate(234), crate::at_rate(831), crate::at_rate(607)];

// Tank delay line lengths
const DATTORRO_DELAY_L: [usize; 2] = [crate::at_rate(3163), crate::at_rate(3289)];
const DATTORRO_DELAY_R: [usize; 2] = [crate::at_rate(3187), crate::at_rate(3533)];

// Modulated allpass lengths
const DATTORRO_MOD_AP_L: usize = crate::at_rate(1800);
const DATTORRO_MOD_AP_R: usize = crate::at_rate(2053);

// Static tank allpass lengths
const DATTORRO_STATIC_AP_L: usize = crate::at_rate(2389);
const DATTORRO_STATIC_AP_R: usize = crate::at_rate(1973);

pub struct DattorroReverb {
    pre_delay: PreDelay,
    // Input diffusion
    diff: [StaticAllpass; 4],
    // Tank L
    delay_l: [FractionalDelay; 2],
    mod_ap_l: ModAllpass,
    lp_l: f32, // one-pole LP state
    static_ap_l: StaticAllpass,
    // Tank R
    delay_r: [FractionalDelay; 2],
    mod_ap_r: ModAllpass,
    lp_r: f32,
    static_ap_r: StaticAllpass,
    // Cross-feedback
    feedback_l: f32,
    feedback_r: f32,
    // Parameters
    decay: f32,
    damping: f32,
    mix: f32,
}

impl DattorroReverb {
    pub fn new(_sample_rate: f32) -> Self {
        Self {
            pre_delay: PreDelay::new(),
            diff: [
                StaticAllpass::new(DATTORRO_INPUT_AP[0], 0.75),
                StaticAllpass::new(DATTORRO_INPUT_AP[1], 0.75),
                StaticAllpass::new(DATTORRO_INPUT_AP[2], 0.625),
                StaticAllpass::new(DATTORRO_INPUT_AP[3], 0.625),
            ],
            delay_l: [FractionalDelay::new(DATTORRO_DELAY_L[0]), FractionalDelay::new(DATTORRO_DELAY_L[1])],
            mod_ap_l: ModAllpass::new(DATTORRO_MOD_AP_L, 0.1, 8.0),
            lp_l: 0.0,
            static_ap_l: StaticAllpass::new(DATTORRO_STATIC_AP_L, 0.5),
            delay_r: [FractionalDelay::new(DATTORRO_DELAY_R[0]), FractionalDelay::new(DATTORRO_DELAY_R[1])],
            mod_ap_r: ModAllpass::new(DATTORRO_MOD_AP_R, 0.143, 8.0),
            lp_r: 0.0,
            static_ap_r: StaticAllpass::new(DATTORRO_STATIC_AP_R, 0.5),
            feedback_l: 0.0,
            feedback_r: 0.0,
            decay: 0.7,
            damping: 0.5,
            mix: 0.3,
        }
    }

    pub fn set_room_size(&mut self, size: f32) {
        self.decay = math::clamp(size, 0.0, 1.0) * 0.5 + 0.45; // 0.45..0.95
    }

    pub fn set_damping(&mut self, damp: f32) {
        self.damping = math::clamp(damp, 0.0, 1.0);
    }

    pub fn set_mix(&mut self, mix: f32) {
        self.mix = math::clamp(mix, 0.0, 1.0);
    }

    pub fn set_pre_delay(&mut self, ms: f32) {
        self.pre_delay.set_delay_ms(ms);
    }

    pub fn process_stereo_in(&mut self, input_l: f32, input_r: f32) -> (f32, f32) {
        let mono = (input_l + input_r) * 0.5;
        let pd = self.pre_delay.process(mono);

        // Input diffusion
        let mut x = pd;
        for ap in &mut self.diff {
            x = ap.process(x);
        }

        let damp_coeff = self.damping * 0.4 + 0.1;

        // Tank L: feedback from R → delay1 → mod_allpass → LP → delay2 → static_allpass
        let tank_in_l = x + self.feedback_r * self.decay;
        self.delay_l[0].write(tank_in_l);
        let d1_l = self.delay_l[0].read((DATTORRO_DELAY_L[0] - 1) as f32);
        let ma_l = self.mod_ap_l.process(d1_l);
        self.lp_l = ma_l * (1.0 - damp_coeff) + self.lp_l * damp_coeff;
        let lp_out_l = self.lp_l * self.decay;
        self.delay_l[1].write(lp_out_l);
        let d2_l = self.delay_l[1].read((DATTORRO_DELAY_L[1] - 1) as f32);
        let tank_out_l = self.static_ap_l.process(d2_l);
        self.feedback_l = tank_out_l;

        // Tank R: feedback from L → delay1 → mod_allpass → LP → delay2 → static_allpass
        let tank_in_r = x + self.feedback_l * self.decay;
        self.delay_r[0].write(tank_in_r);
        let d1_r = self.delay_r[0].read((DATTORRO_DELAY_R[0] - 1) as f32);
        let ma_r = self.mod_ap_r.process(d1_r);
        self.lp_r = ma_r * (1.0 - damp_coeff) + self.lp_r * damp_coeff;
        let lp_out_r = self.lp_r * self.decay;
        self.delay_r[1].write(lp_out_r);
        let d2_r = self.delay_r[1].read((DATTORRO_DELAY_R[1] - 1) as f32);
        let tank_out_r = self.static_ap_r.process(d2_r);
        self.feedback_r = tank_out_r;

        // Output: cross-channel taps for stereo width
        // L output taps from R tank, R output taps from L tank
        let tap_l = self.delay_r[0].read_at(const { crate::at_rate(266) })
            + self.delay_r[0].read_at(const { crate::at_rate(1913) })
            - self.delay_r[1].read_at(const { crate::at_rate(1066) })
            + self.delay_l[1].read_at(const { crate::at_rate(353) });
        let tap_r = self.delay_l[0].read_at(const { crate::at_rate(266) })
            + self.delay_l[0].read_at(const { crate::at_rate(1913) })
            - self.delay_l[1].read_at(const { crate::at_rate(1066) })
            + self.delay_r[1].read_at(const { crate::at_rate(353) });

        let wet_l = tap_l * 0.3;
        let wet_r = tap_r * 0.3;

        let dry = 1.0 - self.mix;
        let wet = self.mix;
        (input_l * dry + wet_l * wet, input_r * dry + wet_r * wet)
    }

    pub fn process_stereo(&mut self, input: f32) -> (f32, f32) {
        self.process_stereo_in(input, input)
    }

    pub fn process(&mut self, input: f32) -> f32 {
        let (l, r) = self.process_stereo(input);
        (l + r) * 0.5
    }

    pub fn reset(&mut self) {
        self.pre_delay.reset();
        for ap in &mut self.diff {
            ap.reset();
        }
        for d in &mut self.delay_l {
            d.reset();
        }
        for d in &mut self.delay_r {
            d.reset();
        }
        self.mod_ap_l.reset();
        self.mod_ap_r.reset();
        self.static_ap_l.reset();
        self.static_ap_r.reset();
        self.lp_l = 0.0;
        self.lp_r = 0.0;
        self.feedback_l = 0.0;
        self.feedback_r = 0.0;
    }
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pre_delay_bypass() {
        let mut pd = PreDelay::new();
        // 0ms delay → returns input immediately
        pd.set_delay_ms(0.0);
        assert!((pd.process(1.0) - 1.0).abs() < 0.0001);
        assert!((pd.process(0.5) - 0.5).abs() < 0.0001);
        assert!((pd.process(0.0) - 0.0).abs() < 0.0001);
    }

    #[test]
    fn test_pre_delay_impulse() {
        let mut pd = PreDelay::new();
        pd.set_delay_ms(20.0); // ~882 samples

        let delay_samples = (20.0 * crate::SAMPLE_RATE / 1000.0) as usize; // 882 at 44.1 kHz

        // Feed impulse then silence
        let _ = pd.process(1.0);
        for _ in 1..delay_samples {
            let out = pd.process(0.0);
            assert!(out.abs() < 0.0001, "Should be silent before delay");
        }
        // At the delay point, impulse should appear
        let out = pd.process(0.0);
        assert!((out - 1.0).abs() < 0.0001, "Impulse should appear after {} samples, got {}", delay_samples, out);
    }

    #[test]
    fn test_mod_comb_non_silent() {
        let mut comb = ModCombFilter::new(1116, 0.5, 0.0);
        comb.set_feedback(0.8);
        comb.set_damp(0.3);

        // Feed impulse
        let _ = comb.process(1.0);

        // Process many samples and check that output is non-zero eventually
        let mut found_non_zero = false;
        for _ in 0..4000 {
            let out = comb.process(0.0);
            if out.abs() > 0.001 {
                found_non_zero = true;
                break;
            }
        }
        assert!(found_non_zero, "ModCombFilter should produce non-zero output after impulse");
    }
}
