use alloc::boxed::Box;
use crate::modules::bass::BassModule;
use crate::modules::fm::FmModule;
use crate::modules::keys::KeysModule;
use crate::modules::beats::BeatsModule;
use crate::effects::chorus::Chorus;
use crate::effects::delay::{Delay, DelaySync};
use crate::effects::reverb::Reverb;
use crate::effects::eq::{TiltEq, ThreeBandEq};
use crate::effects::saturator::Saturator;
use crate::effects::compressor::Compressor;
use crate::effects::limiter::Limiter;
use crate::effects::bitcrusher::Bitcrusher;
use crate::effects::tape_stop::TapeStop;
use crate::primitives::filter::BiquadFilter;
use crate::primitives::lfo::{LfoWaveform, LfoSyncMode, LfoTarget, ModulationRouter};
use crate::modules::beats::BeatsParam;
use crate::sequencer::{ParamLock, Sequencer, SequencerEvent, MAX_LOCKS_PER_STEP, MAX_TICK_EVENTS};
use crate::harmony::HarmonyContext;
use crate::track::{TrackSlot, InsertFxSlot, InstrumentKind, InsertFxType, MAX_TRACKS, MAX_INSERT_FX, MAX_MASTER_FX};
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
    // ── Module pool (primary instances, backward-compat public fields) ──
    pub bass: BassModule,
    pub fm: FmModule,
    pub keys: KeysModule,
    pub beats: BeatsModule,
    // ── Module pool (secondary instances for extra tracks) ──
    pub bass2: BassModule,
    pub fm2: FmModule,
    pub keys2: KeysModule,
    // ── Track routing ──
    pub tracks: [TrackSlot; MAX_TRACKS],
    // ── Insert FX pool (heap-allocated, assigned to tracks + master at runtime) ──
    pub insert_filters: Box<[BiquadFilter; 12]>,
    pub insert_saturators: Box<[Saturator; 8]>,
    pub insert_choruses: Box<[Chorus; 6]>,
    pub insert_eqs: Box<[TiltEq; 8]>,
    pub insert_compressors: Box<[Compressor; 8]>,
    pub insert_delays: Box<[Delay; 4]>,
    pub insert_reverbs: Box<[Reverb; 4]>,
    pub insert_limiters: Box<[Limiter; 4]>,
    pub insert_three_band_eqs: Box<[ThreeBandEq; 4]>,
    pub insert_bitcrushers: Box<[Bitcrusher; 6]>,
    pub insert_tape_stops: Box<[TapeStop; 4]>,
    // ── Sequencer ──
    pub sequencer: Sequencer,
    // ── Master FX chain (configurable) ──
    pub master_fx: [InsertFxSlot; MAX_MASTER_FX],
    // ── Send effects (global) ──
    pub delay: Delay,
    pub reverb: Reverb,
    sidechain_amount: f32,
    pub sc_envelope: f32,
    // ── Master mix ──
    master_level: f32,
    // ── Delay LFO ──
    delay_lfo: ModulationRouter,
    delay_time_base: f32,
    // ── Pitch bend ──
    pitch_bend: f32,
    bend_range: f32,
    // ── Parameter lock interpolation ──
    param_slides: [Option<ParamSlide>; MAX_PARAM_SLIDES],
    param_values: [f32; 256],
}

impl Engine {
    pub fn new() -> Self {
        Self {
            bass: BassModule::new(),
            fm: FmModule::new(),
            keys: KeysModule::new(),
            beats: BeatsModule::new(),
            bass2: BassModule::new(),
            fm2: FmModule::new(),
            keys2: KeysModule::new(),
            insert_filters: Box::new(core::array::from_fn(|_| BiquadFilter::new(SAMPLE_RATE))),
            insert_saturators: Box::new(core::array::from_fn(|_| Saturator::new(1.5))),
            insert_choruses: Box::new(core::array::from_fn(|_| Chorus::new())),
            insert_eqs: Box::new(core::array::from_fn(|_| TiltEq::new(SAMPLE_RATE))),
            insert_compressors: Box::new(core::array::from_fn(|_| Compressor::new(SAMPLE_RATE))),
            insert_delays: Box::new(core::array::from_fn(|_| Delay::new(SAMPLE_RATE, 2.0))),
            insert_reverbs: Box::new(core::array::from_fn(|_| Reverb::new(SAMPLE_RATE))),
            insert_limiters: Box::new(core::array::from_fn(|_| Limiter::new(SAMPLE_RATE))),
            insert_three_band_eqs: Box::new(core::array::from_fn(|_| ThreeBandEq::new(SAMPLE_RATE))),
            insert_bitcrushers: Box::new(core::array::from_fn(|_| Bitcrusher::new())),
            insert_tape_stops: Box::new(core::array::from_fn(|_| TapeStop::new(SAMPLE_RATE))),
            tracks: [
                TrackSlot::new(InstrumentKind::Bass, 0).with_level(0.55),
                TrackSlot::new(InstrumentKind::Keys, 0).with_level(0.35),
                TrackSlot::new(InstrumentKind::Fm, 0).with_level(0.4).with_pan(0.1),
                TrackSlot::new(InstrumentKind::Beats, 0).with_level(0.7),
                TrackSlot::empty(),
                TrackSlot::empty(),
            ],
            sequencer: Sequencer::new(120.0),
            master_fx: [
                InsertFxSlot::new(InsertFxType::TiltEq, 0),
                InsertFxSlot::new(InsertFxType::ThreeBandEq, 0),
                InsertFxSlot::new(InsertFxType::Compressor, 0),
                InsertFxSlot::new(InsertFxType::Limiter, 0),
                InsertFxSlot::empty(),
                InsertFxSlot::empty(),
            ],
            delay: Delay::new(SAMPLE_RATE, 2.0),
            reverb: Reverb::new(SAMPLE_RATE),
            sidechain_amount: 0.5,
            sc_envelope: 0.0,
            master_level: 0.8,
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
        }
    }

    /// Set harmony context for all harmony-aware modules.
    pub fn set_harmony(&mut self, ctx: HarmonyContext) {
        self.bass.harmony = Some(ctx);
        self.bass2.harmony = Some(ctx);
        self.keys.harmony = Some(ctx);
        self.keys2.harmony = Some(ctx);
        self.bass.on_harmony_change();
        self.bass2.on_harmony_change();
        self.keys.on_harmony_change();
        self.keys2.on_harmony_change();
    }

    /// Process one stereo block with sample-accurate event dispatch.
    pub fn process_block_stereo(&mut self, output_l: &mut [f32], output_r: &mut [f32]) {
        let len = output_l.len();

        // Collect sequencer events with their sample offsets (multi-track)
        let mut events: [(usize, SequencerEvent); 128] =
            [(0, SequencerEvent::Tick { track: 0, locks: [None; MAX_LOCKS_PER_STEP], lock_slide: false, step_samples: 0.0 }); 128];
        let mut event_count = 0;
        for i in 0..len {
            let mut tick_buf: [Option<SequencerEvent>; MAX_TICK_EVENTS] = [None; MAX_TICK_EVENTS];
            let n = self.sequencer.tick_events(&mut tick_buf);
            for j in 0..n {
                if let Some(evt) = tick_buf[j] {
                    if event_count < 128 {
                        events[event_count] = (i, evt);
                        event_count += 1;
                    }
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
        self.bass2.pitch_bend_ratio = pb_ratio;
        self.keys.pitch_bend_ratio = pb_ratio;
        self.keys2.pitch_bend_ratio = pb_ratio;
        self.fm.pitch_bend_ratio = pb_ratio;
        self.fm2.pitch_bend_ratio = pb_ratio;

        // Per-track audio buffers
        let mut trk_l = [[0.0f32; BLOCK_SIZE]; MAX_TRACKS];
        let mut trk_r = [[0.0f32; BLOCK_SIZE]; MAX_TRACKS];

        // Tick arp processors per-sample before rendering
        // (ArpProcessor emits note events that feed into instruments)
        for i in 0..len {
            for t in 0..MAX_TRACKS {
                if let Some(ref mut arp) = self.tracks[t].arp {
                    if let Some(evt) = arp.tick() {
                        use crate::primitives::arp_processor::ArpEvent;
                        match evt {
                            ArpEvent::NoteOn(note, vel) => self.track_note_on(t, note, vel, false),
                            ArpEvent::NoteOff(note) => self.track_note_off(t, note),
                        }
                    }
                }
            }
            // We need to process sample-by-sample for arp, but modules process in blocks.
            // So we break this into: tick arps for the full sub-block, then render instruments.
            // The arp events land on the instruments and affect the next process_block call.
            let _ = i; // arp ticks happen before the block render below
        }

        // Render each track's instrument into its buffer
        for t in 0..MAX_TRACKS {
            match (self.tracks[t].kind, self.tracks[t].instance_idx) {
                (InstrumentKind::Bass, 0) => self.bass.process_block(&mut trk_l[t][..len]),
                (InstrumentKind::Bass, _) => self.bass2.process_block(&mut trk_l[t][..len]),
                (InstrumentKind::Fm, 0) => self.fm.process_block(&mut trk_l[t][..len]),
                (InstrumentKind::Fm, _) => self.fm2.process_block(&mut trk_l[t][..len]),
                (InstrumentKind::Keys, 0) => self.keys.process_block_stereo(&mut trk_l[t][..len], &mut trk_r[t][..len]),
                (InstrumentKind::Keys, _) => self.keys2.process_block_stereo(&mut trk_l[t][..len], &mut trk_r[t][..len]),
                (InstrumentKind::Beats, _) => self.beats.process_block_stereo(&mut trk_l[t][..len], &mut trk_r[t][..len]),
                (InstrumentKind::None, _) => {},
            }
        }

        // Apply per-track insert FX chains
        for t in 0..MAX_TRACKS {
            if self.tracks[t].kind == InstrumentKind::None { continue; }
            for slot_idx in 0..MAX_INSERT_FX {
                let slot = self.tracks[t].insert_fx[slot_idx];
                if slot.fx_type == InsertFxType::None || !slot.enabled { continue; }
                let idx = slot.instance_idx as usize;
                for i in 0..len {
                    let (nl, nr) = self.apply_fx_sample(slot.fx_type, idx, trk_l[t][i], trk_r[t][i]);
                    trk_l[t][i] = nl;
                    trk_r[t][i] = nr;
                }
            }
        }

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

            // Track-based mixing — all tracks into one bus
            let mut mix_l = 0.0f32;
            let mut mix_r = 0.0f32;
            let mut delay_in_l = 0.0f32;
            let mut delay_in_r = 0.0f32;
            let mut reverb_in_l = 0.0f32;
            let mut reverb_in_r = 0.0f32;

            for t in 0..MAX_TRACKS {
                let slot = &self.tracks[t];
                if slot.kind == InstrumentKind::None { continue; }
                if !self.is_track_audible(t) { continue; }

                let (tl, tr) = if slot.kind.is_stereo() {
                    (trk_l[t][i] * slot.level, trk_r[t][i] * slot.level)
                } else {
                    pan_apply(trk_l[t][i] * slot.level, slot.pan)
                };

                mix_l += tl;
                mix_r += tr;

                delay_in_l += tl * slot.delay_send;
                delay_in_r += tr * slot.delay_send;
                reverb_in_l += tl * slot.reverb_send;
                reverb_in_r += tr * slot.reverb_send;
            }

            // Delay LFO modulation
            if self.delay_lfo.enabled {
                let lfo_val = self.delay_lfo.next_sample();
                let time = math::clamp(self.delay_time_base + lfo_val * 0.1, 0.001, 2.0);
                self.delay.set_time(time, SAMPLE_RATE);
            }

            // Wet-only effect returns (send/return topology)
            let (dl, dr) = self.delay.process_stereo_wet(delay_in_l, delay_in_r);
            let (rl, rr) = self.reverb.process_stereo_in_wet(reverb_in_l, reverb_in_r);

            // All tracks + send returns → sidechain duck → master level → master FX chain
            let (mut ml, mut mr) = (
                (mix_l + dl + rl) * self.master_level * duck,
                (mix_r + dr + rr) * self.master_level * duck,
            );

            // Configurable master FX chain
            for s in 0..MAX_MASTER_FX {
                let slot = self.master_fx[s];
                if slot.fx_type == InsertFxType::None || !slot.enabled { continue; }
                let (nl, nr) = self.apply_fx_sample(slot.fx_type, slot.instance_idx as usize, ml, mr);
                ml = nl;
                mr = nr;
            }
            output_l[i] = ml;
            output_r[i] = mr;
        }
    }

    /// Apply a single FX to a stereo sample pair. Shared by track inserts and master chain.
    #[inline]
    fn apply_fx_sample(&mut self, fx_type: InsertFxType, idx: usize, l: f32, r: f32) -> (f32, f32) {
        match fx_type {
            InsertFxType::Filter => {
                if idx < self.insert_filters.len() {
                    let ol = self.insert_filters[idx].process(l);
                    let or = if idx + 1 < self.insert_filters.len() {
                        self.insert_filters[idx + 1].process(r)
                    } else { r };
                    (ol, or)
                } else { (l, r) }
            }
            InsertFxType::Saturator => {
                if idx < self.insert_saturators.len() {
                    (self.insert_saturators[idx].process(l), self.insert_saturators[idx].process(r))
                } else { (l, r) }
            }
            InsertFxType::Chorus => {
                if idx < self.insert_choruses.len() {
                    (self.insert_choruses[idx].process(l), r)
                } else { (l, r) }
            }
            InsertFxType::TiltEq => {
                if idx < self.insert_eqs.len() {
                    self.insert_eqs[idx].process_stereo(l, r)
                } else { (l, r) }
            }
            InsertFxType::Compressor => {
                if idx < self.insert_compressors.len() {
                    self.insert_compressors[idx].process_stereo(l, r)
                } else { (l, r) }
            }
            InsertFxType::Delay => {
                if idx < self.insert_delays.len() {
                    self.insert_delays[idx].process_stereo(l, r)
                } else { (l, r) }
            }
            InsertFxType::Reverb => {
                if idx < self.insert_reverbs.len() {
                    self.insert_reverbs[idx].process_stereo_in(l, r)
                } else { (l, r) }
            }
            InsertFxType::Limiter => {
                if idx < self.insert_limiters.len() {
                    self.insert_limiters[idx].process_stereo(l, r)
                } else { (l, r) }
            }
            InsertFxType::ThreeBandEq => {
                if idx < self.insert_three_band_eqs.len() {
                    self.insert_three_band_eqs[idx].process_stereo(l, r)
                } else { (l, r) }
            }
            InsertFxType::Bitcrusher => {
                if idx < self.insert_bitcrushers.len() {
                    self.insert_bitcrushers[idx].process_stereo(l, r)
                } else { (l, r) }
            }
            InsertFxType::TapeStop => {
                if idx < self.insert_tape_stops.len() {
                    self.insert_tape_stops[idx].process_stereo(l, r)
                } else { (l, r) }
            }
            InsertFxType::None => (l, r),
        }
    }

    /// Route a sequencer event to the appropriate module via track slot.
    fn dispatch_event(&mut self, evt: &SequencerEvent) {
        match evt {
            SequencerEvent::NoteOn { track, note, velocity, slide, locks, lock_slide, step_samples } => {
                self.dispatch_locks(locks, *lock_slide, *step_samples);
                let t = *track as usize;
                if t < MAX_TRACKS {
                    self.track_note_on(t, *note, *velocity, *slide);
                }
            }
            SequencerEvent::NoteOff { track, note } => {
                let t = *track as usize;
                if t < MAX_TRACKS {
                    self.track_note_off(t, *note);
                }
            }
            SequencerEvent::Tick { locks, lock_slide, step_samples, .. } => {
                self.dispatch_locks(locks, *lock_slide, *step_samples);
            }
        }
    }

    fn track_note_on(&mut self, track: usize, note: u8, velocity: f32, slide: bool) {
        match (self.tracks[track].kind, self.tracks[track].instance_idx) {
            (InstrumentKind::Bass, 0) => {
                if slide { self.bass.slide_to(note, velocity); }
                else { self.bass.note_on(note, velocity); }
            }
            (InstrumentKind::Bass, _) => {
                if slide { self.bass2.slide_to(note, velocity); }
                else { self.bass2.note_on(note, velocity); }
            }
            (InstrumentKind::Keys, 0) => self.keys.note_on(note, velocity),
            (InstrumentKind::Keys, _) => self.keys2.note_on(note, velocity),
            (InstrumentKind::Fm, 0) => self.fm.note_on(note, velocity),
            (InstrumentKind::Fm, _) => self.fm2.note_on(note, velocity),
            (InstrumentKind::Beats, _) => self.beats.note_on(note, velocity),
            (InstrumentKind::None, _) => {}
        }
    }

    fn track_note_off(&mut self, track: usize, note: u8) {
        match (self.tracks[track].kind, self.tracks[track].instance_idx) {
            (InstrumentKind::Bass, 0) => self.bass.note_off(note),
            (InstrumentKind::Bass, _) => self.bass2.note_off(note),
            (InstrumentKind::Keys, 0) => self.keys.note_off(note),
            (InstrumentKind::Keys, _) => self.keys2.note_off(note),
            (InstrumentKind::Fm, 0) => self.fm.note_off(note),
            (InstrumentKind::Fm, _) => self.fm2.note_off(note),
            (InstrumentKind::Beats, _) => self.beats.note_off(note),
            (InstrumentKind::None, _) => {}
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
                    18 => BassParam::Decay,
                    19 => BassParam::Sustain,
                    20 => BassParam::Release,
                    21 => BassParam::VelEnv,
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
                    44 => KeysParam::Attack,
                    45 => KeysParam::Decay,
                    46 => KeysParam::Sustain,
                    47 => KeysParam::Release,
                    48 => KeysParam::Resonance,
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
                    76 => FmParam::Attack,
                    77 => FmParam::Decay,
                    78 => FmParam::Sustain,
                    79 => FmParam::Release,
                    // Per-operator ADSR (80-95)
                    80 => FmParam::Op0Attack, 81 => FmParam::Op0Decay,
                    82 => FmParam::Op0Sustain, 83 => FmParam::Op0Release,
                    84 => FmParam::Op1Attack, 85 => FmParam::Op1Decay,
                    86 => FmParam::Op1Sustain, 87 => FmParam::Op1Release,
                    88 => FmParam::Op2Attack, 89 => FmParam::Op2Decay,
                    90 => FmParam::Op2Sustain, 91 => FmParam::Op2Release,
                    92 => FmParam::Op3Attack, 93 => FmParam::Op3Decay,
                    94 => FmParam::Op3Sustain, 95 => FmParam::Op3Release,
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
            // FM extended 132..=139 (arp params 128-131, 140-147 removed)
            128..=159 => {
                match param_id {
                    // FM per-operator feedback (132-135)
                    132 => self.fm.set_param(FmParam::Op0Feedback, value),
                    133 => self.fm.set_param(FmParam::Op1Feedback, value),
                    134 => self.fm.set_param(FmParam::Op2Feedback, value),
                    135 => self.fm.set_param(FmParam::Op3Feedback, value),
                    // FM per-operator ratio (136-139)
                    136 => self.fm.set_param(FmParam::Op0Ratio, value),
                    137 => self.fm.set_param(FmParam::Op1Ratio, value),
                    138 => self.fm.set_param(FmParam::Op2Ratio, value),
                    139 => self.fm.set_param(FmParam::Op3Ratio, value),
                    _ => {}
                }
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
            // EQ / Compressor / Sidechain 192..=223 (routes to master FX pool instances)
            192..=223 => {
                match param_id {
                    192 => self.insert_eqs[0].set_tilt(value * 2.0 - 1.0), // 0..1 → -1..+1
                    193 => self.insert_three_band_eqs[0].set_low(value * 24.0 - 12.0), // 0..1 → -12..+12 dB
                    194 => self.insert_three_band_eqs[0].set_mid(value * 24.0 - 12.0),
                    195 => self.insert_three_band_eqs[0].set_high(value * 24.0 - 12.0),
                    196 => self.insert_compressors[0].set_threshold(value * -40.0), // 0..1 → 0..-40 dB
                    197 => self.insert_compressors[0].set_ratio(1.0 + value * 19.0), // 0..1 → 1..20
                    198 => self.insert_compressors[0].set_attack(0.1 + value * 99.9), // 0..1 → 0.1..100ms
                    199 => self.insert_compressors[0].set_release(10.0 + value * 490.0), // 0..1 → 10..500ms
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

    /// Backward-compatible: previously set the active module for single-track routing.
    /// Now a no-op since multi-track routing uses the track field in events.
    pub fn set_active_module(&mut self, _module: u8) {
        // No-op: kept for API compatibility
    }

    /// Mute or unmute a track. Legacy module indices: 0=bass, 1=keys, 2=fm, 3=beats, 4=arp.
    pub fn set_module_mute(&mut self, module: u8, muted: bool) {
        if (module as usize) < MAX_TRACKS {
            self.tracks[module as usize].muted = muted;
        }
    }

    /// Solo or unsolo a track. Legacy module indices: 0=bass, 1=keys, 2=fm, 3=beats, 4=arp.
    pub fn set_module_solo(&mut self, module: u8, solo: bool) {
        if (module as usize) < MAX_TRACKS {
            self.tracks[module as usize].solo = solo;
        }
    }

    /// Set per-track output level. Legacy module indices: 0=bass, 1=keys, 2=fm, 3=beats, 4=arp.
    pub fn set_module_level(&mut self, module: u8, level: f32) {
        if (module as usize) < MAX_TRACKS {
            self.tracks[module as usize].level = level.clamp(0.0, 3.0);
        }
    }

    /// Set per-track stereo pan (-1.0 left to 1.0 right).
    pub fn set_module_pan(&mut self, module: u8, pan: f32) {
        if (module as usize) < MAX_TRACKS {
            self.tracks[module as usize].pan = pan.clamp(-1.0, 1.0);
        }
    }

    /// Set per-track delay send amount.
    pub fn set_module_delay_send(&mut self, module: u8, amount: f32) {
        if (module as usize) < MAX_TRACKS {
            self.tracks[module as usize].delay_send = amount.clamp(0.0, 1.0);
        }
    }

    /// Set per-track reverb send amount.
    pub fn set_module_reverb_send(&mut self, module: u8, amount: f32) {
        if (module as usize) < MAX_TRACKS {
            self.tracks[module as usize].reverb_send = amount.clamp(0.0, 1.0);
        }
    }

    /// Check if a track should be audible given mute/solo state.
    fn is_track_audible(&self, track: usize) -> bool {
        if track >= MAX_TRACKS { return false; }
        if self.tracks[track].muted { return false; }
        let any_solo = self.tracks.iter().any(|t| t.solo);
        if any_solo && !self.tracks[track].solo { return false; }
        true
    }

    /// Backward compat: module_mute as a fixed-size slice view.
    pub fn module_mute_flags(&self) -> [bool; MAX_TRACKS] {
        let mut out = [false; MAX_TRACKS];
        for i in 0..MAX_TRACKS {
            out[i] = self.tracks[i].muted;
        }
        out
    }

    pub fn set_bpm(&mut self, bpm: f32) {
        self.sequencer.set_bpm(bpm);
        self.bass.set_bpm(bpm);
        self.bass2.set_bpm(bpm);
        self.keys.set_bpm(bpm);
        self.keys2.set_bpm(bpm);
        self.fm.set_bpm(bpm);
        self.fm2.set_bpm(bpm);
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
        self.insert_eqs[0].set_tilt(amount);
    }

    pub fn set_eq_low(&mut self, gain_db: f32) {
        self.insert_three_band_eqs[0].set_low(gain_db);
    }

    pub fn set_eq_mid(&mut self, gain_db: f32) {
        self.insert_three_band_eqs[0].set_mid(gain_db);
    }

    pub fn set_eq_high(&mut self, gain_db: f32) {
        self.insert_three_band_eqs[0].set_high(gain_db);
    }

    pub fn set_limiter_threshold(&mut self, threshold: f32) {
        self.insert_limiters[0].set_threshold(threshold);
    }

    pub fn set_limiter_release(&mut self, ms: f32) {
        self.insert_limiters[0].set_release(ms, SAMPLE_RATE);
    }

    pub fn set_limiter_makeup(&mut self, gain: f32) {
        self.insert_limiters[0].set_makeup_gain(gain);
    }

    pub fn set_compressor_threshold(&mut self, db: f32) {
        self.insert_compressors[0].set_threshold(db);
    }

    pub fn set_compressor_ratio(&mut self, ratio: f32) {
        self.insert_compressors[0].set_ratio(ratio);
    }

    pub fn set_compressor_attack(&mut self, ms: f32) {
        self.insert_compressors[0].set_attack(ms);
    }

    pub fn set_compressor_release(&mut self, ms: f32) {
        self.insert_compressors[0].set_release(ms);
    }

    pub fn set_compressor_makeup(&mut self, gain: f32) {
        self.insert_compressors[0].set_makeup(gain);
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
        self.bass2.reset();
        self.keys.reset();
        self.keys2.reset();
        self.fm.reset();
        self.fm2.reset();
        // Don't reset beats — drums are one-shot, they'll decay naturally
    }

    pub fn reset(&mut self) {
        self.bass.reset();
        self.bass2.reset();
        self.fm.reset();
        self.fm2.reset();
        self.keys.reset();
        self.keys2.reset();
        self.beats.reset();
        self.sequencer.reset();
        self.delay.reset();
        self.reverb.reset();
        self.delay_lfo.reset();
        self.sc_envelope = 0.0;
        self.pitch_bend = 0.0;
        self.param_slides = [None; MAX_PARAM_SLIDES];
        self.param_values = [0.0; 256];
        for t in self.tracks.iter_mut() {
            t.muted = false;
            t.solo = false;
            t.delay_send = 0.0;
            t.reverb_send = 0.0;
        }
        for f in self.insert_filters.iter_mut() { f.reset(); }
        for c in self.insert_choruses.iter_mut() { c.reset(); }
        for c in self.insert_compressors.iter_mut() { c.reset(); }
        for e in self.insert_eqs.iter_mut() { e.reset(); }
        for d in self.insert_delays.iter_mut() { d.reset(); }
        for r in self.insert_reverbs.iter_mut() { r.reset(); }
        for l in self.insert_limiters.iter_mut() { l.reset(); }
        for e in self.insert_three_band_eqs.iter_mut() { e.reset(); }
        for b in self.insert_bitcrushers.iter_mut() { b.reset(); }
        for t in self.insert_tape_stops.iter_mut() { t.reset(); }
    }

    /// Flush all effects buffers (delay, reverb, master chain FX).
    /// Call on stop to silence lingering tails.
    pub fn flush_effects(&mut self) {
        self.delay.reset();
        self.reverb.reset();
        self.sc_envelope = 0.0;
        // Reset master chain pool instances
        for slot in self.master_fx {
            if slot.fx_type == InsertFxType::None { continue; }
            let idx = slot.instance_idx as usize;
            match slot.fx_type {
                InsertFxType::Compressor => { if idx < self.insert_compressors.len() { self.insert_compressors[idx].reset(); } }
                InsertFxType::Limiter => { if idx < self.insert_limiters.len() { self.insert_limiters[idx].reset(); } }
                InsertFxType::Delay => { if idx < self.insert_delays.len() { self.insert_delays[idx].reset(); } }
                InsertFxType::Reverb => { if idx < self.insert_reverbs.len() { self.insert_reverbs[idx].reset(); } }
                InsertFxType::TiltEq => { if idx < self.insert_eqs.len() { self.insert_eqs[idx].reset(); } }
                InsertFxType::ThreeBandEq => { if idx < self.insert_three_band_eqs.len() { self.insert_three_band_eqs[idx].reset(); } }
                InsertFxType::TapeStop => { if idx < self.insert_tape_stops.len() { self.insert_tape_stops[idx].reset(); } }
                InsertFxType::Bitcrusher => { if idx < self.insert_bitcrushers.len() { self.insert_bitcrushers[idx].reset(); } }
                _ => {}
            }
        }
    }

    /// Execute a single Command. This is the universal entry point for all
    /// engine actions — every host (WASM, VST, native) routes through here.
    pub fn execute(&mut self, cmd: crate::command::Command) {
        use crate::command::Command;
        use crate::sequencer::Step;
        use crate::harmony::{Scale};

        match cmd {
            // ── Transport ──
            Command::Start => {
                self.all_notes_off();
                self.sequencer.start();
            }
            Command::Stop => {
                self.sequencer.stop();
                self.all_notes_off();
                self.flush_effects();
            }
            Command::Reset => self.reset(),
            Command::SetBpm(bpm) => self.set_bpm(bpm),

            // ── Notes (module = track index) ──
            Command::NoteOn { module, note, velocity } => {
                let t = module as usize;
                if t < MAX_TRACKS {
                    self.track_note_on(t, note, velocity, false);
                }
            }
            Command::NoteOff { module, note } => {
                let t = module as usize;
                if t < MAX_TRACKS {
                    self.track_note_off(t, note);
                }
            }
            Command::AllNotesOff => self.all_notes_off(),

            // ── Params ──
            Command::SetParam { id, value } => self.apply_param_lock(id, value),

            // ── Sequencer: melodic tracks ──
            Command::SetStep { track, idx, note, velocity, gate } => {
                let i = idx as usize;
                if i < 16 {
                    match track {
                        0..=2 => self.sequencer.tracks[track as usize].steps[i] = Step::new(note, velocity, gate),
                        _ => {}
                    }
                }
            }
            Command::ClearStep { track, idx } => {
                let i = idx as usize;
                if i < 16 {
                    match track {
                        0..=2 => self.sequencer.tracks[track as usize].steps[i] = Step::empty(),
                        _ => {}
                    }
                }
            }
            Command::SetStepSlide { track, idx, slide } => {
                let i = idx as usize;
                if i < 16 && (track as usize) < 3 {
                    self.sequencer.tracks[track as usize].steps[i].slide = slide;
                }
            }
            Command::SetStepLock { track, idx, param_id, value } => {
                let i = idx as usize;
                if i < 16 && (track as usize) < 3 {
                    let s = &mut self.sequencer.tracks[track as usize].steps[i];
                    // Update existing lock or find empty slot
                    for slot in s.locks.iter_mut() {
                        if let Some((id, _)) = slot {
                            if *id == param_id {
                                *slot = Some((param_id, value));
                                return;
                            }
                        }
                    }
                    for slot in s.locks.iter_mut() {
                        if slot.is_none() {
                            *slot = Some((param_id, value));
                            return;
                        }
                    }
                }
            }
            Command::ClearStepLocks { track, idx } => {
                let i = idx as usize;
                if i < 16 && (track as usize) < 3 {
                    self.sequencer.tracks[track as usize].steps[i].locks = [None; MAX_LOCKS_PER_STEP];
                }
            }
            Command::SetStepLockSlide { track, idx, lock_slide } => {
                let i = idx as usize;
                if i < 16 && (track as usize) < 3 {
                    self.sequencer.tracks[track as usize].steps[i].lock_slide = lock_slide;
                }
            }
            Command::SetStepProbability { track, idx, prob } => {
                let i = idx as usize;
                if i < 16 && (track as usize) < 3 {
                    self.sequencer.tracks[track as usize].steps[i].probability = prob;
                }
            }
            Command::SetStepActive { track, idx, active } => {
                let i = idx as usize;
                if i < 16 && (track as usize) < 3 {
                    self.sequencer.tracks[track as usize].steps[i].active = active;
                }
            }

            // ── Sequencer: drum lanes ──
            Command::SetDrumHit { lane, step, on, velocity } => {
                let l = lane as usize;
                let s = step as usize;
                if l < 6 && s < 16 {
                    self.sequencer.drum_track.lanes[l].on[s] = on;
                    self.sequencer.drum_track.lanes[l].velocity[s] = velocity;
                }
            }
            Command::ClearDrumLane(lane) => {
                let l = lane as usize;
                if l < 6 {
                    self.sequencer.drum_track.lanes[l].on = [false; 16];
                }
            }
            Command::SetDrumLaneNote { lane, note } => {
                let l = lane as usize;
                if l < 6 {
                    self.sequencer.drum_track.lanes[l].note = note;
                }
            }

            // ── Sequencer: global ──
            Command::SetNumSteps(n) => {
                self.sequencer.num_steps = (n as usize).clamp(1, 16);
            }
            Command::SetSwing(v) => self.sequencer.set_swing(v),
            Command::SetHumanize(v) => self.sequencer.set_humanize(v),
            Command::SetTrackEnabled { track, enabled } => {
                match track {
                    0..=2 => self.sequencer.tracks[track as usize].enabled = enabled,
                    3 => self.sequencer.drum_track.enabled = enabled,
                    _ => {}
                }
            }

            // ── Harmony ──
            Command::SetHarmony { root, scale, degree } => {
                let s = match scale {
                    0 => Scale::Major,
                    1 => Scale::Minor,
                    2 => Scale::Dorian,
                    3 => Scale::Mixolydian,
                    4 => Scale::PentatonicMinor,
                    _ => Scale::Minor,
                };
                let mut ctx = HarmonyContext::new(root, s);
                ctx.set_chord_degree(degree);
                self.set_harmony(ctx);
            }

            // ── Effects ──
            Command::SetDelayTime(v) => self.set_delay_time(v),
            Command::SetDelayFeedback(v) => self.set_delay_feedback(v),
            Command::SetDelayFilter(v) => self.set_delay_filter(v),
            Command::SetDelayMix(v) => self.delay.set_mix(v),
            Command::SetDelaySync(v) => {
                let sync = match v {
                    1 => DelaySync::Quarter,
                    2 => DelaySync::Eighth,
                    3 => DelaySync::DottedEighth,
                    4 => DelaySync::TripletEighth,
                    5 => DelaySync::Sixteenth,
                    _ => DelaySync::Free,
                };
                self.set_delay_sync(sync);
            }
            Command::SetTiltEq(v) => self.insert_eqs[0].set_tilt(v),
            Command::SetReverbSize(v) => self.reverb.set_room_size(v),
            Command::SetReverbDamping(v) => self.reverb.set_damping(v),
            Command::SetReverbMix(v) => self.reverb.set_mix(v),
            Command::SetEqLow(v) => self.insert_three_band_eqs[0].set_low(v),
            Command::SetEqMid(v) => self.insert_three_band_eqs[0].set_mid(v),
            Command::SetEqHigh(v) => self.insert_three_band_eqs[0].set_high(v),
            Command::SetCompThreshold(v) => self.insert_compressors[0].set_threshold(v),
            Command::SetCompRatio(v) => self.insert_compressors[0].set_ratio(v),
            Command::SetCompAttack(v) => self.insert_compressors[0].set_attack(v),
            Command::SetCompRelease(v) => self.insert_compressors[0].set_release(v),
            Command::SetCompMakeup(v) => self.insert_compressors[0].set_makeup(v),
            Command::SetSidechain(v) => self.set_sidechain_amount(v),
            Command::SetPitchBend(v) => self.set_pitch_bend(v),

            // ── Module mute/solo ──
            Command::SetMute { module, muted } => self.set_module_mute(module, muted),
            Command::SetSolo { module, solo } => self.set_module_solo(module, solo),

            // Limiter (backward compat — routes to pool instance 0)
            Command::SetLimiterThreshold(v) => self.insert_limiters[0].set_threshold(v),
            Command::SetLimiterRelease(v) => self.insert_limiters[0].set_release(v, SAMPLE_RATE),
            Command::SetLimiterMakeup(v) => self.insert_limiters[0].set_makeup_gain(v),

            // Module level & pan
            Command::SetModuleLevel { module, level } => self.set_module_level(module, level),
            Command::SetModulePan { module, pan } => self.set_module_pan(module, pan),

            // Master
            Command::SetMasterLevel(v) => self.master_level = v.clamp(0.0, 2.0),

            // Bend range
            Command::SetBendRange(v) => self.bend_range = v.clamp(0.0, 24.0),

            // Reverb pre-delay
            Command::SetReverbPreDelay(v) => self.reverb.set_pre_delay(v),

            // Delay LFO
            Command::SetDelayLfoRate(v) => self.set_delay_lfo_rate(v),
            Command::SetDelayLfoDepth(v) => self.set_delay_lfo_depth(v),

            // Per-module sends
            Command::SetModuleDelaySend { module, amount } => self.set_module_delay_send(module, amount),
            Command::SetModuleReverbSend { module, amount } => self.set_module_reverb_send(module, amount),

            // Track routing
            Command::SetTrackInstrument { track, kind } => {
                let t = track as usize;
                if t < MAX_TRACKS {
                    self.tracks[t].kind = match kind {
                        0 => InstrumentKind::Bass,
                        1 => InstrumentKind::Keys,
                        2 => InstrumentKind::Fm,
                        3 => InstrumentKind::Beats,
                        _ => InstrumentKind::None,
                    };
                }
            }
            Command::SetTrackArpEnabled { track, enabled } => {
                let t = track as usize;
                if t < MAX_TRACKS {
                    if enabled && self.tracks[t].arp.is_none() {
                        use crate::primitives::arp_processor::ArpProcessor;
                        self.tracks[t].arp = Some(ArpProcessor::new());
                    } else if !enabled {
                        self.tracks[t].arp = None;
                    }
                }
            }
            Command::SetTrackArpRate { track, rate } => {
                let t = track as usize;
                if t < MAX_TRACKS {
                    if let Some(ref mut arp) = self.tracks[t].arp {
                        arp.set_rate(rate);
                    }
                }
            }
            Command::SetTrackArpGate { track, gate } => {
                let t = track as usize;
                if t < MAX_TRACKS {
                    if let Some(ref mut arp) = self.tracks[t].arp {
                        arp.set_gate(gate);
                    }
                }
            }
            Command::SetTrackArpPattern { track, pattern } => {
                let t = track as usize;
                if t < MAX_TRACKS {
                    if let Some(ref mut arp) = self.tracks[t].arp {
                        arp.set_pattern(pattern);
                    }
                }
            }
            Command::SetTrackArpOctaves { track, octaves } => {
                let t = track as usize;
                if t < MAX_TRACKS {
                    if let Some(ref mut arp) = self.tracks[t].arp {
                        arp.set_octave_range(octaves);
                    }
                }
            }
            Command::SetTrackParam { track, param, value } => {
                self.apply_track_param(track, param, value);
            }

            // Per-track insert FX
            Command::SetTrackInsertFx { track, slot, fx_type } => {
                let t = track as usize;
                let s = slot as usize;
                if t < MAX_TRACKS && s < MAX_INSERT_FX {
                    let new_type = InsertFxType::from_u8(fx_type);
                    self.tracks[t].insert_fx[s].fx_type = new_type;
                    self.tracks[t].insert_fx[s].enabled = new_type != InsertFxType::None;
                }
            }
            Command::SetTrackInsertParam { track, slot, param_idx, value } => {
                self.apply_insert_param(track, slot, param_idx, value);
            }
            Command::SetTrackInsertEnabled { track, slot, enabled } => {
                let t = track as usize;
                let s = slot as usize;
                if t < MAX_TRACKS && s < MAX_INSERT_FX {
                    self.tracks[t].insert_fx[s].enabled = enabled;
                }
            }
            Command::ClearTrackInsert { track, slot } => {
                let t = track as usize;
                let s = slot as usize;
                if t < MAX_TRACKS && s < MAX_INSERT_FX {
                    self.tracks[t].insert_fx[s] = crate::track::InsertFxSlot::empty();
                }
            }
            Command::MoveTrackInsert { track, from_slot, to_slot } => {
                let t = track as usize;
                let f = from_slot as usize;
                let to = to_slot as usize;
                if t < MAX_TRACKS && f < MAX_INSERT_FX && to < MAX_INSERT_FX {
                    self.tracks[t].insert_fx.swap(f, to);
                }
            }
            Command::SetTrackInsertInstance { track, slot, instance } => {
                let t = track as usize;
                let s = slot as usize;
                if t < MAX_TRACKS && s < MAX_INSERT_FX {
                    self.tracks[t].insert_fx[s].instance_idx = instance;
                }
            }

            // ── Master insert FX ──
            Command::SetMasterInsertFx { slot, fx_type } => {
                let s = slot as usize;
                if s < MAX_MASTER_FX {
                    let new_type = InsertFxType::from_u8(fx_type);
                    self.master_fx[s].fx_type = new_type;
                    self.master_fx[s].enabled = new_type != InsertFxType::None;
                }
            }
            Command::SetMasterInsertParam { slot, param_idx, value } => {
                self.apply_master_insert_param(slot, param_idx, value);
            }
            Command::SetMasterInsertEnabled { slot, enabled } => {
                let s = slot as usize;
                if s < MAX_MASTER_FX {
                    self.master_fx[s].enabled = enabled;
                }
            }
            Command::ClearMasterInsert { slot } => {
                let s = slot as usize;
                if s < MAX_MASTER_FX {
                    self.master_fx[s] = InsertFxSlot::empty();
                }
            }
        }
    }

    /// Apply a parameter to a specific track's instrument.
    pub fn apply_track_param(&mut self, track: u8, param: u8, value: f32) {
        let t = track as usize;
        if t >= MAX_TRACKS { return; }

        match (self.tracks[t].kind, self.tracks[t].instance_idx) {
            (InstrumentKind::Bass, 0) => {
                if let Some(p) = Self::bass_param_from_local(param) {
                    self.bass.set_param(p, value);
                }
            }
            (InstrumentKind::Bass, _) => {
                if let Some(p) = Self::bass_param_from_local(param) {
                    self.bass2.set_param(p, value);
                }
            }
            (InstrumentKind::Keys, 0) => {
                if let Some(p) = Self::keys_param_from_local(param) {
                    self.keys.set_param(p, value);
                }
            }
            (InstrumentKind::Keys, _) => {
                if let Some(p) = Self::keys_param_from_local(param) {
                    self.keys2.set_param(p, value);
                }
            }
            (InstrumentKind::Fm, 0) => {
                if let Some(p) = Self::fm_param_from_local(param) {
                    self.fm.set_param(p, value);
                }
            }
            (InstrumentKind::Fm, _) => {
                if let Some(p) = Self::fm_param_from_local(param) {
                    self.fm2.set_param(p, value);
                }
            }
            (InstrumentKind::Beats, _) => {
                if let Some(p) = Self::beats_param_from_local(param) {
                    self.beats.set_param(p, value);
                }
            }
            _ => {}
        }
    }

    /// Apply a parameter to a specific insert FX instance.
    /// param_idx: 0=primary (cutoff/drive/mix), 1=secondary (resonance/feedback), 2=tertiary
    pub fn apply_insert_param(&mut self, track: u8, slot: u8, param_idx: u8, value: f32) {
        use crate::primitives::filter::FilterType;
        let t = track as usize;
        let s = slot as usize;
        if t >= MAX_TRACKS || s >= MAX_INSERT_FX { return; }
        let fx_slot = self.tracks[t].insert_fx[s];
        let idx = fx_slot.instance_idx as usize;

        match fx_slot.fx_type {
            InsertFxType::Filter => {
                if idx < self.insert_filters.len() {
                    match param_idx {
                        0 => { // cutoff (0.0-1.0 → 20-20000Hz)
                            let freq = 20.0 * crate::math::pow(1000.0, value);
                            self.insert_filters[idx].set_cutoff(freq);
                            // stereo pair
                            if idx + 1 < self.insert_filters.len() {
                                self.insert_filters[idx + 1].set_cutoff(freq);
                            }
                        }
                        1 => { // resonance (0.0-1.0)
                            let freq = 20.0 * crate::math::pow(1000.0, 0.5); // keep current
                            self.insert_filters[idx].set_params(FilterType::LowPass, freq, value);
                            if idx + 1 < self.insert_filters.len() {
                                self.insert_filters[idx + 1].set_params(FilterType::LowPass, freq, value);
                            }
                        }
                        2 => { // filter type: 0=LP, 0.5=HP, 1.0=BP
                            let ft = if value < 0.33 { FilterType::LowPass }
                                     else if value < 0.66 { FilterType::HighPass }
                                     else { FilterType::BandPass };
                            self.insert_filters[idx].set_params(ft, 1000.0, 0.5);
                            if idx + 1 < self.insert_filters.len() {
                                self.insert_filters[idx + 1].set_params(ft, 1000.0, 0.5);
                            }
                        }
                        _ => {}
                    }
                }
            }
            InsertFxType::Saturator => {
                if idx < self.insert_saturators.len() {
                    match param_idx {
                        0 => self.insert_saturators[idx].set_drive(0.1 + value * 5.0), // drive
                        _ => {}
                    }
                }
            }
            InsertFxType::Chorus => {
                if idx < self.insert_choruses.len() {
                    match param_idx {
                        0 => self.insert_choruses[idx].set_mix(value), // mix
                        _ => {}
                    }
                }
            }
            InsertFxType::TiltEq => {
                if idx < self.insert_eqs.len() {
                    match param_idx {
                        0 => self.insert_eqs[idx].set_tilt(value * 2.0 - 1.0), // -1..+1
                        _ => {}
                    }
                }
            }
            InsertFxType::Compressor => {
                if idx < self.insert_compressors.len() {
                    match param_idx {
                        0 => self.insert_compressors[idx].set_threshold(value * -40.0), // 0..-40dB
                        1 => self.insert_compressors[idx].set_ratio(1.0 + value * 19.0), // 1..20
                        2 => self.insert_compressors[idx].set_attack(0.1 + value * 99.9), // ms
                        _ => {}
                    }
                }
            }
            InsertFxType::Delay => {
                if idx < self.insert_delays.len() {
                    match param_idx {
                        0 => self.insert_delays[idx].set_time(value, SAMPLE_RATE), // time
                        1 => self.insert_delays[idx].set_feedback(value), // feedback
                        2 => self.insert_delays[idx].set_mix(value), // mix
                        _ => {}
                    }
                }
            }
            InsertFxType::Reverb => {
                if idx < self.insert_reverbs.len() {
                    match param_idx {
                        0 => self.insert_reverbs[idx].set_room_size(value),
                        1 => self.insert_reverbs[idx].set_damping(value),
                        2 => self.insert_reverbs[idx].set_mix(value),
                        _ => {}
                    }
                }
            }
            InsertFxType::Limiter => {
                if idx < self.insert_limiters.len() {
                    match param_idx {
                        0 => self.insert_limiters[idx].set_threshold(value), // 0..1
                        1 => self.insert_limiters[idx].set_release(value * 500.0, SAMPLE_RATE), // 0..500ms
                        2 => self.insert_limiters[idx].set_makeup_gain(value * 6.0), // 0..6
                        _ => {}
                    }
                }
            }
            InsertFxType::ThreeBandEq => {
                if idx < self.insert_three_band_eqs.len() {
                    match param_idx {
                        0 => self.insert_three_band_eqs[idx].set_low(value * 24.0 - 12.0), // -12..+12dB
                        1 => self.insert_three_band_eqs[idx].set_mid(value * 24.0 - 12.0),
                        2 => self.insert_three_band_eqs[idx].set_high(value * 24.0 - 12.0),
                        _ => {}
                    }
                }
            }
            InsertFxType::Bitcrusher => {
                if idx < self.insert_bitcrushers.len() {
                    match param_idx {
                        0 => self.insert_bitcrushers[idx].set_bit_depth(1.0 + value * 15.0), // 1..16 bits
                        1 => self.insert_bitcrushers[idx].set_rate_reduce(value), // 0..1
                        2 => self.insert_bitcrushers[idx].set_mix(value), // 0..1
                        _ => {}
                    }
                }
            }
            InsertFxType::TapeStop => {
                if idx < self.insert_tape_stops.len() {
                    match param_idx {
                        0 => self.insert_tape_stops[idx].set_ramp_time(0.05 + value * 2.95), // 0.05..3.0s
                        1 => self.insert_tape_stops[idx].set_mix(value), // 0..1
                        2 => self.insert_tape_stops[idx].trigger(value > 0.5), // on/off
                        _ => {}
                    }
                }
            }
            InsertFxType::None => {}
        }
    }

    /// Apply a parameter to a master FX slot's pool instance.
    pub fn apply_master_insert_param(&mut self, slot: u8, param_idx: u8, value: f32) {
        let s = slot as usize;
        if s >= MAX_MASTER_FX { return; }
        let fx_slot = self.master_fx[s];
        let idx = fx_slot.instance_idx as usize;
        // Reuse the same param routing as track insert FX
        // We use a fake track=255 to distinguish, but the logic is the same
        match fx_slot.fx_type {
            InsertFxType::Filter => {
                use crate::primitives::filter::FilterType;
                if idx < self.insert_filters.len() {
                    match param_idx {
                        0 => {
                            let freq = 20.0 * crate::math::pow(1000.0, value);
                            self.insert_filters[idx].set_cutoff(freq);
                            if idx + 1 < self.insert_filters.len() {
                                self.insert_filters[idx + 1].set_cutoff(freq);
                            }
                        }
                        1 => {
                            let freq = 20.0 * crate::math::pow(1000.0, 0.5);
                            self.insert_filters[idx].set_params(FilterType::LowPass, freq, value);
                            if idx + 1 < self.insert_filters.len() {
                                self.insert_filters[idx + 1].set_params(FilterType::LowPass, freq, value);
                            }
                        }
                        _ => {}
                    }
                }
            }
            InsertFxType::Saturator => {
                if idx < self.insert_saturators.len() {
                    match param_idx {
                        0 => self.insert_saturators[idx].set_drive(0.1 + value * 5.0),
                        _ => {}
                    }
                }
            }
            InsertFxType::Chorus => {
                if idx < self.insert_choruses.len() {
                    match param_idx {
                        0 => self.insert_choruses[idx].set_mix(value),
                        _ => {}
                    }
                }
            }
            InsertFxType::TiltEq => {
                if idx < self.insert_eqs.len() {
                    match param_idx {
                        0 => self.insert_eqs[idx].set_tilt(value * 2.0 - 1.0),
                        _ => {}
                    }
                }
            }
            InsertFxType::Compressor => {
                if idx < self.insert_compressors.len() {
                    match param_idx {
                        0 => self.insert_compressors[idx].set_threshold(value * -40.0),
                        1 => self.insert_compressors[idx].set_ratio(1.0 + value * 19.0),
                        2 => self.insert_compressors[idx].set_attack(0.1 + value * 99.9),
                        3 => self.insert_compressors[idx].set_release(10.0 + value * 490.0),
                        _ => {}
                    }
                }
            }
            InsertFxType::Delay => {
                if idx < self.insert_delays.len() {
                    match param_idx {
                        0 => self.insert_delays[idx].set_time(value, SAMPLE_RATE),
                        1 => self.insert_delays[idx].set_feedback(value),
                        2 => self.insert_delays[idx].set_mix(value),
                        _ => {}
                    }
                }
            }
            InsertFxType::Reverb => {
                if idx < self.insert_reverbs.len() {
                    match param_idx {
                        0 => self.insert_reverbs[idx].set_room_size(value),
                        1 => self.insert_reverbs[idx].set_damping(value),
                        2 => self.insert_reverbs[idx].set_mix(value),
                        _ => {}
                    }
                }
            }
            InsertFxType::Limiter => {
                if idx < self.insert_limiters.len() {
                    match param_idx {
                        0 => self.insert_limiters[idx].set_threshold(value),
                        1 => self.insert_limiters[idx].set_release(value * 500.0, SAMPLE_RATE),
                        2 => self.insert_limiters[idx].set_makeup_gain(value * 6.0),
                        _ => {}
                    }
                }
            }
            InsertFxType::ThreeBandEq => {
                if idx < self.insert_three_band_eqs.len() {
                    match param_idx {
                        0 => self.insert_three_band_eqs[idx].set_low(value * 24.0 - 12.0),
                        1 => self.insert_three_band_eqs[idx].set_mid(value * 24.0 - 12.0),
                        2 => self.insert_three_band_eqs[idx].set_high(value * 24.0 - 12.0),
                        _ => {}
                    }
                }
            }
            InsertFxType::Bitcrusher => {
                if idx < self.insert_bitcrushers.len() {
                    match param_idx {
                        0 => self.insert_bitcrushers[idx].set_bit_depth(1.0 + value * 15.0),
                        1 => self.insert_bitcrushers[idx].set_rate_reduce(value),
                        2 => self.insert_bitcrushers[idx].set_mix(value),
                        _ => {}
                    }
                }
            }
            InsertFxType::TapeStop => {
                if idx < self.insert_tape_stops.len() {
                    match param_idx {
                        0 => self.insert_tape_stops[idx].set_ramp_time(0.05 + value * 2.95),
                        1 => self.insert_tape_stops[idx].set_mix(value),
                        2 => self.insert_tape_stops[idx].trigger(value > 0.5),
                        _ => {}
                    }
                }
            }
            InsertFxType::None => {}
        }
    }

    fn bass_param_from_local(p: u8) -> Option<crate::modules::bass::BassParam> {
        use crate::modules::bass::BassParam;
        Some(match p {
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
            18 => BassParam::Decay,
            19 => BassParam::Sustain,
            20 => BassParam::Release,
            21 => BassParam::VelEnv,
            _ => return None,
        })
    }

    fn keys_param_from_local(p: u8) -> Option<crate::modules::keys::KeysParam> {
        use crate::modules::keys::KeysParam;
        Some(match p {
            0 => KeysParam::Cutoff,
            1 => KeysParam::Detune,
            2 => KeysParam::ChorusMix,
            3 => KeysParam::Level,
            4 => KeysParam::LfoRate,
            5 => KeysParam::LfoDepth,
            6 => KeysParam::LfoWaveform,
            7 => KeysParam::LfoTarget,
            8 => KeysParam::LfoSync,
            9 => KeysParam::VoiceMode,
            10 => KeysParam::VibratoRate,
            11 => KeysParam::VibratoDepth,
            12 => KeysParam::Attack,
            13 => KeysParam::Decay,
            14 => KeysParam::Sustain,
            15 => KeysParam::Release,
            16 => KeysParam::Resonance,
            _ => return None,
        })
    }

    fn fm_param_from_local(p: u8) -> Option<crate::modules::fm::FmParam> {
        use crate::modules::fm::FmParam;
        Some(match p {
            0 => FmParam::Algorithm,
            1 => FmParam::ModIndex,
            2 => FmParam::LfoRate,
            3 => FmParam::LfoDepth,
            4 => FmParam::LfoWaveform,
            5 => FmParam::LfoTarget,
            6 => FmParam::LfoSync,
            7 => FmParam::Feedback,
            8 => FmParam::Waveform,
            9 => FmParam::ChorusMix,
            10 => FmParam::VibratoRate,
            11 => FmParam::VibratoDepth,
            12 => FmParam::Attack,
            13 => FmParam::Decay,
            14 => FmParam::Sustain,
            15 => FmParam::Release,
            16 => FmParam::Op0Attack, 17 => FmParam::Op0Decay,
            18 => FmParam::Op0Sustain, 19 => FmParam::Op0Release,
            20 => FmParam::Op1Attack, 21 => FmParam::Op1Decay,
            22 => FmParam::Op1Sustain, 23 => FmParam::Op1Release,
            24 => FmParam::Op2Attack, 25 => FmParam::Op2Decay,
            26 => FmParam::Op2Sustain, 27 => FmParam::Op2Release,
            28 => FmParam::Op3Attack, 29 => FmParam::Op3Decay,
            30 => FmParam::Op3Sustain, 31 => FmParam::Op3Release,
            _ => return None,
        })
    }

    fn beats_param_from_local(p: u8) -> Option<BeatsParam> {
        Some(match p {
            0 => BeatsParam::Level,
            1 => BeatsParam::KickDecay,
            2 => BeatsParam::SnareDecay,
            3 => BeatsParam::KickPan,
            4 => BeatsParam::SnarePan,
            5 => BeatsParam::HihatPan,
            6 => BeatsParam::ClapPan,
            7 => BeatsParam::KickClick,
            8 => BeatsParam::KickLevel,
            9 => BeatsParam::SnareLevel,
            10 => BeatsParam::HihatLevel,
            11 => BeatsParam::ClapLevel,
            12 => BeatsParam::KickPitch,
            13 => BeatsParam::SnarePitch,
            14 => BeatsParam::HihatPitch,
            15 => BeatsParam::StutterRate,
            16 => BeatsParam::TomPan,
            17 => BeatsParam::CrashPan,
            _ => return None,
        })
    }

}
