extern crate alloc;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use crate::dsl::compiler::{self, *};
use crate::dsl::error::{ParseError, CompileError};
use crate::effects::delay::Delay;
use crate::effects::reverb::Reverb;
use crate::graph::node::{NodeKind, NodeSpec, MAX_NODE_INPUTS};
use crate::graph::voice::Instrument;
use crate::modules::bass::{BassModule, BassParam};
use crate::modules::beats::BeatsModule;
use crate::modules::fm::FmModule;
use crate::modules::keys::KeysModule;
use crate::params::{self, ParamId, ModuleKind};
use crate::rng::Rng;
use crate::{math, Module, BLOCK_SIZE, SAMPLE_RATE};

/// Wraps both graph-based instruments and real module instruments.
enum SongInstrument {
    Graph(Instrument),
    Bass(BassModule),
    Fm(FmModule),
    Keys(KeysModule),
    Beats(BeatsModule),
}

impl SongInstrument {
    fn kind_str(&self) -> &'static str {
        match self {
            Self::Graph(_) => "graph",
            Self::Bass(_) => "bass",
            Self::Fm(_) => "fm",
            Self::Keys(_) => "keys",
            Self::Beats(_) => "beats",
        }
    }

    fn note_on(&mut self, note: u8, velocity: f32) {
        match self {
            Self::Graph(inst) => inst.note_on(note, velocity),
            Self::Bass(m) => m.note_on(note, velocity),
            Self::Fm(m) => m.note_on(note, velocity),
            Self::Keys(m) => m.note_on(note, velocity),
            Self::Beats(m) => m.note_on(note, velocity),
        }
    }

    fn note_off(&mut self, note: u8) {
        match self {
            Self::Graph(inst) => inst.note_off(note),
            Self::Bass(m) => m.note_off(note),
            Self::Fm(m) => m.note_off(note),
            Self::Keys(m) => m.note_off(note),
            Self::Beats(m) => m.note_off(note),
        }
    }

    fn process_block(&mut self, output: &mut [f32]) {
        match self {
            Self::Graph(inst) => inst.process_block(output),
            Self::Bass(m) => Module::process_block(m, output),
            Self::Fm(m) => Module::process_block(m, output),
            Self::Keys(m) => Module::process_block(m, output),
            Self::Beats(m) => Module::process_block(m, output),
        }
    }

    /// Process a block with stereo output. Returns true if the instrument produced native stereo.
    fn process_block_stereo(&mut self, out_l: &mut [f32], out_r: &mut [f32]) -> bool {
        match self {
            Self::Beats(m) => { m.process_block_stereo(out_l, out_r); true }
            _ => { self.process_block(out_l); false }
        }
    }

    fn stage_plock(&mut self, cutoff: Option<f32>, env_depth: Option<f32>, resonance: Option<f32>) {
        match self {
            Self::Graph(inst) => inst.stage_plock(cutoff, env_depth, resonance),
            Self::Bass(m) => {
                // Bass p-lock values are already normalized 0-1 (matching BassParam range)
                if let Some(v) = cutoff {
                    m.set_param(BassParam::Cutoff, v);
                }
                if let Some(v) = env_depth {
                    m.set_param(BassParam::CutoffEnv, v);
                }
                if let Some(r) = resonance {
                    m.set_param(BassParam::Resonance, r);
                }
            }
            Self::Fm(_) | Self::Keys(_) | Self::Beats(_) => {}
        }
    }

    fn reset(&mut self) {
        match self {
            Self::Graph(inst) => inst.reset(),
            Self::Bass(m) => Module::reset(m),
            Self::Fm(m) => Module::reset(m),
            Self::Keys(m) => Module::reset(m),
            Self::Beats(m) => Module::reset(m),
        }
    }

    fn set_bpm(&mut self, bpm: f32) {
        match self {
            Self::Graph(_) => {} // Graph instruments don't have BPM
            Self::Bass(m) => m.set_bpm(bpm),
            Self::Fm(m) => m.set_bpm(bpm),
            Self::Keys(m) => m.set_bpm(bpm),
            Self::Beats(m) => m.set_bpm(bpm),
        }
    }

    /// Set a parameter by name (for automation).
    fn set_param_by_name(&mut self, name: &str, value: f32) -> bool {
        let kind = match self.module_kind() {
            Some(k) => k,
            None => return false, // graph instruments have no named params
        };
        match params::lookup(kind, name) {
            Some(spec) => { apply_param(self, spec.id, value); true }
            None => false,
        }
    }

    /// Registry kind for built-in modules; None for graph instruments.
    fn module_kind(&self) -> Option<ModuleKind> {
        match self {
            Self::Graph(_) => None,
            Self::Bass(_) => Some(ModuleKind::Bass),
            Self::Fm(_) => Some(ModuleKind::Fm),
            Self::Keys(_) => Some(ModuleKind::Keys),
            Self::Beats(_) => Some(ModuleKind::Beats),
        }
    }
}

/// Apply a registry-typed parameter to the right module.
fn apply_param(inst: &mut SongInstrument, id: ParamId, value: f32) {
    match (inst, id) {
        (SongInstrument::Bass(m), ParamId::Bass(p)) => m.set_param(p, value),
        (SongInstrument::Fm(m), ParamId::Fm(p)) => m.set_param(p, value),
        (SongInstrument::Keys(m), ParamId::Keys(p)) => m.set_param(p, value),
        (SongInstrument::Beats(m), ParamId::Beats(p)) => m.set_param(p, value),
        _ => {}
    }
}

/// FX chain processing helper: a chain of NodeKind applied in series (mono).
struct FxChain {
    nodes: Vec<Box<NodeKind>>,
}

impl FxChain {
    fn new(specs: &[NodeSpec]) -> Self {
        let nodes = specs.iter().map(|s| Box::new(s.instantiate())).collect();
        Self { nodes }
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let mut val = input;
        let inputs_buf = [0.0f32; MAX_NODE_INPUTS];
        for node in self.nodes.iter_mut() {
            let mut inp = inputs_buf;
            inp[0] = val;
            val = node.process(&inp, 1);
        }
        val
    }

    /// Process a stereo pair through the chain (for master bus).
    #[inline]
    fn process_stereo(&mut self, l: f32, r: f32) -> (f32, f32) {
        let mut vl = l;
        let mut vr = r;
        for node in self.nodes.iter_mut() {
            (vl, vr) = node.process_stereo(vl, vr);
        }
        (vl, vr)
    }

    fn reset(&mut self) {
        for node in self.nodes.iter_mut() {
            node.reset();
        }
    }
}

/// Per-track playback state.
struct TrackPlayback {
    instrument_idx: usize,
    pattern_idx: usize,
    velocity: f32,
    level: f32,              // output level 0.0-1.0
    pan: f32,                // raw pan value -1.0 to 1.0
    pan_l: f32,              // pre-computed left gain (equal-power)
    pan_r: f32,              // pre-computed right gain (equal-power)
    gate: f32,               // gate length as fraction of step (0.0-1.0)
    insert_fx: FxChain,
    bus_send: Option<(usize, f32)>,
    to_master: bool,
    delay_send: f32,         // global delay send amount 0.0-1.0
    reverb_send: f32,        // global reverb send amount 0.0-1.0
    sidechain_amount: f32,   // per-track sidechain override (0.0 = use global)
    // Step sequencer state
    current_step: usize,
    current_notes: [u8; compiler::MAX_CHORD_NOTES],  // active MIDI notes (0 = unused)
    current_notes_count: u8,
    gate_samples_remaining: f32,
    active: bool,
    stereo_src: bool,        // true if instrument produces native stereo (BeatsModule)
}

/// Named bus with FX chain.
struct SongBus {
    fx_chain: FxChain,
    buffer: [f32; BLOCK_SIZE],
}

impl SongBus {
    fn new(specs: &[NodeSpec]) -> Self {
        Self {
            fx_chain: FxChain::new(specs),
            buffer: [0.0; BLOCK_SIZE],
        }
    }

    fn clear(&mut self, len: usize) {
        for i in 0..len {
            self.buffer[i] = 0.0;
        }
    }

    fn reset(&mut self) {
        self.buffer = [0.0; BLOCK_SIZE];
        self.fx_chain.reset();
    }
}

/// Compute equal-power panning gains from a pan value (-1.0 to 1.0).
/// Returns (left_gain, right_gain).
#[inline]
fn pan_gains(pan: f32) -> (f32, f32) {
    // Map -1..1 to 0..1 for the trig calculation
    let p = (pan.clamp(-1.0, 1.0) + 1.0) * 0.5;
    // Equal-power: cos/sin panning law
    let angle = p * crate::math::HALF_PI;
    (crate::math::cos(angle), crate::math::sin(angle))
}

/// Self-contained engine for rendering DSL songs.
///
/// Uses graph-based and module-based instruments, named buses, pattern sequencing,
/// and arrangement playback with scene automation.
pub struct SongEngine {
    // Compiled data
    instruments: Vec<SongInstrument>,
    instrument_names: Vec<String>,
    patterns: Vec<CompiledPattern>,

    // Playback tracks
    tracks: Vec<TrackPlayback>,
    track_names: Vec<String>,

    // Buses
    buses: Vec<SongBus>,

    // Global send effects (delay + reverb)
    send_delay: Delay,
    send_reverb: Reverb,

    // Master FX chain
    master_fx: FxChain,

    // Timing
    tempo: f32,
    samples_per_step: f32,
    sample_counter: f32,
    current_step_duration: f32,  // effective duration of current step (with swing)
    steps_per_bar: usize,

    // Groove / humanization
    swing: f32,                  // 0.5 = straight, 0.67 = triplet feel (range 0.5-0.75)
    humanize_velocity: f32,      // velocity jitter amount 0.0-1.0
    humanize_timing: f32,        // timing jitter amount 0.0-1.0
    rng: Rng,                    // deterministic RNG for humanization

    // Master level (applied before master FX, matching reference Engine's 0.8)
    master_level: f32,

    // Automatic gain compensation for track summing
    gain_comp_target: f32,   // 1.0 / sqrt(active_tracks), clamped [0.25, 1.0]
    gain_comp_current: f32,  // smoothed value (exponential approach to target)

    // Sidechain compression
    sidechain_amount: f32,
    sc_envelope: f32,
    kick_track_idx: Option<usize>,

    // Scene effect overrides
    reverb_wet_level: f32,
    delay_wet_level: f32,

    // Automation state
    active_automations: Vec<ActiveAutomation>,
    scene_step: usize,
    scene_total_steps: usize,

    // Arrangement
    scenes: Vec<CompiledScene>,
    arrangement: Vec<(usize, u32)>,
    arrangement_idx: usize,
    arrangement_bar_count: u32,
    current_bar: usize,
    global_step: usize,

    // Pending nudge triggers: (samples_remaining, instrument_idx, midi_note, velocity)
    pending_triggers: Vec<(f32, usize, u8, f32)>,

    running: bool,
}

/// A running automation lane within a scene.
struct ActiveAutomation {
    target: AutoTarget,
    keyframes: Vec<f32>,
}

/// Resolved automation target.
enum AutoTarget {
    InstrumentParam { instrument_idx: usize, param_name: String },
    TrackLevel { track_idx: usize },
    ReverbMix,
    DelayMix,
}

/// Structured error from DSL parsing or compilation, preserving line/col info.
pub enum DslError {
    Parse(Vec<ParseError>),
    Compile(Vec<CompileError>),
}

impl DslError {
    /// Serialize to a JSON string for WASM → JS communication.
    pub fn to_json(&self) -> String {
        let mut out = String::from(r#"{"ok":false,"errors":["#);
        match self {
            DslError::Parse(errs) => {
                for (i, e) in errs.iter().enumerate() {
                    if i > 0 { out.push(','); }
                    out.push_str(&alloc::format!(
                        r#"{{"line":{},"col":{},"msg":"{}"}}"#,
                        e.line, e.col, json_escape(&e.message),
                    ));
                }
            }
            DslError::Compile(errs) => {
                for (i, e) in errs.iter().enumerate() {
                    if i > 0 { out.push(','); }
                    out.push_str(&alloc::format!(
                        r#"{{"line":{},"col":0,"msg":"{}"}}"#,
                        e.line, json_escape(&e.message),
                    ));
                }
            }
        }
        out.push_str("]}");
        out
    }
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str(r#"\""#),
            '\\' => out.push_str(r"\\"),
            '\n' => out.push_str(r"\n"),
            '\r' => out.push_str(r"\r"),
            '\t' => out.push_str(r"\t"),
            _ => out.push(c),
        }
    }
    out
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
        let tempo = song.globals.tempo;
        let instruments: Vec<SongInstrument> = song.instruments
            .into_iter()
            .map(|kind| {
                match kind {
                    CompiledInstrumentKind::Graph(template) => {
                        SongInstrument::Graph(Instrument::new(template))
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
            })
            .collect();

        // Build buses
        let buses: Vec<SongBus> = song.buses
            .iter()
            .map(|b| SongBus::new(&b.fx_chain))
            .collect();

        // Build master FX chain
        let master_fx = FxChain::new(&song.master.fx_chain);

        // Build track playback states from the compiled tracks
        let track_names: Vec<String> = song.tracks.iter().map(|t| t.name.clone()).collect();
        let tracks: Vec<TrackPlayback> = song.tracks
            .iter()
            .map(|t| {
                let (pan_l, pan_r) = pan_gains(t.pan);
                let stereo_src = t.instrument_idx < instruments.len()
                    && matches!(instruments[t.instrument_idx], SongInstrument::Beats(_));
                TrackPlayback {
                    instrument_idx: t.instrument_idx,
                    pattern_idx: t.pattern_idx,
                    velocity: t.velocity,
                    level: t.level,
                    pan: t.pan,
                    pan_l,
                    pan_r,
                    gate: t.gate,
                    insert_fx: FxChain::new(&t.insert_fx),
                    bus_send: t.bus_send,
                    to_master: t.to_master,
                    delay_send: t.delay_send,
                    reverb_send: t.reverb_send,
                    sidechain_amount: t.sidechain.unwrap_or(0.0),
                    current_step: 0,
                    current_notes: [0; compiler::MAX_CHORD_NOTES],
                    current_notes_count: 0,
                    gate_samples_remaining: 0.0,
                    active: true,
                    stereo_src,
                }
            })
            .collect();

        let meter = song.globals.meter;
        let sidechain_amount = song.globals.sidechain;
        let steps_per_bar = (meter.0 as usize) * 4; // 4 steps per beat (16th notes)
        let samples_per_step = SAMPLE_RATE * 60.0 / tempo / 4.0; // 16th note duration

        let instrument_names = song.instrument_names.clone();

        // Auto-detect kick track by instrument name or Beats module
        let kick_track_idx = tracks.iter().position(|t| {
            if t.instrument_idx < instrument_names.len() {
                let name = &instrument_names[t.instrument_idx];
                if name.contains("kick") { return true; }
                // BeatsModule tracks contain kick internally
                if t.instrument_idx < instruments.len() {
                    if matches!(instruments[t.instrument_idx], SongInstrument::Beats(_)) {
                        return true;
                    }
                }
            }
            false
        });

        // Global send effects — match reference Engine defaults
        let mut send_delay = Delay::new(SAMPLE_RATE, 2.0);
        // Tempo-sync delay to 16th note (matching DelaySync::Sixteenth)
        let sixteenth_time = 60.0 / tempo / 4.0;
        send_delay.set_time(sixteenth_time, SAMPLE_RATE);
        send_delay.set_feedback(0.25);
        send_delay.set_filter(0.6);

        let mut send_reverb = Reverb::new(SAMPLE_RATE);
        send_reverb.set_room_size(0.3);  // small room, tight
        send_reverb.set_damping(0.6);

        // Parse swing/humanize from globals
        let swing = song.globals.swing.unwrap_or(0.5);
        let humanize_velocity = song.globals.humanize.unwrap_or(0.0);
        let humanize_timing = song.globals.humanize_timing.unwrap_or(0.0);

        Self {
            instruments,
            instrument_names,
            patterns: song.patterns,
            tracks,
            track_names,
            buses,
            send_delay,
            send_reverb,
            master_fx,
            tempo,
            samples_per_step,
            sample_counter: 0.0,
            current_step_duration: samples_per_step,
            steps_per_bar,
            swing,
            humanize_velocity,
            humanize_timing,
            rng: Rng::new(7919),
            master_level: 0.8,
            gain_comp_target: 1.0,
            gain_comp_current: 1.0,
            sidechain_amount,
            sc_envelope: 0.0,
            kick_track_idx,
            reverb_wet_level: 1.0,
            delay_wet_level: 1.0,
            active_automations: Vec::new(),
            scene_step: 0,
            scene_total_steps: 0,
            scenes: song.scenes,
            arrangement: song.arrangement,
            arrangement_idx: 0,
            arrangement_bar_count: 0,
            current_bar: 0,
            global_step: 0,
            pending_triggers: Vec::new(),
            running: false,
        }
    }

    pub fn start(&mut self) {
        self.running = true;
        self.rng = Rng::new(7919); // reset for deterministic humanization
        self.current_step_duration = self.effective_step_samples(0);
        self.sample_counter = self.current_step_duration; // trigger first step immediately
        self.arrangement_idx = 0;
        self.arrangement_bar_count = 0;
        self.current_bar = 0;
        self.global_step = 0;
        self.reverb_wet_level = 1.0;
        self.delay_wet_level = 1.0;
        self.active_automations.clear();
        self.scene_step = 0;
        self.scene_total_steps = 0;

        // Set BPM on all module instruments
        for inst in self.instruments.iter_mut() {
            inst.set_bpm(self.tempo);
        }

        // If arrangement exists, apply first scene
        if !self.arrangement.is_empty() {
            let (scene_idx, _) = self.arrangement[0];
            self.apply_scene(scene_idx);
        } else {
            self.recompute_gain_comp();
        }
        // Snap gain compensation so the first block starts at the correct level
        self.gain_comp_current = self.gain_comp_target;

        for track in self.tracks.iter_mut() {
            track.current_step = 0;
            track.current_notes_count = 0;
            track.gate_samples_remaining = 0.0;
        }
    }

    /// Apply a scene's track configuration.
    fn apply_scene(&mut self, scene_idx: usize) {
        if scene_idx >= self.scenes.len() { return; }
        let scene = &self.scenes[scene_idx];

        // Update tempo if scene overrides it
        if let Some(t) = scene.tempo {
            self.tempo = t;
            self.samples_per_step = SAMPLE_RATE * 60.0 / t / 4.0;
            // Update BPM on module instruments
            for inst in self.instruments.iter_mut() {
                inst.set_bpm(t);
            }
        }

        // Apply effect overrides
        if let Some(rmix) = scene.reverb_mix {
            self.reverb_wet_level = rmix;
        }
        if let Some(dmix) = scene.delay_mix {
            self.delay_wet_level = dmix;
        }

        // Setup automation lanes
        self.active_automations.clear();
        self.scene_step = 0;
        // Calculate total steps for this scene from arrangement
        if self.arrangement_idx < self.arrangement.len() {
            let (_, repeat) = self.arrangement[self.arrangement_idx];
            self.scene_total_steps = repeat as usize * self.steps_per_bar;
        }
        for auto_def in &scene.automations {
            if let Some(target) = self.resolve_auto_target(&auto_def.target) {
                self.active_automations.push(ActiveAutomation {
                    target,
                    keyframes: auto_def.keyframes.clone(),
                });
            }
        }

        // Update tracks from scene
        // Release all currently-playing notes before deactivating tracks.
        // This prevents stale notes ringing across scene transitions.
        for ti in 0..self.tracks.len() {
            if self.tracks[ti].active && self.tracks[ti].current_notes_count > 0 {
                Self::release_track_notes(&mut self.tracks[ti], &mut self.instruments);
                self.tracks[ti].gate_samples_remaining = 0.0;
            }
        }

        // Reset to make scene tracks the active set
        for track in self.tracks.iter_mut() {
            track.active = false;
        }

        // Activate scene tracks (reusing existing TrackPlayback slots)
        for (i, st) in scene.tracks.iter().enumerate() {
            if i < self.tracks.len() {
                let tp = &mut self.tracks[i];
                tp.instrument_idx = st.instrument_idx;
                tp.pattern_idx = st.pattern_idx;
                tp.velocity = st.velocity;
                tp.level = st.level;
                tp.gate = st.gate;
                let (pan_l, pan_r) = pan_gains(st.pan);
                tp.pan_l = pan_l;
                tp.pan_r = pan_r;
                tp.delay_send = st.delay_send;
                tp.reverb_send = st.reverb_send;
                tp.sidechain_amount = st.sidechain.unwrap_or(0.0);
                tp.stereo_src = st.instrument_idx < self.instruments.len()
                    && matches!(self.instruments[st.instrument_idx], SongInstrument::Beats(_));
                tp.active = true;
                tp.current_step = 0;
                tp.current_notes_count = 0;
                tp.gate_samples_remaining = 0.0;
            }
        }

        // Recompute kick track index after scene reassignment
        self.kick_track_idx = self.tracks.iter().position(|t| {
            t.active && t.instrument_idx < self.instruments.len()
                && matches!(self.instruments[t.instrument_idx], SongInstrument::Beats(_))
        });

        self.recompute_gain_comp();
    }

    /// Recompute automatic gain compensation based on active track count.
    /// Uses equal-power scaling: 1/sqrt(N), clamped to [0.25, 1.0].
    fn recompute_gain_comp(&mut self) {
        let n = self.tracks.iter().filter(|t| t.active).count();
        self.gain_comp_target = if n <= 1 {
            1.0
        } else {
            let raw = 1.0 / math::sqrt(n as f32);
            if raw < 0.25 { 0.25 } else { raw }
        };
    }

    /// Resolve an automation target string to an AutoTarget.
    fn resolve_auto_target(&self, target: &str) -> Option<AutoTarget> {
        match target {
            "reverb_mix" => Some(AutoTarget::ReverbMix),
            "delay_mix" => Some(AutoTarget::DelayMix),
            _ => {
                // Check for "instrument.param" or "track.level"
                if let Some(dot_pos) = target.find('.') {
                    let name = &target[..dot_pos];
                    let param = &target[dot_pos + 1..];

                    if param == "level" {
                        // Look for track by name
                        let track_idx = self.tracks.iter()
                            .enumerate()
                            .position(|(_, _)| false) // tracks don't have names at runtime
                            .or_else(|| {
                                // Try matching by instrument name
                                self.instrument_names.iter()
                                    .position(|n| n == name)
                                    .and_then(|inst_idx| {
                                        self.tracks.iter().position(|t| t.instrument_idx == inst_idx && t.active)
                                    })
                            });
                        if let Some(ti) = track_idx {
                            return Some(AutoTarget::TrackLevel { track_idx: ti });
                        }
                    }

                    // Try as instrument.param
                    if let Some(inst_idx) = self.instrument_names.iter().position(|n| n == name) {
                        return Some(AutoTarget::InstrumentParam {
                            instrument_idx: inst_idx,
                            param_name: String::from(param),
                        });
                    }
                }
                None
            }
        }
    }

    // ── Swing & groove timing ──

    /// Compute effective step duration in samples, applying swing.
    fn effective_step_samples(&self, step_index: usize) -> f32 {
        if math::abs(self.swing - 0.5) < 0.001 {
            return self.samples_per_step;
        }
        // Swing: alternate long/short pairs (like classic drum machines)
        let pair_duration = self.samples_per_step * 2.0;
        if step_index % 2 == 0 {
            pair_duration * self.swing
        } else {
            pair_duration * (1.0 - self.swing)
        }
    }

    /// Release all active notes on a track.
    #[inline]
    fn release_track_notes(track: &mut TrackPlayback, instruments: &mut Vec<SongInstrument>) {
        let count = track.current_notes_count as usize;
        if count > 0 {
            let inst_idx = track.instrument_idx;
            if inst_idx < instruments.len() {
                for ni in 0..count {
                    instruments[inst_idx].note_off(track.current_notes[ni]);
                }
            }
            track.current_notes_count = 0;
        }
    }

    /// Apply velocity humanization (random jitter).
    #[inline]
    fn humanize_vel(velocity: f32, amount: f32, rng: &mut Rng) -> f32 {
        if amount <= 0.0 {
            return velocity;
        }
        let jitter = rng.next_bipolar() * amount * 0.15;
        (velocity * (1.0 + jitter)).clamp(0.01, 1.0)
    }

    /// Process one stereo block.
    pub fn process_block_stereo(&mut self, output_l: &mut [f32], output_r: &mut [f32]) {
        let len = output_l.len().min(BLOCK_SIZE);

        // Clear output
        for i in 0..len {
            output_l[i] = 0.0;
            output_r[i] = 0.0;
        }

        if !self.running { return; }

        // Per-track instrument render buffers (L and R)
        let track_count = self.tracks.len();
        let mut track_bufs_l: Vec<[f32; BLOCK_SIZE]> = (0..track_count)
            .map(|_| [0.0f32; BLOCK_SIZE])
            .collect();
        let mut track_bufs_r: Vec<[f32; BLOCK_SIZE]> = (0..track_count)
            .map(|_| [0.0f32; BLOCK_SIZE])
            .collect();

        // Step sequencer: process sample-by-sample for accurate timing
        for _s in 0..len {
            self.sample_counter += 1.0;

            // Step advance FIRST — so Tie can extend gate before gate-off check
            if self.sample_counter >= self.current_step_duration {
                self.sample_counter -= self.current_step_duration;
                self.advance_step();
                // Pre-compute next step's duration
                self.current_step_duration = self.effective_step_samples(self.global_step);
                // Apply timing humanization as micro-offset on step duration
                if self.humanize_timing > 0.0 {
                    let max_offset = self.samples_per_step * 0.04; // max ±4% of step
                    let offset = self.rng.next_bipolar() * self.humanize_timing * max_offset;
                    self.current_step_duration += offset;
                }
            }

            // Process pending nudge triggers (delayed drum hits from groove blocks)
            let mut i = 0;
            while i < self.pending_triggers.len() {
                self.pending_triggers[i].0 -= 1.0;
                if self.pending_triggers[i].0 <= 0.0 {
                    let (_, inst_idx, midi_note, vel) = self.pending_triggers.swap_remove(i);
                    if inst_idx < self.instruments.len() {
                        self.instruments[inst_idx].note_on(midi_note, vel);
                    }
                } else {
                    i += 1;
                }
            }

            // Gate-off handling (after step advance, so Tie extends before expiry)
            for ti in 0..track_count {
                if !self.tracks[ti].active { continue; }
                if self.tracks[ti].gate_samples_remaining > 0.0 {
                    self.tracks[ti].gate_samples_remaining -= 1.0;
                    if self.tracks[ti].gate_samples_remaining <= 0.0 {
                        let inst_idx = self.tracks[ti].instrument_idx;
                        let count = self.tracks[ti].current_notes_count as usize;
                        if count > 0 && inst_idx < self.instruments.len() {
                            for ni in 0..count {
                                self.instruments[inst_idx].note_off(self.tracks[ti].current_notes[ni]);
                            }
                            self.tracks[ti].current_notes_count = 0;
                        }
                    }
                }
            }
        }

        // Render each track's instrument (stereo-aware)
        for ti in 0..track_count {
            if !self.tracks[ti].active { continue; }
            let inst_idx = self.tracks[ti].instrument_idx;
            if inst_idx < self.instruments.len() {
                let is_stereo = self.instruments[inst_idx]
                    .process_block_stereo(&mut track_bufs_l[ti][..len], &mut track_bufs_r[ti][..len]);
                if !is_stereo {
                    // Mono instrument rendered into L; copy to R
                    track_bufs_r[ti][..len].copy_from_slice(&track_bufs_l[ti][..len]);
                }
            }
        }

        // Apply insert FX per track (mono processing applied to both channels)
        for ti in 0..track_count {
            if !self.tracks[ti].active { continue; }
            if self.tracks[ti].insert_fx.nodes.is_empty() { continue; }
            for s in 0..len {
                let (fl, fr) = self.tracks[ti].insert_fx.process_stereo(track_bufs_l[ti][s], track_bufs_r[ti][s]);
                track_bufs_l[ti][s] = fl;
                track_bufs_r[ti][s] = fr;
            }
        }

        // Sidechain ducking: use kick track to duck other tracks
        // Per-track sidechain_amount overrides the global amount when > 0.
        let has_any_sidechain = self.sidechain_amount > 0.0
            || self.tracks.iter().any(|t| t.active && t.sidechain_amount > 0.0);
        if has_any_sidechain {
            if let Some(kick_idx) = self.kick_track_idx {
                for s in 0..len {
                    // For Beats tracks, use kick_env instead of raw signal
                    let kick_level = if let SongInstrument::Beats(ref m) = self.instruments[self.tracks[kick_idx].instrument_idx] {
                        if s < m.kick_env.len() { m.kick_env[s] } else { 0.0 }
                    } else {
                        track_bufs_l[kick_idx][s].abs()
                    };
                    self.sc_envelope = if kick_level > self.sc_envelope {
                        0.01 * self.sc_envelope + 0.99 * kick_level // fast attack
                    } else {
                        0.995 * self.sc_envelope // slow release
                    };
                    for ti in 0..track_count {
                        if ti != kick_idx && self.tracks[ti].active {
                            let amount = if self.tracks[ti].sidechain_amount > 0.0 {
                                self.tracks[ti].sidechain_amount
                            } else {
                                self.sidechain_amount
                            };
                            if amount > 0.0 {
                                let duck = 1.0 - amount * self.sc_envelope;
                                track_bufs_l[ti][s] *= duck;
                                track_bufs_r[ti][s] *= duck;
                            }
                        }
                    }
                }
            }
        }

        // Clear bus buffers
        for bus in self.buses.iter_mut() {
            bus.clear(len);
        }

        // Send effect accumulation buffers
        let mut delay_in_l = [0.0f32; BLOCK_SIZE];
        let mut delay_in_r = [0.0f32; BLOCK_SIZE];
        let mut reverb_in_l = [0.0f32; BLOCK_SIZE];
        let mut reverb_in_r = [0.0f32; BLOCK_SIZE];

        // Mix tracks into master + bus sends + global sends with level and panning
        // NOTE: velocity is already baked into the instrument output (via voice.velocity
        // which = step_vel * track_vel, set in note_on). Only apply track level here.
        for ti in 0..track_count {
            if !self.tracks[ti].active { continue; }
            let gain = self.tracks[ti].level;
            let pan_l = self.tracks[ti].pan_l;
            let pan_r = self.tracks[ti].pan_r;
            let d_send = self.tracks[ti].delay_send;
            let r_send = self.tracks[ti].reverb_send;
            let is_stereo_src = self.tracks[ti].stereo_src;

            for s in 0..len {
                let (sample_l, sample_r) = if is_stereo_src {
                    // Stereo source (Beats): already internally panned, apply gain + track pan
                    (track_bufs_l[ti][s] * gain * pan_l, track_bufs_r[ti][s] * gain * pan_r)
                } else {
                    // Mono source: apply gain + panning
                    let sample = track_bufs_l[ti][s] * gain;
                    (sample * pan_l, sample * pan_r)
                };

                // Bus send (mono sum to bus)
                if let Some((bus_idx, amount)) = self.tracks[ti].bus_send {
                    if bus_idx < self.buses.len() {
                        self.buses[bus_idx].buffer[s] += (sample_l + sample_r) * 0.5 * amount;
                    }
                }

                // Global delay/reverb sends (post-pan)
                if d_send > 0.0 {
                    delay_in_l[s] += sample_l * d_send;
                    delay_in_r[s] += sample_r * d_send;
                }
                if r_send > 0.0 {
                    reverb_in_l[s] += sample_l * r_send;
                    reverb_in_r[s] += sample_r * r_send;
                }

                // Direct to master with stereo panning
                if self.tracks[ti].to_master {
                    output_l[s] += sample_l;
                    output_r[s] += sample_r;
                }
            }
        }

        // Process bus FX chains and mix into master
        for bus in self.buses.iter_mut() {
            for s in 0..len {
                if bus.buffer[s] != 0.0 {
                    let processed = bus.fx_chain.process(bus.buffer[s]);
                    output_l[s] += processed;
                    output_r[s] += processed;
                }
            }
        }

        // Process global send effects (wet-only returns, scaled by wet levels)
        let dwet = self.delay_wet_level;
        let rwet = self.reverb_wet_level;
        for s in 0..len {
            if delay_in_l[s] != 0.0 || delay_in_r[s] != 0.0 {
                let (dl, dr) = self.send_delay.process_stereo_wet(delay_in_l[s], delay_in_r[s]);
                output_l[s] += dl * dwet;
                output_r[s] += dr * dwet;
            }
            if reverb_in_l[s] != 0.0 || reverb_in_r[s] != 0.0 {
                let (rl, rr) = self.send_reverb.process_stereo_in_wet(reverb_in_l[s], reverb_in_r[s]);
                output_l[s] += rl * rwet;
                output_r[s] += rr * rwet;
            }
        }

        // Apply master level + gain compensation + master FX chain
        // Smoothing: ~5ms exponential approach to avoid clicks on scene transitions
        const SMOOTH_COEFF: f32 = 0.9955; // exp(-1/220) at 44.1kHz
        let ml = self.master_level;
        if !self.master_fx.nodes.is_empty() {
            for s in 0..len {
                self.gain_comp_current = SMOOTH_COEFF * self.gain_comp_current
                    + (1.0 - SMOOTH_COEFF) * self.gain_comp_target;
                let g = ml * self.gain_comp_current;
                let (fl, fr) = self.master_fx.process_stereo(
                    output_l[s] * g,
                    output_r[s] * g,
                );
                output_l[s] = fl;
                output_r[s] = fr;
            }
        } else {
            for s in 0..len {
                self.gain_comp_current = SMOOTH_COEFF * self.gain_comp_current
                    + (1.0 - SMOOTH_COEFF) * self.gain_comp_target;
                let g = ml * self.gain_comp_current;
                output_l[s] *= g;
                output_r[s] *= g;
            }
        }
    }

    fn advance_step(&mut self) {
        // Process automation before note events
        if self.scene_total_steps > 0 && !self.active_automations.is_empty() {
            let progress = self.scene_step as f32 / self.scene_total_steps as f32;
            for auto_lane in &self.active_automations {
                let value = interpolate_automation(&auto_lane.keyframes, progress);
                match &auto_lane.target {
                    AutoTarget::InstrumentParam { instrument_idx, param_name } => {
                        if *instrument_idx < self.instruments.len() {
                            self.instruments[*instrument_idx].set_param_by_name(param_name, value);
                        }
                    }
                    AutoTarget::TrackLevel { track_idx } => {
                        if *track_idx < self.tracks.len() {
                            self.tracks[*track_idx].level = value;
                        }
                    }
                    AutoTarget::ReverbMix => self.reverb_wet_level = value,
                    AutoTarget::DelayMix => self.delay_wet_level = value,
                }
            }
        }
        self.scene_step += 1;

        let track_count = self.tracks.len();

        for ti in 0..track_count {
            if !self.tracks[ti].active { continue; }

            let pat_idx = self.tracks[ti].pattern_idx;
            if pat_idx >= self.patterns.len() { continue; }

            let pattern = &self.patterns[pat_idx];

            // Multi-lane drum pattern
            if !pattern.lanes.is_empty() {
                let lane_len = pattern.lanes[0].steps.len();
                if lane_len == 0 { continue; }
                let step_idx = self.tracks[ti].current_step % lane_len;
                let inst_idx = self.tracks[ti].instrument_idx;
                if inst_idx < self.instruments.len() {
                    for lane in &pattern.lanes {
                        if step_idx < lane.steps.len() {
                            if let CompiledStep::DrumHit { velocity, probability, roll, .. } = lane.steps[step_idx] {
                                // Probability gate: skip hit if random exceeds probability
                                if probability < 1.0 {
                                    let chance = self.rng.next_f32();
                                    if chance > probability {
                                        continue; // skip this hit
                                    }
                                }
                                let raw_vel = velocity * self.tracks[ti].velocity;
                                let vel = Self::humanize_vel(raw_vel, self.humanize_velocity, &mut self.rng);

                                // Per-lane nudge: delay trigger by nudge * step_samples
                                let nudge_samples = lane.nudge * self.samples_per_step;
                                if nudge_samples.abs() > 0.5 && nudge_samples > 0.0 {
                                    // Positive nudge = delay trigger
                                    self.pending_triggers.push((nudge_samples, inst_idx, lane.midi_note, vel));
                                } else {
                                    // No nudge or negative nudge (trigger immediately, can't go back in time)
                                    self.instruments[inst_idx].note_on(lane.midi_note, vel);
                                }

                                // Roll: schedule additional retriggers within this step
                                if roll > 1 {
                                    let step_samples = self.effective_step_samples(self.global_step) as u32;
                                    let interval = step_samples / (roll as u32);
                                    if let SongInstrument::Beats(ref mut beats) = self.instruments[inst_idx] {
                                        beats.stutter_drum = Some(lane.midi_note);
                                        beats.stutter_velocity = vel * 0.9;
                                        beats.stutter_interval = interval;
                                        beats.stutter_counter = 0;
                                        beats.stutter_remaining = (roll - 1) as u32;
                                    }
                                }
                            }
                        }
                    }
                }
                // No gate management — drums self-decay
                self.tracks[ti].current_step += 1;
                continue;
            }

            // Sequential pattern (existing behavior)
            if pattern.steps.is_empty() { continue; }

            let step_idx = self.tracks[ti].current_step % pattern.steps.len();
            let step = pattern.steps[step_idx];

            let gate = self.tracks[ti].gate;

            // Debug: track pad step processing
            match step {
                CompiledStep::Tie => {
                    // Keep previous note alive — extend gate for another step.
                    // Use generous duration to survive swing/humanization timing variance.
                    let next_step_idx = (step_idx + 1) % pattern.steps.len();
                    let next_continues = match pattern.steps[next_step_idx] {
                        CompiledStep::Tie => true,
                        CompiledStep::NoteOn { midi_note, .. } => {
                            self.tracks[ti].current_notes_count == 1
                                && self.tracks[ti].current_notes[0] == midi_note
                        }
                        CompiledStep::Chord { notes, count, .. } => {
                            let c = count as usize;
                            let pc = self.tracks[ti].current_notes_count as usize;
                            c == pc && (0..c).all(|i| {
                                self.tracks[ti].current_notes[i] == notes[i].midi_note
                            })
                        }
                        _ => false,
                    };
                    if next_continues {
                        // Use 2x step duration to guarantee survival across swing variance.
                        // The next tie/note will reset this anyway.
                        self.tracks[ti].gate_samples_remaining = self.samples_per_step * 2.0;
                    } else {
                        // Last tie in chain — apply track gate for natural release
                        self.tracks[ti].gate_samples_remaining = self.current_step_duration * gate;
                    }
                }
                _ => {
                    // Peek at next step: if it's a Tie, force gate=1.0 so note
                    // survives until the Tie can extend it.
                    let next_step_idx = (step_idx + 1) % pattern.steps.len();
                    let next_is_tie = matches!(pattern.steps[next_step_idx], CompiledStep::Tie);

                    match step {
                        CompiledStep::NoteOn { midi_note, velocity, plock } => {
                            // If the same single note is already playing (pattern loop),
                            // just extend gate — don't re-trigger (avoids click/re-attack).
                            let same_note = self.tracks[ti].current_notes_count == 1
                                && self.tracks[ti].current_notes[0] == midi_note;
                            if same_note && next_is_tie {
                                // Sustain continuation — treat as tie
                                self.tracks[ti].gate_samples_remaining = self.samples_per_step * 2.0;
                            } else {
                                // Release previous notes
                                Self::release_track_notes(
                                    &mut self.tracks[ti],
                                    &mut self.instruments,
                                );
                                let inst_idx = self.tracks[ti].instrument_idx;
                                let raw_vel = velocity * self.tracks[ti].velocity;
                                let vel = Self::humanize_vel(raw_vel, self.humanize_velocity, &mut self.rng);
                                if inst_idx < self.instruments.len() {
                                    self.instruments[inst_idx].stage_plock(plock.cutoff, plock.env_depth, plock.resonance);
                                    self.instruments[inst_idx].note_on(midi_note, vel);
                                }
                                self.tracks[ti].current_notes[0] = midi_note;
                                self.tracks[ti].current_notes_count = 1;
                                if next_is_tie {
                                    self.tracks[ti].gate_samples_remaining = self.samples_per_step * 2.0;
                                } else {
                                    let step_gate = plock.gate.unwrap_or(gate);
                                    self.tracks[ti].gate_samples_remaining = self.current_step_duration * step_gate;
                                }
                            }
                        }
                        CompiledStep::Chord { notes, count, plock } => {
                            // Check if the exact same chord is already playing (pattern loop).
                            let c = count as usize;
                            let prev_c = self.tracks[ti].current_notes_count as usize;
                            let same_chord = c == prev_c && (0..c).all(|i| {
                                self.tracks[ti].current_notes[i] == notes[i].midi_note
                            });
                            if same_chord && next_is_tie {
                                // Sustain continuation — treat as tie
                                self.tracks[ti].gate_samples_remaining = self.samples_per_step * 2.0;
                            } else {
                                // Release previous notes
                                Self::release_track_notes(
                                    &mut self.tracks[ti],
                                    &mut self.instruments,
                                );
                                let inst_idx = self.tracks[ti].instrument_idx;
                                if inst_idx < self.instruments.len() {
                                    self.instruments[inst_idx].stage_plock(plock.cutoff, plock.env_depth, plock.resonance);
                                    for ni in 0..c {
                                        let raw_vel = notes[ni].velocity * self.tracks[ti].velocity;
                                        let vel = Self::humanize_vel(raw_vel, self.humanize_velocity, &mut self.rng);
                                        self.instruments[inst_idx].note_on(notes[ni].midi_note, vel);
                                        self.tracks[ti].current_notes[ni] = notes[ni].midi_note;
                                    }
                                    self.tracks[ti].current_notes_count = count;
                                }
                                if next_is_tie {
                                    self.tracks[ti].gate_samples_remaining = self.samples_per_step * 2.0;
                                } else {
                                    let step_gate = plock.gate.unwrap_or(gate);
                                    self.tracks[ti].gate_samples_remaining = self.current_step_duration * step_gate;
                                }
                            }
                        }
                        CompiledStep::DrumHit { velocity, plock, .. } => {
                            Self::release_track_notes(
                                &mut self.tracks[ti],
                                &mut self.instruments,
                            );
                            let inst_idx = self.tracks[ti].instrument_idx;
                            let raw_vel = velocity * self.tracks[ti].velocity;
                            let vel = Self::humanize_vel(raw_vel, self.humanize_velocity, &mut self.rng);
                            if inst_idx < self.instruments.len() {
                                self.instruments[inst_idx].stage_plock(plock.cutoff, plock.env_depth, plock.resonance);
                                self.instruments[inst_idx].note_on(36, vel);
                            }
                            self.tracks[ti].current_notes[0] = 36;
                            self.tracks[ti].current_notes_count = 1;
                            if next_is_tie {
                                self.tracks[ti].gate_samples_remaining = self.samples_per_step * 2.0;
                            } else {
                                let step_gate = plock.gate.unwrap_or(0.5);
                                self.tracks[ti].gate_samples_remaining = self.current_step_duration * step_gate;
                            }
                        }
                        CompiledStep::Rest => {
                            Self::release_track_notes(
                                &mut self.tracks[ti],
                                &mut self.instruments,
                            );
                        }
                        CompiledStep::Tie => unreachable!(),
                    }
                }
            }

            self.tracks[ti].current_step += 1;
        }

        self.global_step += 1;

        // Bar boundary check for arrangement
        if self.steps_per_bar > 0 && self.global_step % self.steps_per_bar == 0 {
            self.current_bar += 1;
            self.check_arrangement_advance();
        }
    }

    fn check_arrangement_advance(&mut self) {
        if self.arrangement.is_empty() { return; }
        if self.arrangement_idx >= self.arrangement.len() { return; }

        self.arrangement_bar_count += 1;
        let (_, repeat) = self.arrangement[self.arrangement_idx];

        // Each repeat = one bar's worth of the pattern
        if self.arrangement_bar_count >= repeat {
            self.arrangement_bar_count = 0;
            self.arrangement_idx += 1;

            if self.arrangement_idx < self.arrangement.len() {
                let (scene_idx, _) = self.arrangement[self.arrangement_idx];
                self.apply_scene(scene_idx);
            } else {
                // Arrangement finished
                self.running = false;
            }
        }
    }

    /// Render the entire song (or specified number of bars) to stereo buffers.
    pub fn render(&mut self, bars: u32) -> (Vec<f32>, Vec<f32>) {
        let total_samples = (bars as f32 * self.steps_per_bar as f32 * self.samples_per_step) as usize;
        let mut out_l = Vec::with_capacity(total_samples + BLOCK_SIZE);
        let mut out_r = Vec::with_capacity(total_samples + BLOCK_SIZE);

        self.start();

        let mut rendered = 0;
        while rendered < total_samples && self.running {
            let chunk = (total_samples - rendered).min(BLOCK_SIZE);
            let mut bl = [0.0f32; BLOCK_SIZE];
            let mut br = [0.0f32; BLOCK_SIZE];
            self.process_block_stereo(&mut bl[..chunk], &mut br[..chunk]);
            out_l.extend_from_slice(&bl[..chunk]);
            out_r.extend_from_slice(&br[..chunk]);
            rendered += chunk;
        }

        (out_l, out_r)
    }

    /// Calculate total bars from arrangement.
    pub fn arrangement_bars(&self) -> u32 {
        self.arrangement.iter().map(|(_, r)| *r).sum()
    }

    pub fn tempo(&self) -> f32 { self.tempo }

    pub fn reset(&mut self) {
        self.running = false;
        self.sample_counter = 0.0;
        self.current_step_duration = self.samples_per_step;
        self.rng = Rng::new(7919);
        self.arrangement_idx = 0;
        self.arrangement_bar_count = 0;
        self.current_bar = 0;
        self.global_step = 0;
        self.sc_envelope = 0.0;
        self.reverb_wet_level = 1.0;
        self.delay_wet_level = 1.0;
        self.active_automations.clear();
        self.scene_step = 0;
        self.scene_total_steps = 0;

        for inst in self.instruments.iter_mut() {
            inst.reset();
        }
        for bus in self.buses.iter_mut() {
            bus.reset();
        }
        self.send_delay.reset();
        self.send_reverb.reset();
        self.master_fx.reset();
        for track in self.tracks.iter_mut() {
            track.current_step = 0;
            track.current_notes_count = 0;
            track.gate_samples_remaining = 0.0;
        }
    }

    pub fn global_step(&self) -> usize { self.global_step }
    pub fn running(&self) -> bool { self.running }
    pub fn current_bar(&self) -> usize { self.current_bar }

    /// Start playback from a specific bar (for hot-swap continuity).
    /// Fast-forwards through the arrangement to land on the right scene.
    pub fn start_from_bar(&mut self, bar: usize) {
        self.running = true;
        self.rng = Rng::new(7919);
        self.current_step_duration = self.effective_step_samples(0);
        self.sample_counter = self.current_step_duration;
        self.reverb_wet_level = 1.0;
        self.delay_wet_level = 1.0;
        self.active_automations.clear();
        self.scene_step = 0;
        self.scene_total_steps = 0;

        for inst in self.instruments.iter_mut() {
            inst.set_bpm(self.tempo);
        }

        // Fast-forward arrangement to the target bar
        self.arrangement_idx = 0;
        self.arrangement_bar_count = 0;
        self.current_bar = 0;
        self.global_step = 0;

        if !self.arrangement.is_empty() {
            let mut bars_remaining = bar;
            while self.arrangement_idx < self.arrangement.len() {
                let (_, repeat) = self.arrangement[self.arrangement_idx];
                let scene_bars = repeat as usize;
                if bars_remaining < scene_bars {
                    self.arrangement_bar_count = bars_remaining as u32;
                    break;
                }
                bars_remaining -= scene_bars;
                self.arrangement_idx += 1;
            }
            // Clamp to last scene if past the end
            if self.arrangement_idx >= self.arrangement.len() {
                self.arrangement_idx = self.arrangement.len() - 1;
                self.arrangement_bar_count = 0;
            }
            let (scene_idx, _) = self.arrangement[self.arrangement_idx];
            self.apply_scene(scene_idx);
        } else {
            self.recompute_gain_comp();
        }

        self.current_bar = bar;
        self.global_step = bar * self.steps_per_bar;
        self.gain_comp_current = self.gain_comp_target;

        for track in self.tracks.iter_mut() {
            track.current_step = 0;
            track.current_notes_count = 0;
            track.gate_samples_remaining = 0.0;
        }
    }

    // ═══════════════════════════════════════════════════════
    //  Real-time track control (no recompile)
    // ═══════════════════════════════════════════════════════

    pub fn track_count(&self) -> usize { self.tracks.len() }

    pub fn track_name(&self, idx: usize) -> &str {
        if idx < self.track_names.len() { &self.track_names[idx] } else { "" }
    }

    pub fn track_kind(&self, idx: usize) -> &str {
        self.tracks.get(idx)
            .and_then(|t| self.instruments.get(t.instrument_idx))
            .map_or("unknown", |inst| inst.kind_str())
    }

    pub fn track_level(&self, idx: usize) -> f32 {
        self.tracks.get(idx).map_or(0.0, |t| t.level)
    }

    pub fn track_pan(&self, idx: usize) -> f32 {
        self.tracks.get(idx).map_or(0.0, |t| t.pan)
    }

    pub fn set_track_level(&mut self, idx: usize, level: f32) {
        if let Some(track) = self.tracks.get_mut(idx) {
            track.level = level.clamp(0.0, 1.0);
        }
    }

    pub fn set_track_pan(&mut self, idx: usize, pan: f32) {
        if let Some(track) = self.tracks.get_mut(idx) {
            let p = pan.clamp(-1.0, 1.0);
            track.pan = p;
            let (l, r) = pan_gains(p);
            track.pan_l = l;
            track.pan_r = r;
        }
    }

    // ═══════════════════════════════════════════════════════
    //  Phase 2: Runtime mutations (no recompile)
    // ═══════════════════════════════════════════════════════

    pub fn set_tempo(&mut self, bpm: f32) {
        let bpm = bpm.clamp(20.0, 999.0);
        self.tempo = bpm;
        self.samples_per_step = SAMPLE_RATE * 60.0 / bpm / 4.0;
        self.current_step_duration = self.effective_step_samples(self.global_step);
        for inst in self.instruments.iter_mut() {
            inst.set_bpm(bpm);
        }
    }

    pub fn set_track_pattern(&mut self, track_idx: usize, pattern_idx: usize) {
        if let Some(track) = self.tracks.get_mut(track_idx) {
            if pattern_idx < self.patterns.len() {
                track.pattern_idx = pattern_idx;
                track.current_step = 0;
            }
        }
    }

    pub fn set_track_velocity(&mut self, track_idx: usize, velocity: f32) {
        if let Some(track) = self.tracks.get_mut(track_idx) {
            track.velocity = velocity.clamp(0.0, 1.0);
        }
    }

    pub fn set_track_gate(&mut self, track_idx: usize, gate: f32) {
        if let Some(track) = self.tracks.get_mut(track_idx) {
            track.gate = gate.clamp(0.0, 1.0);
        }
    }

    pub fn pattern_count(&self) -> usize { self.patterns.len() }

    pub fn pattern_name(&self, idx: usize) -> &str {
        self.patterns.get(idx).map_or("", |p| &p.name)
    }

    /// Set a named module parameter at runtime. Returns false if the
    /// instrument does not exist or the name is not in the registry.
    pub fn set_module_param(&mut self, inst_idx: usize, name: &str, value: f32) -> bool {
        match self.instruments.get_mut(inst_idx) {
            Some(inst) => inst.set_param_by_name(name, value),
            None => false,
        }
    }

    pub fn instrument_count(&self) -> usize { self.instruments.len() }

    pub fn instrument_name(&self, idx: usize) -> &str {
        self.instrument_names.get(idx).map_or("", |n| n.as_str())
    }

    pub fn instrument_index(&self, name: &str) -> Option<usize> {
        self.instrument_names.iter().position(|n| n == name)
    }

    /// Swing 0.5 (straight) ..= 0.75 (hard shuffle). Takes effect on the next step.
    pub fn set_swing(&mut self, swing: f32) {
        self.swing = swing.clamp(0.5, 0.75);
    }

    pub fn set_humanize(&mut self, velocity: f32, timing: f32) {
        self.humanize_velocity = velocity.clamp(0.0, 1.0);
        self.humanize_timing = timing.clamp(0.0, 1.0);
    }

    pub fn swing(&self) -> f32 { self.swing }

    pub fn humanize(&self) -> (f32, f32) { (self.humanize_velocity, self.humanize_timing) }
}

/// Interpolate automation keyframes at a given progress (0.0 - 1.0).
fn interpolate_automation(keyframes: &[f32], progress: f32) -> f32 {
    let p = progress.clamp(0.0, 1.0);
    match keyframes.len() {
        0 => 0.0,
        1 => keyframes[0],
        2 => {
            // Linear: start → end
            keyframes[0] + (keyframes[1] - keyframes[0]) * p
        }
        3 => {
            // Triangle: start → peak (at midpoint) → end
            if p < 0.5 {
                let t = p * 2.0;
                keyframes[0] + (keyframes[1] - keyframes[0]) * t
            } else {
                let t = (p - 0.5) * 2.0;
                keyframes[1] + (keyframes[2] - keyframes[1]) * t
            }
        }
        _ => keyframes[0], // shouldn't happen
    }
}
