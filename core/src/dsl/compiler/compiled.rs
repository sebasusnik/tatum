//! What the compiler hands the engine: songs, scenes, tracks, patterns and
//! steps, already resolved to indices, MIDI notes and node specs, so nothing
//! on the audio side looks a name up.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use crate::dsl::ast::Globals;
use crate::graph::GraphTemplate;
use crate::graph::node::ChainStep;

// ── Compiled output types ──

/// Per-step parameter lock data (compiled, Copy-able).
#[derive(Clone, Copy, Debug, Default)]
pub struct StepPLock {
    pub cutoff: Option<f32>,
    pub env_depth: Option<f32>,
    pub resonance: Option<f32>,
    pub gate: Option<f32>,
    /// The chance the step sounds; `None` is always. Drawn from the track's
    /// own random stream, so a song sounds the same every time it renders.
    pub probability: Option<f32>,
}

/// Maximum notes in a single chord step.
pub const MAX_CHORD_NOTES: usize = 8;
/// How many notes one step can be split into. Eight inside a sixteenth at
/// 120 BPM is 64 notes a second, well past anything playable.
pub const MAX_SUBDIV: usize = 8;

/// A note within a chord.
#[derive(Clone, Copy, Debug, Default)]
pub struct ChordNote {
    pub midi_note: u8,
    pub velocity: f32,
}

/// One note inside a subdivided step. Carries its own velocity and slide so
/// a fast run can shape itself, which is the whole point of writing it out
/// instead of handing a chord to the arpeggiator.
#[derive(Clone, Copy, Debug, Default)]
pub struct SubNote {
    pub midi_note: u8,
    pub velocity: f32,
    pub slide: bool,
}

/// A compiled step: either a note event, chord, or silence.
#[derive(Clone, Copy, Debug)]
pub enum CompiledStep {
    NoteOn {
        midi_note: u8,
        velocity: f32,
        plock: StepPLock,
        slide: bool,
    },
    Chord {
        notes: [ChordNote; MAX_CHORD_NOTES],
        count: u8,
        plock: StepPLock,
    },
    /// `<B4 C#5 D5>`: notes played in sequence inside one step, evenly spaced
    /// across whatever that step's real duration turns out to be, so swing and
    /// humanize carry through instead of being bypassed.
    Subdiv {
        notes: [SubNote; MAX_SUBDIV],
        count: u8,
        plock: StepPLock,
    },
    DrumHit {
        velocity: f32,
        probability: f32,
        roll: u8,
        plock: StepPLock,
    },
    /// `<x o - X>` on a drum lane: hits spread evenly across the step, each
    /// at its own velocity; 0 is a rest.
    DrumSub {
        hits: [f32; MAX_SUBDIV],
        count: u8,
        plock: StepPLock,
    },
    Rest,
    Tie,
}

impl CompiledStep {
    /// A step that starts something, rather than holding or resting.
    pub fn is_onset(&self) -> bool {
        !matches!(self, CompiledStep::Rest | CompiledStep::Tie)
    }
}

/// How a track chooses, loop by loop, which of its patterns plays: the
/// patterns of `play a, b` and every transformed version of them, compiled
/// ahead so the audio thread only ever reads steps.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayPlan {
    /// Pattern indices in mixed radix: `alternatives` first, then each
    /// condition's states, in order.
    pub variants: Vec<usize>,
    pub alternatives: usize,
    pub conditions: Vec<LoopCondition>,
    /// The `play` line as written, for the live screen.
    pub text: String,
}

/// What decides a loop's variant, per transform that is not always on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LoopCondition {
    /// On the last of every N loops.
    Every(u32),
    /// On a loop with this chance.
    Chance(f32),
    /// Loop k of N starts k/N of the way in: N states.
    Iter(u8),
}

impl LoopCondition {
    pub fn states(&self) -> usize {
        match self {
            LoopCondition::Every(_) | LoopCondition::Chance(_) => 2,
            LoopCondition::Iter(n) => *n as usize,
        }
    }
}

/// A compiled drum lane: MIDI note + step sequence for one drum.
#[derive(Clone, Debug)]
pub struct CompiledLane {
    pub midi_note: u8,
    pub steps: Vec<CompiledStep>,
    pub swing_override: Option<f32>, // per-lane swing from groove block
    pub nudge: f32,                  // per-lane timing offset from groove block
}

/// A compiled pattern: flat array of steps.
#[derive(Clone, Debug)]
pub struct CompiledPattern {
    pub name: String,
    pub steps: Vec<CompiledStep>,
    pub steps_per_row: usize,
    pub lanes: Vec<CompiledLane>, // empty = sequential, non-empty = parallel drum pattern
}

/// A compiled track.
#[derive(Clone, Debug)]
pub struct CompiledTrack {
    pub name: String,
    pub instrument_idx: usize,
    pub pattern_idx: usize,
    pub velocity: f32,
    pub level: f32, // output level 0.0-1.0 (default 0.8)
    pub pan: f32,   // stereo pan -1.0 (L) to 1.0 (R), 0.0 = center
    pub gate: f32,  // gate length as fraction of step (default 0.85)
    pub insert_fx: Vec<ChainStep>,
    /// `as <name>` per insert node, so `auto <track>.<name> wet` can find it.
    /// Parallel to `insert_fx`; `None` where a node was left unnamed.
    pub insert_fx_labels: Vec<Option<String>>,
    pub bus_send: Option<(usize, f32)>, // (bus_idx, amount)
    pub to_master: bool,
    pub delay_send: f32,        // global delay send amount 0.0-1.0
    pub reverb_send: f32,       // global reverb send amount 0.0-1.0
    pub sidechain: Option<f32>, // per-track sidechain override (None = use global)
    /// Track whose level ducks this one. `None` = whatever the song's global
    /// source is, which is the kick unless `sidechain ... from=` says otherwise.
    pub sidechain_source: Option<String>,
    pub arp: Option<ArpConfig>, // arpeggiator driven by the pattern's held notes
    /// Index into `CompiledSong::plays` when the `play` line alternates or
    /// transforms on some loops and not others. `None`: `pattern_idx` loops.
    pub play: Option<usize>,
    /// The `play` line as written, for the live screen.
    pub play_text: String,
}

/// Compiled arpeggiator settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArpConfig {
    /// Normalized pattern for `ArpProcessor::set_pattern`: 0 = up, 0.5 = down, 1 = updown.
    pub pattern: f32,
    /// Multiplier on the song's 16th-note clock (rate 16 = 1.0, rate 8 = 0.5, rate 32 = 2.0).
    pub rate_mult: f32,
    /// Gate fraction of each arp step.
    pub gate: f32,
    /// Octave range 1..=4.
    pub octaves: u8,
}

/// A compiled bus.
#[derive(Clone, Debug)]
pub struct CompiledBus {
    pub name: String,
    pub fx_chain: Vec<ChainStep>,
}

/// A compiled scene snapshot.
#[derive(Clone, Debug)]
pub struct CompiledScene {
    pub name: String,
    pub tempo: Option<f32>,
    pub tracks: Vec<CompiledTrack>,
    pub reverb_mix: Option<f32>,
    pub delay_mix: Option<f32>,
    /// `reverb_freeze = 1` holds the global reverb tail for the scene.
    pub reverb_freeze: Option<bool>,
    pub automations: Vec<CompiledAutomation>,
}

/// Compiled automation lane.
#[derive(Clone, Debug)]
pub struct CompiledAutomation {
    pub target: String,
    pub keyframes: Vec<f32>,
    /// Bars a top-level lane runs over before it holds; `None` in a scene,
    /// whose lanes span the scene.
    pub over: Option<u32>,
}

/// Master FX chain.
#[derive(Clone, Debug)]
pub struct CompiledMaster {
    pub fx_chain: Vec<ChainStep>,
    /// `as <name>` per node, for `auto master <name> <param>`.
    pub labels: Vec<Option<String>>,
}

/// A compiled instrument — either a graph template or a module preset.
#[derive(Clone)]
pub enum CompiledInstrumentKind {
    /// Boxed: the template is ~2 KB and every other variant is under 50, so
    /// inline it made a Vec of mostly-module instruments carry the graph's
    /// footprint each. Built on the control thread, so the allocation never
    /// touches the audio path.
    Graph(Box<GraphTemplate>),
    Bass(ModulePreset),
    Fm(FmPreset),
    Keys(ModulePreset),
    Beats(ModulePreset),
}

impl CompiledInstrumentKind {
    /// Get the inner graph template, if this is a Graph variant.
    pub fn as_graph(&self) -> Option<&GraphTemplate> {
        match self {
            Self::Graph(t) => Some(t),
            _ => None,
        }
    }
}

/// Generic module preset: named params with 0-1 normalized values.
#[derive(Clone, Debug, Default)]
pub struct ModulePreset {
    pub params: Vec<(String, f32)>,
}

/// FM module preset: params + per-operator envelopes.
#[derive(Clone, Debug, Default)]
pub struct FmPreset {
    pub params: Vec<(String, f32)>,
    pub op_envelopes: Vec<(usize, (f32, f32, f32, f32))>, // (op_idx, (a, d, s, r))
}

/// Full compiled song.
/// Compiled per-lane groove: maps MIDI note to timing adjustments.
#[derive(Clone, Debug)]
pub struct CompiledGrooveLane {
    pub midi_note: u8,
    pub swing_override: Option<f32>,
    pub nudge: f32,
}

#[derive(Clone, Debug, Default)]
pub struct CompiledGroove {
    pub name: String,
    pub lanes: Vec<CompiledGrooveLane>,
}

#[derive(Clone)]
pub struct CompiledSong {
    pub globals: Globals,
    pub instruments: Vec<CompiledInstrumentKind>,
    pub instrument_names: Vec<String>,
    pub patterns: Vec<CompiledPattern>,
    pub tracks: Vec<CompiledTrack>,
    pub buses: Vec<CompiledBus>,
    pub master: CompiledMaster,
    pub scenes: Vec<CompiledScene>,
    /// Every track's and scene track's `PlayPlan`, by `CompiledTrack::play`.
    pub plays: Vec<PlayPlan>,
    pub arrangement: Vec<(usize, u32)>, // (scene_idx, repeat_count)
    pub grooves: Vec<CompiledGroove>,
    /// Insert chains on the global send returns: `reverb_return { in > ... > out }`.
    pub reverb_return: Vec<ChainStep>,
    pub delay_return: Vec<ChainStep>,
    /// Top-level `auto ... over N` lanes of a song without scenes.
    pub automations: Vec<CompiledAutomation>,
    /// The slowest tempo the song can be played at: its own, its scenes', and
    /// the bottom of any knob on `tempo` (60 BPM for a knob with no range).
    /// Delay lines are sized for an echo at this tempo.
    pub slowest_tempo: f32,
}
