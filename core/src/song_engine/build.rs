//! Building an engine: from source text or a compiled song to instruments,
//! tracks, buses, sends and master, ready to start.

use alloc::string::String;
use alloc::vec::Vec;

use crate::analysis::BandMeter;
use crate::dsl::compiler::{self, *};
use crate::effects::delay::{Delay, DelaySync};
use crate::effects::reverb::Reverb;
use crate::graph::voice::Instrument;
use crate::modules::bass::BassModule;
use crate::modules::beats::BeatsModule;
use crate::modules::fm::FmModule;
use crate::modules::keys::KeysModule;
use crate::params::{self, ParamId, ModuleKind};
use crate::output::OutputStage;
use crate::rng::Rng;
use crate::{math, BLOCK_SIZE, SAMPLE_RATE};

use super::automation::InlineName;
use super::bus::SongBus;
use super::fx_chain::FxChain;
use super::instrument::SongInstrument;
use super::sequencer::MAX_PENDING_TRIGGERS;
use super::track::{make_arp, pan_gains, seed_from_name, Heard, TrackPlayback};
use super::{DslError, SongEngine};

/// Share of a new peak the follower takes in one sample.
///
/// The default used to be 0.005 ms, which reproduced an old hardcoded 0.99:
/// effectively instant. An instant duck halves a sustained pad inside six
/// samples, and that step is a click on every kick -- `tatum debug` counted
/// fifty on one drone. One millisecond is too short to hear as a softer pump
/// and long enough to take the corner off.
const DEFAULT_SC_ATTACK_MS: f32 = 1.0;

fn attack_alpha(ms: f32) -> f32 {
    let samples = (ms * 0.001 * SAMPLE_RATE).max(0.001);
    1.0 - math::exp(-1.0 / samples)
}

/// What the envelope keeps per sample while it falls. The 4.5 ms default
/// reproduces the 0.995 that used to be hardcoded.
fn release_coeff(ms: f32) -> f32 {
    let samples = (ms * 0.001 * SAMPLE_RATE).max(1.0);
    math::exp(-1.0 / samples)
}

impl SongEngine {
    /// Load a DSL song from source text.
    pub fn from_source(source: &str) -> Result<Self, alloc::string::String> {
        let ast = crate::dsl::parse(source).map_err(|errs| {
            let mut msg = alloc::string::String::from("parse errors:\n");
            for e in &errs {
                msg.push_str(&alloc::format!("  line {}: {}\n", e.line, e.message));
            }
            msg
        })?;
        let compiled = crate::dsl::compiler::compile(&ast).map_err(|errs| {
            let mut msg = alloc::string::String::from("compile errors:\n");
            for e in &errs {
                msg.push_str(&alloc::format!("  {}\n", e.message));
            }
            msg
        })?;
        Ok(Self::from_compiled(compiled))
    }

    /// Load from source with structured errors (for WASM bridge).
    pub fn try_from_source(source: &str) -> Result<Self, DslError> {
        let ast = crate::dsl::parse(source).map_err(DslError::Parse)?;
        let compiled = crate::dsl::compiler::compile(&ast).map_err(DslError::Compile)?;
        Ok(Self::from_compiled(compiled))
    }

    /// Create from an already-compiled song.
    pub fn from_compiled(song: CompiledSong) -> Self {
        // Build instruments from compiled instrument kinds
        // The parser refuses anything outside these spans; a song built
        // some other way is held to the same ones the setters use, so a
        // tempo of 0 cannot make a step infinitely long.
        let tempo = song.globals.tempo.clamp(20.0, 999.0);
        let mut instruments: Vec<SongInstrument> = song.instruments
            .iter()
            .map(|kind| Self::build_instrument(kind, tempo))
            .collect();

        // One instrument, one track. Two tracks naming the same module used to
        // share a single instance, and the render loop then called it once per
        // track: each track got every other block of one stream, with the
        // envelopes, LFOs and vibrato running at twice their written rate. A
        // 1.2 s attack measured 0.6 s and the discarded half left a comb at the
        // block rate (344.5 Hz), four times the energy a track that owned its
        // module had there. Extra users get their own copy, appended past the
        // compiled indices so `inherit_from` still lines up.
        //
        // A scene can point a track at another module, so a copy is owed to
        // each (track, module) pair the song actually names, top level and in
        // every scene. `inst_for` is that table, flat, one row per track.
        let n_inst = song.instruments.len();
        let n_tracks = song.tracks.len();
        let mut inst_for = vec![usize::MAX; n_tracks * n_inst];
        let mut claimed = vec![false; n_inst];
        let mut instrument_names = song.instrument_names.clone();

        let mut pairs: Vec<(usize, usize)> = Vec::new();
        for (ti, t) in song.tracks.iter().enumerate() {
            pairs.push((ti, t.instrument_idx));
        }
        for scene in &song.scenes {
            for st in &scene.tracks {
                if let Some(ti) = song.tracks.iter().position(|t| t.name == st.name) {
                    pairs.push((ti, st.instrument_idx));
                }
            }
        }
        for (ti, ii) in pairs {
            if ti >= n_tracks || ii >= n_inst { continue; }
            let slot = ti * n_inst + ii;
            if inst_for[slot] != usize::MAX { continue; }
            if !claimed[ii] {
                claimed[ii] = true;
                inst_for[slot] = ii;
            } else {
                instruments.push(Self::build_instrument(&song.instruments[ii], tempo));
                let name = instrument_names.get(ii).cloned().unwrap_or_default();
                instrument_names.push(name);
                inst_for[slot] = instruments.len() - 1;
            }
        }
        // Pairs the song never names still need an answer, in case a lookup
        // arrives for one: the compiled instrument itself.
        for (slot, v) in inst_for.iter_mut().enumerate() {
            if *v == usize::MAX { *v = slot % n_inst.max(1); }
        }

        Self::assemble(song, instruments, instrument_names, inst_for, n_inst, tempo)
    }

    /// Build one live instrument from a compiled preset. Called once per track
    /// that names the module, so each gets its own voices and envelopes.
    fn build_instrument(kind: &CompiledInstrumentKind, tempo: f32) -> SongInstrument {
                match kind.clone() {
                    CompiledInstrumentKind::Graph(template) => {
                        SongInstrument::Graph(Instrument::new(*template))
                    }
                    CompiledInstrumentKind::Bass(preset) => {
                        let mut m = BassModule::new();
                        for (name, value) in &preset.params {
                            if let Some(spec) = params::lookup(ModuleKind::Bass, name) {
                                if let ParamId::Bass(p) = spec.id { m.set_param(p, *value); }
                            }
                        }
                        m.set_bpm(tempo);
                        SongInstrument::Bass(m)
                    }
                    CompiledInstrumentKind::Fm(preset) => {
                        let mut m = FmModule::new();
                        for (name, value) in &preset.params {
                            if let Some(spec) = params::lookup(ModuleKind::Fm, name) {
                                if let ParamId::Fm(p) = spec.id { m.set_param(p, *value); }
                            }
                        }
                        // Apply per-operator envelopes if specified
                        for (op_idx, env) in &preset.op_envelopes {
                            m.set_op_envelope(*op_idx, env.0, env.1, env.2, env.3);
                        }
                        m.set_bpm(tempo);
                        SongInstrument::Fm(m)
                    }
                    CompiledInstrumentKind::Keys(preset) => {
                        let mut m = KeysModule::new();
                        for (name, value) in &preset.params {
                            if let Some(spec) = params::lookup(ModuleKind::Keys, name) {
                                if let ParamId::Keys(p) = spec.id { m.set_param(p, *value); }
                            }
                        }
                        m.set_bpm(tempo);
                        SongInstrument::Keys(m)
                    }
                    CompiledInstrumentKind::Beats(preset) => {
                        let mut m = BeatsModule::new();
                        for (name, value) in &preset.params {
                            if let Some(spec) = params::lookup(ModuleKind::Beats, name) {
                                if let ParamId::Beats(p) = spec.id { m.set_param(p, *value); }
                            }
                        }
                        m.set_bpm(tempo);
                        SongInstrument::Beats(m)
                    }
                }
    }

    /// Everything after the instruments exist: buses, tracks, sends, master.
    fn assemble(
        song: CompiledSong,
        instruments: Vec<SongInstrument>,
        instrument_names: Vec<String>,
        inst_for: Vec<usize>,
        n_inst: usize,
        tempo: f32,
    ) -> Self {
        // Build buses
        let buses: Vec<SongBus> = song.buses
            .iter()
            .map(|b| SongBus::new(b.name.clone(), &b.fx_chain))
            .collect();

        // Build master FX chain
        let master_fx = FxChain::new(&song.master.fx_chain);
        let reverb_return = FxChain::new(&song.reverb_return);
        let delay_return = FxChain::new(&song.delay_return);
        let reverb_sidechain = song.globals.send_reverb.sidechain.unwrap_or(0.0);
        let delay_sidechain = song.globals.send_delay.sidechain.unwrap_or(0.0);

        // Build track playback states from the compiled tracks
        let track_names: Vec<String> = song.tracks.iter().map(|t| t.name.clone()).collect();
        let tracks: Vec<TrackPlayback> = song.tracks
            .iter()
            .enumerate()
            .map(|(ti, t)| {
                let inst_idx = inst_for
                    .get(ti * n_inst + t.instrument_idx)
                    .copied()
                    .unwrap_or(t.instrument_idx);
                let (pan_l, pan_r) = pan_gains(t.pan);
                let stereo_src = inst_idx < instruments.len()
                    && matches!(instruments[inst_idx], SongInstrument::Beats(_));
                TrackPlayback {
                    instrument_idx: inst_idx,
                    pattern_idx: t.pattern_idx,
                    velocity: t.velocity,
                    level: t.level,
                    pan: t.pan,
                    pan_l,
                    pan_r,
                    gate: t.gate,
                    rng: Rng::new(seed_from_name(&t.name)),
                    insert_fx: FxChain::new(&t.insert_fx),
                    fx_labels: t.insert_fx_labels.iter()
                        .map(|l| l.as_deref().and_then(InlineName::new)).collect(),
                    bus_send: t.bus_send,
                    to_master: t.to_master,
                    delay_send: t.delay_send,
                    reverb_send: t.reverb_send,
                    sidechain_amount: t.sidechain,
                    current_step: 0,
                    current_notes: [0; compiler::MAX_CHORD_NOTES],
                    current_notes_count: 0,
                    gate_samples_remaining: 0.0,
                    active: true,
                    leaving: 0,
                    stereo_src,
                    arp: t.arp.map(|c| make_arp(&c, tempo)),
                    arp_cfg: t.arp,
                    meter_peak: 0.0,
                    meter_sum_sq: 0.0,
                    meter_samples: 0,
                    meter_mid_sq: 0.0,
                    meter_side_sq: 0.0,
                    band: BandMeter::new(SAMPLE_RATE),
                    sc_source: None,
                    sc_env: 0.0,
                    is_sc_source: false,
                    heard: Heard {
                        left: t.level * pan_l,
                        right: t.level * pan_r,
                        delay: t.delay_send,
                        reverb: t.reverb_send,
                    },
                    heard_duck: 0.0,
                }
            })
            .collect();

        let meter = (song.globals.meter.0.clamp(1, 16), song.globals.meter.1);
        let sidechain_amount = song.globals.sidechain;
        let steps_per_bar = (meter.0 as usize) * 4; // 4 steps per beat (16th notes)
        let samples_per_step = SAMPLE_RATE * 60.0 / tempo / 4.0; // 16th note duration

        // Auto-detect kick track by instrument name or Beats module
        let kick_track_idx = tracks.iter().position(|t| {
            if t.instrument_idx < instrument_names.len() {
                let name = &instrument_names[t.instrument_idx];
                if name.contains("kick") { return true; }
                // BeatsModule tracks contain kick internally
                if t.instrument_idx < instruments.len()
                    && matches!(instruments[t.instrument_idx], SongInstrument::Beats(_))
                {
                    return true;
                }
            }
            false
        });

        // Global send effects — match reference Engine defaults
        // Defaults: 16th-note delay, small tight room. Overridable with the
        // top-level `delay ...` / `reverb ...` lines.
        let mut send_delay = Delay::new(SAMPLE_RATE, 2.0);
        let d = &song.globals.send_delay;
        let sync = match d.sync.as_deref() {
            Some("free") => DelaySync::Free,
            Some("quarter") => DelaySync::Quarter,
            Some("dotted_eighth") => DelaySync::DottedEighth,
            Some("eighth") => DelaySync::Eighth,
            Some("triplet_eighth") => DelaySync::TripletEighth,
            _ => DelaySync::Sixteenth,
        };
        if let (DelaySync::Free, Some(t)) = (sync, d.time) {
            send_delay.set_time(t.clamp(0.001, 2.0), SAMPLE_RATE);
        }
        send_delay.set_sync(sync, tempo, SAMPLE_RATE);
        send_delay.set_feedback(d.feedback.unwrap_or(0.25));
        send_delay.set_filter(d.filter.unwrap_or(0.6));

        let mut send_reverb = Reverb::new(SAMPLE_RATE);
        let r = &song.globals.send_reverb;
        send_reverb.set_room_size(r.size.unwrap_or(0.3));
        send_reverb.set_damping(r.damp.unwrap_or(0.6));
        if let Some(pd) = r.predelay {
            send_reverb.set_pre_delay(pd);
        }

        // Parse swing/humanize from globals
        let swing = song.globals.swing.unwrap_or(0.5).clamp(0.5, 0.75);
        let humanize_velocity = song.globals.humanize.unwrap_or(0.0).clamp(0.0, 1.0);
        let humanize_timing = song.globals.humanize_timing.unwrap_or(0.0).clamp(0.0, 1.0);

        let track_buf_count = tracks.len();
        let mut engine = Self {
            instruments,
            instrument_names,
            inst_for,
            n_instruments: n_inst,
            patterns: song.patterns,
            tracks,
            track_names,
            buses,
            send_delay,
            send_reverb,
            master_fx,
            output: OutputStage::new(),
            reverb_return,
            delay_return,
            reverb_sidechain,
            delay_sidechain,
            tempo,
            samples_per_step,
            sample_counter: 0.0,
            current_step_duration: samples_per_step,
            steps_per_bar,
            swing,
            humanize_velocity,
            humanize_timing,
            timing_rng: Rng::new(7919),
            master_level: 0.8,
            sidechain_amount,
            sc_envelope: 0.0,
            global_sc_source: song.globals.sidechain_source.clone(),
            sc_attack_coeff: attack_alpha(song.globals.sidechain_attack_ms.unwrap_or(DEFAULT_SC_ATTACK_MS)),
            sc_release_coeff: release_coeff(song.globals.sidechain_release_ms.unwrap_or(4.5)),
            global_sc_idx: None,
            kick_track_idx,
            reverb_wet_level: 1.0,
            delay_wet_level: 1.0,
            reverb_wet_heard: 1.0,
            delay_wet_heard: 1.0,
            // Sized for the busiest scene so the first scene change, which
            // happens on the audio thread, does not grow it.
            active_automations: Vec::with_capacity(
                song.scenes.iter().map(|s| s.automations.len()).max().unwrap_or(0)
            ),
            scene_step: 0,
            scene_total_steps: 0,
            scenes: song.scenes,
            arrangement: song.arrangement,
            arrangement_idx: 0,
            arrangement_bar_count: 0,
            current_bar: 0,
            global_step: 0,
            band_metering: false,
            master_in_peak: 0.0,
            master_in_sum_sq: 0.0,
            master_in_samples: 0,
            pending_triggers: Vec::with_capacity(MAX_PENDING_TRIGGERS),
            track_silent: vec![false; track_buf_count],
            track_bufs_l: vec![[0.0f32; BLOCK_SIZE]; track_buf_count],
            track_bufs_r: vec![[0.0f32; BLOCK_SIZE]; track_buf_count],
            taps: None,
            running: false,
        };
        engine.retune_fx(tempo);
        engine.resolve_sidechain_sources(usize::MAX);
        engine
    }

    /// Bar-synced modulation inside chains follows the tempo.
    pub(super) fn retune_fx(&mut self, bpm: f32) {
        for t in self.tracks.iter_mut() { t.insert_fx.set_bpm(bpm); }
        for b in self.buses.iter_mut() { b.fx_chain.set_bpm(bpm); }
        self.master_fx.set_bpm(bpm);
        self.reverb_return.set_bpm(bpm);
        self.delay_return.set_bpm(bpm);
    }
}
