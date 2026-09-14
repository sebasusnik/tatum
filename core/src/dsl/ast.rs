extern crate alloc;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

/// Top-level song structure.
#[derive(Debug, Clone)]
pub struct Song {
    pub globals: Globals,
    pub buses: Vec<BusDef>,
    pub instruments: Vec<InstrumentDef>,
    pub module_defs: Vec<ModuleDef>,
    pub patterns: Vec<PatternDef>,
    pub tracks: Vec<TrackDef>,
    pub bus_chains: Vec<BusChainDef>,
    pub master: Option<MasterDef>,
    pub scenes: Vec<SceneDef>,
    pub arrangement: Vec<ArrangeEntry>,
    pub grooves: Vec<GrooveDef>,
}

#[derive(Debug, Clone)]
pub struct Globals {
    pub tempo: f32,
    pub meter: (u8, u8),
    pub scale: Option<ScaleDef>,
    pub sidechain: f32,
    pub swing: Option<f32>,              // 0.5 = straight, 0.67 = triplet feel
    pub humanize: Option<f32>,           // velocity humanization 0.0-1.0
    pub humanize_timing: Option<f32>,    // timing humanization 0.0-1.0
}

impl Default for Globals {
    fn default() -> Self {
        Self {
            tempo: 120.0,
            meter: (4, 4),
            scale: None,
            sidechain: 0.0,
            swing: None,
            humanize: None,
            humanize_timing: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScaleDef {
    pub root: String,    // "A", "C#", "Bb", etc
    pub kind: String,    // "minor", "major", "dorian", etc
}

#[derive(Debug, Clone)]
pub struct BusDef {
    pub name: String,
}

/// Instrument definition: a modular signal graph.
#[derive(Debug, Clone)]
pub struct InstrumentDef {
    pub name: String,
    pub gain: Option<f32>,
    pub nodes: Vec<NodeDef>,
    pub connections: Vec<ConnectionDef>,
}

/// A node in an instrument graph.
#[derive(Debug, Clone)]
pub struct NodeDef {
    pub kind: String,           // "osc", "lowpass", "adsr", "noise", "mix", etc
    pub alias: Option<String>,  // "as osc1"
    pub params: Vec<Param>,     // positional + named params
}

/// A connection between nodes: a > b > c
#[derive(Debug, Clone)]
pub struct ConnectionDef {
    pub from: String,  // node alias or "in"
    pub to: String,    // node alias or "out" / "mix" / "master"
}

/// Parameter value in a node definition.
#[derive(Debug, Clone)]
pub enum Param {
    Float(f32),
    Named(String, f32),
    Waveform(String),        // "saw", "sine", "square", "triangle"
    RhythmDiv(u8, u8),       // 1/4, 1/8, etc
    Expr(Expr),              // 55 * 2 — simple arithmetic
}

/// Simple expression for frequency math like `55 * 2`.
#[derive(Debug, Clone)]
pub enum Expr {
    Num(f32),
    Mul(Box<Expr>, Box<Expr>),
    Add(Box<Expr>, Box<Expr>),
}

impl Expr {
    pub fn eval(&self) -> f32 {
        match self {
            Expr::Num(v) => *v,
            Expr::Mul(a, b) => a.eval() * b.eval(),
            Expr::Add(a, b) => a.eval() + b.eval(),
        }
    }
}

/// Note pattern definition.
#[derive(Debug, Clone)]
pub struct PatternDef {
    pub name: String,
    pub rows: Vec<Vec<Step>>,  // rows of steps (each row = one bar line)
    pub lane_labels: Vec<String>,  // empty = sequential, non-empty = parallel drum lanes (lane_labels[i] names rows[i])
}

/// Per-step parameter lock: overrides instrument params for a single step.
#[derive(Debug, Clone, Default)]
pub struct PLock {
    pub cutoff: Option<f32>,
    pub env_depth: Option<f32>,
    pub resonance: Option<f32>,
    pub gate: Option<f32>,
}

/// A single step in a pattern.
#[derive(Debug, Clone)]
pub enum Step {
    Note(NoteStep),
    Chord(ChordStep),
    DrumHit(DrumStep),
    Rest,
    Tie,
}

/// A chord: multiple notes triggered simultaneously on one step.
#[derive(Debug, Clone)]
pub struct ChordStep {
    pub notes: Vec<NoteStep>,
    pub velocity: Option<f32>,  // shared velocity (overrides individual if set)
    pub plock: PLock,
}

#[derive(Debug, Clone)]
pub enum NoteRef {
    Absolute(String),        // "A1", "C#4", "G0"
    Degree(u8, u8),          // (degree 1-7, octave 0-9)
}

#[derive(Debug, Clone)]
pub struct NoteStep {
    pub note: NoteRef,
    pub velocity: Option<f32>,
    pub plock: PLock,
}

#[derive(Debug, Clone)]
pub struct DrumStep {
    pub velocity: f32,       // default 0.8
    pub probability: f32,    // 0.0-1.0, default 1.0 (always play). Syntax: g?0.4
    pub roll: u8,            // retrigger count, default 1 (no roll). Syntax: x*3
    pub plock: PLock,
}

/// Track definition.
#[derive(Debug, Clone)]
pub struct TrackDef {
    pub name: String,
    pub play: String,                     // pattern name
    pub using_instrument: String,         // instrument name
    pub velocity: Option<f32>,
    pub level: Option<f32>,               // track output level 0.0-1.0
    pub pan: Option<f32>,                 // stereo pan -1.0 (L) to 1.0 (R), 0.0 = center
    pub gate: Option<f32>,                // gate length as fraction of step (0.0-1.0)
    pub routing: Vec<RoutingNode>,        // out > drive(0.2) > master
    pub delay_send: Option<f32>,          // global delay send amount 0.0-1.0
    pub reverb_send: Option<f32>,         // global reverb send amount 0.0-1.0
    pub sidechain: Option<f32>,           // per-track sidechain amount (overrides global)
}

/// A node in a routing chain: effect or bus target.
#[derive(Debug, Clone)]
pub struct RoutingNode {
    pub kind: String,         // "drive", "chorus", "master", bus name, etc
    pub params: Vec<Param>,
}

/// Bus FX chain definition.
#[derive(Debug, Clone)]
pub struct BusChainDef {
    pub bus_name: String,
    pub chain: Vec<ChainNode>,
}

#[derive(Debug, Clone)]
pub struct ChainNode {
    pub kind: String,
    pub params: Vec<Param>,
}

/// Master FX chain definition.
#[derive(Debug, Clone)]
pub struct MasterDef {
    pub chain: Vec<ChainNode>,
}

/// Module-based instrument definition (bass, fm, keys).
#[derive(Debug, Clone)]
pub struct ModuleDef {
    pub module_type: String,     // "bass", "fm", "keys"
    pub name: String,
    pub params: Vec<ModuleParam>,
    pub op_envelopes: Vec<OpEnvelopeDef>,
}

/// A named parameter in a module definition.
#[derive(Debug, Clone)]
pub struct ModuleParam {
    pub name: String,
    pub value: f32,
    /// Source line, for compile errors.
    pub line: usize,
}

/// Per-operator envelope for FM modules.
#[derive(Debug, Clone)]
pub struct OpEnvelopeDef {
    pub op_index: usize,
    pub a: f32,
    pub d: f32,
    pub s: f32,
    pub r: f32,
}

/// Automation definition within a scene.
#[derive(Debug, Clone)]
pub struct AutomationDef {
    pub target: String,           // "funk_bass.cutoff" or "reverb_mix"
    pub keyframes: Vec<f32>,      // 2 = linear, 3 = triangle
}

/// Scene definition (for arrangement).
#[derive(Debug, Clone)]
pub struct SceneDef {
    pub name: String,
    pub extends: Option<String>,
    pub tempo: Option<f32>,
    pub overrides: Vec<Override>,
    pub automations: Vec<AutomationDef>,
    pub tracks: Vec<TrackDef>,
}

/// Parameter override in a scene.
#[derive(Debug, Clone)]
pub struct Override {
    pub target: String,       // "bass.velocity"
    pub value: f32,
}

/// Per-lane groove definition: per-drum timing offsets and swing overrides.
#[derive(Debug, Clone)]
pub struct GrooveDef {
    pub name: String,
    pub lanes: Vec<GrooveLane>,
}

/// A single lane in a groove block.
#[derive(Debug, Clone)]
pub struct GrooveLane {
    pub drum_name: String,      // "hat", "snare", "kick", etc.
    pub swing: Option<f32>,     // per-lane swing override (0.5-0.75)
    pub nudge: Option<f32>,     // timing offset as fraction of step (-0.05 to 0.05)
}

/// Arrangement entry: scene name + repeat count.
#[derive(Debug, Clone)]
pub struct ArrangeEntry {
    pub scene_name: String,
    pub repeat: u32,
}
