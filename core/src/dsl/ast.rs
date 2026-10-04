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
    /// `auto <target> a > b over 8` outside any scene: lanes of a song with
    /// no scenes, each over its own number of bars from the first bar this
    /// text plays. A song with scenes keeps its lanes in them.
    pub automations: Vec<AutomationDef>,
    /// The keyboard as a performer splits it, the scenes that change what
    /// it plays, and the computer keys that call them. Read by a live
    /// session, like `midi`.
    pub perform: PerformSetup,
}

/// `zone`, `lock` and `key` lines of the `midi` blocks, the `perform`
/// blocks and the `keyboard` block, as the text has them after every
/// redefinition.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PerformSetup {
    pub zones: Vec<ZoneDef>,
    /// `lock snap`: how the bass and lead zones keep to the scale.
    pub lock: Option<crate::perform::scale::Lock>,
    pub scenes: Vec<PerformDef>,
    pub keyboard: Vec<KeyBinding>,
    /// `bend lead 2st` in a `midi` block: what the pitch strip does to a
    /// zone when no scene says otherwise.
    pub bends: Vec<BendDef>,
    /// `page bass { cc 74 > bass cutoff }`: the knobs' voices, in order. The
    /// first is the one a session starts on; a page's line for a controller
    /// stands in for the `midi` block's while the page is chosen.
    pub pages: Vec<PageDef>,
    /// `takeover pickup`: a knob whose position does not match what it now
    /// moves, after a page change, waits until it passes the value before
    /// it moves anything. `None` or `jump`: it moves at once.
    pub pickup: Option<bool>,
    /// `quantize bar` in the `keyboard` block: where a scene called from the
    /// computer comes in, in bars. `None`: the next bar.
    pub scene_bars: Option<u32>,
}

/// `zone bass 48..59 > bass roll kick=drums`.
#[derive(Debug, Clone, PartialEq)]
pub struct ZoneDef {
    pub kind: ZoneKind,
    /// Lowest and highest MIDI note, both in the zone.
    pub low: u8,
    pub high: u8,
    /// What the zone plays until a scene says otherwise. A trigger zone
    /// plays nothing itself: its keys are `key` lines.
    pub play: Option<ZonePlay>,
    pub line: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneKind {
    /// Each key does something: `key 36 > hold riser`.
    Triggers,
    /// One note at a time, rolled or not.
    Bass,
    /// The rest: a lead, or sounds.
    Lead,
}

impl ZoneKind {
    pub fn from_word(w: &str) -> Option<Self> {
        match w {
            "triggers" => Some(Self::Triggers),
            "bass" => Some(Self::Bass),
            "lead" => Some(Self::Lead),
            _ => None,
        }
    }
    pub fn word(self) -> &'static str {
        match self {
            Self::Triggers => "triggers",
            Self::Bass => "bass",
            Self::Lead => "lead",
        }
    }
}

/// The track a zone plays, and how.
#[derive(Debug, Clone, PartialEq)]
pub struct ZonePlay {
    pub track: String,
    /// `roll`: a held key rolls the note on the three sixteenths after each
    /// beat, in place of the track's own pattern.
    pub roll: bool,
    /// `kick=drums`: a roll also strikes this track's kick on each beat.
    pub kick: Option<String>,
    /// `octave 1`: the zone's first key on the scale's root plays the root
    /// in this octave. `None`: the bass zone plays where the track's own
    /// line sits, the lead zone as written.
    pub octave: Option<i8>,
}

/// `perform drop { ... }`: one scene of a performance.
#[derive(Debug, Clone, PartialEq)]
pub struct PerformDef {
    pub name: String,
    /// The scale the zones keep to; `None`, the song's.
    pub scale: Option<ScaleDef>,
    pub lock: Option<crate::perform::scale::Lock>,
    pub bass: Option<ZonePlay>,
    pub lead: Option<ZonePlay>,
    /// `set reverb_mix 20%`: values put in place when the scene comes in.
    pub sets: Vec<PerformSet>,
    /// `bend lead 24st`: the pitch strip, zone by zone, in this scene.
    pub bends: Vec<BendDef>,
    /// `wheel lead > laser talk wet` and `cc 74 idle > ...`: what a
    /// controller moves while this scene plays, in place of the `midi`
    /// block's line for the same controller, and when. The wheel is
    /// controller 1.
    pub knobs: Vec<SceneKnob>,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PerformSet {
    /// Dotted, as a knob's: `master.dj.cutoff`.
    pub target: String,
    pub value: RangeEnd,
    pub line: usize,
}

/// `bend lead 2deg`: how far the pitch strip takes a zone's notes.
#[derive(Debug, Clone, PartialEq)]
pub struct BendDef {
    pub zone: ZoneKind,
    pub range: BendRange,
    /// `bend bass 12st idle`: when it bends. Without a word, while a key of
    /// its own zone is the newest held.
    pub hands: Hands,
    pub line: usize,
}

/// When a scene's bend or wheel line answers the controller, by where the
/// hands are on the keyboard: the zone of the newest key held, or none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hands {
    /// While a key of this zone is the newest held: the sound you play.
    Playing(ZoneKind),
    /// Whatever the hands do: the song.
    Always,
    /// Only with no key held in the bass or lead zone: the song, while the
    /// hands are off the keys.
    Idle,
}

impl Hands {
    pub fn word(self) -> &'static str {
        match self {
            Hands::Playing(z) => z.word(),
            Hands::Always => "always",
            Hands::Idle => "idle",
        }
    }

    /// Whether it answers with the hands at `zone` (`None`: no key held).
    pub fn answers(self, zone: Option<ZoneKind>) -> bool {
        match self {
            Hands::Playing(z) => zone == Some(z),
            Hands::Always => true,
            Hands::Idle => zone.is_none(),
        }
    }
}

/// A knob line in a scene, with when it answers.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneKnob {
    pub hands: Hands,
    pub map: MidiMapDef,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BendRange {
    /// Up to this many semitones each way: `24st` for a dive.
    Semitones(f32),
    /// To the note this many degrees of the scale away, up or down, from
    /// the note held: the bend lands in the scale.
    Degrees(u8),
    /// The strip leaves the zone alone.
    Off,
}

/// `page bass { ... }` in a `midi` block: what the knobs move while the
/// bass is the voice chosen.
#[derive(Debug, Clone, PartialEq)]
pub struct PageDef {
    pub name: String,
    pub knobs: Vec<MidiMapDef>,
    pub line: usize,
}

/// Choosing the knobs' voice.
#[derive(Debug, Clone, PartialEq)]
pub enum VoiceMove {
    Next,
    Prev,
    To(String),
}

/// `f1 > perform intro` in the `keyboard` block.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyBinding {
    /// As written, lowercase: `f1`, `a`, `space`, `left`.
    pub key: String,
    pub action: KeyAction,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum KeyAction {
    Perform(String),
    /// `tab > voice next`: the knobs' page.
    Voice(VoiceMove),
    Next,
    Prev,
    Step(usize),
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
    /// `pad 44 > mute drums q=bar`: the pad's press and release wait for the
    /// next line of this grid. `None` acts at once.
    pub quantize: Option<Quantize>,
    /// Source line, for compile errors.
    pub line: usize,
}

/// The grid a quantized pad lands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quantize {
    Bar,
    Beat,
    Eighth,
    Sixteenth,
}

impl Quantize {
    pub fn from_word(w: &str) -> Option<Option<Self>> {
        match w {
            "bar" => Some(Some(Self::Bar)),
            "beat" => Some(Some(Self::Beat)),
            "1/8" => Some(Some(Self::Eighth)),
            "1/16" => Some(Some(Self::Sixteenth)),
            "off" => Some(None),
            _ => None,
        }
    }

    /// The division in sixteenths, given how many a bar has.
    pub fn sixteenths(self, steps_per_bar: usize) -> f32 {
        match self {
            Self::Bar => steps_per_bar.max(1) as f32,
            Self::Beat => 4.0,
            Self::Eighth => 2.0,
            Self::Sixteenth => 1.0,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Self::Bar => "bar",
            Self::Beat => "beat",
            Self::Eighth => "1/8",
            Self::Sixteenth => "1/16",
        }
    }
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
    /// `key 36 > hold riser`: one key of the keyboard's trigger zone, which
    /// does what a pad does instead of playing a note.
    Key(u8),
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
    /// `A2?0.5`: the chance the step sounds. `None` is always.
    pub probability: Option<f32>,
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
    /// `<x o - X>` on a drum lane: hits spread evenly inside one step, each
    /// with its own velocity; 0 is a rest.
    DrumSub(Vec<f32>),
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
    pub play: String, // pattern name
    /// `play a, b, c`: the patterns after the first, one loop each in turn.
    pub play_also: Vec<String>,
    /// `play a every 4 rev shift 2`: what is done to the pattern, in order.
    pub transforms: Vec<Transform>,
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

/// A change made to a pattern on the `play` line. Each works on the
/// pattern's events -- a note and its ties, a chord, a hit -- so a tied note
/// is never cut in two. See `docs/DSL.md`, "Transforming a pattern".
#[derive(Debug, Clone, PartialEq)]
pub enum Transform {
    /// The events in reverse order.
    Rev,
    /// The pattern N times in its own length (2..4).
    Fast(u8),
    /// Every step N steps long; the pattern N times longer.
    Slow(u8),
    /// Rotated N steps later; what falls off the end comes in at the start.
    Shift(i32),
    /// Up N degrees of the song's scale, or N semitones with `st`.
    Up { amount: i32, semitones: bool },
    /// Up N octaves.
    Octave(i32),
    /// Each event has this chance of not sounding.
    Degrade(f32),
    /// Each event played N times inside its step.
    Ply(u8),
    /// Loop k of N starts k/N of the way in.
    Iter(u8),
    /// The transform on the last of every N loops.
    Every(u32, alloc::boxed::Box<Transform>),
    /// The transform on a loop with this chance, decided loop by loop.
    Sometimes(f32, alloc::boxed::Box<Transform>),
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

/// An `auto` lane: in a scene, or at the top level of a song without scenes.
#[derive(Debug, Clone, PartialEq)]
pub struct AutomationDef {
    pub target: String,      // "funk_bass.cutoff" or "reverb_mix"
    pub keyframes: Vec<f32>, // evenly spaced: 2 = linear, 3 = triangle, more = a curve
    /// `over 8`: the lane's length in bars. Only a top-level lane has one,
    /// and it must; a scene lane spans its scene.
    pub over: Option<u32>,
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
