use crate::modules::bass::BassModule;
use crate::modules::fm::FmModule;
use crate::modules::keys::KeysModule;
use crate::modules::beats::BeatsModule;
use crate::modules::arp::ArpModule;
use crate::effects::delay::{Delay, DelaySync};
use crate::effects::reverb::Reverb;
use crate::effects::eq::{TiltEq, ThreeBandEq};
use crate::effects::saturator::Saturator;
use crate::effects::compressor::Compressor;
use crate::effects::limiter::Limiter;
use crate::primitives::lfo::{LfoWaveform, LfoSyncMode, LfoTarget, ModulationRouter};
use crate::modules::arp::ArpParam;
use crate::modules::beats::BeatsParam;
use crate::sequencer::{ParamLock, Sequencer, SequencerEvent, MAX_LOCKS_PER_STEP};
use crate::harmony::HarmonyContext;
use crate::{math, Module, BLOCK_SIZE, SAMPLE_RATE};

const MAX_PARAM_SLIDES: usize = 8;

#[derive(Clone, Copy)]
struct ParamSlide {
    param_id: u8,
    current_value: f32,
    target_value: f32,
    increment: f32,
    remaining_samples: u32,
}

impl ParamSlide {
    fn tick(&mut self) -> f32 {
        if self.remaining_samples == 0 {
            return self.target_value;
        }
        self.current_value += self.increment;
        self.remaining_samples -= 1;
        if self.remaining_samples == 0 {
            self.current_value = self.target_value;
        }
        self.current_value
    }
}

/// Equal-power panning: pan in -1.0 (left) .. 1.0 (right)
fn pan_apply(signal: f32, pan: f32) -> (f32, f32) {
    let r = (pan + 1.0) * 0.5; // 0.0..1.0
    (signal * math::sqrt(1.0 - r), signal * math::sqrt(r))
}

pub struct Engine {
    pub bass: BassModule,
    pub fm: FmModule,
    pub keys: KeysModule,
    pub beats: BeatsModule,
    pub arp: ArpModule,
    pub sequencer: Sequencer,
    // Master effects
    pub delay: Delay,
    pub reverb: Reverb,
    pub tilt_eq: TiltEq,
    pub three_band_eq: ThreeBandEq,
    pub saturator: Saturator,
    pub compressor: Compressor,
    pub limiter: Limiter,
    sidechain_amount: f32,
    sc_envelope: f32,
    // Mix levels
    bass_level: f32,
    fm_level: f32,
    keys_level: f32,
    beats_level: f32,
    arp_level: f32,
    master_level: f32,
    // Per-module panning (-1.0 left .. 1.0 right)
    bass_pan: f32,
    fm_pan: f32,
    arp_pan: f32,
    // Delay LFO
    delay_lfo: ModulationRouter,
    delay_time_base: f32,
    // Pitch bend
    pitch_bend: f32,
    bend_range: f32,
    // Parameter lock interpolation
    param_slides: [Option<ParamSlide>; MAX_PARAM_SLIDES],
    param_values: [f32; 256],
    // Active module: 0=bass, 1=keys, 2=fm, 3=beats
    active_module: u8,
}

impl Engine {
    pub fn new() -> Self {
        Self {
            bass: BassModule::new(),
            fm: FmModule::new(),
            keys: KeysModule::new(),
            beats: BeatsModule::new(),
            arp: ArpModule::new(),
            sequencer: Sequencer::new(120.0),
            delay: Delay::new(SAMPLE_RATE, 2.0),
            reverb: Reverb::new(SAMPLE_RATE),
            tilt_eq: TiltEq::new(SAMPLE_RATE),
            three_band_eq: ThreeBandEq::new(SAMPLE_RATE),
            saturator: Saturator::new(1.2),
            compressor: Compressor::new(SAMPLE_RATE),
            limiter: Limiter::new(SAMPLE_RATE),
            sidechain_amount: 0.5,
            sc_envelope: 0.0,
            bass_level: 0.55,
            fm_level: 0.4,
            keys_level: 0.35,
            beats_level: 1.5,
            arp_level: 0.25,
            master_level: 0.8,
            bass_pan: 0.0,
            fm_pan: 0.1,
            arp_pan: 0.2,
            delay_lfo: {
                let mut r = ModulationRouter::new(SAMPLE_RATE, 5004);
                r.set_target(LfoTarget::DelayTime);
                r
            },
            delay_time_base: 0.3,
            pitch_bend: 0.0,
            bend_range: 2.0,
            param_slides: [None; MAX_PARAM_SLIDES],
            param_values: [0.0; 256],
            active_module: 0,
        }
    }

    /// Set harmony context for all harmony-aware modules.
    pub fn set_harmony(&mut self, ctx: HarmonyContext) {
        self.bass.harmony = Some(ctx);
        self.keys.harmony = Some(ctx);
        self.arp.harmony = Some(ctx);
        self.bass.on_harmony_change();
        self.keys.on_harmony_change();
        self.arp.rebuild_notes();
    }

    /// Process one stereo block with sample-accurate event dispatch.
    pub fn process_block_stereo(&mut self, output_l: &mut [f32], output_r: &mut [f32]) {
        let len = output_l.len();

        // Collect sequencer events with their sample offsets
        let mut events: [(usize, SequencerEvent); 32] =
            [(0, SequencerEvent::Tick { locks: [None; MAX_LOCKS_PER_STEP], lock_slide: false, step_samples: 0.0 }); 32];
        let mut event_count = 0;
        for i in 0..len {
            if let Some(evt) = self.sequencer.tick() {
                if event_count < 32 {
                    events[event_count] = (i, evt);
                    event_count += 1;
                }
            }
        }

        // Process sub-blocks between events for sample-accurate timing
        let mut pos = 0;
        for ei in 0..event_count {
            let (offset, evt) = events[ei];
            if offset > pos {
                self.process_sub_block_stereo(
                    &mut output_l[pos..offset],
                    &mut output_r[pos..offset],
                );
            }
            self.dispatch_event(&evt);
            pos = offset;
        }

        if pos < len {
            self.process_sub_block_stereo(
                &mut output_l[pos..len],
                &mut output_r[pos..len],
            );
        }
    }

    /// Backward-compatible mono process_block: delegates to stereo and sums.
    pub fn process_block(&mut self, output: &mut [f32]) {
        let len = output.len();
        let mut buf_l = [0.0f32; BLOCK_SIZE];
        let mut buf_r = [0.0f32; BLOCK_SIZE];
        self.process_block_stereo(&mut buf_l[..len], &mut buf_r[..len]);
        for i in 0..len {
            output[i] = (buf_l[i] + buf_r[i]) * 0.5;
        }
    }

    /// Process a stereo sub-block through all modules and effects.
    fn process_sub_block_stereo(&mut self, output_l: &mut [f32], output_r: &mut [f32]) {
        let len = output_l.len();
        if len == 0 {
            return;
        }

        // Compute pitch bend ratio once per sub-block
        let pb_ratio = math::pow2(self.pitch_bend * self.bend_range / 12.0);
        self.bass.pitch_bend_ratio = pb_ratio;
        self.keys.pitch_bend_ratio = pb_ratio;
        self.fm.pitch_bend_ratio = pb_ratio;

        let mut bass_buf = [0.0f32; BLOCK_SIZE];
        let mut fm_buf = [0.0f32; BLOCK_SIZE];
        let mut keys_buf_l = [0.0f32; BLOCK_SIZE];
        let mut keys_buf_r = [0.0f32; BLOCK_SIZE];
        let mut arp_buf = [0.0f32; BLOCK_SIZE];
        let mut beats_buf_l = [0.0f32; BLOCK_SIZE];
        let mut beats_buf_r = [0.0f32; BLOCK_SIZE];

        // Modules generate mono
        self.bass.process_block(&mut bass_buf[..len]);
        self.fm.process_block(&mut fm_buf[..len]);
        self.arp.process_block(&mut arp_buf[..len]);
        // Keys and Beats generate stereo (internal per-voice panning)
        self.keys.process_block_stereo(&mut keys_buf_l[..len], &mut keys_buf_r[..len]);
        self.beats.process_block_stereo(&mut beats_buf_l[..len], &mut beats_buf_r[..len]);

        for i in 0..len {
            // Tick active param lock slides
            {
                let mut updates: [(u8, f32); MAX_PARAM_SLIDES] = [(0, 0.0); MAX_PARAM_SLIDES];
                let mut count = 0;
                for slot in self.param_slides.iter_mut() {
                    if let Some(slide) = slot {
                        let val = slide.tick();
                        updates[count] = (slide.param_id, val);
                        count += 1;
                        if slide.remaining_samples == 0 {
                            *slot = None;
                        }
                    }
                }
                for j in 0..count {
                    let (pid, val) = updates[j];
                    self.apply_param_lock(pid, val);
                }
            }

            // Sidechain: kick ducks synth bus
            let sc_input = self.beats.kick_env[i];
            self.sc_envelope = if sc_input > self.sc_envelope {
                0.01 * self.sc_envelope + 0.99 * sc_input
            } else {
                0.995 * self.sc_envelope
            };
            let duck = 1.0 - self.sidechain_amount * self.sc_envelope;

            // Pan mono modules into stereo
            let (bl, br) = pan_apply(bass_buf[i] * self.bass_level, self.bass_pan);
            let (fl, fr) = pan_apply(fm_buf[i] * self.fm_level, self.fm_pan);
            // Keys generates stereo internally (like beats)
            let kl = keys_buf_l[i] * self.keys_level;
            let kr = keys_buf_r[i] * self.keys_level;
            let (al, ar) = pan_apply(arp_buf[i] * self.arp_level, self.arp_pan);

            // Synth bus (without drums)
            let synth_l = bl + fl + kl + al;
            let synth_r = br + fr + kr + ar;

            // Delay LFO modulation
            if self.delay_lfo.enabled {
                let lfo_val = self.delay_lfo.next_sample();
                let time = math::clamp(self.delay_time_base + lfo_val * 0.1, 0.001, 2.0);
                self.delay.set_time(time, SAMPLE_RATE);
            }

            // Synth effects chain: delay → reverb → tilt_eq → three_band_eq → saturator
            let (dl, dr) = self.delay.process_stereo(synth_l, synth_r);
            let (rl, rr) = self.reverb.process_stereo_in(dl, dr);
            let (tl, tr) = self.tilt_eq.process_stereo(rl, rr);
            let (el, er) = self.three_band_eq.process_stereo(tl, tr);
            // Duck the effected synth bus (post-effects, ducks reverb/delay tails too)
            let sl = self.saturator.process(el) * self.master_level * duck;
            let sr = self.saturator.process(er) * self.master_level * duck;

            // Add drums post-effects (clean, punchy) then compressor → limiter
            let pre_l = sl + beats_buf_l[i] * self.beats_level;
            let pre_r = sr + beats_buf_r[i] * self.beats_level;
            let (cl, cr) = self.compressor.process_stereo(pre_l, pre_r);
            let (ll, lr) = self.limiter.process_stereo(cl, cr);
            output_l[i] = ll;
            output_r[i] = lr;
        }
    }

    /// Route a sequencer event to the active module.
    fn dispatch_event(&mut self, evt: &SequencerEvent) {
        match evt {
            SequencerEvent::NoteOn { note, velocity, slide, locks, lock_slide, step_samples } => {
                self.dispatch_locks(locks, *lock_slide, *step_samples);
                match self.active_module {
                    0 => {
                        if *slide {
                            self.bass.slide_to(*note, *velocity);
                        } else {
                            self.bass.note_on(*note, *velocity);
                        }
                    }
                    1 => self.keys.note_on(*note, *velocity),
                    2 => self.fm.note_on(*note, *velocity),
                    3 => self.beats.note_on(*note, *velocity),
                    _ => {}
                }
            }
            SequencerEvent::NoteOff { note } => {
                match self.active_module {
                    0 => self.bass.note_off(*note),
                    1 => self.keys.note_off(*note),
                    2 => self.fm.note_off(*note),
                    3 => self.beats.note_off(*note),
                    _ => {}
                }
            }
            SequencerEvent::Tick { locks, lock_slide, step_samples } => {
                self.dispatch_locks(locks, *lock_slide, *step_samples);
            }
        }
    }

    fn dispatch_locks(&mut self, locks: &[Option<ParamLock>; MAX_LOCKS_PER_STEP], lock_slide: bool, step_samples: f32) {
        if lock_slide && step_samples > 1.0 {
            self.start_lock_slides(locks, step_samples);
        } else {
            self.cancel_slides_for(locks);
            for lock in locks.iter().flatten() {
                self.apply_param_lock(lock.0, lock.1);
            }
        }
    }

    fn start_lock_slides(&mut self, locks: &[Option<ParamLock>; MAX_LOCKS_PER_STEP], step_samples: f32) {
        let total = step_samples as u32;
        for lock in locks.iter().flatten() {
            let (param_id, target) = *lock;
            let start = self.param_values[param_id as usize];
            let increment = (target - start) / step_samples;

            // Find slot: prefer existing slide for same param_id, else first empty
            let mut target_idx = None;
            let mut empty_idx = None;
            for (i, slot) in self.param_slides.iter().enumerate() {
                match slot {
                    Some(s) if s.param_id == param_id => { target_idx = Some(i); break; }
                    None if empty_idx.is_none() => { empty_idx = Some(i); }
                    _ => {}
                }
            }
            let idx = target_idx.or(empty_idx);
            if let Some(idx) = idx {
                self.param_slides[idx] = Some(ParamSlide {
                    param_id,
                    current_value: start,
                    target_value: target,
                    increment,
                    remaining_samples: total,
                });
            }
        }
    }

    fn cancel_slides_for(&mut self, locks: &[Option<ParamLock>; MAX_LOCKS_PER_STEP]) {
        for lock in locks.iter().flatten() {
            for slot in self.param_slides.iter_mut() {
                if let Some(s) = slot {
                    if s.param_id == lock.0 {
                        *slot = None;
                    }
                }
            }
        }
    }

    /// Route a param lock to the appropriate module by ParamId.
    pub fn apply_param_lock(&mut self, param_id: u8, value: f32) {
        self.param_values[param_id as usize] = value;
        use crate::modules::bass::BassParam;
        use crate::modules::keys::KeysParam;
        use crate::modules::fm::FmParam;

        match param_id {
            // Bass 0..=31
            0..=31 => {
                let param = match param_id {
                    0 => BassParam::Cutoff,
                    1 => BassParam::CutoffEnv,
                    2 => BassParam::Resonance,
                    3 => BassParam::Glide,
                    4 => BassParam::Attack,
                    5 => BassParam::LfoRate,
                    6 => BassParam::LfoDepth,
                    7 => BassParam::LfoWaveform,
                    8 => BassParam::LfoTarget,
                    9 => BassParam::LfoSync,
                    10 => BassParam::Osc2Pitch,
                    11 => BassParam::Osc3Pitch,
                    12 => BassParam::Osc1Wave,
                    13 => BassParam::Osc2Wave,
                    14 => BassParam::Osc3Wave,
                    15 => BassParam::Keytrack,
                    16 => BassParam::VibratoRate,
                    17 => BassParam::VibratoDepth,
                    _ => return,
                };
                self.bass.set_param(param, value);
            }
            // Keys 32..=63
            32..=63 => {
                let param = match param_id {
                    32 => KeysParam::Cutoff,
                    33 => KeysParam::Detune,
                    34 => KeysParam::ChorusMix,
                    35 => KeysParam::Level,
                    36 => KeysParam::LfoRate,
                    37 => KeysParam::LfoDepth,
                    38 => KeysParam::LfoWaveform,
                    39 => KeysParam::LfoTarget,
                    40 => KeysParam::LfoSync,
                    41 => KeysParam::VoiceMode,
                    42 => KeysParam::VibratoRate,
                    43 => KeysParam::VibratoDepth,
                    _ => return,
                };
                self.keys.set_param(param, value);
            }
            // FM 64..=95
            64..=95 => {
                let param = match param_id {
                    64 => FmParam::Algorithm,
                    65 => FmParam::ModIndex,
                    66 => FmParam::LfoRate,
                    67 => FmParam::LfoDepth,
                    68 => FmParam::LfoWaveform,
                    69 => FmParam::LfoTarget,
                    70 => FmParam::LfoSync,
                    71 => FmParam::Feedback,
                    72 => FmParam::Waveform,
                    73 => FmParam::ChorusMix,
                    74 => FmParam::VibratoRate,
                    75 => FmParam::VibratoDepth,
                    _ => return,
                };
                self.fm.set_param(param, value);
            }
            // Beats 96..=127
            96..=127 => {
                let param = match param_id {
                    96 => BeatsParam::Level,
                    97 => BeatsParam::KickDecay,
                    98 => BeatsParam::SnareDecay,
                    99 => BeatsParam::KickPan,
                    100 => BeatsParam::SnarePan,
                    101 => BeatsParam::HihatPan,
                    102 => BeatsParam::ClapPan,
                    103 => BeatsParam::KickClick,
                    104 => BeatsParam::KickLevel,
                    105 => BeatsParam::SnareLevel,
                    106 => BeatsParam::HihatLevel,
                    107 => BeatsParam::ClapLevel,
                    108 => BeatsParam::KickPitch,
                    109 => BeatsParam::SnarePitch,
                    110 => BeatsParam::HihatPitch,
                    111 => BeatsParam::StutterRate,
                    112 => BeatsParam::TomPan,
                    113 => BeatsParam::CrashPan,
                    _ => return,
                };
                self.beats.set_param(param, value);
            }
            // Arp 128..=159
            128..=159 => {
                let param = match param_id {
                    128 => ArpParam::Rate,
                    129 => ArpParam::Gate,
                    130 => ArpParam::Pattern,
                    131 => ArpParam::Level,
                    _ => return,
                };
                self.arp.set_param(param, value);
            }
            // Reverb 160..=191
            160..=191 => {
                match param_id {
                    160 => self.reverb.set_room_size(value),
                    161 => self.reverb.set_damping(value),
                    162 => self.reverb.set_mix(value),
                    163 => self.reverb.set_pre_delay(value * 100.0), // 0..1 → 0..100ms
                    _ => {}
                }
            }
            // EQ / Compressor / Sidechain 192..=223
            192..=223 => {
                match param_id {
                    192 => self.tilt_eq.set_tilt(value * 2.0 - 1.0), // 0..1 → -1..+1
                    193 => self.three_band_eq.set_low(value * 24.0 - 12.0), // 0..1 → -12..+12 dB
                    194 => self.three_band_eq.set_mid(value * 24.0 - 12.0),
                    195 => self.three_band_eq.set_high(value * 24.0 - 12.0),
                    196 => self.compressor.set_threshold(value * -40.0), // 0..1 → 0..-40 dB
                    197 => self.compressor.set_ratio(1.0 + value * 19.0), // 0..1 → 1..20
                    198 => self.compressor.set_attack(0.1 + value * 99.9), // 0..1 → 0.1..100ms
                    199 => self.compressor.set_release(10.0 + value * 490.0), // 0..1 → 10..500ms
                    200 => self.set_sidechain_amount(value), // 0..1 direct
                    201 => self.set_pitch_bend(value * 2.0 - 1.0), // 0..1 → −1..+1
                    _ => {}
                }
            }
            // Engine delay 224..=255
            224..=255 => {
                match param_id {
                    224 => self.set_delay_time(value),
                    225 => self.set_delay_lfo_rate(0.1 + value * 19.9),
                    226 => self.set_delay_lfo_depth(value),
                    227 => self.set_delay_feedback(value),
                    228 => self.set_delay_filter(value),
                    229 => self.set_delay_sync(DelaySync::from_normalized(value)),
                    _ => {}
                }
            }
        }
    }

    /// Set the active module for sequencer note routing.
    /// 0=bass, 1=keys, 2=fm, 3=beats
    pub fn set_active_module(&mut self, module: u8) {
        self.active_module = module;
    }

    pub fn set_bpm(&mut self, bpm: f32) {
        self.sequencer.set_bpm(bpm);
        self.arp.set_bpm(bpm);
        self.bass.set_bpm(bpm);
        self.keys.set_bpm(bpm);
        self.fm.set_bpm(bpm);
        self.beats.set_bpm(bpm);
        self.delay_lfo.set_bpm(bpm);
        self.delay.set_bpm(bpm, SAMPLE_RATE);
    }

    pub fn set_delay_feedback(&mut self, feedback: f32) {
        self.delay.set_feedback(feedback);
    }

    pub fn set_delay_filter(&mut self, amount: f32) {
        self.delay.set_filter(amount);
    }

    pub fn set_delay_sync(&mut self, sync: DelaySync) {
        self.delay.set_sync(sync, self.sequencer.bpm(), SAMPLE_RATE);
    }

    pub fn set_delay_lfo_rate(&mut self, rate: f32) {
        self.delay_lfo.lfo.set_rate(rate);
    }

    pub fn set_delay_lfo_depth(&mut self, depth: f32) {
        self.delay_lfo.lfo.set_depth(depth);
        self.delay_lfo.enabled = depth > 0.001;
    }

    pub fn set_delay_lfo_waveform(&mut self, waveform: LfoWaveform) {
        self.delay_lfo.lfo.set_waveform(waveform);
    }

    pub fn set_delay_lfo_sync(&mut self, mode: LfoSyncMode) {
        self.delay_lfo.lfo.set_sync_mode(mode);
    }

    pub fn set_delay_time(&mut self, time: f32) {
        self.delay_time_base = math::clamp(time, 0.001, 2.0);
        self.delay.set_time(self.delay_time_base, SAMPLE_RATE);
    }

    pub fn set_reverb_pre_delay(&mut self, ms: f32) {
        self.reverb.set_pre_delay(ms);
    }

    pub fn set_tilt_eq(&mut self, amount: f32) {
        self.tilt_eq.set_tilt(amount);
    }

    pub fn set_eq_low(&mut self, gain_db: f32) {
        self.three_band_eq.set_low(gain_db);
    }

    pub fn set_eq_mid(&mut self, gain_db: f32) {
        self.three_band_eq.set_mid(gain_db);
    }

    pub fn set_eq_high(&mut self, gain_db: f32) {
        self.three_band_eq.set_high(gain_db);
    }

    pub fn set_limiter_threshold(&mut self, threshold: f32) {
        self.limiter.set_threshold(threshold);
    }

    pub fn set_limiter_release(&mut self, ms: f32) {
        self.limiter.set_release(ms, SAMPLE_RATE);
    }

    pub fn set_limiter_makeup(&mut self, gain: f32) {
        self.limiter.set_makeup_gain(gain);
    }

    pub fn set_compressor_threshold(&mut self, db: f32) {
        self.compressor.set_threshold(db);
    }

    pub fn set_compressor_ratio(&mut self, ratio: f32) {
        self.compressor.set_ratio(ratio);
    }

    pub fn set_compressor_attack(&mut self, ms: f32) {
        self.compressor.set_attack(ms);
    }

    pub fn set_compressor_release(&mut self, ms: f32) {
        self.compressor.set_release(ms);
    }

    pub fn set_compressor_makeup(&mut self, gain: f32) {
        self.compressor.set_makeup(gain);
    }

    pub fn set_pitch_bend(&mut self, bend: f32) {
        self.pitch_bend = math::clamp(bend, -1.0, 1.0);
    }

    pub fn set_bend_range(&mut self, semitones: f32) {
        self.bend_range = math::clamp(semitones, 0.0, 24.0);
    }

    pub fn set_sidechain_amount(&mut self, amount: f32) {
        self.sidechain_amount = math::clamp(amount, 0.0, 1.0);
    }

    /// Silence all modules (send note-off to everything).
    pub fn all_notes_off(&mut self) {
        self.bass.reset();
        self.keys.reset();
        self.fm.reset();
        // Don't reset beats — drums are one-shot, they'll decay naturally
    }

    pub fn reset(&mut self) {
        self.bass.reset();
        self.fm.reset();
        self.keys.reset();
        self.beats.reset();
        self.arp.reset();
        self.sequencer.reset();
        self.delay.reset();
        self.reverb.reset();
        self.tilt_eq.reset();
        self.three_band_eq.reset();
        self.compressor.reset();
        self.limiter.reset();
        self.delay_lfo.reset();
        self.sc_envelope = 0.0;
        self.pitch_bend = 0.0;
        self.param_slides = [None; MAX_PARAM_SLIDES];
        self.param_values = [0.0; 256];
    }
}
