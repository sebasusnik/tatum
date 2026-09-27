extern crate alloc;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

/// Top-level song structure.
#[derive(Debug, Clone, PartialEq)]
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
    /// `midi { cc 74 > acid cutoff }`: which controller moves what. Read by a
    /// live session and ignored by a render.
    pub midi: Vec<MidiMapDef>,
}

/// One line of a `midi { }` block. The target is stored joined with dots, the
/// way an `auto` target is: `acid.cutoff`, `pad.level`, `warm.ph.wet`,
/// `reverb_mix` for a knob, a track name for the keys, `kick.kick` for a pad.
#[derive(Debug, Clone, PartialEq)]
pub struct MidiMapDef {
    pub source: MidiSource,
    pub target: String,
    /// `cc 74 > acid cutoff 200hz..4khz`: where the bottom and the top of the
    /// knob's travel land, in the target's units, with an optional third
    /// point in between for half travel (`20hz..20hz..2khz`). Written high to
    /// low, the knob works backwards. `None`: the target's whole range.
    pub range: Option<Vec<RangeEnd>>,
    /// Source line, for compile errors.
    pub line: usize,
}

/// One end of a knob's range as written: `200hz`, `60%`, `-12db`, `0.8`.
#[derive(Debug, Clone, PartialEq)]
pub struct RangeEnd {
    pub value: f32,
    /// The unit written against the number, lowercase; `None` for a plain
    /// number, which means what the same number means in the text.
    pub unit: Option<String>,
}

/// What on the controller a `midi` line listens to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MidiSource {
    /// `cc 74 > acid cutoff`: a knob or a fader.
    Cc(u8),
    /// `keys > solo`: every note that is not on the drum channel, and the
    /// pitch bend with them.
    Keys,
    /// `pad 36 > kick kick`: one note on the drum channel, channel 10, which
    /// is where General MIDI puts drums and where pads send.
    Pad(u8),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Globals {
    pub tempo: f32,
    pub meter: (u8, u8),
    pub scale: Option<ScaleDef>,
    pub sidechain: f32,
    /// Track or module whose level drives the ducking. `None` = the kick.
    pub sidechain_source: Option<String>,
    /// Shape of the ducking envelope, in milliseconds.
    pub sidechain_attack_ms: Option<f32>,
    pub sidechain_release_ms: Option<f32>,
    pub swing: Option<f32>,           // 0.5 = straight, 0.67 = triplet feel
    pub humanize: Option<f32>,        // velocity humanization 0.0-1.0
    pub humanize_timing: Option<f32>, // timing humanization 0.0-1.0
    /// `gain_comp`, which no longer does anything: kept so a file that sets it
    /// still compiles, and so the lint can say so.
    pub gain_comp: Option<f32>,
    pub send_delay: SendDelayDef,   // `delay sync=dotted_eighth feedback=0.45 filter=0.5`
    pub send_reverb: SendReverbDef, // `reverb size=0.7 damp=0.4 predelay=20`
}

/// Global send delay settings (top-level `delay ...` line).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SendDelayDef {
    pub sync: Option<String>, // free | quarter | dotted_eighth | eighth | sixteenth | triplet_eighth
    pub time: Option<f32>,    // seconds, used when sync is free
    pub feedback: Option<f32>,
    pub filter: Option<f32>,
    pub sidechain: Option<f32>, // duck the delay return against the kick
}

/// Global send reverb settings (top-level `reverb ...` line).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SendReverbDef {
    pub size: Option<f32>,
    pub damp: Option<f32>,
    pub predelay: Option<f32>,  // milliseconds
    pub sidechain: Option<f32>, // duck the reverb return against the kick
}

impl Default for Globals {
    fn default() -> Self {
        Self {
            tempo: 120.0,
            meter: (4, 4),
            scale: None,
            sidechain: 0.0,
            sidechain_source: None,
            sidechain_attack_ms: None,
            sidechain_release_ms: None,
            swing: None,
            humanize: None,
            humanize_timing: None,
            gain_comp: None,
            send_delay: SendDelayDef::default(),
            send_reverb: SendReverbDef::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScaleDef {
    pub root: String, // "A", "C#", "Bb", etc
    pub kind: String, // "minor", "major", "dorian", etc
}

#[derive(Debug, Clone, PartialEq)]
pub struct BusDef {
    pub name: String,
}

/// Instrument definition: a modular signal graph.
#[derive(Debug, Clone, PartialEq)]
pub struct InstrumentDef {
    pub name: String,
    /// Where `instrument <name> {` is.
    pub line: Line,
    pub gain: Option<f32>,
    pub nodes: Vec<NodeDef>,
    pub connections: Vec<ConnectionDef>,
}

/// Where something was written, for error messages. Two definitions that
/// differ only in where they sit are the same definition, so equality ignores
/// it: the live diff must not take an instrument that moved down the file for
/// one that changed.
#[derive(Debug, Clone, Copy, Default)]
pub struct Line(pub usize);

impl PartialEq for Line {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

/// A node in an instrument graph.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeDef {
    pub kind: String,          // "osc", "lowpass", "adsr", "noise", "mix", etc
    pub alias: Option<String>, // "as osc1"
    pub params: Vec<Param>,    // positional + named params
    pub line: Line,
}

/// A connection between nodes: a > b > c
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectionDef {
    pub from: String, // node alias or "in"
    pub to: String,   // node alias or "out" / "mix" / "master"
    pub line: Line,
}

/// Parameter value in a node definition.
#[derive(Debug, Clone, PartialEq)]
pub enum Param {
    Float(f32),
    Named(String, f32),
    Waveform(String),  // "saw", "sine", "square", "triangle"
    RhythmDiv(u8, u8), // 1/4, 1/8, etc
    Expr(Expr),        // 55 * 2 — simple arithmetic
}

/// Simple expression for frequency math like `55 * 2`.
#[derive(Debug, Clone, PartialEq)]
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
#[derive(Debug, Clone, PartialEq)]
pub struct PatternDef {
    pub name: String,
    pub rows: Vec<Vec<Step>>,     // rows of steps (each row = one bar line)
    pub lane_labels: Vec<String>, // empty = sequential, non-empty = parallel drum lanes (lane_labels[i] names rows[i])
}

/// Per-step parameter lock: overrides instrument params for a single step.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PLock {
    pub cutoff: Option<f32>,
    pub env_depth: Option<f32>,
    pub resonance: Option<f32>,
    pub gate: Option<f32>,
}

/// A single step in a pattern.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Note(NoteStep),
    Chord(ChordStep),
    DrumHit(DrumStep),
    /// Several notes inside ONE step, evenly spaced: `<B4 C#5 D5>`. This is
    /// how a pattern gets finer than a sixteenth without changing the clock.
    /// `A4*3` parses to the same thing with the note repeated.
    Subdiv(Vec<NoteStep>),
    Rest,
    Tie,
}

/// A chord: multiple notes triggered simultaneously on one step.
#[derive(Debug, Clone, PartialEq)]
pub struct ChordStep {
    pub notes: Vec<NoteStep>,
    pub velocity: Option<f32>, // shared velocity (overrides individual if set)
    pub plock: PLock,
}

#[derive(Debug, Clone, PartialEq)]
pub enum NoteRef {
    Absolute(String), // "A1", "C#4", "G0"
    Degree(u8, u8),   // (degree 1-7, octave 0-9)
    Midi(u8),         // resolved, e.g. from a chord symbol
}

#[derive(Debug, Clone, PartialEq)]
pub struct NoteStep {
    pub note: NoteRef,
    pub velocity: Option<f32>,
    pub plock: PLock,
    /// `~note`: glide into this note from the previous one without retriggering.
    pub slide: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DrumStep {
    pub velocity: f32,    // default 0.8
    pub probability: f32, // 0.0-1.0, default 1.0 (always play). Syntax: g?0.4
    pub roll: u8,         // retrigger count, default 1 (no roll). Syntax: x*3
    pub plock: PLock,
}

/// Track definition.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackDef {
    pub name: String,
    pub play: String,             // pattern name
    pub using_instrument: String, // instrument name
    pub velocity: Option<f32>,
    pub level: Option<f32>,               // track output level 0.0-1.0
    pub pan: Option<f32>,                 // stereo pan -1.0 (L) to 1.0 (R), 0.0 = center
    pub gate: Option<f32>,                // gate length as fraction of step (0.0-1.0)
    pub routing: Vec<RoutingNode>,        // out > drive(0.2) > master
    pub delay_send: Option<f32>,          // global delay send amount 0.0-1.0
    pub reverb_send: Option<f32>,         // global reverb send amount 0.0-1.0
    pub sidechain: Option<f32>,           // per-track sidechain amount (overrides global)
    pub sidechain_source: Option<String>, // what ducks it; None = the global source
    pub arp: Option<ArpDef>,              // `arp up rate=16 gate=0.6 octaves=2`, `arp off`
}

/// Arpeggiator settings on a track. The pattern supplies the held notes
/// (single notes or chords, extended with ties); the arp plays through them.
#[derive(Debug, Clone, PartialEq)]
pub struct ArpDef {
    pub mode: String,         // up | down | updown | off
    pub rate: Option<f32>,    // steps per bar subdivision: 4, 8, 16 (default), 32
    pub gate: Option<f32>,    // 0.1..1.0 fraction of each arp step
    pub octaves: Option<f32>, // 1..4
}

/// A node in a routing chain: effect or bus target.
#[derive(Debug, Clone, PartialEq)]
pub struct RoutingNode {
    pub kind: String, // "drive", "chorus", "master", bus name, etc
    pub params: Vec<Param>,
    /// `> autowah(...) as wah`. Naming a node is what lets anything else refer
    /// to it -- an `auto` sweep, or a person reading the chain. Counting
    /// positions breaks the moment someone inserts a node earlier in the line.
    pub label: Option<String>,
}

/// Bus FX chain definition.
#[derive(Debug, Clone, PartialEq)]
pub struct BusChainDef {
    pub bus_name: String,
    pub chain: Vec<ChainNode>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChainNode {
    pub kind: String,
    pub params: Vec<Param>,
    pub label: Option<String>,
}

/// Master FX chain definition.
#[derive(Debug, Clone, PartialEq)]
pub struct MasterDef {
    pub chain: Vec<ChainNode>,
}

/// Module-based instrument definition (bass, fm, keys).
#[derive(Debug, Clone, PartialEq)]
pub struct ModuleDef {
    pub module_type: String, // "bass", "fm", "keys"
    pub name: String,
    pub params: Vec<ModuleParam>,
    pub op_envelopes: Vec<OpEnvelopeDef>,
}

/// A named parameter in a module definition.
#[derive(Debug, Clone, PartialEq)]
pub struct ModuleParam {
    pub name: String,
    pub value: f32,
    /// Source line, for compile errors.
    pub line: usize,
    /// Written as a plain number rather than in a unit or by name. On a
    /// parameter with a real unit that number is a knob position, which the
    /// lint asks to see written as what it means.
    pub bare: bool,
}

/// Per-operator envelope for FM modules.
#[derive(Debug, Clone, PartialEq)]
pub struct OpEnvelopeDef {
    pub op_index: usize,
    pub a: f32,
    pub d: f32,
    pub s: f32,
    pub r: f32,
}

/// Automation definition within a scene.
#[derive(Debug, Clone, PartialEq)]
pub struct AutomationDef {
    pub target: String,      // "funk_bass.cutoff" or "reverb_mix"
    pub keyframes: Vec<f32>, // evenly spaced: 2 = linear, 3 = triangle, more = a curve
}

/// Scene definition (for arrangement).
#[derive(Debug, Clone, PartialEq)]
pub struct SceneDef {
    pub name: String,
    pub extends: Option<String>,
    pub tempo: Option<f32>,
    pub overrides: Vec<Override>,
    pub automations: Vec<AutomationDef>,
    pub tracks: Vec<TrackDef>,
}

/// Parameter override in a scene.
#[derive(Debug, Clone, PartialEq)]
pub struct Override {
    pub target: String, // "bass.velocity"
    pub value: f32,
}

/// Per-lane groove definition: per-drum timing offsets and swing overrides.
#[derive(Debug, Clone, PartialEq)]
pub struct GrooveDef {
    pub name: String,
    pub lanes: Vec<GrooveLane>,
}

/// A single lane in a groove block.
#[derive(Debug, Clone, PartialEq)]
pub struct GrooveLane {
    pub drum_name: String,  // "hat", "snare", "kick", etc.
    pub swing: Option<f32>, // per-lane swing override (0.5-0.75)
    pub nudge: Option<f32>, // timing offset as fraction of step (-0.05 to 0.05)
}

/// Arrangement entry: scene name + repeat count.
#[derive(Debug, Clone, PartialEq)]
pub struct ArrangeEntry {
    pub scene_name: String,
    pub repeat: u32,
}
