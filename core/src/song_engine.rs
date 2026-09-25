extern crate alloc;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use crate::analysis::BandMeter;
use crate::dsl::compiler::{self, *};
use crate::dsl::error::{ParseError, CompileError};
use crate::effects::delay::{Delay, DelaySync};
use crate::effects::reverb::Reverb;
use crate::graph::node::{ChainStep, NodeKind};
use crate::graph::voice::Instrument;
use crate::modules::bass::{BassModule, BassParam};
use crate::modules::beats::BeatsModule;
use crate::modules::fm::FmModule;
use crate::modules::keys::KeysModule;
use crate::params::{self, ParamId, ModuleKind};
use crate::primitives::arp_processor::{ArpProcessor, ArpEvent};
use crate::rng::Rng;
use crate::{math, Module, BLOCK_SIZE, SAMPLE_RATE};

/// What the current step is played with. Copied out of the engine so the track,
/// instrument and rng borrows can all stay live across the call.
#[derive(Clone, Copy)]
struct StepTiming {
    humanize_velocity: f32,
    samples_per_step: f32,
    step_duration: f32,
}

/// Wraps both graph-based instruments and real module instruments.
// large_enum_variant: boxing would put a pointer chase in front of every
// instrument on every sample, the engine's hottest loop, to save memory the
// engine has already reserved off-thread.
#[allow(clippy::large_enum_variant)]
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

    /// Glide to a new note without retriggering envelopes. Returns false when
    /// the instrument has no portamento, so the caller falls back to note_on.
    fn slide_to(&mut self, note: u8, velocity: f32) -> bool {
        match self {
            Self::Bass(m) => { m.slide_to(note, velocity); true }
            _ => false,
        }
    }

    /// Bend every note the instrument plays by `ratio` of its frequency. A
    /// graph instrument or a drum kit has nothing to bend.
    fn set_pitch_bend(&mut self, ratio: f32) {
        match self {
            Self::Bass(m) => m.pitch_bend_ratio = ratio,
            Self::Fm(m) => m.pitch_bend_ratio = ratio,
            Self::Keys(m) => m.pitch_bend_ratio = ratio,
            Self::Graph(_) | Self::Beats(_) => {}
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
            // Keys pans its voices (unison spreads eight of them across the
            // field) and runs the chorus in stereo. Routing it through the mono
            // `process_block` downmixed all of that and then copied one channel
            // into the other.
            Self::Keys(m) => { m.process_block_stereo(out_l, out_r); true }
            Self::Fm(m) => { m.process_block_stereo(out_l, out_r); true }
            _ => { self.process_block(out_l); false }
        }
    }

    /// True when the instrument is producing nothing and has nothing left
    /// releasing, so the engine can skip it whole.
    fn is_idle(&self) -> bool {
        match self {
            Self::Graph(inst) => inst.is_idle(),
            Self::Bass(m) => m.is_idle(),
            Self::Fm(m) => m.is_idle(),
            Self::Keys(m) => m.is_idle(),
            Self::Beats(m) => m.is_idle(),
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
            Some(spec) => apply_param(self, spec.id, value),
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

/// Apply a registry-typed parameter to the right module. False when the id
/// belongs to another module kind: that is a caller bug, not a no-op.
fn apply_param(inst: &mut SongInstrument, id: ParamId, value: f32) -> bool {
    match (inst, id) {
        (SongInstrument::Bass(m), ParamId::Bass(p)) => m.set_param(p, value),
        (SongInstrument::Fm(m), ParamId::Fm(p)) => m.set_param(p, value),
        (SongInstrument::Keys(m), ParamId::Keys(p)) => m.set_param(p, value),
        (SongInstrument::Beats(m), ParamId::Beats(p)) => m.set_param(p, value),
        _ => return false,
    }
    true
}

/// FX chain processing helper: a chain of NodeKind applied in series.
struct FxChain {
    nodes: Vec<Box<NodeKind>>,
    /// Dry/wet per node. 0 bypasses: the node is not processed at all, which
    /// is what makes a rig of switched-off effects free rather than merely
    /// silent. Kept beside the nodes rather than inside them so every node
    /// type gets it without knowing about it.
    wet: Vec<f32>,
    /// True while a node is bypassed, so entering bypass can clear it once.
    /// Without this a delay or reverb resumes with whatever it was holding
    /// when it was switched out, which arrives as a stale burst.
    bypassed: Vec<bool>,
}

impl FxChain {
    fn new(steps: &[ChainStep]) -> Self {
        let nodes = steps.iter().map(|s| Box::new(s.spec.instantiate())).collect();
        let wet = steps.iter().map(|s| s.wet).collect();
        let bypassed = steps.iter().map(|s| s.wet <= 0.0).collect();
        Self { nodes, wet, bypassed }
    }

    /// Set one node's dry/wet by position. Returns false if there is no such
    /// node, so a caller can tell a no-op from a real change.
    fn set_wet(&mut self, idx: usize, value: f32) -> bool {
        let Some(slot) = self.wet.get_mut(idx) else { return false };
        *slot = value.clamp(0.0, 1.0);
        let now_off = *slot <= 0.0;
        if now_off && !self.bypassed[idx] {
            self.nodes[idx].reset();
        }
        self.bypassed[idx] = now_off;
        true
    }


    /// Process a stereo pair through the chain (for master bus).
    #[inline]
    fn process_stereo(&mut self, l: f32, r: f32) -> (f32, f32) {
        let mut vl = l;
        let mut vr = r;
        for (i, node) in self.nodes.iter_mut().enumerate() {
            let wet = self.wet[i];
            // Bypassed: not processed at all. This is the whole point -- an
            // effect that is switched off has to cost nothing, or a rig with a
            // full chain on every voice is unaffordable.
            if wet <= 0.0 { continue; }
            let (wl, wr) = node.process_stereo(vl, vr);
            if wet >= 1.0 {
                (vl, vr) = (wl, wr);
            } else {
                vl += (wl - vl) * wet;
                vr += (wr - vr) * wet;
            }
        }
        (vl, vr)
    }

    fn reset(&mut self) {
        for node in self.nodes.iter_mut() {
            node.reset();
        }
    }

    fn set_bpm(&mut self, bpm: f32) {
        for node in self.nodes.iter_mut() {
            node.set_bpm(bpm);
        }
    }

    /// Set a named parameter on the first node that has it.
    fn set_param(&mut self, name: &str, value: f32) -> bool {
        self.nodes.iter_mut().any(|n| n.set_named(name, value))
    }
}

/// Per-track playback state.
/// How far a track's `level` can be pushed: +12 dB, the same ceiling a
/// compressor's `makeup` has.
///
/// It used to be 1.0, but only on the live path. A level written in the file
/// was unbounded and so was an `auto <track> level` sweep, so the corpus has
/// tracks at 1.4 and 2.0 that play fine -- until the same value arrives
/// through a live edit or a hot swap, where it silently became 1.0 and the
/// track dropped. Three paths for one number have to agree, and the one that
/// disagreed was the one nobody had written a file against.
pub const MAX_TRACK_LEVEL: f32 = 4.0;

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
    /// `as <name>` per insert node, for resolving automation targets.
    fx_labels: Vec<Option<InlineName>>,
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
    // Arpeggiator: the pattern supplies held notes, the arp schedules them per sample
    arp: Option<ArpProcessor>,
    arp_cfg: Option<ArpConfig>,
    // Metering: post-level, post-pan, pre-master. For mix reports.
    meter_peak: f32,
    meter_sum_sq: f64,
    meter_samples: u64,
    /// Mid and side energy, so the report can say where in the stereo field a
    /// track sits. Two instruments in the same place cannot be told apart.
    meter_mid_sq: f64,
    meter_side_sq: f64,
    /// Band split of this track alone. Two tracks sitting in the same band is
    /// masking, and it used to take reasoning to notice.
    band: BandMeter,
    /// Amount this track ducks, and the track it ducks against. `None` source
    /// means the song's global source, which is the kick unless said otherwise.
    sc_source: Option<usize>,
    /// This track's own envelope, maintained only when something ducks against it.
    sc_env: f32,
    is_sc_source: bool,
    /// Every random draw this track makes: velocity humanization and the
    /// probability gate on its drum hits.
    ///
    /// One shared stream used to serve the whole song, and that made a track's
    /// groove depend on its neighbours. The draws are interleaved in track
    /// order within a step, so muting a track, adding one, or reordering them
    /// shifted the numbers every other track received and the feel of the
    /// whole song moved. Seeding from the name rather than the index keeps a
    /// track's stream its own across all three.
    rng: Rng,
}

/// A stable seed from a track's name. FNV-1a, which is four lines and spreads
/// single-character differences across the whole word -- `hat` and `hats` have
/// to land far apart or two tracks in the same song jitter in lockstep.
fn seed_from_name(name: &str) -> u32 {
    let mut h: u32 = 2166136261;
    for b in name.as_bytes() {
        h ^= *b as u32;
        h = h.wrapping_mul(16777619);
    }
    if h == 0 { 1 } else { h }
}

/// Share of a new peak the follower takes in one sample. The 0.005 ms default
/// reproduces the 0.99 that used to be hardcoded, which is effectively instant.
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

/// Build a fresh arp processor from compiled settings at the given tempo.
fn make_arp(cfg: &ArpConfig, tempo: f32) -> ArpProcessor {
    let mut arp = ArpProcessor::new();
    arp.set_bpm(tempo * cfg.rate_mult);
    arp.set_gate(cfg.gate);
    arp.set_pattern(cfg.pattern);
    // set_octave_range maps 0..1 → 1..4; nudge past float error so 3 stays 3.
    arp.set_octave_range((cfg.octaves.saturating_sub(1)) as f32 / 3.0 + 0.01);
    arp
}

/// Named bus with FX chain.
struct SongBus {
    name: String,
    fx_chain: FxChain,
    /// Stereo. A bus used to sum L and R and return the result down the middle,
    /// so any track routed into one lost its position in the stereo field --
    /// which is most of what tells instruments apart.
    buffer: [f32; BLOCK_SIZE],
    buffer_r: [f32; BLOCK_SIZE],
    // Metered after the bus chain: a track can read fine on its own meter and
    // still arrive at master 12 dB down because of what the bus does to it.
    meter_peak: f32,
    meter_sum_sq: f64,
    meter_samples: u64,
}

impl SongBus {
    fn new(name: String, specs: &[ChainStep]) -> Self {
        Self {
            name,
            fx_chain: FxChain::new(specs),
            buffer: [0.0; BLOCK_SIZE],
            buffer_r: [0.0; BLOCK_SIZE],
            meter_peak: 0.0,
            meter_sum_sq: 0.0,
            meter_samples: 0,
        }
    }

    fn clear(&mut self, len: usize) {
        for i in 0..len {
            self.buffer[i] = 0.0;
            self.buffer_r[i] = 0.0;
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
    /// Which live instrument a track plays for each compiled module it may
    /// name, flat: `inst_for[track * n_instruments + module]`. Every pair
    /// gets its own instance so no two tracks drive one set of voices.
    inst_for: Vec<usize>,
    n_instruments: usize,
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
    reverb_return: FxChain,
    delay_return: FxChain,
    reverb_sidechain: f32,
    delay_sidechain: f32,

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
    /// Jitter on the step clock, which is one clock for the whole song. Every
    /// other random draw belongs to a track and lives on the track, so that
    /// this stream is not perturbed by how many tracks the song happens to
    /// have. See `TrackPlayback::rng`.
    timing_rng: Rng,

    // Master level (applied before master FX, matching reference Engine's 0.8)
    master_level: f32,

    // Automatic gain compensation for track summing
    gain_comp_amount: f32,   // 0 = off, 1 = full 1/sqrt(active_tracks)
    gain_comp_target: f32,   // 1.0 / sqrt(active_tracks), clamped [0.25, 1.0]
    gain_comp_current: f32,  // smoothed value (exponential approach to target)

    // Sidechain compression
    sidechain_amount: f32,
    sc_envelope: f32,
    kick_track_idx: Option<usize>,
    /// Song-wide `sidechain ... from=`; `None` falls back to the kick track.
    global_sc_source: Option<String>,
    /// One-pole coefficients for the source envelope. The release is what makes
    /// a duck read as a pump: too fast and the bass snaps back inside the kick,
    /// too slow and it never comes back.
    sc_attack_coeff: f32,
    sc_release_coeff: f32,
    /// Resolved once per scene: the audio path must not search by name.
    global_sc_idx: Option<usize>,

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
    // Capacity is reserved up front; see `push_trigger` for what happens when it fills.
    pending_triggers: Vec<PendingTrigger>,

    // Band metering costs nine one-poles per track per sample, so it is off
    // unless a render report asks for it.
    band_metering: bool,
    master_in_peak: f32,
    master_in_sum_sq: f64,
    master_in_samples: u64,

    // Per-track render buffers, allocated once. The audio path takes them with
    // `mem::take` and puts them back, so a block never touches the allocator.
    track_bufs_l: Vec<[f32; BLOCK_SIZE]>,
    track_bufs_r: Vec<[f32; BLOCK_SIZE]>,
    /// Recomputed every block: a track that is muted and has nothing left
    /// ringing is skipped whole. Allocated once, like the buffers above.
    track_silent: Vec<bool>,

    running: bool,
}

/// The arpeggiator holds at most this many notes (chord notes x octaves).
const MAX_ARP_NOTES: usize = 16;

/// How many delayed drum hits can be in flight at once. A nudge resolves within a
/// fraction of a step, so this is far above anything a pattern can produce.
const MAX_PENDING_TRIGGERS: usize = 128;

/// A note the sequencer owes the future: a nudged drum hit, or one note of a
/// subdivided step. `release` is the note this one replaces (255 = none), so a
/// run inside one step does not pile eight voices up on a poly instrument.
#[derive(Clone, Copy, Debug)]
struct PendingTrigger {
    samples: f32,
    inst_idx: usize,
    midi_note: u8,
    velocity: f32,
    release: u8,
    slide: bool,
}
const NO_RELEASE: u8 = 255;

/// The longest name in the parameter registry is 13 bytes; this leaves room.
/// `inline_name_holds_every_registry_param` keeps that true as params are added.
pub const INLINE_NAME_CAP: usize = 24;

/// A parameter name stored inline. Automation targets are resolved when a scene
/// starts, which happens on the audio thread, where a `String` would allocate.
#[derive(Clone, Copy)]
struct InlineName {
    bytes: [u8; INLINE_NAME_CAP],
    len: u8,
}

impl InlineName {
    /// `None` when the name does not fit. No registry name is that long.
    fn new(s: &str) -> Option<Self> {
        let src = s.as_bytes();
        if src.len() > INLINE_NAME_CAP {
            return None;
        }
        let mut bytes = [0u8; INLINE_NAME_CAP];
        bytes[..src.len()].copy_from_slice(src);
        Some(Self { bytes, len: src.len() as u8 })
    }

    fn as_str(&self) -> &str {
        // Always built from a &str, so the prefix is valid UTF-8.
        core::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or("")
    }
}

/// A running automation lane within a scene. `keyframes` points back into
/// `scenes[scene_idx].automations[auto_idx]` rather than copying them: applying a
/// scene happens on the audio thread.
struct ActiveAutomation {
    target: AutoTarget,
    scene_idx: usize,
    auto_idx: usize,
}

/// Resolved automation target.
enum AutoTarget {
    InstrumentParam { instrument_idx: usize, param_name: InlineName },
    MasterParam { param_name: InlineName },
    TrackLevel { track_idx: usize },
    /// Dry/wet of one named node in a track's insert chain, resolved to its
    /// position at compile time so the audio thread never looks up a name.
    TrackNodeWet { track_idx: usize, node_idx: usize },
    ReverbMix,
    DelayMix,
    ReverbFreeze,
}

/// Structured error from DSL parsing or compilation, preserving line/col info.
#[derive(Debug, Clone)]
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
                    sidechain_amount: t.sidechain.unwrap_or(0.0),
                    current_step: 0,
                    current_notes: [0; compiler::MAX_CHORD_NOTES],
                    current_notes_count: 0,
                    gate_samples_remaining: 0.0,
                    active: true,
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
                }
            })
            .collect();

        let meter = song.globals.meter;
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
        let swing = song.globals.swing.unwrap_or(0.5);
        let humanize_velocity = song.globals.humanize.unwrap_or(0.0);
        let humanize_timing = song.globals.humanize_timing.unwrap_or(0.0);

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
            gain_comp_amount: song.globals.gain_comp.unwrap_or(1.0),
            gain_comp_target: 1.0,
            gain_comp_current: 1.0,
            sidechain_amount,
            sc_envelope: 0.0,
            global_sc_source: song.globals.sidechain_source.clone(),
            sc_attack_coeff: attack_alpha(song.globals.sidechain_attack_ms.unwrap_or(0.005)),
            sc_release_coeff: release_coeff(song.globals.sidechain_release_ms.unwrap_or(4.5)),
            global_sc_idx: None,
            kick_track_idx,
            reverb_wet_level: 1.0,
            delay_wet_level: 1.0,
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
            running: false,
        };
        engine.retune_fx(tempo);
        engine.resolve_sidechain_sources(usize::MAX);
        engine
    }

    /// Bar-synced modulation inside chains follows the tempo.
    fn retune_fx(&mut self, bpm: f32) {
        for t in self.tracks.iter_mut() { t.insert_fx.set_bpm(bpm); }
        for b in self.buses.iter_mut() { b.fx_chain.set_bpm(bpm); }
        self.master_fx.set_bpm(bpm);
        self.reverb_return.set_bpm(bpm);
        self.delay_return.set_bpm(bpm);
    }

    pub fn start(&mut self) {
        self.running = true;
        self.timing_rng = Rng::new(7919); // reset for deterministic humanization
        self.reseed_tracks();
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

        // Update tempo if scene overrides it
        if let Some(t) = self.scenes[scene_idx].tempo {
            self.tempo = t;
            self.samples_per_step = SAMPLE_RATE * 60.0 / t / 4.0;
            // Update BPM on module instruments
            for inst in self.instruments.iter_mut() {
                inst.set_bpm(t);
            }
            self.send_delay.set_bpm(t, SAMPLE_RATE);
            self.retune_fx(t);
        }
        let tempo = self.tempo;
        let scene = &self.scenes[scene_idx];

        // Apply effect overrides
        if let Some(rmix) = scene.reverb_mix {
            self.reverb_wet_level = rmix;
        }
        if let Some(dmix) = scene.delay_mix {
            self.delay_wet_level = dmix;
        }
        // Freeze is per scene: it holds only where asked for.
        self.send_reverb.set_freeze(scene.reverb_freeze.unwrap_or(false));

        // Automation lanes are set up after the scene's tracks are activated
        // (targets resolve against the new layout, not the previous scene's).
        self.active_automations.clear();
        self.scene_step = 0;
        if self.arrangement_idx < self.arrangement.len() {
            let (_, repeat) = self.arrangement[self.arrangement_idx];
            self.scene_total_steps = repeat as usize * self.steps_per_bar;
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

        // Activate scene tracks: each scene track reconfigures the top-level
        // track slot with the same name (never by position).
        for st in scene.tracks.iter() {
            if let Some(i) = self.track_names.iter().position(|n| n == &st.name) {
                // The scene names a module; this track's own copy of it plays.
                let inst_idx = self.inst_for
                    .get(i * self.n_instruments + st.instrument_idx)
                    .copied()
                    .unwrap_or(st.instrument_idx);
                let tp = &mut self.tracks[i];
                tp.instrument_idx = inst_idx;
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
                tp.stereo_src = inst_idx < self.instruments.len()
                    && matches!(self.instruments[inst_idx], SongInstrument::Beats(_));
                tp.active = true;
                tp.current_step = 0;
                tp.current_notes_count = 0;
                tp.gate_samples_remaining = 0.0;
                tp.arp_cfg = st.arp;
                tp.arp = st.arp.map(|c| make_arp(&c, tempo));
            }
        }

        // Recompute kick track index after scene reassignment
        self.kick_track_idx = self.tracks.iter().position(|t| {
            t.active && t.instrument_idx < self.instruments.len()
                && matches!(self.instruments[t.instrument_idx], SongInstrument::Beats(_))
        });
        self.resolve_sidechain_sources(scene_idx);

        self.recompute_gain_comp();

        // Reuses the existing capacity: applying a scene runs on the audio thread.
        let mut lanes = core::mem::take(&mut self.active_automations);
        lanes.clear();
        for auto_idx in 0..self.scenes[scene_idx].automations.len() {
            let target_name = core::mem::take(&mut self.scenes[scene_idx].automations[auto_idx].target);
            let resolved = self.resolve_auto_target(&target_name);
            self.scenes[scene_idx].automations[auto_idx].target = target_name;
            if let Some(target) = resolved {
                lanes.push(ActiveAutomation { target, scene_idx, auto_idx });
            }
        }
        self.active_automations = lanes;
    }

    /// Bind every track's `sidechain from=` to a track index, and mark which
    /// tracks have to maintain an envelope. Names are resolved here rather than
    /// at compile time because a scene can rebind which track plays what.
    fn resolve_sidechain_sources(&mut self, scene_idx: usize) {
        for t in self.tracks.iter_mut() {
            t.sc_source = None;
            t.is_sc_source = false;
        }
        // Names are borrowed, never cloned: this runs on the audio thread at
        // every scene change and at every live swap.
        let global_idx = match self.global_sc_source.as_deref() {
            Some(name) => self.find_source_track(name),
            None => self.kick_track_idx,
        };
        for ti in 0..self.tracks.len() {
            if !self.tracks[ti].active { continue; }
            let named = self.scenes.get(scene_idx)
                .and_then(|s| s.tracks.iter().find(|st| st.name == self.track_names[ti]))
                .and_then(|st| st.sidechain_source.as_deref());
            let source = match named {
                Some(name) => self.find_source_track(name),
                None => global_idx,
            };
            if let Some(si) = source {
                if si != ti {
                    self.tracks[ti].sc_source = Some(si);
                    self.tracks[si].is_sc_source = true;
                }
            }
        }
        // The global sends duck against the same source the song names.
        self.global_sc_idx = global_idx;
        if let Some(si) = self.global_sc_idx {
            self.tracks[si].is_sc_source = true;
        }
        if let Some(ki) = self.kick_track_idx {
            self.tracks[ki].is_sc_source = true;
        }
    }

    /// A sidechain source names a track, or the module a track plays.
    fn find_source_track(&self, name: &str) -> Option<usize> {
        self.track_names.iter().position(|n| n == name)
            .filter(|i| self.tracks[*i].active)
            .or_else(|| {
                let inst = self.instrument_names.iter().position(|n| n == name)?;
                self.tracks.iter().position(|t| t.active && t.instrument_idx == inst)
            })
    }

    /// Recompute automatic gain compensation based on active track count.
    /// Uses equal-power scaling: 1/sqrt(N), clamped to [0.25, 1.0].
    fn recompute_gain_comp(&mut self) {
        let n = self.tracks.iter().filter(|t| t.active).count();
        let raw = if n <= 1 { 1.0 } else { (1.0 / math::sqrt(n as f32)).max(0.25) };
        // `gain_comp 0` disables it: every scene is mixed exactly as written.
        self.gain_comp_target = 1.0 + (raw - 1.0) * self.gain_comp_amount;
    }

    // ── Metering (for mix reports) ──

    /// Peak of a track after its level and pan, before the master chain.
    pub fn track_peak(&self, idx: usize) -> f32 {
        self.tracks.get(idx).map_or(0.0, |t| t.meter_peak)
    }

    /// RMS of a track after its level and pan, before the master chain.
    pub fn track_rms(&self, idx: usize) -> f32 {
        self.tracks.get(idx).map_or(0.0, |t| {
            if t.meter_samples == 0 { 0.0 } else { math::sqrt((t.meter_sum_sq / t.meter_samples as f64) as f32) }
        })
    }

    pub fn reset_meters(&mut self) {
        for t in self.tracks.iter_mut() {
            t.meter_peak = 0.0;
            t.meter_sum_sq = 0.0;
            t.meter_samples = 0;
            t.meter_mid_sq = 0.0;
            t.meter_side_sq = 0.0;
            t.band.reset();
        }
        for b in self.buses.iter_mut() {
            b.meter_peak = 0.0;
            b.meter_sum_sq = 0.0;
            b.meter_samples = 0;
        }
        self.master_in_peak = 0.0;
        self.master_in_sum_sq = 0.0;
        self.master_in_samples = 0;
    }

    /// Turn on per-track band analysis. Off by default: it costs nine one-pole
    /// sections per track per sample, which a live audio thread should not pay.
    pub fn set_band_metering(&mut self, on: bool) {
        self.band_metering = on;
    }

    /// Where this track sits in the stereo field: 0 is dead centre, 0.5 is hard
    /// to one side.
    pub fn track_width(&self, idx: usize) -> f32 {
        self.tracks.get(idx).map_or(0.0, |t| {
            let total = t.meter_mid_sq + t.meter_side_sq;
            if total <= 0.0 { 0.0 } else { (t.meter_side_sq / total) as f32 }
        })
    }

    /// Share of this track's energy in each of `analysis::BAND_NAMES`, in percent.
    pub fn track_bands(&self, idx: usize) -> [f32; 5] {
        self.tracks.get(idx).map_or([0.0; 5], |t| t.band.percentages())
    }

    /// Index into `analysis::BAND_NAMES` of the band this track mostly occupies.
    pub fn track_dominant_band(&self, idx: usize) -> Option<usize> {
        self.tracks.get(idx).and_then(|t| t.band.dominant())
    }

    pub fn bus_count(&self) -> usize { self.buses.len() }

    pub fn bus_name(&self, idx: usize) -> &str {
        self.buses.get(idx).map_or("", |b| b.name.as_str())
    }

    /// Peak of a bus after its own chain, before it reaches master.
    pub fn bus_peak(&self, idx: usize) -> f32 {
        self.buses.get(idx).map_or(0.0, |b| b.meter_peak)
    }

    pub fn bus_rms(&self, idx: usize) -> f32 {
        self.buses.get(idx).map_or(0.0, |b| {
            if b.meter_samples == 0 { 0.0 } else { math::sqrt((b.meter_sum_sq / b.meter_samples as f64) as f32) }
        })
    }

    /// Peak and RMS entering the master chain, so the caller can compare the
    /// crest factor before and after and see what the limiter took.
    pub fn master_input_peak_rms(&self) -> (f32, f32) {
        let rms = if self.master_in_samples == 0 {
            0.0
        } else {
            math::sqrt((self.master_in_sum_sq / self.master_in_samples as f64) as f32)
        };
        (self.master_in_peak, rms)
    }

    /// Automatic gain compensation applied right now (1.0 = none).
    pub fn gain_compensation(&self) -> f32 { self.gain_comp_current }

    /// How many tracks the last rendered block skipped whole: muted, nothing
    /// left ringing, and not feeding the sidechain. Skipping is audio-neutral
    /// by construction -- a muted track contributed zero before too -- so this
    /// is the only way to see the difference from outside without a stopwatch.
    pub fn silent_track_count(&self) -> usize {
        self.track_silent.iter().filter(|s| **s).count()
    }

    /// Resolve an automation target string to an AutoTarget.
    fn resolve_auto_target(&self, target: &str) -> Option<AutoTarget> {
        match target {
            "reverb_mix" => Some(AutoTarget::ReverbMix),
            "reverb_freeze" => Some(AutoTarget::ReverbFreeze),
            "delay_mix" => Some(AutoTarget::DelayMix),
            _ => {
                // Check for "instrument.param" or "track.level"
                if let Some(dot_pos) = target.find('.') {
                    let name = &target[..dot_pos];
                    let param = &target[dot_pos + 1..];

                    if name == "master" {
                        return InlineName::new(param).map(|param_name| AutoTarget::MasterParam { param_name });
                    }

                    // `<track>.<node> wet`: two dots, because the node is
                    // named inside a track. Resolved to a position here, once,
                    // so the audio thread never compares a string.
                    if let Some(inner) = param.strip_suffix(".wet") {
                        if let Some(ti) = self.track_names.iter().position(|n| n == name) {
                            if let Some(ni) = self.tracks[ti].fx_labels.iter()
                                .position(|l| l.as_ref().is_some_and(|l| l.as_str() == inner))
                            {
                                return Some(AutoTarget::TrackNodeWet { track_idx: ti, node_idx: ni });
                            }
                        }
                    }

                    if param == "level" {
                        // `<track> level` by track name, else by the instrument a track uses
                        let track_idx = self.track_names.iter()
                            .position(|n| n == name)
                            .or_else(|| {
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
                        return InlineName::new(param).map(|param_name| AutoTarget::InstrumentParam {
                            instrument_idx: inst_idx,
                            param_name,
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
        if step_index.is_multiple_of(2) {
            pair_duration * self.swing
        } else {
            pair_duration * (1.0 - self.swing)
        }
    }

    /// Release all active notes on a track.
    #[inline]
    fn release_track_notes(track: &mut TrackPlayback, instruments: &mut [SongInstrument]) {
        if let Some(arp) = track.arp.as_mut() {
            if let Some(ArpEvent::NoteOff(n)) = arp.stop() {
                if track.instrument_idx < instruments.len() {
                    instruments[track.instrument_idx].note_off(n);
                }
            }
            track.current_notes_count = 0;
            return;
        }
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

        if !self.running {
            return;
        }

        // Per-track instrument render buffers (L and R), borrowed from the engine so
        // the block allocates nothing. Instruments write the whole slice they are
        // given, so no clearing is needed between blocks.
        let track_count = self.tracks.len();
        let mut track_bufs_l = core::mem::take(&mut self.track_bufs_l);
        let mut track_bufs_r = core::mem::take(&mut self.track_bufs_r);

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
                    let offset = self.timing_rng.next_bipolar() * self.humanize_timing * max_offset;
                    self.current_step_duration += offset;
                }
            }

            // Process pending nudge triggers (delayed drum hits from groove blocks)
            let mut i = 0;
            while i < self.pending_triggers.len() {
                self.pending_triggers[i].samples -= 1.0;
                if self.pending_triggers[i].samples <= 0.0 {
                    let t = self.pending_triggers.swap_remove(i);
                    if t.inst_idx < self.instruments.len() {
                        if t.slide && self.instruments[t.inst_idx].slide_to(t.midi_note, t.velocity) {
                            // glided, nothing to release
                        } else {
                            if t.release != NO_RELEASE {
                                self.instruments[t.inst_idx].note_off(t.release);
                            }
                            self.instruments[t.inst_idx].note_on(t.midi_note, t.velocity);
                        }
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
                        if self.tracks[ti].arp.is_some() {
                            Self::release_track_notes(&mut self.tracks[ti], &mut self.instruments);
                            continue;
                        }
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

            // Arpeggiators: one tick per sample, events go straight to the instrument
            for ti in 0..track_count {
                if !self.tracks[ti].active { continue; }
                let inst_idx = self.trigger_instrument(ti);
                if inst_idx >= self.instruments.len() { continue; }
                if let Some(arp) = self.tracks[ti].arp.as_mut() {
                    match arp.tick() {
                        Some(ArpEvent::NoteOn(n, v)) => self.instruments[inst_idx].note_on(n, v),
                        Some(ArpEvent::NoteOff(n)) => self.instruments[inst_idx].note_off(n),
                        None => {}
                    }
                }
            }
        }

        // A muted track with nothing left ringing costs nothing: no voices, no
        // insert chain, no sidechain read, no mix. `level` is a fast-path edit
        // while scene membership is a structural one, so this is what lets a
        // live set keep a rig of voices loaded and bring them in without a swap.
        for ti in 0..track_count {
            // A ghost kick -- `level 0` but feeding the sidechain so it ducks the
            // mix without being heard -- is a real idiom, and it has to keep
            // running. Muting the fader does not mute the trigger.
            let muted = self.tracks[ti].active
                && self.tracks[ti].level <= 0.0
                && !self.tracks[ti].is_sc_source;
            // Pulling the fader releases what is held, the way a mute does.
            // Without this a tied note never ends: the tie branch extends the
            // gate every step whether or not the track fired the note, so a
            // held chord would keep one voice alive forever and the track
            // would never become idle enough to skip.
            if muted && self.tracks[ti].current_notes_count > 0 {
                Self::release_track_notes(&mut self.tracks[ti], &mut self.instruments);
            }
            let silent = muted
                && self.instruments.get(self.tracks[ti].instrument_idx)
                    .is_some_and(|i| i.is_idle());
            // Clear the chain on the way in, so a delay or reverb sitting in it
            // does not come back stale when the track is brought back.
            if silent && !self.track_silent[ti] {
                self.tracks[ti].insert_fx.reset();
            }
            self.track_silent[ti] = silent;
        }

        // Render each track's instrument (stereo-aware)
        for ti in 0..track_count {
            if !self.tracks[ti].active || self.track_silent[ti] { continue; }
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
            if !self.tracks[ti].active || self.track_silent[ti] { continue; }
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
            || self.reverb_sidechain > 0.0 || self.delay_sidechain > 0.0
            || self.tracks.iter().any(|t| t.active && t.sidechain_amount > 0.0);
        let (sc_attack, sc_release) = (self.sc_attack_coeff, self.sc_release_coeff);
        if has_any_sidechain {
            for s in 0..len {
                // Each source keeps its own envelope, so `sidechain from=bass`
                // breathes with the bass while the drums still duck the pads.
                for (si, buf_l) in track_bufs_l[..track_count].iter().enumerate() {
                    if !self.tracks[si].is_sc_source || !self.tracks[si].active { continue; }
                    // A Beats track uses its kick envelope rather than the raw
                    // signal, so hats and snares do not duck the mix.
                    let level = if let SongInstrument::Beats(ref m) = self.instruments[self.tracks[si].instrument_idx] {
                        if s < m.kick_env.len() { m.kick_env[s] } else { 0.0 }
                    } else {
                        math::abs(buf_l[s])
                    };
                    let env = self.tracks[si].sc_env;
                    self.tracks[si].sc_env = if level > env {
                        env + (level - env) * sc_attack
                    } else {
                        sc_release * env
                    };
                }
                // The sends follow the kick, or the song's named source.
                self.sc_envelope = self.global_sc_idx.map_or(0.0, |si| self.tracks[si].sc_env);
                for ti in 0..track_count {
                    if !self.tracks[ti].active { continue; }
                    let Some(si) = self.tracks[ti].sc_source else { continue };
                    let amount = if self.tracks[ti].sidechain_amount > 0.0 {
                        self.tracks[ti].sidechain_amount
                    } else {
                        self.sidechain_amount
                    };
                    if amount > 0.0 {
                        let duck = 1.0 - amount * self.tracks[si].sc_env;
                        track_bufs_l[ti][s] *= duck;
                        track_bufs_r[ti][s] *= duck;
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
        self.apply_automation();

        let band_metering = self.band_metering;
        for ti in 0..track_count {
            if !self.tracks[ti].active || self.track_silent[ti] { continue; }
            let gain = self.tracks[ti].level;
            let pan_l = self.tracks[ti].pan_l;
            let pan_r = self.tracks[ti].pan_r;
            let d_send = self.tracks[ti].delay_send;
            let r_send = self.tracks[ti].reverb_send;
            for s in 0..len {
                // Both channels always: mono sources were copied to R before the
                // insert chain, and stereo inserts (autopan, chorus) rely on R.
                let (sample_l, sample_r) = (
                    track_bufs_l[ti][s] * gain * pan_l,
                    track_bufs_r[ti][s] * gain * pan_r,
                );
                let t = &mut self.tracks[ti];
                t.meter_peak = t.meter_peak.max(sample_l.abs()).max(sample_r.abs());
                t.meter_sum_sq += (sample_l * sample_l + sample_r * sample_r) as f64;
                t.meter_samples += 2;
                let (mid, side) = ((sample_l + sample_r) as f64, (sample_l - sample_r) as f64);
                t.meter_mid_sq += mid * mid;
                t.meter_side_sq += side * side;
                if band_metering {
                    t.band.push_stereo(sample_l, sample_r);
                }

                // Bus send (mono sum to bus)
                if let Some((bus_idx, amount)) = self.tracks[ti].bus_send {
                    if bus_idx < self.buses.len() {
                        self.buses[bus_idx].buffer[s] += sample_l * amount;
                        self.buses[bus_idx].buffer_r[s] += sample_r * amount;
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
        // Always, even on silence. Gating on a non-zero input truncated the
        // tail of any reverb or delay on a bus the instant its tracks stopped,
        // and made `capture(start=N)` count processed samples rather than bars,
        // so its window landed on the wrong music. The global sends have always
        // been ticked unconditionally for the same reason.
        for bus in self.buses.iter_mut() {
            for s in 0..len {
                let (pl, pr) = bus.fx_chain.process_stereo(bus.buffer[s], bus.buffer_r[s]);
                bus.meter_peak = bus.meter_peak.max(math::abs(pl)).max(math::abs(pr));
                bus.meter_sum_sq += ((pl * pl + pr * pr) * 0.5) as f64;
                bus.meter_samples += 1;
                output_l[s] += pl;
                output_r[s] += pr;
            }
        }

        // Process global send effects (wet-only returns, scaled by wet levels)
        let dwet = self.delay_wet_level;
        let rwet = self.reverb_wet_level;
        // Always tick the sends: their tails must ring out (and freeze must
        // hold) after every track has gone silent.
        let duck_delay = self.delay_sidechain;
        let duck_reverb = self.reverb_sidechain;
        let sc = self.sc_envelope;
        for s in 0..len {
            let (mut dl, mut dr) = self.send_delay.process_stereo_wet(delay_in_l[s], delay_in_r[s]);
            if !self.delay_return.nodes.is_empty() {
                (dl, dr) = self.delay_return.process_stereo(dl, dr);
            }
            if duck_delay > 0.0 {
                let g = 1.0 - duck_delay * sc;
                dl *= g; dr *= g;
            }
            output_l[s] += dl * dwet;
            output_r[s] += dr * dwet;
            let (mut rl, mut rr) = self.send_reverb.process_stereo_in_wet(reverb_in_l[s], reverb_in_r[s]);
            if !self.reverb_return.nodes.is_empty() {
                (rl, rr) = self.reverb_return.process_stereo(rl, rr);
            }
            if duck_reverb > 0.0 {
                let g = 1.0 - duck_reverb * sc;
                rl *= g; rr *= g;
            }
            output_l[s] += rl * rwet;
            output_r[s] += rr * rwet;
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
                let (in_l, in_r) = (output_l[s] * g, output_r[s] * g);
                self.master_in_peak = self.master_in_peak.max(math::abs(in_l)).max(math::abs(in_r));
                self.master_in_sum_sq += (in_l * in_l + in_r * in_r) as f64;
                self.master_in_samples += 2;
                let (fl, fr) = self.master_fx.process_stereo(in_l, in_r);
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
                self.master_in_peak = self.master_in_peak.max(math::abs(output_l[s])).max(math::abs(output_r[s]));
                self.master_in_sum_sq += (output_l[s] * output_l[s] + output_r[s] * output_r[s]) as f64;
                self.master_in_samples += 2;
            }
        }

        self.track_bufs_l = track_bufs_l;
        self.track_bufs_r = track_bufs_r;
    }

    /// Schedule a nudged hit. If the queue is full the hit fires now rather than
    /// growing the queue: the audio thread must not allocate, and losing a few
    /// milliseconds of swing is better than losing the hit.
    fn push_trigger(
        pending: &mut Vec<PendingTrigger>,
        instruments: &mut [SongInstrument],
        samples: f32,
        inst_idx: usize,
        midi_note: u8,
        vel: f32,
    ) {
        Self::push_pending(pending, instruments,
            PendingTrigger { samples, inst_idx, midi_note, velocity: vel, release: NO_RELEASE, slide: false });
    }

    /// Queue a trigger. If the queue is full it fires now rather than growing:
    /// the audio thread must not allocate, and losing a few milliseconds of
    /// swing is better than losing the note.
    fn push_pending(
        pending: &mut Vec<PendingTrigger>,
        instruments: &mut [SongInstrument],
        t: PendingTrigger,
    ) {
        if pending.len() < MAX_PENDING_TRIGGERS {
            pending.push(t);
        } else if t.inst_idx < instruments.len() {
            if t.release != NO_RELEASE { instruments[t.inst_idx].note_off(t.release); }
            instruments[t.inst_idx].note_on(t.midi_note, t.velocity);
        }
    }

    /// Automation, evaluated once per block. It used to run in `advance_step`,
    /// which meant a sweep moved in sixteenth-note stairs: eight jumps a second
    /// on a resonant filter, which is heard as steps rather than a sweep.
    fn apply_automation(&mut self) {
        if self.scene_total_steps == 0 || self.active_automations.is_empty() {
            return;
        }
        let within_step = if self.current_step_duration > 0.0 {
            (self.sample_counter / self.current_step_duration).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let progress = ((self.scene_step as f32 + within_step) / self.scene_total_steps as f32).clamp(0.0, 1.0);
        let lanes = core::mem::take(&mut self.active_automations);
        for auto_lane in &lanes {
            let keyframes = &self.scenes[auto_lane.scene_idx].automations[auto_lane.auto_idx].keyframes;
            let value = interpolate_automation(keyframes, progress);
            match &auto_lane.target {
                AutoTarget::InstrumentParam { instrument_idx, param_name } => {
                    // `auto cloud cutoff` means the module, so it reaches every
                    // copy of it — one per track that named it.
                    if *instrument_idx < self.instruments.len() {
                        let names = &self.instrument_names;
                        for (i, inst) in self.instruments.iter_mut().enumerate() {
                            let same = match (names.get(*instrument_idx), names.get(i)) {
                                (Some(a), Some(b)) => a == b,
                                _ => i == *instrument_idx,
                            };
                            if same {
                                inst.set_param_by_name(param_name.as_str(), value);
                            }
                        }
                    }
                }
                AutoTarget::MasterParam { param_name } => {
                    self.master_fx.set_param(param_name.as_str(), value);
                }
                AutoTarget::TrackNodeWet { track_idx, node_idx } => {
                    if let Some(t) = self.tracks.get_mut(*track_idx) {
                        t.insert_fx.set_wet(*node_idx, value);
                    }
                }
                AutoTarget::TrackLevel { track_idx } => {
                    if *track_idx < self.tracks.len() {
                        self.tracks[*track_idx].level = value.clamp(0.0, MAX_TRACK_LEVEL);
                    }
                }
                AutoTarget::ReverbMix => self.reverb_wet_level = value,
                AutoTarget::ReverbFreeze => self.send_reverb.set_freeze(value >= 0.5),
                AutoTarget::DelayMix => self.delay_wet_level = value,
            }
        }
        self.active_automations = lanes;
    }

    /// The instrument a track triggers into, or `usize::MAX` when the track is
    /// muted. Every trigger site is already guarded by `inst_idx < len`, so a
    /// muted track fires nothing while its pattern keeps advancing: bringing it
    /// back in drops it onto the grid rather than where it left off. Firing
    /// nothing is what lets its voices run out, and an idle muted track is
    /// skipped whole by the render loop.
    fn trigger_instrument(&self, ti: usize) -> usize {
        if self.tracks[ti].level <= 0.0 && !self.tracks[ti].is_sc_source {
            return usize::MAX;
        }
        self.tracks[ti].instrument_idx
    }

    fn advance_step(&mut self) {
        // Cross the bar line before triggering, when the step about to fire is
        // the first of a bar. It used to be counted after the last step of the
        // previous bar fired, so `current_bar` read one sixteenth early: scene
        // changes killed the note the last step had just started, every
        // arranged render stopped a sixteenth short, and a hot-swap keyed on
        // the bar counter landed a sixteenth before the downbeat.
        if let Some(bar_of_step) = self.global_step.checked_div(self.steps_per_bar) {
            if bar_of_step > self.current_bar {
                self.current_bar = bar_of_step;
                self.check_arrangement_advance();
                if !self.running { return; }
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
                let inst_idx = self.trigger_instrument(ti);
                if inst_idx < self.instruments.len() {
                    for lane in &pattern.lanes {
                        if step_idx < lane.steps.len() {
                            if let CompiledStep::DrumHit { velocity, probability, roll, .. } = lane.steps[step_idx] {
                                // Probability gate: skip hit if random exceeds probability
                                if probability < 1.0 {
                                    let chance = self.tracks[ti].rng.next_f32();
                                    if chance > probability {
                                        continue; // skip this hit
                                    }
                                }
                                let raw_vel = velocity * self.tracks[ti].velocity;
                                let vel = Self::humanize_vel(raw_vel, self.humanize_velocity, &mut self.tracks[ti].rng);

                                // Per-lane nudge: delay trigger by nudge * step_samples
                                let nudge_samples = lane.nudge * self.samples_per_step;
                                if nudge_samples.abs() > 0.5 && nudge_samples > 0.0 {
                                    // Positive nudge = delay trigger
                                    Self::push_trigger(&mut self.pending_triggers, &mut self.instruments,
                                        nudge_samples, inst_idx, lane.midi_note, vel);
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

            // Arp tracks: the step only updates the held notes; the arp plays them.
            if self.tracks[ti].arp.is_some() && !matches!(step, CompiledStep::Tie) {
                let next_step_idx = (step_idx + 1) % pattern.steps.len();
                let next_is_tie = matches!(pattern.steps[next_step_idx], CompiledStep::Tie);
                let timing = StepTiming {
                    humanize_velocity: self.humanize_velocity,
                    samples_per_step: self.samples_per_step,
                    step_duration: self.current_step_duration,
                };
                Self::advance_arp_track(
                    &mut self.tracks[ti], &mut self.instruments,
                    timing, step, next_is_tie,
                );
                self.tracks[ti].current_step += 1;
                continue;
            }

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
                    // A `~note` next step needs this note still gated to glide from.
                    let next_slides = matches!(pattern.steps[next_step_idx], CompiledStep::NoteOn { slide: true, .. });

                    match step {
                        // A run inside one step. The first note fires now and the
                        // rest go on the pending queue, spaced across this step's
                        // REAL duration -- `effective_step_samples` already has the
                        // swing in it, so a subdivided step swings with everything
                        // else instead of quietly opting out.
                        CompiledStep::Subdiv { notes: subs, count, plock } => {
                            let n = (count as usize).clamp(1, compiler::MAX_SUBDIV);
                            let tv = self.tracks[ti].velocity;
                            let mut vels = [0.0f32; compiler::MAX_SUBDIV];
                            for k in 0..n {
                                vels[k] = Self::humanize_vel(
                                    subs[k].velocity * tv, self.humanize_velocity, &mut self.tracks[ti].rng);
                            }
                            let step_samples = self.effective_step_samples(self.global_step);
                            let interval = step_samples / n as f32;
                            Self::release_track_notes(&mut self.tracks[ti], &mut self.instruments);
                            let inst_idx = self.trigger_instrument(ti);
                            if inst_idx < self.instruments.len() {
                                self.instruments[inst_idx].stage_plock(
                                    plock.cutoff, plock.env_depth, plock.resonance);
                                self.instruments[inst_idx].note_on(subs[0].midi_note, vels[0]);
                                for k in 1..n {
                                    Self::push_pending(
                                        &mut self.pending_triggers, &mut self.instruments,
                                        PendingTrigger {
                                            samples: interval * k as f32,
                                            inst_idx,
                                            midi_note: subs[k].midi_note,
                                            velocity: vels[k],
                                            // each note takes the previous one's place,
                                            // so eight of them do not stack up on a poly
                                            release: if subs[k].slide {
                                                NO_RELEASE
                                            } else {
                                                subs[k - 1].midi_note
                                            },
                                            slide: subs[k].slide,
                                        });
                                }
                            }
                            // The step ends holding its LAST note, so a tie or a
                            // slide after it continues from where the run landed.
                            self.tracks[ti].current_notes[0] = subs[n - 1].midi_note;
                            self.tracks[ti].current_notes_count = 1;
                            let step_gate = plock.gate.unwrap_or(gate);
                            self.tracks[ti].gate_samples_remaining = if next_is_tie {
                                self.samples_per_step * 2.0
                            } else if next_slides {
                                self.samples_per_step * 1.5
                            } else {
                                // hold until the last note has started, then gate
                                // that one: a short gate must not cut the run off
                                interval * (n - 1) as f32 + interval * step_gate
                            };
                        }
                        CompiledStep::NoteOn { midi_note, velocity, plock, slide } => {
                            // If the same single note is already playing (pattern loop),
                            // just extend gate — don't re-trigger (avoids click/re-attack).
                            let same_note = self.tracks[ti].current_notes_count == 1
                                && self.tracks[ti].current_notes[0] == midi_note;
                            let held = self.tracks[ti].current_notes_count > 0;
                            if same_note && next_is_tie {
                                // Sustain continuation — treat as tie
                                self.tracks[ti].gate_samples_remaining = self.samples_per_step * 2.0;
                            } else if slide && held {
                                // Slide: glide pitch without retriggering (303-style)
                                let inst_idx = self.trigger_instrument(ti);
                                let raw_vel = velocity * self.tracks[ti].velocity;
                                let vel = Self::humanize_vel(raw_vel, self.humanize_velocity, &mut self.tracks[ti].rng);
                                let handled = inst_idx < self.instruments.len() && {
                                    self.instruments[inst_idx].stage_plock(plock.cutoff, plock.env_depth, plock.resonance);
                                    self.instruments[inst_idx].slide_to(midi_note, vel)
                                };
                                if !handled {
                                    Self::release_track_notes(&mut self.tracks[ti], &mut self.instruments);
                                    if inst_idx < self.instruments.len() {
                                        self.instruments[inst_idx].note_on(midi_note, vel);
                                    }
                                }
                                self.tracks[ti].current_notes[0] = midi_note;
                                self.tracks[ti].current_notes_count = 1;
                                if next_is_tie {
                                    self.tracks[ti].gate_samples_remaining = self.samples_per_step * 2.0;
                                } else if next_slides {
                                    self.tracks[ti].gate_samples_remaining = self.samples_per_step * 1.5;
                                } else {
                                    let step_gate = plock.gate.unwrap_or(gate);
                                    self.tracks[ti].gate_samples_remaining = self.current_step_duration * step_gate;
                                }
                            } else {
                                // Release previous notes
                                Self::release_track_notes(
                                    &mut self.tracks[ti],
                                    &mut self.instruments,
                                );
                                let inst_idx = self.trigger_instrument(ti);
                                let raw_vel = velocity * self.tracks[ti].velocity;
                                let vel = Self::humanize_vel(raw_vel, self.humanize_velocity, &mut self.tracks[ti].rng);
                                if inst_idx < self.instruments.len() {
                                    self.instruments[inst_idx].stage_plock(plock.cutoff, plock.env_depth, plock.resonance);
                                    self.instruments[inst_idx].note_on(midi_note, vel);
                                }
                                self.tracks[ti].current_notes[0] = midi_note;
                                self.tracks[ti].current_notes_count = 1;
                                if next_is_tie {
                                    self.tracks[ti].gate_samples_remaining = self.samples_per_step * 2.0;
                                } else if next_slides {
                                    self.tracks[ti].gate_samples_remaining = self.samples_per_step * 1.5;
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
                                let inst_idx = self.trigger_instrument(ti);
                                if inst_idx < self.instruments.len() {
                                    self.instruments[inst_idx].stage_plock(plock.cutoff, plock.env_depth, plock.resonance);
                                    for (ni, n) in notes[..c].iter().enumerate() {
                                        let raw_vel = n.velocity * self.tracks[ti].velocity;
                                        let vel = Self::humanize_vel(raw_vel, self.humanize_velocity, &mut self.tracks[ti].rng);
                                        self.instruments[inst_idx].note_on(n.midi_note, vel);
                                        self.tracks[ti].current_notes[ni] = n.midi_note;
                                    }
                                    self.tracks[ti].current_notes_count = count;
                                }
                                if next_is_tie {
                                    self.tracks[ti].gate_samples_remaining = self.samples_per_step * 2.0;
                                } else if next_slides {
                                    self.tracks[ti].gate_samples_remaining = self.samples_per_step * 1.5;
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
                            let inst_idx = self.trigger_instrument(ti);
                            let raw_vel = velocity * self.tracks[ti].velocity;
                            let vel = Self::humanize_vel(raw_vel, self.humanize_velocity, &mut self.tracks[ti].rng);
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
    }

    /// Step handler for arp tracks: update the held notes, (re)start the arp,
    /// and set the track gate that will eventually stop it.
    fn advance_arp_track(
        track: &mut TrackPlayback,
        instruments: &mut [SongInstrument],
        timing: StepTiming,
        step: CompiledStep,
        next_is_tie: bool,
    ) {
        let inst_idx = track.instrument_idx;
        let mut notes = [0u8; compiler::MAX_CHORD_NOTES];
        let (count, step_vel, step_gate): (usize, f32, Option<f32>) = match step {
            CompiledStep::NoteOn { midi_note, velocity, plock, .. } => {
                notes[0] = midi_note;
                if inst_idx < instruments.len() {
                    instruments[inst_idx].stage_plock(plock.cutoff, plock.env_depth, plock.resonance);
                }
                (1, velocity, plock.gate)
            }
            // On an arp track a group is just the note set to cycle: the arp
            // owns the timing, so its own rate wins over the subdivision.
            CompiledStep::Subdiv { notes: sn, count: c, plock } => {
                let count = (c as usize).min(compiler::MAX_CHORD_NOTES);
                for i in 0..count {
                    notes[i] = sn[i].midi_note;
                }
                if inst_idx < instruments.len() {
                    instruments[inst_idx].stage_plock(plock.cutoff, plock.env_depth, plock.resonance);
                }
                (count, sn[0].velocity, plock.gate)
            }
            CompiledStep::Chord { notes: cn, count: c, plock } => {
                let count = (c as usize).min(compiler::MAX_CHORD_NOTES);
                for i in 0..count {
                    notes[i] = cn[i].midi_note;
                }
                if inst_idx < instruments.len() {
                    instruments[inst_idx].stage_plock(plock.cutoff, plock.env_depth, plock.resonance);
                }
                (count, cn[0].velocity, plock.gate)
            }
            CompiledStep::DrumHit { velocity, plock, .. } => {
                notes[0] = 36;
                (1, velocity, plock.gate)
            }
            CompiledStep::Rest => {
                Self::release_track_notes(track, instruments);
                return;
            }
            CompiledStep::Tie => return,
        };

        let same = count == track.current_notes_count as usize
            && (0..count).all(|i| track.current_notes[i] == notes[i]);
        track.current_notes[..count].copy_from_slice(&notes[..count]);
        track.current_notes_count = count as u8;

        let raw_vel = step_vel * track.velocity;
        let vel = Self::humanize_vel(raw_vel, timing.humanize_velocity, &mut track.rng);

        // Sorted ascending and expanded across octaves so "up" really goes up.
        // Fixed buffers: this runs on the audio thread, once per step.
        let mut sorted = [0u8; compiler::MAX_CHORD_NOTES];
        sorted[..count].copy_from_slice(&notes[..count]);
        sorted[..count].sort_unstable();
        let octaves = track.arp_cfg.map_or(1, |c| c.octaves).max(1);
        let mut list = [0u8; MAX_ARP_NOTES];
        let mut list_len = 0usize;
        for o in 0..octaves {
            for &n in &sorted[..count] {
                let v = n as u16 + 12 * o as u16;
                if v <= 127 && list_len < MAX_ARP_NOTES {
                    list[list_len] = v as u8;
                    list_len += 1;
                }
            }
        }
        let list = &list[..list_len];

        let mut pending_off = None;
        if let Some(arp) = track.arp.as_mut() {
            arp.set_notes(list);
            arp.set_velocity(vel);
            if !same || !arp.is_active() {
                // New material: close the open arp note and restart from the first note.
                if let Some(ArpEvent::NoteOff(n)) = arp.stop() {
                    pending_off = Some(n);
                }
                arp.start(vel);
            }
        }
        if let Some(n) = pending_off {
            if inst_idx < instruments.len() {
                instruments[inst_idx].note_off(n);
            }
        }

        track.gate_samples_remaining = if next_is_tie {
            timing.samples_per_step * 2.0
        } else {
            timing.step_duration * step_gate.unwrap_or(track.gate)
        };
    }

    /// True while a track's arpeggiator is running (for UI activity LEDs).
    pub fn track_arp_active(&self, idx: usize) -> bool {
        self.tracks.get(idx).and_then(|t| t.arp.as_ref()).is_some_and(|a| a.is_active())
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
    /// Render `steps` sequencer steps from the current position without
    /// restarting. Call `start()` first. Useful for tests and previews.
    pub fn render_steps(&mut self, steps: usize) -> (Vec<f32>, Vec<f32>) {
        let total = (steps as f32 * self.samples_per_step) as usize;
        let mut out_l = vec![0.0f32; total];
        let mut out_r = vec![0.0f32; total];
        let mut pos = 0;
        while pos < total {
            let chunk = BLOCK_SIZE.min(total - pos);
            self.process_block_stereo(&mut out_l[pos..pos + chunk], &mut out_r[pos..pos + chunk]);
            pos += chunk;
        }
        (out_l, out_r)
    }

    pub fn arrangement_bars(&self) -> u32 {
        self.arrangement.iter().map(|(_, r)| *r).sum()
    }

    /// The arrangement as `(scene name, bars, BPM)`, in order. A mix report
    /// can only talk about a song's shape if it knows where the sections are,
    /// and the tempo has to come along because a scene can change it -- put
    /// every section on the song's opening tempo and the later ones land in
    /// the wrong place.
    pub fn sections(&self) -> Vec<(&str, u32, f32)> {
        let mut bpm = self.tempo;
        let mut out = Vec::new();
        for (i, repeat) in &self.arrangement {
            let Some(scene) = self.scenes.get(*i) else { continue };
            if let Some(t) = scene.tempo { bpm = t; }
            out.push((scene.name.as_str(), *repeat, bpm));
        }
        out
    }

    pub fn tempo(&self) -> f32 { self.tempo }
    /// Sixteenths in a bar: 16 in 4/4, 12 in 3/4. A set counts its steps in
    /// bars and a step can change the tempo or the meter, so the length of a
    /// bar has to be asked of the engine that is about to play it.
    pub fn steps_per_bar(&self) -> usize { self.steps_per_bar }

    pub fn reset(&mut self) {
        for track in self.tracks.iter_mut() {
            if let Some(arp) = track.arp.as_mut() {
                arp.reset();
            }
        }
        self.running = false;
        self.sample_counter = 0.0;
        self.current_step_duration = self.samples_per_step;
        self.timing_rng = Rng::new(7919);
        self.reseed_tracks();
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
        self.reverb_return.reset();
        self.delay_return.reset();
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
        self.timing_rng = Rng::new(7919);
        self.reseed_tracks();
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
            // `apply_scene` starts the scene clock at zero. Landing mid-scene
            // must not restart its automation: a sweep that was three bars in
            // stays three bars in.
            self.scene_step = self.arrangement_bar_count as usize * self.steps_per_bar;
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
            track.level = level.clamp(0.0, MAX_TRACK_LEVEL);
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
        for track in self.tracks.iter_mut() {
            if let (Some(arp), Some(cfg)) = (track.arp.as_mut(), track.arp_cfg) {
                arp.set_bpm(bpm * cfg.rate_mult);
            }
        }
        self.send_delay.set_bpm(bpm, SAMPLE_RATE);
        self.retune_fx(bpm);
    }

    /// Pattern index a track is currently playing (for tests and UI).
    pub fn track_pattern(&self, idx: usize) -> usize {
        self.tracks.get(idx).map_or(0, |t| t.pattern_idx)
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

    /// Switch one node of a track's insert chain in or out. `wet` 0 bypasses
    /// it and stops it being processed, 1 is the node alone. Returns false if
    /// there is no such track or node, so a live edit can tell a no-op from a
    /// real one. This is a fast-path edit: the node is already built, so it
    /// applies inside the bar with no swap and no voice restart.
    pub fn set_node_wet(&mut self, track_idx: usize, node_idx: usize, wet: f32) -> bool {
        self.tracks.get_mut(track_idx)
            .is_some_and(|t| t.insert_fx.set_wet(node_idx, wet))
    }

    /// The `as` name of each node in a track's insert chain, in chain order;
    /// `None` for a node nobody named.
    pub fn track_node_labels(&self, track_idx: usize) -> impl Iterator<Item = Option<&str>> {
        self.tracks.get(track_idx).into_iter()
            .flat_map(|t| t.fx_labels.iter().map(|l| l.as_ref().map(|l| l.as_str())))
    }

    /// A note played from outside the pattern -- a key, a pad -- on the
    /// instrument track `track_idx` plays. A muted track does not sound it,
    /// the same as its own pattern: bring the fader up to hear what you play.
    pub fn live_note_on(&mut self, track_idx: usize, note: u8, velocity: f32) {
        if track_idx >= self.tracks.len() { return; }
        let inst = self.trigger_instrument(track_idx);
        if let Some(i) = self.instruments.get_mut(inst) {
            i.note_on(note, velocity.clamp(0.0, 1.0));
        }
    }

    /// Release a note played with [`Self::live_note_on`]. Reaches the
    /// instrument even on a muted track, so a note held while the fader
    /// came down still lets go.
    pub fn live_note_off(&mut self, track_idx: usize, note: u8) {
        let Some(inst) = self.tracks.get(track_idx).map(|t| t.instrument_idx) else { return };
        if let Some(i) = self.instruments.get_mut(inst) {
            i.note_off(note);
        }
    }

    /// Bend instrument `inst_idx` by `ratio` of its frequency (1.0 is none).
    pub fn set_pitch_bend(&mut self, inst_idx: usize, ratio: f32) {
        if let Some(i) = self.instruments.get_mut(inst_idx) {
            i.set_pitch_bend(ratio);
        }
    }

    /// Wet level of the global reverb return, 0..1 of what `reverb` sets up.
    /// The same value `reverb_mix =` and `auto reverb_mix` write, so a scene
    /// that sets it takes over again when it starts.
    pub fn set_reverb_mix(&mut self, mix: f32) {
        self.reverb_wet_level = mix.clamp(0.0, 1.0);
    }

    /// Wet level of the global delay return, like [`Self::set_reverb_mix`].
    pub fn set_delay_mix(&mut self, mix: f32) {
        self.delay_wet_level = mix.clamp(0.0, 1.0);
    }

    /// Hold the reverb tail: no decay and no new input while frozen.
    pub fn set_reverb_freeze(&mut self, frozen: bool) {
        self.send_reverb.set_freeze(frozen);
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

    /// The live instrument a track plays right now. With one copy per track
    /// that names a module this is not the compiled index.
    pub fn track_instrument(&self, idx: usize) -> Option<usize> {
        self.tracks.get(idx).map(|t| t.instrument_idx)
    }

    /// Set a parameter already resolved to a registry id. False if the
    /// instrument does not exist or the id belongs to another module kind.
    pub fn set_module_param_id(&mut self, inst_idx: usize, id: ParamId, value: f32) -> bool {
        match self.instruments.get_mut(inst_idx) {
            Some(inst) => apply_param(inst, id, value),
            None => false,
        }
    }

    // ═══════════════════════════════════════════════════════
    //  Live session support (see live.rs)
    // ═══════════════════════════════════════════════════════

    /// Samples this engine will render before the next step fires: the trigger
    /// happens on the first sample where the step clock reaches the step
    /// duration. Zero means the next sample rendered is that step.
    fn samples_until_step(&self) -> f32 {
        let k = -math::floor(-(self.current_step_duration - self.sample_counter));
        (k - 1.0).max(0.0)
    }

    /// Samples this engine will render before the first step of the next bar
    /// fires. Exact within the current step; across the remaining steps it
    /// assumes no timing humanization, which a caller absorbs by asking again
    /// every block. `usize::MAX` when not running.
    pub fn samples_until_bar(&self) -> usize {
        if !self.running || self.steps_per_bar == 0 {
            return usize::MAX;
        }
        let mut total = self.samples_until_step();
        let mut step = self.global_step;
        while !step.is_multiple_of(self.steps_per_bar) {
            step += 1;
            total += self.effective_step_samples(step);
        }
        total as usize
    }

    /// Bar the next step to fire belongs to. On a bar line this is the bar
    /// about to start, while `current_bar` still reads the one that ended.
    pub fn bar_of_next_step(&self) -> usize {
        self.global_step.checked_div(self.steps_per_bar).unwrap_or(0)
    }

    /// Position the engine as if it had just played through bar `bar - 1`
    /// and were about to fire the downbeat of `bar`: the previous entry's
    /// scene is active, the bar line has not been crossed. A live swap starts
    /// the new engine here so that it crosses the line itself, on the same
    /// sample and with the same consequences (scene change, arrangement end,
    /// automation progress) as the engine it replaces would have.
    pub fn start_before_bar(&mut self, bar: usize) {
        if bar == 0 {
            self.start_from_bar(0);
            return;
        }
        self.start_from_bar(bar - 1);
        self.global_step = bar * self.steps_per_bar;
        if !self.arrangement.is_empty() {
            self.scene_step = (self.arrangement_bar_count as usize + 1) * self.steps_per_bar;
        }
        self.current_step_duration = self.effective_step_samples(self.global_step);
        self.sample_counter = self.current_step_duration;
    }

    /// Stop sequencing but keep rendering: what sounds decays, nothing new
    /// fires. The retiring half of a swap runs like this under a fade-out.
    pub fn coast(&mut self) {
        self.current_step_duration = f32::MAX;
        self.sample_counter = 0.0;
        self.pending_triggers.clear();
        self.active_automations.clear();
        self.scene_total_steps = 0;
        for t in self.tracks.iter_mut() {
            t.arp = None;
            t.gate_samples_remaining = 0.0;
        }
    }

    /// Take over state from `old`, the engine this one replaces at the start
    /// of the block in which the next bar line falls, `until` samples in.
    /// What the source text did not change keeps sounding: instruments keep
    /// their voices, sends and buses keep their tails, inherited tracks keep
    /// their place in the pattern and their held notes. Everything moves with
    /// `mem::swap`, so this allocates nothing. Call after `start_before_bar`.
    pub fn inherit_from(&mut self, old: &mut SongEngine, map: &crate::live::Inherit, until: usize) {
        // Old tracks nobody continues release their notes now, on the old
        // instruments, before an instrument moves over with a note that no
        // track in the new engine would ever release.
        for j in 0..old.tracks.len() {
            let continued = map.tracks.contains(&Some(j));
            if !continued {
                Self::release_track_notes(&mut old.tracks[j], &mut old.instruments);
                old.tracks[j].gate_samples_remaining = 0.0;
            }
        }
        for (i, m) in map.instruments.iter().enumerate() {
            if let Some(j) = *m {
                if i < self.instruments.len() && j < old.instruments.len() {
                    core::mem::swap(&mut self.instruments[i], &mut old.instruments[j]);
                    self.instruments[i].set_bpm(self.tempo);
                }
            }
        }
        for (i, m) in map.tracks.iter().enumerate() {
            let Some(j) = *m else { continue };
            if i >= self.tracks.len() || j >= old.tracks.len() { continue; }
            let n = &mut self.tracks[i];
            let o = &mut old.tracks[j];
            core::mem::swap(&mut n.current_step, &mut o.current_step);
            core::mem::swap(&mut n.current_notes, &mut o.current_notes);
            core::mem::swap(&mut n.current_notes_count, &mut o.current_notes_count);
            core::mem::swap(&mut n.gate_samples_remaining, &mut o.gate_samples_remaining);
            core::mem::swap(&mut n.arp, &mut o.arp);
            core::mem::swap(&mut n.insert_fx, &mut o.insert_fx);
            n.insert_fx.set_bpm(self.tempo);
            if let (Some(arp), Some(cfg)) = (n.arp.as_mut(), n.arp_cfg) {
                arp.set_bpm(self.tempo * cfg.rate_mult);
            }
        }
        // Nudged hits still in flight follow their instrument. The queue's
        // capacity is reserved, so this cannot allocate.
        for k in 0..old.pending_triggers.len() {
            let t = old.pending_triggers[k];
            if let Some(i) = map.instruments.iter().position(|m| *m == Some(t.inst_idx)) {
                if self.pending_triggers.len() < MAX_PENDING_TRIGGERS {
                    self.pending_triggers.push(PendingTrigger { inst_idx: i, ..t });
                }
            }
        }
        old.pending_triggers.clear();
        // The sidechain follower of a track is the envelope of whatever it
        // plays: it continues by name even where the track itself does not.
        for i in 0..self.tracks.len() {
            if let Some(j) = old.track_names.iter().position(|n| *n == self.track_names[i]) {
                self.tracks[i].sc_env = old.tracks[j].sc_env;
                // Same name, same voice: it keeps the point it had reached in
                // its own random stream, so a swap does not rewind its feel.
                core::mem::swap(&mut self.tracks[i].rng, &mut old.tracks[j].rng);
            }
        }
        for (i, m) in map.buses.iter().enumerate() {
            if let Some(j) = *m {
                if i < self.buses.len() && j < old.buses.len() {
                    core::mem::swap(&mut self.buses[i].fx_chain, &mut old.buses[j].fx_chain);
                    self.buses[i].fx_chain.set_bpm(self.tempo);
                }
            }
        }
        if map.sends {
            // Freeze belongs to the scene this engine is starting, not to the
            // reverb that carried the tail here.
            let frozen = self.send_reverb.is_frozen();
            core::mem::swap(&mut self.send_delay, &mut old.send_delay);
            core::mem::swap(&mut self.send_reverb, &mut old.send_reverb);
            core::mem::swap(&mut self.reverb_return, &mut old.reverb_return);
            core::mem::swap(&mut self.delay_return, &mut old.delay_return);
            self.send_reverb.set_freeze(frozen);
            self.send_delay.set_bpm(self.tempo, SAMPLE_RATE);
            self.reverb_return.set_bpm(self.tempo);
            self.delay_return.set_bpm(self.tempo);
        }
        if map.master {
            core::mem::swap(&mut self.master_fx, &mut old.master_fx);
            self.master_fx.set_bpm(self.tempo);
        }
        // Smoothed and random state continues regardless of what changed: a
        // gain ramp restarting or the humanize sequence rewinding is audible.
        core::mem::swap(&mut self.timing_rng, &mut old.timing_rng);
        self.gain_comp_current = old.gain_comp_current;
        self.sc_envelope = old.sc_envelope;
        // The downbeat fires `until` samples into this block, on the old
        // clock; the new clock is set so it fires there too, carrying the
        // fractional residue (5512.5 samples per step at 120 BPM) with it.
        if old.running {
            let until = until as f32;
            let residue = (old.sample_counter + until + 1.0 - old.current_step_duration).clamp(0.0, 1.0);
            self.sample_counter = self.current_step_duration - until - 1.0 + residue;
        }
    }

    /// Swing 0.5 (straight) ..= 0.75 (hard shuffle). Takes effect on the next step.
    pub fn set_swing(&mut self, swing: f32) {
        self.swing = swing.clamp(0.5, 0.75);
    }

    /// Put every track's random stream back to its seed. Called wherever the
    /// clock stream is reset, so a render is reproducible start to start.
    fn reseed_tracks(&mut self) {
        for (i, t) in self.tracks.iter_mut().enumerate() {
            let name = self.track_names.get(i).map(|s| s.as_str()).unwrap_or("");
            t.rng = Rng::new(seed_from_name(name));
        }
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
