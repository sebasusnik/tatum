use crate::math;
use crate::primitives::oscillator::{Oscillator, Waveform};
use crate::primitives::envelope::{Envelope, EnvStage};
use crate::primitives::filter::{BiquadFilter, FilterType};
use crate::primitives::lfo::{LfoWaveform, LfoSyncMode, LfoTarget, ModulationRouter};
use crate::effects::chorus::Chorus;
use crate::harmony::HarmonyContext;
use crate::{Module, MAX_VOICES, SAMPLE_RATE};

#[derive(Clone, Copy, PartialEq)]
pub enum VoiceMode {
    Poly,
    Unison,
    Octave,
    Fifth,
    RingMod,
}

pub struct KeysVoice {
    osc1: Oscillator,
    osc2: Oscillator,
    pub env: Envelope,
    filter: BiquadFilter,
    pub note: u8,
    pub root_note: u8,
    pub active: bool,
    detune: f32,
    age: u32,
    pan: f32,
    freq_mult: f32,
    pub ring_mod: bool,
}

impl KeysVoice {
    fn new(seed_offset: u32) -> Self {
        let mut osc1 = Oscillator::new(Waveform::Saw, SAMPLE_RATE);
        osc1.set_drift_seed(100 + seed_offset);
        let mut osc2 = Oscillator::new(Waveform::Saw, SAMPLE_RATE);
        osc2.set_drift_seed(200 + seed_offset);

        let mut env = Envelope::new(SAMPLE_RATE);
        env.set_adsr(0.3, 0.5, 0.7, 1.0);

        let mut filter = BiquadFilter::new(SAMPLE_RATE);
        filter.set_params(FilterType::LowPass, 3000.0, 0.2);

        Self {
            osc1,
            osc2,
            env,
            filter,
            note: 0,
            root_note: 0,
            active: false,
            detune: 0.003, // ~5 cents
            age: 0,
            pan: 0.0,
            freq_mult: 1.0,
            ring_mod: false,
        }
    }

    fn note_on(&mut self, note: u8, velocity: f32, root_note: u8, freq_mult: f32, pan: f32, ring_mod: bool) {
        let freq = math::midi_to_freq(note) * freq_mult;
        self.note = note;
        self.root_note = root_note;
        self.active = true;
        self.freq_mult = freq_mult;
        self.pan = pan;
        self.ring_mod = ring_mod;
        self.osc1.set_frequency(freq * (1.0 - self.detune));
        self.osc2.set_frequency(freq * (1.0 + self.detune));
        self.osc1.reset();
        self.osc2.reset();
        self.env.gate_on();
        let _ = velocity;
    }

    fn note_off(&mut self) {
        self.env.gate_off();
    }

    fn next_sample(&mut self) -> f32 {
        if !self.active {
            return 0.0;
        }

        let env = self.env.next_sample();
        if self.env.is_idle() {
            self.active = false;
            return 0.0;
        }

        let mix = if self.ring_mod {
            self.osc1.next_sample() * self.osc2.next_sample()
        } else {
            (self.osc1.next_sample() + self.osc2.next_sample()) * 0.5
        };
        let filtered = self.filter.process(mix);
        filtered * env
    }

    fn next_sample_stereo(&mut self) -> (f32, f32) {
        let mono = self.next_sample();
        let r = (self.pan + 1.0) * 0.5;
        (mono * math::sqrt(1.0 - r), mono * math::sqrt(r))
    }

    fn reset(&mut self) {
        self.osc1.reset();
        self.osc2.reset();
        self.env.reset();
        self.filter.reset();
        self.active = false;
        self.root_note = 0;
        self.pan = 0.0;
        self.freq_mult = 1.0;
        self.ring_mod = false;
    }
}

pub enum KeysParam {
    Cutoff,
    Detune,
    ChorusMix,
    Level,
    LfoRate,
    LfoDepth,
    LfoWaveform,
    LfoTarget,
    LfoSync,
    VoiceMode,
    VibratoRate,
    VibratoDepth,
    Attack,
    Decay,
    Sustain,
    Release,
    Resonance,
}

pub struct KeysModule {
    pub harmony: Option<HarmonyContext>,
    pub voices: [KeysVoice; MAX_VOICES],
    voice_counter: u32,
    chorus: Chorus,
    level: f32,
    // LFO
    lfo_router: ModulationRouter,
    cutoff_base: f32,
    resonance: f32,
    voice_mode: VoiceMode,
    // Pitch bend (set by engine)
    pub pitch_bend_ratio: f32,
    // Vibrato
    vibrato_phase: f32,
    vibrato_rate: f32,
    vibrato_depth: f32,
    vibrato_delay: f32,
    vibrato_onset: f32,
}

impl KeysModule {
    pub fn new() -> Self {
        Self {
            harmony: Some(HarmonyContext::new(57, crate::harmony::Scale::Minor)),
            voices: core::array::from_fn(|i| KeysVoice::new(i as u32)),
            voice_counter: 0,
            chorus: Chorus::new(),
            level: 0.5,
            lfo_router: ModulationRouter::new(SAMPLE_RATE, 5002),
            cutoff_base: 3000.0,
            resonance: 0.2,
            voice_mode: VoiceMode::Poly,
            pitch_bend_ratio: 1.0,
            vibrato_phase: 0.0,
            vibrato_rate: 5.0,
            vibrato_depth: 0.0,
            vibrato_delay: 0.3,
            vibrato_onset: 0.0,
        }
    }

    /// React to a harmony change: release all active voices, then trigger the chord.
    pub fn on_harmony_change(&mut self) {
        if let Some(ref harmony) = self.harmony {
            // Release all active voices
            for voice in &mut self.voices {
                if voice.active {
                    voice.note_off();
                }
            }
            // Trigger chord notes
            let chord = harmony.chord_notes();
            for note_opt in &chord {
                if let Some(note) = note_opt {
                    self.note_on(*note, 0.7);
                }
            }
        }
    }

    /// Find `count` voices to use as a group. Prefers idle voices; falls back to stealing oldest.
    fn find_voice_group(&self, count: usize) -> [usize; MAX_VOICES] {
        let mut result = [0usize; MAX_VOICES];
        let mut found = 0;

        // 1. Prefer idle voices
        for (i, v) in self.voices.iter().enumerate() {
            if found >= count {
                break;
            }
            if !v.active {
                result[found] = i;
                found += 1;
            }
        }

        if found >= count {
            return result;
        }

        // 2. Fill remaining by stealing oldest voices (by age)
        let mut age_sorted: [(usize, u32); MAX_VOICES] = [(0, u32::MAX); MAX_VOICES];
        for (i, v) in self.voices.iter().enumerate() {
            age_sorted[i] = (i, v.age);
        }
        // Simple selection sort by age ascending
        for i in 0..MAX_VOICES {
            let mut min_idx = i;
            for j in (i + 1)..MAX_VOICES {
                if age_sorted[j].1 < age_sorted[min_idx].1 {
                    min_idx = j;
                }
            }
            age_sorted.swap(i, min_idx);
        }

        // Pick from oldest, skipping any already selected
        for &(idx, _) in age_sorted.iter() {
            if found >= count {
                break;
            }
            let mut already_picked = false;
            for k in 0..found {
                if result[k] == idx {
                    already_picked = true;
                    break;
                }
            }
            if !already_picked {
                result[found] = idx;
                found += 1;
            }
        }

        result
    }

    pub fn set_param(&mut self, param: KeysParam, value: f32) {
        match param {
            KeysParam::Cutoff => {
                // Exponential: 200Hz at 0.0, ~2kHz at 0.5, ~20kHz at 1.0
                let freq = 200.0 * crate::math::pow(100.0, value);
                self.cutoff_base = freq;
                for voice in &mut self.voices {
                    voice.filter.set_params(FilterType::LowPass, freq, self.resonance);
                }
            }
            KeysParam::Detune => {
                for voice in &mut self.voices {
                    voice.detune = value * 0.02;
                }
            }
            KeysParam::ChorusMix => self.chorus.mix = value,
            KeysParam::Level => self.level = value,
            KeysParam::LfoRate => {
                self.lfo_router.lfo.set_rate(0.1 + value * 19.9);
            }
            KeysParam::LfoDepth => {
                self.lfo_router.lfo.set_depth(value);
                self.lfo_router.enabled = value > 0.001;
            }
            KeysParam::LfoWaveform => {
                let idx = (value * 4.0) as u8;
                let wf = match idx {
                    0 => LfoWaveform::Sine,
                    1 => LfoWaveform::Triangle,
                    2 => LfoWaveform::Saw,
                    3 => LfoWaveform::Square,
                    _ => LfoWaveform::SampleHold,
                };
                self.lfo_router.lfo.set_waveform(wf);
            }
            KeysParam::LfoTarget => {
                let idx = (value * 2.0) as u8;
                let target = match idx {
                    0 => LfoTarget::Cutoff,
                    1 => LfoTarget::Pitch,
                    _ => LfoTarget::Amplitude,
                };
                self.lfo_router.set_target(target);
            }
            KeysParam::LfoSync => {
                let idx = (value * 5.0) as u8;
                let mode = match idx {
                    0 => LfoSyncMode::FreeHz,
                    1 => LfoSyncMode::Quarter,
                    2 => LfoSyncMode::Eighth,
                    3 => LfoSyncMode::Sixteenth,
                    4 => LfoSyncMode::DottedEighth,
                    _ => LfoSyncMode::TripletEighth,
                };
                self.lfo_router.lfo.set_sync_mode(mode);
            }
            KeysParam::VoiceMode => {
                let idx = (value * 4.0) as u8;
                self.voice_mode = match idx {
                    0 => VoiceMode::Poly,
                    1 => VoiceMode::Unison,
                    2 => VoiceMode::Octave,
                    3 => VoiceMode::Fifth,
                    _ => VoiceMode::RingMod,
                };
            }
            KeysParam::VibratoRate => self.vibrato_rate = 0.5 + value * 9.5,
            KeysParam::VibratoDepth => self.vibrato_depth = value * 0.5,
            KeysParam::Attack => {
                let a = 0.001 * crate::math::pow(2000.0, value);
                for voice in &mut self.voices { voice.env.set_attack(a); }
            }
            KeysParam::Decay => {
                let d = 0.001 * crate::math::pow(2000.0, value);
                for voice in &mut self.voices { voice.env.set_decay(d); }
            }
            KeysParam::Sustain => {
                for voice in &mut self.voices { voice.env.set_sustain(value); }
            }
            KeysParam::Release => {
                let r = 0.001 * crate::math::pow(2000.0, value);
                for voice in &mut self.voices { voice.env.set_release(r); }
            }
            KeysParam::Resonance => {
                self.resonance = value;
                for voice in &mut self.voices {
                    voice.filter.set_params(FilterType::LowPass, self.cutoff_base, self.resonance);
                }
            }
        }
    }

    pub fn set_bpm(&mut self, bpm: f32) {
        self.lfo_router.set_bpm(bpm);
    }

    fn find_voice(&self) -> usize {
        // 1. Prefer idle voice
        for (i, v) in self.voices.iter().enumerate() {
            if !v.active {
                return i;
            }
        }
        // 2. Prefer voice in release stage (oldest first)
        let mut oldest_release: Option<(usize, u32)> = None;
        for (i, v) in self.voices.iter().enumerate() {
            if v.env.stage() == EnvStage::Release {
                match oldest_release {
                    None => oldest_release = Some((i, v.age)),
                    Some((_, age)) if v.age < age => oldest_release = Some((i, v.age)),
                    _ => {}
                }
            }
        }
        if let Some((i, _)) = oldest_release {
            return i;
        }
        // 3. Steal the oldest active voice
        let mut oldest_idx = 0;
        let mut oldest_age = u32::MAX;
        for (i, v) in self.voices.iter().enumerate() {
            if v.age < oldest_age {
                oldest_age = v.age;
                oldest_idx = i;
            }
        }
        oldest_idx
    }
}

impl KeysModule {
    /// Process a stereo block with internal per-voice panning.
    pub fn process_block_stereo(&mut self, output_l: &mut [f32], output_r: &mut [f32]) {
        let mut sample_counter = 0u32;
        let is_unison = self.voice_mode == VoiceMode::Unison;

        for i in 0..output_l.len() {
            let lfo_val = self.lfo_router.next_sample();
            let mut amp_mod = 1.0;

            // Cutoff modulation: apply every 4 samples to reduce overhead
            if self.lfo_router.target == LfoTarget::Cutoff && self.lfo_router.enabled {
                if sample_counter % 4 == 0 {
                    let mod_cutoff = self.cutoff_base + lfo_val * 5000.0;
                    for voice in &mut self.voices {
                        voice.filter.set_cutoff(mod_cutoff);
                    }
                }
            }

            // Pitch LFO modulation
            let mut pitch_mult = 1.0;
            if self.lfo_router.target == LfoTarget::Pitch && self.lfo_router.enabled {
                pitch_mult = math::pow2(lfo_val);
            }

            // Vibrato with delayed onset
            let delay_samples = self.vibrato_delay * SAMPLE_RATE;
            if self.vibrato_onset < delay_samples {
                self.vibrato_onset += 1.0;
            }
            let onset_amt = if delay_samples > 0.0 {
                (self.vibrato_onset / delay_samples).min(1.0)
            } else {
                1.0
            };
            let vibrato_mult = if self.vibrato_depth > 0.0 {
                let vib = math::sin(self.vibrato_phase * math::TWO_PI)
                    * self.vibrato_depth * onset_amt;
                self.vibrato_phase += self.vibrato_rate / SAMPLE_RATE;
                if self.vibrato_phase >= 1.0 { self.vibrato_phase -= 1.0; }
                math::pow2(vib / 12.0)
            } else {
                1.0
            };

            // Apply pitch to all active voices (always — ensures pitch_bend + vibrato apply)
            let combined_pitch = pitch_mult * vibrato_mult * self.pitch_bend_ratio;
            for voice in &mut self.voices {
                if voice.active {
                    let freq = math::midi_to_freq(voice.note) * voice.freq_mult;
                    voice.osc1.set_frequency(freq * (1.0 - voice.detune) * combined_pitch);
                    voice.osc2.set_frequency(freq * (1.0 + voice.detune) * combined_pitch);
                }
            }

            // Amplitude modulation
            if self.lfo_router.target == LfoTarget::Amplitude && self.lfo_router.enabled {
                amp_mod = 1.0 + lfo_val;
            }

            let mut sum_l = 0.0f32;
            let mut sum_r = 0.0f32;
            let mut active_count = 0u32;
            for voice in &mut self.voices {
                let (vl, vr) = voice.next_sample_stereo();
                sum_l += vl;
                sum_r += vr;
                if voice.active {
                    active_count += 1;
                }
            }

            if is_unison {
                // Skip chorus for Unison — 8 detuned voices already create thickness
                output_l[i] = sum_l * self.level * amp_mod * 0.35;
                output_r[i] = sum_r * self.level * amp_mod * 0.35;
            } else {
                // Equal-power scaling: 1/sqrt(N) prevents clipping with multiple voices
                let voice_scaler = if active_count > 1 {
                    1.0 / math::sqrt(active_count as f32)
                } else {
                    1.0
                };
                // Non-Unison: apply chorus to mono sum, output L=R (centered)
                let mono = (sum_l + sum_r) * voice_scaler;
                let chorused = self.chorus.process(mono * self.level * amp_mod);
                output_l[i] = chorused;
                output_r[i] = chorused;
            }

            sample_counter += 1;
        }
    }
}

impl Module for KeysModule {
    fn process_block(&mut self, output: &mut [f32]) {
        // Delegate to stereo and downmix
        let len = output.len();
        let mut buf_l = [0.0f32; 128];
        let mut buf_r = [0.0f32; 128];
        self.process_block_stereo(&mut buf_l[..len], &mut buf_r[..len]);
        for i in 0..len {
            output[i] = (buf_l[i] + buf_r[i]) * 0.5;
        }
    }

    fn note_on(&mut self, note: u8, velocity: f32) {
        self.vibrato_onset = 0.0;
        match self.voice_mode {
            VoiceMode::Poly => {
                let idx = self.find_voice();
                self.voice_counter += 1;
                self.voices[idx].age = self.voice_counter;
                self.voices[idx].note_on(note, velocity, note, 1.0, 0.0, false);
            }
            VoiceMode::Unison => {
                // Release all active voices first
                for voice in &mut self.voices {
                    if voice.active {
                        voice.note_off();
                    }
                }
                // Trigger all 8 voices with spread
                let cents: [f32; 8] = [-12.0, -8.0, -5.0, -2.0, 2.0, 5.0, 8.0, 12.0];
                for i in 0..MAX_VOICES {
                    self.voice_counter += 1;
                    self.voices[i].age = self.voice_counter;
                    let freq_mult = math::pow2(cents[i] / 1200.0);
                    let pan = -1.0 + 2.0 * (i as f32) / 7.0;
                    self.voices[i].note_on(note, velocity, note, freq_mult, pan, false);
                }
            }
            VoiceMode::Octave => {
                // 3 voices: note-12, note, note+12
                let group = self.find_voice_group(3);
                let notes = [note.saturating_sub(12), note, note.saturating_add(12).min(127)];
                let mults = [0.5f32, 1.0, 2.0];
                for k in 0..3 {
                    let idx = group[k];
                    self.voice_counter += 1;
                    self.voices[idx].age = self.voice_counter;
                    self.voices[idx].note_on(notes[k], velocity, note, mults[k], 0.0, false);
                }
            }
            VoiceMode::Fifth => {
                // 2 voices: note, note+7
                let group = self.find_voice_group(2);
                let notes = [note, note.saturating_add(7).min(127)];
                for k in 0..2 {
                    let idx = group[k];
                    self.voice_counter += 1;
                    self.voices[idx].age = self.voice_counter;
                    self.voices[idx].note_on(notes[k], velocity, note, 1.0, 0.0, false);
                }
            }
            VoiceMode::RingMod => {
                let idx = self.find_voice();
                self.voice_counter += 1;
                self.voices[idx].age = self.voice_counter;
                self.voices[idx].note_on(note, velocity, note, 1.0, 0.0, true);
            }
        }
    }

    fn note_off(&mut self, note: u8) {
        // Match on root_note (not voice note) — releases entire group
        for voice in &mut self.voices {
            if voice.active && voice.root_note == note {
                voice.note_off();
                // No break — release all voices in the group
            }
        }
    }

    fn reset(&mut self) {
        for voice in &mut self.voices {
            voice.reset();
        }
        self.chorus.reset();
        self.lfo_router.reset();
        self.pitch_bend_ratio = 1.0;
        self.vibrato_phase = 0.0;
        self.vibrato_onset = 0.0;
    }
}
