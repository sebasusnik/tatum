//! Live session: editing the source while it plays.
//!
//! Two halves, because natively they live on different threads and in WASM on
//! the same one:
//!
//! - [`LivePlanner`] (control thread) takes the new source, always parses and
//!   compiles it (so the live path validates exactly what `tatum check`
//!   validates), diffs it against what is playing, and returns a [`Plan`].
//!   Anything the fast path cannot resolve becomes a swap; nothing is dropped.
//! - [`LivePlayer`] (audio thread) applies plans and renders. A swap happens
//!   at the start of the block in which the next bar line falls. The new
//!   engine takes that whole block, positioned just before the line with its
//!   step clock phase-aligned to the old one, so it crosses the line itself on
//!   the exact sample, and the downbeat fires once. (The engine triggers steps
//!   at the start of the block they fall in, so handing over a whole block is
//!   what keeps a swapped render identical to a straight one.) The new engine
//!   inherits, by name, every piece of state whose definition did not change
//!   in the text (instruments keep their voices, sends and buses keep their
//!   tails, unchanged tracks keep their place and their held notes), all moved
//!   with `mem::swap`. What the old engine still owns afterwards, which is
//!   exactly what disappeared from the text, coasts under a short fade-out.
//!   Retired engines are handed back so their memory is freed off the audio
//!   thread.
//!
//! The rule the whole thing serves: what did not change in the text keeps its
//! state in the engine. A swap to an identical song is bit-identical to not
//! swapping.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use crate::dsl::ast::{ChainNode, Quantize, Song, ZoneKind};
use crate::dsl::compiler::CompiledSong;
use crate::dsl::diff::{self, DslChange};
use crate::midi;
use crate::params::{self, ModuleKind, ParamId};
use crate::song_engine::{DslError, SongEngine};
use crate::{BLOCK_SIZE, SAMPLE_RATE};

/// How far the pitch strip bends the keys, each way.
pub const BEND_SEMITONES: f32 = 2.0;

/// Within this of the middle the pitch strip is at rest: a gesture ends,
/// and the next one picks its lines by where the hands are then.
const BEND_REST: f32 = 0.02;

/// A controller at or below this is at rest, for the same purpose.
const KNOB_REST: u8 = 2;

/// Length of the fade-out applied to what the old engine still owns after a
/// swap. About 12 ms: long enough to hide a cut voice, short enough not to
/// read as a transition.
pub const CROSSFADE_SAMPLES: usize = 512;

/// Identifies an engine instance across planner and player. A fast edit keeps
/// the generation (same engine, new values); a swap starts a new one.
pub type Generation = u64;

/// A change resolved to engine indices: nothing on the audio thread looks a
/// name up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FastOp {
    Tempo(f32),
    Swing(f32),
    Humanize {
        velocity: f32,
        timing: f32,
    },
    TrackLevel {
        track: usize,
        level: f32,
    },
    TrackPan {
        track: usize,
        pan: f32,
    },
    TrackVelocity {
        track: usize,
        velocity: f32,
    },
    TrackGate {
        track: usize,
        gate: f32,
    },
    ModuleParam {
        instrument: usize,
        id: ParamId,
        value: f32,
    },
    /// Switch one node of a track's insert chain in or out, or anywhere
    /// between. The node is already built; only how much of it is heard moves.
    NodeWet {
        track: usize,
        node: usize,
        wet: f32,
    },
    /// Wet level of the global returns, 0..1. Only a knob sends these: in the
    /// text they are per-scene values, which a swap carries.
    ReverbMix(f32),
    DelayMix(f32),
    ReverbFreeze(bool),
    /// From a pad: silence a track while held (or until tapped again), send
    /// it whole into the delay while held, freeze the reverb while held.
    TrackMute {
        track: usize,
        muted: bool,
    },
    TrackThrow {
        track: usize,
        on: bool,
    },
    FreezeHold(bool),
    /// A track's send into the global reverb or delay, 0..1.
    TrackSend {
        track: usize,
        reverb: bool,
        amount: f32,
    },
    /// A named parameter of one node of an effect chain: a track's inserts,
    /// or the master's when `track` is `None`. `param` is one of the names
    /// `auto master` takes (`cutoff`, `tilt`, `drive`...).
    NodeParam {
        track: Option<usize>,
        node: usize,
        param: &'static str,
        value: f32,
    },
    /// A note from the keyboard or a pad, on the instrument a track plays.
    /// Only ever sent as [`Plan::Play`].
    NoteOn {
        track: usize,
        note: u8,
        velocity: f32,
    },
    NoteOff {
        track: usize,
        note: u8,
    },
    /// The pitch strip: every note on the instrument, by `ratio` of its
    /// frequency.
    PitchBend {
        instrument: usize,
        ratio: f32,
    },
    /// A key held in a roll zone: `note` rolled on `track` from the next
    /// sixteenth, in place of its pattern; velocity 0 stops it. `kick` is
    /// the drum track and note struck on each beat.
    Roll {
        track: usize,
        note: u8,
        velocity: f32,
        kick: Option<(usize, u8)>,
    },
}

/// What a new engine takes over from the one it replaces, by index in the new
/// engine: `instruments[i] == Some(j)` means new instrument `i` continues old
/// instrument `j`. Built from the two ASTs by name and definition equality.
#[derive(Debug, Clone, PartialEq)]
pub struct Inherit {
    /// Global delay and reverb, with their return chains.
    pub sends: bool,
    pub master: bool,
    pub buses: Vec<Option<usize>>,
    pub instruments: Vec<Option<usize>>,
    pub tracks: Vec<Option<usize>>,
    /// The top-level `auto` lanes are the same and keep their progress
    /// rather than starting again at the swap. Never for a set's steps.
    pub lanes: bool,
}

impl Inherit {
    /// Nothing carries over: the state of the swap before this module existed.
    pub fn none(compiled: &CompiledSong) -> Self {
        Self {
            sends: false,
            master: false,
            buses: vec![None; compiled.buses.len()],
            instruments: vec![None; compiled.instruments.len()],
            tracks: vec![None; compiled.tracks.len()],
            lanes: false,
        }
    }

    pub fn is_complete(&self) -> bool {
        self.sends
            && self.lanes
            && self.master
            && self.buses.iter().all(|m| m.is_some())
            && self.instruments.iter().all(|m| m.is_some())
            && self.tracks.iter().all(|m| m.is_some())
    }
}

pub enum Plan {
    /// The text means the same song as before.
    Unchanged,
    /// Values on the engine of generation `base`, applied immediately.
    Fast { base: Generation, ops: Vec<FastOp> },
    /// A new engine, ready built, to take over on the next bar line. `inherit`
    /// holds one map per engine the player may be running when the swap lands
    /// (the current one, and a pending one it may have swapped to meanwhile).
    Swap {
        engine: Box<SongEngine>,
        generation: Generation,
        inherit: Vec<(Generation, Inherit)>,
        /// Bars the outgoing engine keeps playing under the new one, DJ
        /// style, instead of handing over on the line. 0 is the plain swap.
        blend_bars: f32,
    },
    /// One value from a knob, for the engine of generation `base`: the one
    /// playing or the one queued behind it. A single op and no `Vec`, because
    /// a knob sends these many times a second and whatever the plan owns is
    /// freed on the audio thread.
    Control { base: Generation, op: FastOp },
    /// A note, for the engine of generation `base` if and only if it is the
    /// one playing when the plan lands. The planner sends one per engine it
    /// knows of, resolved against each, so whichever is playing takes it --
    /// even when a swap lands between the key going down and the plan
    /// arriving -- and a queued engine never holds a note that would start
    /// sounding at the bar line.
    Play { base: Generation, op: FastOp },
    /// A beat repeat on the player's output: `Some(sixteenths)` arms one,
    /// to start on the next division of that length; `None` lets go. It
    /// belongs to the player, not to an engine, so it rolls on across a swap.
    Repeat(Option<f32>),
    /// A pad written with `q=`: `inner` waits for the next line of `grid`,
    /// on the audio thread, which is the only place that knows where the
    /// line is. `pad` and `down` pair a release with its press; a trigger
    /// key is its note with the top bit set, so it never pairs with a pad.
    Quantized { grid: Quantize, pad: u8, down: bool, inner: Deferred },
    /// `inner` on the line where bar `bar` starts, counted as the engine
    /// counts them; a bar already started acts at once. What a scene called
    /// from the computer puts in place, on its bar.
    AtBar { bar: usize, inner: Deferred },
}

/// What a quantized pad does when its line comes: a [`Plan::Control`] or a
/// [`Plan::Play`], held without anything to free.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Deferred {
    Control {
        base: Generation,
        op: FastOp,
    },
    Play {
        base: Generation,
        op: FastOp,
    },
    /// A value as the text would write it: it lets go of a knob holding
    /// the same target, as a `Plan::Fast` does.
    Set {
        base: Generation,
        op: FastOp,
    },
}

impl Plan {
    pub fn describe(&self) -> &'static str {
        match self {
            Plan::Unchanged => "unchanged",
            Plan::Fast { .. } => "fast",
            Plan::Swap { .. } => "swap",
            Plan::Control { .. } => "control",
            Plan::Play { .. } => "play",
            Plan::Repeat(_) => "repeat",
            Plan::Quantized { .. } => "quantized",
            Plan::AtBar { .. } => "at bar",
        }
    }
}

/// What turning one knob asks of the player, and what to show for it.
pub struct KnobTurn {
    /// One per operation per engine: the one playing, and the one queued to
    /// take over at the next bar if there is one, so the value survives it.
    pub plans: Vec<Plan>,
    /// `acid cutoff 1.2khz`, one per line in the `midi` block that names the
    /// controller. Empty when nothing is mapped to it.
    pub readings: Vec<String>,
}

/// A song the planner knows the player has (or will have) an engine for.
/// Instrument names come from the built engine, not the compiled song: the
/// engine gives every track that names a module its own copy, appended past
/// the compiled indices under the same name.
struct Known {
    generation: Generation,
    ast: Song,
    instruments: Vec<String>,
    /// Which engine instrument each track plays (top level).
    track_instruments: Vec<usize>,
    tracks: Vec<String>,
    buses: Vec<String>,
    /// The `as` names in each track's insert chain.
    track_nodes: Vec<Vec<Option<String>>>,
    /// The note each track plays most, for a bass zone's register.
    track_home: Vec<Option<u8>>,
    /// The `midi` block resolved against this engine. Re-resolved whenever
    /// the text changes, which needs only the names above, so remapping a
    /// knob takes effect at once instead of waiting for a swap.
    controls: midi::Controls,
}

impl Known {
    fn new(generation: Generation, ast: Song, engine: &SongEngine) -> Self {
        let mut known = Self {
            generation,
            ast,
            instruments: (0..engine.instrument_count()).map(|i| String::from(engine.instrument_name(i))).collect(),
            track_instruments: (0..engine.track_count())
                .map(|i| engine.track_instrument(i).unwrap_or(usize::MAX))
                .collect(),
            tracks: (0..engine.track_count()).map(|i| String::from(engine.track_name(i))).collect(),
            buses: (0..engine.bus_count()).map(|i| String::from(engine.bus_name(i))).collect(),
            track_nodes: (0..engine.track_count())
                .map(|i| engine.track_node_labels(i).map(|l| l.map(String::from)).collect())
                .collect(),
            track_home: (0..engine.track_count()).map(|i| engine.track_home_note(i)).collect(),
            controls: midi::Controls::default(),
        };
        known.resolve_controls();
        known
    }

    fn set_ast(&mut self, ast: Song) {
        self.ast = ast;
        self.resolve_controls();
    }

    fn names(&self) -> midi::Names<'_> {
        midi::Names {
            instruments: &self.instruments,
            tracks: &self.tracks,
            track_instruments: &self.track_instruments,
            track_nodes: &self.track_nodes,
            track_home: &self.track_home,
        }
    }

    fn resolve_controls(&mut self) {
        self.controls = midi::resolve_all(&self.ast, &self.names());
    }

    /// What the `midi` block's lines for controller `cc` do at `value`.
    fn block_knob_ops(&self, cc: u8, value: u8) -> impl Iterator<Item = FastOp> + '_ {
        self.controls.knobs.iter().filter(move |k| k.cc == cc).flat_map(move |k| k.ops(value))
    }
}

/// A key held in the bass or lead zone, with what it sounds on: by track
/// name, so the note lets go where it started even after a scene has
/// changed what the zone plays, or a rebuild has moved the track's index.
#[derive(Debug, Clone, PartialEq)]
struct ZoneNote {
    /// When it went down, counted by the planner: the newest held key of
    /// either zone is where the hands are.
    seq: u64,
    key: u8,
    /// The note after the scale lock.
    note: u8,
    velocity: f32,
    track: String,
    roll: bool,
    /// One note at a time on this track.
    mono: bool,
    /// The drum track whose kick a roll strikes.
    kick: Option<String>,
}

impl ZoneNote {
    fn same_sound(a: Option<&ZoneNote>, b: Option<&ZoneNote>) -> bool {
        match (a, b) {
            (Some(a), Some(b)) => a.track == b.track && a.note == b.note,
            (None, None) => true,
            _ => false,
        }
    }

    /// The roll this note asks of `known`, or the stop when `on` is false.
    fn roll_op(&self, known: &Known, on: bool) -> Option<FastOp> {
        let track = known.tracks.iter().position(|t| *t == self.track)?;
        let kick = self
            .kick
            .as_ref()
            .and_then(|k| Some((known.tracks.iter().position(|t| t == k)?, crate::dsl::compiler::drum_note("kick")?)));
        Some(if on {
            FastOp::Roll { track, note: self.note, velocity: self.velocity, kick }
        } else {
            FastOp::Roll { track, note: 0, velocity: 0.0, kick: None }
        })
    }

    fn note_op(&self, known: &Known, on: bool) -> Option<FastOp> {
        let track = known.tracks.iter().position(|t| *t == self.track)?;
        Some(if on {
            FastOp::NoteOn { track, note: self.note, velocity: self.velocity }
        } else {
            FastOp::NoteOff { track, note: self.note }
        })
    }
}

/// Which of the two ways to hit something: the pads, or the keys of the
/// keyboard's trigger zone.
#[derive(Clone, Copy)]
enum Bank {
    Pads,
    Keys,
}

impl Bank {
    fn of(self, controls: &midi::Controls) -> &[midi::Pad] {
        match self {
            Bank::Pads => &controls.pads,
            Bank::Keys => &controls.triggers,
        }
    }
}

/// Control-thread half. Owns the ASTs of what plays and what is queued.
pub struct LivePlanner {
    running: Option<Known>,
    pending: Option<Known>,
    next_generation: Generation,
    /// Where each knob that has been turned was left, by controller number.
    /// The last hand wins: a save that rebuilds the engine keeps these, and
    /// editing a knob's target in the text drops it, so the edit plays.
    knob_values: Vec<(u8, u8)>,
    /// Tracks a `toggle` pad or the keyboard has taken out, by name.
    muted: Vec<String>,
    /// The tracks soloed from the keyboard, and what was muted before, to put
    /// back when the solo comes off.
    solo: Option<(Vec<String>, Vec<String>)>,
    /// Pads held down right now, by note.
    pads_down: Vec<u8>,
    /// Keys of the trigger zone held down, by note.
    keys_down: Vec<u8>,
    /// The scene playing, by name; `None` before any.
    scene: Option<String>,
    /// Keys held in the bass zone and the lead zone, oldest first.
    bass_held: Vec<ZoneNote>,
    lead_held: Vec<ZoneNote>,
    /// Keys counted as they go down, for `ZoneNote::seq`.
    seq: u64,
    /// The bend lines the pitch strip follows while it is away from rest:
    /// chosen by where the hands were as it left rest, so a gesture ends on
    /// what it started on whatever the hands do meanwhile.
    bend_gesture: Option<Vec<(ZoneKind, crate::dsl::ast::BendRange)>>,
    /// The same for each controller a scene has lines for: the lines, by
    /// index in the scene's list, a move away from rest started on.
    knob_gesture: Vec<(u8, Vec<usize>)>,
    /// Keys held down, oldest first, with the velocity each was struck at.
    /// A mono instrument plays the newest; letting it go falls back to the
    /// one before it, which is how a monosynth answers a keyboard.
    held: Vec<(u8, f32)>,
    /// Where the pitch strip is, -1..1, 0 at rest. Kept like a knob value,
    /// so an engine built mid-bend starts bent.
    bend: f32,
    /// `--solo`/`--mute`, applied to every version of the file as it is read.
    isolation: crate::dsl::isolate::Isolation,
    /// The output gain measured when the song was loaded. Edits keep it: a
    /// song that re-levelled itself on every save would move under the hands.
    output_gain: Option<f32>,
    /// Every new text starts its top-level lanes on the bar it takes over,
    /// even a lane it shares with the text before: a set's steps, each of
    /// which is its own moment. Off, an unchanged lane keeps its progress.
    restart_lanes: bool,
}

impl Default for LivePlanner {
    fn default() -> Self {
        Self::new()
    }
}

impl LivePlanner {
    pub fn new() -> Self {
        Self {
            running: None,
            pending: None,
            next_generation: 1,
            knob_values: Vec::new(),
            muted: Vec::new(),
            solo: None,
            pads_down: Vec::new(),
            keys_down: Vec::new(),
            scene: None,
            bass_held: Vec::new(),
            lead_held: Vec::new(),
            seq: 0,
            bend_gesture: None,
            knob_gesture: Vec::new(),
            held: Vec::new(),
            bend: 0.0,
            output_gain: None,
            isolation: Default::default(),
            restart_lanes: false,
        }
    }

    /// Plan for a set: each text is a step, and its `auto ... over N` lanes
    /// start on the step's first bar even when the step before had the same
    /// line. A step with lanes therefore always takes over with a swap,
    /// which is what gives it a first bar.
    pub fn restart_lanes(&mut self) {
        self.restart_lanes = true;
    }

    /// A queued swap that the player reports as landed becomes what runs.
    fn catch_up(&mut self, playing: Generation) {
        if self.pending.as_ref().is_some_and(|p| p.generation == playing) {
            self.running = self.pending.take();
        }
    }

    /// Whether the song being played maps any controller.
    pub fn has_knobs(&self) -> bool {
        self.pending.as_ref().or(self.running.as_ref()).is_some_and(|k| !k.ast.midi.is_empty())
    }

    /// The engines a live event is resolved against: the one playing and
    /// the one queued behind it.
    fn known(&self) -> impl Iterator<Item = &Known> {
        self.running.iter().chain(self.pending.iter())
    }

    /// A key on the keyboard went down (`velocity` above zero) or up. `None`
    /// when the song maps no keys.
    pub fn key(&mut self, note: u8, velocity: u8, playing: Generation) -> Option<Vec<Plan>> {
        self.catch_up(playing);
        let latest = self.pending.as_ref().or(self.running.as_ref())?;
        if latest.controls.triggers.iter().any(|t| t.note == note) {
            return self.hit(Bank::Keys, note, velocity);
        }
        match latest.controls.zone_of(note).map(|z| z.kind) {
            Some(ZoneKind::Triggers) => return None,
            Some(kind) => return Some(self.zone_key(kind, note, velocity)),
            None => {}
        }
        if self.known().all(|k| k.controls.keys.is_empty()) {
            return None;
        }
        let velocity = velocity.min(127) as f32 / 127.0;
        // Where the mono instruments were before this key, and after it.
        let before = self.held.last().copied();
        self.held.retain(|(n, _)| *n != note);
        if velocity > 0.0 {
            self.held.push((note, velocity));
        }
        let after = self.held.last().copied();

        let mut plans = Vec::new();
        for known in self.known() {
            for keys in &known.controls.keys {
                let track = keys.track;
                let mut play = |op| plans.push(Plan::Play { base: known.generation, op });
                if !keys.mono {
                    play(if velocity > 0.0 {
                        FastOp::NoteOn { track, note, velocity }
                    } else {
                        FastOp::NoteOff { track, note }
                    });
                    continue;
                }
                match (before, after) {
                    // The key that sounds did not change: a key under it
                    // went up, or went down under a newer one. Nothing.
                    (b, a) if b.map(|x| x.0) == a.map(|x| x.0) => {}
                    // A newer key, or back to the one before: the bass
                    // glides there if a note is still sounding.
                    (_, Some((n, v))) => play(FastOp::NoteOn { track, note: n, velocity: v }),
                    (Some((n, _)), None) => play(FastOp::NoteOff { track, note: n }),
                    (None, None) => {}
                }
            }
        }
        Some(plans)
    }

    /// A pad on the drum channel went down (`velocity` above zero) or up.
    /// `None` when nothing is mapped to it. A drum is struck on the way down;
    /// `mute`, `throw` and `freeze` last until the pad comes up; `toggle`
    /// flips on the way down. A set move (`next`, `step 3`) plans nothing
    /// here: [`LivePlanner::pad_navigation`] says what it asks for.
    pub fn pad(&mut self, note: u8, velocity: u8, playing: Generation) -> Option<Vec<Plan>> {
        self.catch_up(playing);
        self.hit(Bank::Pads, note, velocity)
    }

    /// A pad, or a key of the trigger zone, which does what a pad does.
    fn hit(&mut self, bank: Bank, note: u8, velocity: u8) -> Option<Vec<Plan>> {
        if !self.known().any(|k| bank.of(&k.controls).iter().any(|p| p.note == note)) {
            return None;
        }
        let down = velocity > 0;
        let velocity = velocity.min(127) as f32 / 127.0;
        let held = match bank {
            Bank::Pads => &mut self.pads_down,
            Bank::Keys => &mut self.keys_down,
        };
        held.retain(|&n| n != note);
        if down {
            held.push(note);
        }
        // What the player pairs a quantized release with: a key never with
        // the pad of the same number.
        let line_id = match bank {
            Bank::Pads => note,
            Bank::Keys => note | 0x80,
        };
        // A toggle flips once per hit, by track name, so every engine hears
        // the same state and a rebuilt one inherits it.
        let latest = self.pending.as_ref().or(self.running.as_ref());
        let toggled: Vec<String> = latest
            .map(|k| {
                bank.of(&k.controls)
                    .iter()
                    .filter(|p| p.note == note)
                    .filter_map(|p| match p.action {
                        midi::PadAction::Toggle { track } => k.tracks.get(track).cloned(),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        if down {
            for name in toggled {
                match self.muted.iter().position(|m| *m == name) {
                    Some(i) => {
                        self.muted.remove(i);
                    }
                    None => self.muted.push(name),
                }
            }
        }
        let mut plans = Vec::new();
        // A repeat is the player's, not an engine's: one plan, from the
        // latest mapping.
        let repeat = self
            .pending
            .as_ref()
            .or(self.running.as_ref())
            .and_then(|k| bank.of(&k.controls).iter().find(|p| p.note == note))
            .and_then(|p| match p.action {
                midi::PadAction::Repeat { sixteenths } => Some(sixteenths),
                _ => None,
            });
        if let Some(sixteenths) = repeat {
            plans.push(Plan::Repeat(down.then_some(sixteenths)));
        }
        for known in self.known() {
            for pad in bank.of(&known.controls).iter().filter(|p| p.note == note) {
                let base = known.generation;
                let plan = match pad.action {
                    midi::PadAction::Drum { track, drum } if down => {
                        Plan::Play { base, op: FastOp::NoteOn { track, note: drum, velocity } }
                    }
                    midi::PadAction::Play { track, note } => {
                        let op = if down {
                            FastOp::NoteOn { track, note, velocity }
                        } else {
                            FastOp::NoteOff { track, note }
                        };
                        Plan::Play { base, op }
                    }
                    // Every way a pad takes a track out, asked together, so
                    // letting go of one does not undo another still held.
                    midi::PadAction::Mute { track } | midi::PadAction::Hold { track } => {
                        Plan::Control { base, op: FastOp::TrackMute { track, muted: self.muted_now(known, track) } }
                    }
                    midi::PadAction::Throw { track } => {
                        Plan::Control { base, op: FastOp::TrackThrow { track, on: down } }
                    }
                    midi::PadAction::Freeze => Plan::Control { base, op: FastOp::FreezeHold(down) },
                    midi::PadAction::Toggle { track } if down => {
                        Plan::Control { base, op: FastOp::TrackMute { track, muted: self.muted_now(known, track) } }
                    }
                    _ => continue,
                };
                plans.push(match (pad.quantize, plan) {
                    (Some(grid), Plan::Play { base, op }) => {
                        Plan::Quantized { grid, pad: line_id, down, inner: Deferred::Play { base, op } }
                    }
                    (Some(grid), Plan::Control { base, op }) => {
                        Plan::Quantized { grid, pad: line_id, down, inner: Deferred::Control { base, op } }
                    }
                    (_, plan) => plan,
                });
            }
        }
        Some(plans)
    }

    /// Mute a track, or bring it back, by name: the state a toggle pad keeps,
    /// so a pad, a key and a save all see the same thing.
    pub fn toggle_mute(&mut self, name: &str, playing: Generation) -> Vec<Plan> {
        self.catch_up(playing);
        match self.muted.iter().position(|m| m == name) {
            Some(i) => {
                self.muted.remove(i);
            }
            None => self.muted.push(String::from(name)),
        }
        self.mute_plans()
    }

    /// Solo a track: every other one muted. The same track again takes the
    /// solo off and puts back what was muted before it; another track moves
    /// the solo there.
    pub fn toggle_solo(&mut self, name: &str, playing: Generation) -> Vec<Plan> {
        self.toggle_solo_group(&[String::from(name)], playing)
    }

    /// Solo several tracks together: every track not among them muted. The
    /// same group again takes the solo off; another group moves it.
    pub fn toggle_solo_group(&mut self, names: &[String], playing: Generation) -> Vec<Plan> {
        self.catch_up(playing);
        let mut group: Vec<String> = names.to_vec();
        group.sort();
        group.dedup();
        let before = match self.solo.take() {
            Some((soloed, before)) if soloed == group => {
                self.muted = before;
                return self.mute_plans();
            }
            Some((_, before)) => before,
            None => self.muted.clone(),
        };
        // The running engine and one waiting to take over list the same
        // tracks, and a swap may add some: each name once.
        let mut muted: Vec<String> = Vec::new();
        for t in self.known().flat_map(|k| k.tracks.iter()) {
            if !group.contains(t) && !muted.contains(t) {
                muted.push(t.clone());
            }
        }
        self.muted = muted;
        self.solo = Some((group, before));
        self.mute_plans()
    }

    /// The tracks soloed from the keyboard, if any are.
    pub fn soloed(&self) -> &[String] {
        self.solo.as_ref().map_or(&[], |(n, _)| n.as_slice())
    }

    /// Whether a track is out right now: toggled or soloed out, a `mute`
    /// pad on it held down, or a `hold` pad on it not held.
    fn muted_now(&self, known: &Known, track: usize) -> bool {
        let toggled = known.tracks.get(track).is_some_and(|n| self.muted.contains(n));
        let out = |list: &[midi::Pad], down: &[u8]| {
            list.iter().any(|p| match p.action {
                midi::PadAction::Mute { track: t } => t == track && down.contains(&p.note),
                midi::PadAction::Hold { track: t } => t == track && !down.contains(&p.note),
                _ => false,
            })
        };
        toggled || out(&known.controls.pads, &self.pads_down) || out(&known.controls.triggers, &self.keys_down)
    }

    /// Every track's mute as the planner has it, for every engine it knows.
    fn mute_plans(&self) -> Vec<Plan> {
        let mut plans = Vec::new();
        for known in self.known() {
            for (track, _) in known.tracks.iter().enumerate() {
                let muted = self.muted_now(known, track);
                plans.push(Plan::Control { base: known.generation, op: FastOp::TrackMute { track, muted } });
            }
        }
        plans
    }

    /// The grid a pad waits for, if it is quantized: for a status line.
    pub fn pad_quantize(&self, note: u8) -> Option<Quantize> {
        let latest = self.pending.as_ref().or(self.running.as_ref())?;
        latest.controls.pads.iter().find(|p| p.note == note).and_then(|p| p.quantize)
    }

    /// The set move a pad asks for when it goes down, if it is mapped to one.
    pub fn pad_navigation(&self, note: u8) -> Option<midi::PadAction> {
        let latest = self.pending.as_ref().or(self.running.as_ref())?;
        latest.controls.pads.iter().find(|p| p.note == note && p.action.navigation()).map(|p| p.action)
    }

    /// What the pads hold on a fresh engine: toggled tracks out, and whatever
    /// a pad still pressed does.
    fn pad_ops(&self, known: &Known) -> Vec<FastOp> {
        let mut ops = Vec::new();
        for track in 0..known.tracks.len() {
            if self.muted_now(known, track) {
                ops.push(FastOp::TrackMute { track, muted: true });
            }
        }
        let down =
            self.pads_down.iter().flat_map(|&n| known.controls.pads.iter().filter(move |p| p.note == n)).chain(
                self.keys_down.iter().flat_map(|&n| known.controls.triggers.iter().filter(move |p| p.note == n)),
            );
        for pad in down {
            match pad.action {
                midi::PadAction::Throw { track } => ops.push(FastOp::TrackThrow { track, on: true }),
                midi::PadAction::Freeze => ops.push(FastOp::FreezeHold(true)),
                _ => {}
            }
        }
        // A roll held across a rebuild keeps rolling on the new engine.
        if let Some(op) = self.bass_held.last().filter(|h| h.roll).and_then(|h| h.roll_op(known, true)) {
            ops.push(op);
        }
        ops
    }

    /// A key in the bass or lead zone: kept to the scene's scale, and
    /// played on the zone's track as the scene has it. The bass zone and a
    /// lead on a `bass` module play one note at a time, the newest held; a
    /// roll rolls it.
    fn zone_key(&mut self, kind: ZoneKind, note: u8, velocity: u8) -> Vec<Plan> {
        self.seq += 1;
        let seq = self.seq;
        let Some(latest) = self.pending.as_ref().or(self.running.as_ref()) else { return Vec::new() };
        let scene = self.scene.as_deref();
        let stack = match kind {
            ZoneKind::Bass => &mut self.bass_held,
            _ => &mut self.lead_held,
        };
        let before = stack.last().cloned();
        let mut released = None;
        if velocity > 0 {
            let Some(t) = latest.controls.zone_target(kind, scene) else { return Vec::new() };
            let (scale, lock) = latest.controls.lock_under(scene);
            let out = match scale {
                Some(s) => s.lock(lock, note),
                None => Some(note),
            };
            // A black key under `lock white` plays nothing; the zone's
            // register moves every key by whole octaves.
            let Some(out) = out.map(|n| n as i32 + t.shift as i32).filter(|n| (0..=127).contains(n)) else {
                return Vec::new();
            };
            let out = out as u8;
            let name = |i: usize| latest.tracks.get(i).cloned();
            let Some(track) = name(t.track) else { return Vec::new() };
            stack.retain(|h| h.key != note);
            stack.push(ZoneNote {
                seq,
                key: note,
                note: out,
                velocity: velocity.min(127) as f32 / 127.0,
                track,
                roll: t.roll,
                mono: kind == ZoneKind::Bass || t.mono,
                kick: t.kick.and_then(|k| name(k.0)),
            });
        } else {
            let Some(i) = stack.iter().position(|h| h.key == note) else { return Vec::new() };
            released = Some(stack.remove(i));
        }
        let after = stack.last().cloned();
        let mut plans = Vec::new();
        for known in self.known() {
            let base = known.generation;
            let mut control = |op: Option<FastOp>| {
                if let Some(op) = op {
                    plans.push(Plan::Control { base, op });
                }
            };
            let mut ops: Vec<FastOp> = Vec::new();
            // What sounds is the newest held key, on a mono track or a
            // roll; on a poly track, every key held.
            let mono = after.as_ref().or(before.as_ref()).is_some_and(|h| h.mono || h.roll);
            if mono {
                if ZoneNote::same_sound(before.as_ref(), after.as_ref()) {
                    continue;
                }
                if let Some(b) = &before {
                    let moved_track = after.as_ref().is_none_or(|a| a.track != b.track);
                    if b.roll && moved_track {
                        control(b.roll_op(known, false));
                    } else if !b.roll && (moved_track || after.as_ref().is_some_and(|a| a.roll)) {
                        ops.extend(b.note_op(known, false));
                    }
                }
                match &after {
                    Some(a) if a.roll => control(a.roll_op(known, true)),
                    // A newer key, or back to the one before: a bass glides.
                    Some(a) => ops.extend(a.note_op(known, true)),
                    None => {}
                }
            } else if let Some(r) = &released {
                // Two keys can snap to one note: it stops when the last does.
                let held = match kind {
                    ZoneKind::Bass => &self.bass_held,
                    _ => &self.lead_held,
                };
                if !held.iter().any(|h| h.track == r.track && h.note == r.note) {
                    ops.extend(r.note_op(known, false));
                }
            } else if let Some(a) = &after {
                ops.extend(a.note_op(known, true));
            }
            plans.extend(ops.into_iter().map(|op| Plan::Play { base, op }));
        }
        // A bend held while the key changes: in degrees its reach depends on
        // the note, and it follows the newest key's track.
        if self.bend != 0.0 {
            plans.extend(self.bend_plans());
        }
        plans
    }

    /// The song as the latest engine has it.
    pub fn song(&self) -> Option<&Song> {
        self.pending.as_ref().or(self.running.as_ref()).map(|k| &k.ast)
    }

    /// What a key on the keyboard channel plays now, for the screen: its
    /// zone, the track, and the note after the scale lock and the zone's
    /// register. `None` outside the bass and lead zones, or for a key that
    /// plays nothing.
    pub fn zone_note(&self, note: u8) -> Option<(ZoneKind, String, u8, bool)> {
        let known = self.pending.as_ref().or(self.running.as_ref())?;
        let kind = known.controls.zone_of(note)?.kind;
        if kind == ZoneKind::Triggers {
            return None;
        }
        let scene = self.scene.as_deref();
        let t = known.controls.zone_target(kind, scene)?;
        let (scale, lock) = known.controls.lock_under(scene);
        let out = match scale {
            Some(s) => s.lock(lock, note)?,
            None => note,
        } as i32
            + t.shift as i32;
        let out = u8::try_from(out).ok().filter(|n| *n <= 127)?;
        Some((kind, known.tracks.get(t.track)?.clone(), out, t.roll))
    }

    /// The scenes the song has, in the order written.
    pub fn scene_names(&self) -> Vec<String> {
        self.pending
            .as_ref()
            .or(self.running.as_ref())
            .map(|k| k.ast.perform.scenes.iter().map(|s| s.name.clone()).collect())
            .unwrap_or_default()
    }

    /// The scene playing.
    pub fn scene(&self) -> Option<&str> {
        self.scene.as_deref()
    }

    /// A short line for the screen: `drop · E phrygian_dominant · roll`.
    pub fn describe_scene(&self, name: &str) -> String {
        let Some(k) = self.pending.as_ref().or(self.running.as_ref()) else { return String::from(name) };
        let Some(sc) = k.ast.perform.scenes.iter().find(|s| s.name == name) else { return String::from(name) };
        let scale = sc.scale.as_ref().or(k.ast.globals.scale.as_ref());
        let mut out = String::from(name);
        if let Some(s) = scale {
            out.push_str(&alloc::format!(" · {} {}", s.root, s.kind));
        }
        if let Some(b) = &sc.bass {
            out.push_str(&alloc::format!(" · bass {}{}", b.track, if b.roll { " roll" } else { "" }));
        }
        if let Some(l) = &sc.lead {
            out.push_str(&alloc::format!(" · lead {}", l.track));
        }
        out
    }

    /// Scene `name`'s `set` values, to land on bar `bar` as the engine
    /// counts bars (`None`: at once). Sent when the scene is asked for, so
    /// they wait on the audio thread and land on the sample of the line.
    /// `None` when the song has no such scene.
    pub fn scene_values(&mut self, name: &str, bar: Option<usize>, playing: Generation) -> Option<Vec<Plan>> {
        self.catch_up(playing);
        self.pending.as_ref().or(self.running.as_ref())?.controls.scene(name)?;
        let mut plans = Vec::new();
        for known in self.known() {
            let base = known.generation;
            let Some(sc) = known.controls.scene(name) else { continue };
            let ops: Vec<FastOp> = sc.sets.iter().flat_map(|k| k.ops(0)).collect();
            match bar {
                Some(bar) => {
                    for op in ops {
                        plans.push(Plan::AtBar { bar, inner: Deferred::Set { base, op } });
                    }
                }
                None if !ops.is_empty() => plans.push(Plan::Fast { base, ops }),
                None => {}
            }
        }
        Some(plans)
    }

    /// Scene `name` takes the keyboard: what the zones play and the scale
    /// they keep to change from the next key. A key already held sounds on
    /// where it started until it is let go, so a note played on the line
    /// is not cut. `None` when the song has no such scene.
    ///
    /// What the leaving scene's controller lines moved goes back to the
    /// bottom of their travel, where a scene's ranges start from the sound
    /// as written. The new scene's lines that answer where the hands are
    /// move to where each controller is, and a controller or the strip held
    /// away from rest carries on with them.
    pub fn enter_scene(&mut self, name: &str, playing: Generation) -> Option<Vec<Plan>> {
        self.catch_up(playing);
        self.pending.as_ref().or(self.running.as_ref())?.controls.scene(name)?;
        let old = self.scene.replace(String::from(name));
        let mut plans = Vec::new();
        for known in self.known() {
            let base = known.generation;
            if let Some(sc) = old.as_deref().and_then(|o| known.controls.scene(o)) {
                for op in sc.knobs.iter().flat_map(|(_, k)| k.ops(0)) {
                    plans.push(Plan::Control { base, op });
                }
            }
        }
        self.knob_gesture.clear();
        let ccs: Vec<u8> = self
            .pending
            .as_ref()
            .or(self.running.as_ref())
            .and_then(|k| k.controls.scene(name))
            .map(|sc| sc.knobs.iter().map(|(_, k)| k.cc).collect())
            .unwrap_or_default();
        for cc in ccs {
            let value = self.knob_values.iter().find(|(c, _)| *c == cc).map_or(0, |(_, v)| *v);
            let lines = self.knob_lines(cc, value);
            for known in self.known() {
                for op in self.scene_knob_ops(known, &lines, value) {
                    plans.push(Plan::Control { base: known.generation, op });
                }
            }
        }
        self.bend_gesture = None;
        if self.bend.abs() > BEND_REST {
            self.bend_gesture = Some(self.bend_lines_now());
        }
        plans.extend(self.bend_plans());
        Some(plans)
    }

    /// Where the hands are: the zone of the newest key held in the bass or
    /// the lead zone, `None` with both empty.
    pub fn hands(&self) -> Option<ZoneKind> {
        match (self.bass_held.last(), self.lead_held.last()) {
            (Some(b), Some(l)) => Some(if b.seq > l.seq { ZoneKind::Bass } else { ZoneKind::Lead }),
            (Some(_), None) => Some(ZoneKind::Bass),
            (None, Some(_)) => Some(ZoneKind::Lead),
            (None, None) => None,
        }
    }

    /// The scene's lines for controller `cc` that move at `value`, starting
    /// or ending a gesture: away from rest, the ones the gesture started
    /// on (chosen now if it starts now); back at rest, those and the ones
    /// the hands choose now, so whatever was moved comes home. `None` when
    /// the scene has no line for it.
    fn knob_lines(&mut self, cc: u8, value: u8) -> Option<Vec<usize>> {
        let latest = self.pending.as_ref().or(self.running.as_ref())?;
        let scene = self.scene.as_deref();
        let all = latest.controls.scene_knobs(cc, scene)?;
        let sc = latest.controls.scene(scene?)?;
        let hands = self.hands();
        let now: Vec<usize> = all.iter().copied().filter(|&i| sc.knobs[i].0.answers(hands)).collect();
        let started = self.knob_gesture.iter().position(|(c, _)| *c == cc);
        if value <= KNOB_REST {
            let mut lines = started.map(|i| self.knob_gesture.remove(i).1).unwrap_or_default();
            for i in now {
                if !lines.contains(&i) {
                    lines.push(i);
                }
            }
            return Some(lines);
        }
        match started {
            Some(i) => Some(self.knob_gesture[i].1.clone()),
            None => {
                self.knob_gesture.push((cc, now.clone()));
                Some(now)
            }
        }
    }

    /// What `lines` of the scene (or, with none, the `midi` block's line)
    /// do to `known` at `value`.
    fn scene_knob_ops(&self, known: &Known, lines: &Option<Vec<usize>>, value: u8) -> Vec<FastOp> {
        let sc = self.scene.as_deref().and_then(|n| known.controls.scene(n));
        match (lines, sc) {
            (Some(lines), Some(sc)) => {
                lines.iter().filter_map(|&i| sc.knobs.get(i)).flat_map(|(_, k)| k.ops(value)).collect()
            }
            _ => Vec::new(),
        }
    }

    /// The song's `keyboard` bindings, and how many bars a scene called
    /// from them waits for (1: the next bar).
    pub fn keyboard(&self) -> (Vec<crate::dsl::ast::KeyBinding>, u32) {
        match self.pending.as_ref().or(self.running.as_ref()) {
            Some(k) => (k.ast.perform.keyboard.clone(), k.ast.perform.scene_bars.unwrap_or(1).max(1)),
            None => (Vec::new(), 1),
        }
    }

    /// Whether the song has scenes or binds computer keys.
    pub fn has_keyboard(&self) -> bool {
        self.pending
            .as_ref()
            .or(self.running.as_ref())
            .is_some_and(|k| !k.ast.perform.keyboard.is_empty() || !k.ast.perform.scenes.is_empty())
    }

    /// The pitch strip moved to `value`, 14 bits with 8192 at rest. On a
    /// zone it bends as the scene says (`bend lead 24st`, `bend lead 2deg`);
    /// `keys >` tracks, up to two semitones either way. Sent like a knob, to
    /// the engine playing and the one queued, since it is a position rather
    /// than an event.
    pub fn bend(&mut self, value: u16, playing: Generation) -> Vec<Plan> {
        self.catch_up(playing);
        self.bend = ((value.min(16383) as f32 - 8192.0) / 8192.0).clamp(-1.0, 1.0);
        if self.bend.abs() <= BEND_REST {
            let plans = self.bend_plans();
            self.bend_gesture = None;
            return plans;
        }
        if self.bend_gesture.is_none() {
            self.bend_gesture = Some(self.bend_lines_now());
        }
        self.bend_plans()
    }

    /// The bend line each zone follows with the hands where they are now:
    /// the first of its lines that answers them.
    fn bend_lines_now(&self) -> Vec<(ZoneKind, crate::dsl::ast::BendRange)> {
        let Some(latest) = self.pending.as_ref().or(self.running.as_ref()) else { return Vec::new() };
        let hands = self.hands();
        let scene = self.scene.as_deref();
        [ZoneKind::Bass, ZoneKind::Lead]
            .into_iter()
            .filter_map(|zone| {
                let line = latest.controls.bends_under(zone, scene).into_iter().find(|(_, h)| h.answers(hands));
                line.map(|(range, _)| (zone, range))
            })
            .collect()
    }

    fn bend_plans(&self) -> Vec<Plan> {
        let mut plans = Vec::new();
        for known in self.known() {
            for op in self.bend_ops(known) {
                plans.push(Plan::Control { base: known.generation, op });
            }
        }
        plans
    }

    /// Where the strip puts every instrument it reaches in `known`. A zone
    /// bends the track its newest held key sounds on, or the one the scene
    /// gives it; in degrees, from that key's note to the note so many
    /// degrees of the scale away, so a full bend lands in the scale.
    fn bend_ops(&self, known: &Known) -> Vec<FastOp> {
        let v = self.bend;
        let ratio = |st: f32| crate::math::pow(2.0, st / 12.0);
        let mut ops: Vec<FastOp> = known
            .controls
            .keys
            .iter()
            .map(|k| FastOp::PitchBend { instrument: k.instrument, ratio: ratio(v * BEND_SEMITONES) })
            .collect();
        let scene = self.scene.as_deref();
        for (zone, held) in [(ZoneKind::Bass, &self.bass_held), (ZoneKind::Lead, &self.lead_held)] {
            if !known.controls.zones.iter().any(|z| z.kind == zone) {
                continue;
            }
            let newest = held.last();
            let track = newest
                .and_then(|h| known.tracks.iter().position(|t| *t == h.track))
                .or_else(|| known.controls.zone_target(zone, scene).map(|t| t.track));
            let Some(&instrument) = track.and_then(|t| known.track_instruments.get(t)) else { continue };
            if instrument == usize::MAX {
                continue;
            }
            // With the strip at rest, nothing is bent; away from it, the
            // gesture's line for this zone, or none.
            let range = match &self.bend_gesture {
                Some(lines) => lines.iter().find(|l| l.0 == zone).map(|l| l.1),
                None => None,
            };
            let st = self.zone_semitones(known, zone, range, newest);
            ops.push(FastOp::PitchBend { instrument, ratio: ratio(st) });
        }
        ops
    }

    /// How far the strip, where it is, takes `zone` under `range`.
    fn zone_semitones(
        &self,
        known: &Known,
        zone: ZoneKind,
        range: Option<crate::dsl::ast::BendRange>,
        newest: Option<&ZoneNote>,
    ) -> f32 {
        let v = self.bend;
        let scene = self.scene.as_deref();
        match range.unwrap_or(crate::dsl::ast::BendRange::Off) {
            crate::dsl::ast::BendRange::Off => 0.0,
            crate::dsl::ast::BendRange::Semitones(n) => v * n,
            crate::dsl::ast::BendRange::Degrees(d) => {
                let (scale, _) = known.controls.lock_under(scene);
                let scale = scale.unwrap_or(crate::perform::scale::Scale::new(0, [0, 2, 4, 5, 7, 9, 11]));
                // With nothing held, the root of the octave the zone
                // starts in stands in for the note.
                let note = newest.map(|h| h.note).unwrap_or_else(|| {
                    let low = known.controls.zones.iter().find(|z| z.kind == zone).map_or(60, |z| z.low);
                    scale.snap(low)
                });
                let to = scale.degrees_from(note, if v >= 0.0 { d as i32 } else { -(d as i32) });
                v.abs() * (to as f32 - note as f32)
            }
        }
    }

    /// What the pitch strip does right now, for the screen: `strip: bass
    /// -7.2 st (idle)`, or what it waits for.
    pub fn bend_reading(&self) -> String {
        let Some(known) = self.pending.as_ref().or(self.running.as_ref()) else { return String::from("strip") };
        if self.bend.abs() <= BEND_REST {
            return String::from("strip at rest");
        }
        let scene = self.scene.as_deref();
        let mut said = Vec::new();
        for (zone, range) in self.bend_gesture.iter().flatten() {
            let held = match zone {
                ZoneKind::Bass => &self.bass_held,
                _ => &self.lead_held,
            };
            let newest = held.last();
            let track = newest
                .map(|h| h.track.clone())
                .or_else(|| known.controls.zone_target(*zone, scene).and_then(|t| known.tracks.get(t.track).cloned()));
            let st = self.zone_semitones(known, *zone, Some(*range), newest);
            said.push(format!("{} {:+.1} st", track.unwrap_or_else(|| String::from(zone.word())), st));
        }
        if said.is_empty() {
            // Nothing answers where the hands were as the strip left rest.
            let when: Vec<String> = [ZoneKind::Bass, ZoneKind::Lead]
                .into_iter()
                .flat_map(|z| known.controls.bends_under(z, scene).into_iter().map(move |(_, h)| (z, h)))
                .map(|(z, h)| match h {
                    crate::dsl::ast::Hands::Playing(_) => format!("{} while played", z.word()),
                    other => format!("{} {}", z.word(), other.word()),
                })
                .collect();
            return if when.is_empty() {
                String::from("strip: nothing to bend here")
            } else {
                format!("strip: bends {}", when.join(", "))
            };
        }
        format!("strip: {}", said.join(", "))
    }

    /// Controller `cc` moved to `value` (0..127). Returns a plan per
    /// operation for the engine that plays and for the one queued behind it,
    /// and what the knob now reads. A controller nothing is mapped to returns
    /// nothing and is not remembered.
    pub fn knob(&mut self, cc: u8, value: u8, playing: Generation) -> KnobTurn {
        self.catch_up(playing);
        let mut turn = KnobTurn { plans: Vec::new(), readings: Vec::new() };
        if self.pending.is_none() && self.running.is_none() {
            return turn;
        }
        let lines = self.knob_lines(cc, value);
        let latest = self.pending.as_ref().or(self.running.as_ref()).expect("checked above");
        match (&lines, self.scene.as_deref().and_then(|n| latest.controls.scene(n))) {
            (Some(lines), Some(sc)) => {
                turn.readings = lines
                    .iter()
                    .filter_map(|&i| sc.knobs.get(i))
                    .filter(|(_, k)| !k.moves.is_empty())
                    .map(|(_, k)| k.reading(value))
                    .collect();
                if turn.readings.is_empty() {
                    // The scene has lines for it, waiting for the hands.
                    let when: Vec<&str> = sc.knobs.iter().filter(|(_, k)| k.cc == cc).map(|(h, _)| h.word()).collect();
                    turn.readings.push(format!("cc {}: this scene moves it with {}", cc, when.join(", ")));
                }
                for known in self.known() {
                    for op in self.scene_knob_ops(known, &Some(lines.clone()), value) {
                        turn.plans.push(Plan::Control { base: known.generation, op });
                    }
                }
            }
            _ => {
                turn.readings = latest
                    .controls
                    .knobs
                    .iter()
                    .filter(|k| k.cc == cc && !k.moves.is_empty())
                    .map(|k| k.reading(value))
                    .collect();
                if turn.readings.is_empty() {
                    return turn;
                }
                for known in self.known() {
                    for op in known.block_knob_ops(cc, value) {
                        turn.plans.push(Plan::Control { base: known.generation, op });
                    }
                }
            }
        }
        match self.knob_values.iter_mut().find(|(c, _)| *c == cc) {
            Some(slot) => slot.1 = value,
            None => self.knob_values.push((cc, value)),
        }
        turn
    }

    /// Controller at `value` put on `target` (`acid.cutoff`) as if a `midi`
    /// line mapped it, without one: what trying a knob on a parameter before
    /// keeping it sounds like. Not remembered: a swap puts the text's value
    /// back until the knob is kept and turned again.
    pub fn try_knob(&mut self, target: &str, value: u8, playing: Generation) -> KnobTurn {
        self.catch_up(playing);
        let mut turn = KnobTurn { plans: Vec::new(), readings: Vec::new() };
        for (n, known) in self.known().enumerate() {
            let names = known.names();
            let knob = midi::Knob {
                cc: 0,
                target: target.into(),
                moves: midi::resolve(&known.ast, &names, target),
                span: None,
            };
            if knob.moves.is_empty() {
                continue;
            }
            if n == 0 || turn.readings.is_empty() {
                turn.readings = Vec::from([knob.reading(value)]);
            }
            for op in knob.ops(value) {
                turn.plans.push(Plan::Control { base: known.generation, op });
            }
        }
        turn
    }

    /// Forget the knobs whose target the text now writes differently.
    fn forget_edited_knobs(&mut self, new: &Song) {
        let Some(latest) = self.pending.as_ref().or(self.running.as_ref()) else { return };
        let edited: Vec<u8> = latest
            .controls
            .knobs
            .iter()
            .filter(|k| midi::text_value(&latest.ast, &k.target) != midi::text_value(new, &k.target))
            .map(|k| k.cc)
            .collect();
        self.knob_values.retain(|(cc, _)| !edited.contains(cc));
    }

    /// Level every engine from now on by `gain` instead of measuring the
    /// first song planned. A set measures all its steps and passes the gain
    /// of the loudest: measured on a quiet opening step, the peak of the set
    /// came out many dB too hot and was flattened by the limiter.
    pub fn set_output_gain(&mut self, gain: f32) {
        self.output_gain = Some(gain);
    }

    /// Keep these tracks out of everything planned from now on. Set it before
    /// the first `plan`: it is part of what the file means, so changing it
    /// later shows up as an edit on the next save.
    pub fn isolate(&mut self, isolation: crate::dsl::isolate::Isolation) {
        self.isolation = isolation;
    }

    /// Plan how to get the player from what it has to `source`. `playing` is
    /// the generation the player reports right now ([`LivePlayer::generation`]);
    /// it tells the planner whether a queued swap has landed.
    pub fn plan(&mut self, source: &str, playing: Generation) -> Result<Plan, DslError> {
        self.catch_up(playing);
        let mut ast = crate::dsl::parse(source).map_err(DslError::Parse)?;
        self.isolation.apply(&mut ast);
        let compiled = crate::dsl::compiler::compile(&ast).map_err(DslError::Compile)?;
        self.forget_edited_knobs(&ast);
        // A performance starts in the first scene its text writes, so the
        // wheel and the strip answer before any key has called one.
        if self.scene.is_none() {
            self.scene = ast.perform.scenes.first().map(|s| s.name.clone());
        }

        let latest = self.pending.as_mut().or(self.running.as_mut());
        if let Some(latest) = latest {
            if latest.ast == ast {
                return Ok(Plan::Unchanged);
            }
            let changes = diff::diff(&latest.ast, &ast);
            let restarts = self.restart_lanes && !ast.automations.is_empty();
            if !restarts && !diff::has_structural_change(&changes) {
                if let Some(ops) = resolve_fast(&changes, &ast, &compiled, &latest.instruments) {
                    latest.set_ast(ast);
                    return Ok(Plan::Fast { base: latest.generation, ops });
                }
            }
        }

        // Measured on the whole song, not what `--solo` leaves of it, so a
        // soloed track plays as loud as it sits in the mix.
        let gain = match self.output_gain {
            Some(g) => g,
            None => {
                // As the session will start it: a track a `hold` pad keeps
                // out is not in the measurement.
                let mut rest = crate::dsl::parse(source).map_err(DslError::Parse)?;
                let lufs = if self.isolation.is_empty() && !midi::at_rest(&mut rest) {
                    SongEngine::loudness(&compiled)
                } else {
                    let full = crate::dsl::compiler::compile(&rest).map_err(DslError::Compile)?;
                    SongEngine::loudness(&full)
                };
                *self.output_gain.insert(crate::output::gain_for(lufs))
            }
        };
        let generation = self.next_generation;
        self.next_generation += 1;
        let mut engine = Box::new(SongEngine::from_compiled(compiled));
        engine.set_output_gain(gain);
        let known = Known::new(generation, ast, &engine);
        // The new engine starts where the knobs were left, not where the
        // text puts them, so a save does not undo what the hands did.
        for &(cc, value) in &self.knob_values {
            let scene_has = known.controls.scene_knobs(cc, self.scene.as_deref()).is_some();
            let gesture = self.knob_gesture.iter().find(|(c, _)| *c == cc).map(|(_, l)| l.clone());
            let ops: Vec<FastOp> = if scene_has {
                self.scene_knob_ops(&known, &gesture, value)
            } else {
                known.block_knob_ops(cc, value).collect()
            };
            for op in ops {
                engine.hold(op);
            }
        }
        for op in self.pad_ops(&known) {
            engine.hold(op);
        }
        for op in self.bend_ops(&known) {
            apply_op(&mut engine, op);
        }
        let mut inherit = Vec::with_capacity(2);
        for old in self.running.iter().chain(self.pending.iter()) {
            inherit.push((old.generation, inherit_map(old, &known, self.restart_lanes)));
        }
        if self.running.is_none() {
            self.running = Some(known);
        } else {
            self.pending = Some(known);
        }
        Ok(Plan::Swap { engine, generation, inherit, blend_bars: 0.0 })
    }
}

/// Resolve non-structural changes to indices in the new song. `None` when a
/// name does not resolve, which makes the caller swap instead. `instruments`
/// are the engine's names, one per copy: a module edit reaches every copy.
fn resolve_fast(
    changes: &[DslChange],
    ast: &Song,
    compiled: &CompiledSong,
    instruments: &[String],
) -> Option<Vec<FastOp>> {
    let track = |name: &str| compiled.tracks.iter().position(|t| t.name == name);
    let mut ops = Vec::with_capacity(changes.len());
    for change in changes {
        let op = match change {
            DslChange::TempoChanged(_) => FastOp::Tempo(compiled.globals.tempo),
            DslChange::SwingChanged(_) => FastOp::Swing(compiled.globals.swing.unwrap_or(0.5)),
            DslChange::HumanizeChanged { .. } => FastOp::Humanize {
                velocity: compiled.globals.humanize.unwrap_or(0.0),
                timing: compiled.globals.humanize_timing.unwrap_or(0.0),
            },
            // Values come from the compiled song, which applied the defaults
            // and units, rather than from the diff.
            DslChange::TrackLevelChanged { track_name, .. } => {
                let t = track(track_name)?;
                FastOp::TrackLevel { track: t, level: compiled.tracks[t].level }
            }
            DslChange::TrackPanChanged { track_name, .. } => {
                let t = track(track_name)?;
                FastOp::TrackPan { track: t, pan: compiled.tracks[t].pan }
            }
            DslChange::TrackVelocityChanged { track_name, .. } => {
                let t = track(track_name)?;
                FastOp::TrackVelocity { track: t, velocity: compiled.tracks[t].velocity }
            }
            DslChange::TrackGateChanged { track_name, .. } => {
                let t = track(track_name)?;
                FastOp::TrackGate { track: t, gate: compiled.tracks[t].gate }
            }
            DslChange::ModuleParamChanged { module_name, param_name, value } => {
                let def = ast.module_defs.iter().find(|m| &m.name == module_name)?;
                let kind = ModuleKind::from_str(&def.module_type)?;
                let spec = params::lookup(kind, param_name)?;
                let mut any = false;
                for (instrument, name) in instruments.iter().enumerate() {
                    if name == module_name {
                        ops.push(FastOp::ModuleParam { instrument, id: spec.id, value: *value });
                        any = true;
                    }
                }
                if !any {
                    return None;
                }
                continue;
            }
            DslChange::TrackNodeWetChanged { track_name, node_index, wet } => {
                let t = track(track_name)?;
                FastOp::NodeWet { track: t, node: *node_index, wet: *wet }
            }
            DslChange::StructuralChange => return None,
        };
        ops.push(op);
    }
    Some(ops)
}

fn chain_of<'a>(song: &'a Song, bus: &str) -> Option<&'a Vec<ChainNode>> {
    song.bus_chains.iter().find(|c| c.bus_name == bus).map(|c| &c.chain)
}

/// Whether the instrument called `name` is defined identically in both songs.
fn same_instrument(old: &Song, new: &Song, name: &str) -> bool {
    let og = old.instruments.iter().find(|i| i.name == name);
    let ng = new.instruments.iter().find(|i| i.name == name);
    let om = old.module_defs.iter().find(|m| m.name == name);
    let nm = new.module_defs.iter().find(|m| m.name == name);
    match (og, ng, om, nm) {
        (Some(a), Some(b), None, None) => a == b,
        (None, None, Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// Index in `names` of the `k`-th entry equal to `name`, where `k` is how
/// many entries before `at` in `own` carry that same name: copies of a
/// module pair up by occurrence, first with first.
fn nth_same(names: &[String], own: &[String], at: usize) -> Option<usize> {
    let name = &own[at];
    let k = own[..at].iter().filter(|n| *n == name).count();
    names.iter().enumerate().filter(|(_, n)| *n == name).nth(k).map(|(i, _)| i)
}

/// Every `auto` target a song writes, in its scenes and at the top level.
fn automated(song: &Song) -> impl Iterator<Item = &str> {
    song.scenes.iter().flat_map(|s| s.automations.iter()).chain(song.automations.iter()).map(|a| a.target.as_str())
}

/// What the new song can take over from `old`, by name and definition.
fn inherit_map(old: &Known, known: &Known, restart_lanes: bool) -> Inherit {
    let o = &old.ast;
    let new = &known.ast;

    // A lane moves a value inside something that carries over by `mem::swap`:
    // a module's parameter, a master node's, a node's wet. Where the old text
    // automated it and the new one does not, the piece would arrive holding
    // wherever the lane left it instead of what the new text writes. It is
    // rebuilt from the text instead, as if its definition had changed.
    // Levels, the send mixes and freeze belong to the new engine already.
    let mut master_moved = false;
    let mut moved_modules: Vec<&str> = Vec::new();
    let mut moved_tracks: Vec<&str> = Vec::new();
    for target in automated(o).filter(|t| !automated(new).any(|n| n == *t)) {
        let Some((name, param)) = target.split_once('.') else { continue };
        if name == "master" {
            master_moved = true;
        } else if param.ends_with(".wet") {
            moved_tracks.push(name);
        } else if param != "level" {
            moved_modules.push(name);
        }
    }
    let sends = o.globals.send_delay == new.globals.send_delay
        && o.globals.send_reverb == new.globals.send_reverb
        && chain_of(o, "reverb_return") == chain_of(new, "reverb_return")
        && chain_of(o, "delay_return") == chain_of(new, "delay_return");
    let master = o.master == new.master && !master_moved;

    let buses = known
        .buses
        .iter()
        .map(|name| {
            let j = old.buses.iter().position(|n| n == name)?;
            (chain_of(o, name) == chain_of(new, name)).then_some(j)
        })
        .collect::<Vec<_>>();

    let instruments = (0..known.instruments.len())
        .map(|i| {
            let name = known.instruments[i].as_str();
            let j = nth_same(&old.instruments, &known.instruments, i)?;
            (same_instrument(o, new, name) && !moved_modules.contains(&name)).then_some(j)
        })
        .collect::<Vec<_>>();

    // A track continues only where everything that decides what it plays is
    // the same: its own definition, its pattern, its instrument, the scale and
    // meter the pattern compiles against, the grooves, and the scenes that may
    // override it.
    let composition_same = o.scenes == new.scenes
        && o.arrangement == new.arrangement
        && o.grooves == new.grooves
        && o.globals.scale == new.globals.scale
        && o.globals.meter == new.globals.meter;
    let tracks = known
        .tracks
        .iter()
        .enumerate()
        .map(|(i, name)| {
            if !composition_same || moved_tracks.contains(&name.as_str()) {
                return None;
            }
            let j = old.tracks.iter().position(|n| n == name)?;
            let od = o.tracks.iter().find(|d| &d.name == name)?;
            let nd = new.tracks.iter().find(|d| &d.name == name)?;
            if od != nd {
                return None;
            }
            let op = o.patterns.iter().find(|p| p.name == nd.play)?;
            let np = new.patterns.iter().find(|p| p.name == nd.play)?;
            if op != np {
                return None;
            }
            // The copy this track plays must be the one inherited from the copy
            // the old track played.
            let inst = *known.track_instruments.get(i)?;
            let old_inst = *old.track_instruments.get(j)?;
            (instruments.get(inst).copied().flatten() == Some(old_inst)).then_some(j)
        })
        .collect::<Vec<_>>();

    let lanes = !restart_lanes && o.automations == new.automations;
    Inherit { sends, master, buses, instruments, tracks, lanes }
}

/// What `LivePlayer::apply` did with a plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applied {
    Unchanged,
    /// Values changed on a running engine.
    Fast,
    /// The engine was replaced at once: nothing was playing.
    Loaded,
    /// The engine will take over on the next bar line.
    Queued,
    /// The plan was made against an engine the player no longer has. The
    /// caller re-plans; this is never silently dropped.
    Stale,
    /// A knob value, applied. One against an engine that has since been
    /// replaced lands here too and is dropped on purpose: the planner puts
    /// every knob's value into each engine it builds, so there is nothing to
    /// lose.
    Control,
}

/// The inherit maps an engine carries, each tagged with the generation it was
/// built for.
type InheritMaps = Vec<(Generation, Inherit)>;

/// Something the player is done with: an engine and its inherit maps, or the
/// op list of a plan it has applied. Drop it off the audio thread.
pub struct Retired(
    /// Never read: held so its memory is freed wherever this is dropped.
    #[allow(dead_code)]
    Spent,
);

// Never read: what it holds is only there to be freed with it.
#[allow(dead_code)]
enum Spent {
    Engine(Box<SongEngine>, InheritMaps),
    Ops(Vec<FastOp>),
}

/// Where the low end is split for a blend's bass swap: under it is the bass
/// and the kick, which two tracks cannot share without a pile-up.
const BLEND_SPLIT_HZ: f32 = 150.0;

/// How long the bass swap takes: short enough to read as a cut on the beat,
/// long enough not to click.
const BASS_SWAP_SAMPLES: f32 = 2205.0;

/// Two cascaded one-pole lowpasses, one per channel: the lows of a signal,
/// and by subtraction its highs, which add back to it exactly.
#[derive(Default, Clone, Copy)]
struct Split {
    a: [f32; 2],
    b: [f32; 2],
}

impl Split {
    fn low(&mut self, ch: usize, x: f32, k: f32) -> f32 {
        self.a[ch] += (x - self.a[ch]) * k;
        self.b[ch] += (self.a[ch] - self.b[ch]) * k;
        self.b[ch]
    }
}

/// Two engines playing at once while one hands over to the other, the way a
/// DJ mixes: the new one comes in as the old one goes, and the low end changes hands at the midpoint in
/// 50 ms, so two basses never play together. The old engine follows the new
/// one's tempo, so they stay on the beat through a ramp. Nothing is inherited:
/// each plays its own voices, and the old one takes its tails with it.
struct Blend {
    old: Box<SongEngine>,
    maps: InheritMaps,
    done: usize,
    len: usize,
    split_old: Split,
    split_new: Split,
}

/// The longest a beat repeat can hold: a quarter note at the slowest tempo
/// the engine takes, 20 BPM.
const REPEAT_MAX: usize = (SAMPLE_RATE * 60.0 / 20.0) as usize;

/// Each pass of a repeat fades in and out over this many samples, so the
/// seam where the end of the slice meets its start does not click.
const REPEAT_EDGE: usize = 32;

/// Letting go of a repeat crossfades back to the live mix over this long.
const REPEAT_RELEASE: usize = 256;

#[derive(Clone, Copy, PartialEq)]
enum RepeatState {
    Off,
    /// Pressed: waiting `wait` samples for the next division line.
    Armed {
        wait: usize,
        len: usize,
    },
    /// Copying one division of the output while it plays live.
    Capturing {
        at: usize,
        len: usize,
    },
    /// Playing the slice over and over; `release` counts the way back out.
    Looping {
        at: usize,
        len: usize,
        release: Option<usize>,
    },
}

/// A DJ roll: one division of the output, caught on the grid and looped
/// while the pad is held. The engine keeps playing underneath, so letting
/// go lands back where the song has got to, not where the roll began. Its
/// buffer is allocated with the player, so a pad never allocates.
struct Repeat {
    l: Vec<f32>,
    r: Vec<f32>,
    state: RepeatState,
}

impl Repeat {
    fn new() -> Self {
        Self { l: vec![0.0; REPEAT_MAX], r: vec![0.0; REPEAT_MAX], state: RepeatState::Off }
    }

    fn press(&mut self, wait: usize, len: usize) {
        let len = len.clamp(2 * REPEAT_EDGE, REPEAT_MAX);
        self.state = match self.state {
            // Another division while one rolls: the slice already caught,
            // cut to the new length, as a beat repeat does going 1/8 to 1/16.
            RepeatState::Looping { at, len: old, release: None } => {
                let len = len.min(old);
                RepeatState::Looping { at: at % len, len, release: None }
            }
            _ => RepeatState::Armed { wait, len },
        };
    }

    fn release(&mut self) {
        self.state = match self.state {
            RepeatState::Looping { at, len, release: None } => RepeatState::Looping { at, len, release: Some(0) },
            RepeatState::Looping { .. } => self.state,
            _ => RepeatState::Off,
        };
    }

    fn process(&mut self, out_l: &mut [f32], out_r: &mut [f32]) {
        if self.state == RepeatState::Off {
            return;
        }
        for i in 0..out_l.len() {
            if let RepeatState::Armed { wait, len } = self.state {
                if wait > 0 {
                    self.state = RepeatState::Armed { wait: wait - 1, len };
                    continue;
                }
                self.state = RepeatState::Capturing { at: 0, len };
            }
            match self.state {
                RepeatState::Capturing { at, len } => {
                    self.l[at] = out_l[i];
                    self.r[at] = out_r[i];
                    self.state = if at + 1 >= len {
                        RepeatState::Looping { at: 0, len, release: None }
                    } else {
                        RepeatState::Capturing { at: at + 1, len }
                    };
                }
                RepeatState::Looping { at, len, release } => {
                    let edge = (at.min(len - 1 - at) as f32 / REPEAT_EDGE as f32).min(1.0);
                    let live = release.map_or(0.0, |r| r as f32 / REPEAT_RELEASE as f32);
                    out_l[i] = self.l[at] * edge * (1.0 - live) + out_l[i] * live;
                    out_r[i] = self.r[at] * edge * (1.0 - live) + out_r[i] * live;
                    self.state = match release {
                        Some(r) if r + 1 >= REPEAT_RELEASE => RepeatState::Off,
                        _ => RepeatState::Looping { at: (at + 1) % len, len, release: release.map(|r| r + 1) },
                    };
                    if self.state == RepeatState::Off {
                        return;
                    }
                }
                _ => {}
            }
        }
    }
}

/// How many quantized pad actions can wait for their line at once: two
/// engines' worth of every pad on a controller, down and up.
const MAX_WAITING: usize = 64;

/// A quantized pad that arrives within this share of a division after its
/// line counts as that line, and acts at once: the player was just late.
/// Later than that it waits for the next line.
pub const QUANTIZE_FORGIVENESS: f32 = 0.25;

/// Audio-thread half. Nothing in here allocates or frees once constructed,
/// except `apply` when handed more to retire than it has room for, which
/// then drops one in place rather than lose it.
pub struct LivePlayer {
    engine: Option<Box<SongEngine>>,
    generation: Generation,
    pending: Option<(Box<SongEngine>, Generation, InheritMaps)>,
    /// The engine that just handed over, how far its fade-out has run, and
    /// the inherit maps of that handover, which retire with it.
    fading: Option<(Box<SongEngine>, usize, InheritMaps)>,
    /// A queued swap's blend, in bars; see `Plan::Swap`.
    pending_blend: f32,
    /// Two engines playing at once: see [`Blend`].
    blending: Option<Blend>,
    retired: Vec<Retired>,
    swaps: u32,
    /// Swaps that found no inherit map for the running generation. Bookkeeping
    /// makes this impossible; it is counted so a regression is visible.
    blind_swaps: u32,
    /// A `repeat` pad's roll over the whole output.
    repeat: Repeat,
    /// Samples rendered since the player was made: the clock quantized pads
    /// are queued against.
    clock: u64,
    /// Quantized pad actions waiting for their line, with the sample it is.
    waiting: [Option<(u64, Deferred)>; MAX_WAITING],
    /// Per pad note (and trigger key, with the top bit set), the line its
    /// last press landed on (or will) and the division it was on, so a
    /// release is never earlier than the next one.
    pressed: [Option<(u64, u64)>; 256],
    /// Actions waiting for the start of a bar, by the bar's number.
    at_bar: [Option<(usize, Deferred)>; MAX_WAITING],
}

impl Default for LivePlayer {
    fn default() -> Self {
        Self::new()
    }
}

impl LivePlayer {
    pub fn new() -> Self {
        Self {
            engine: None,
            generation: 0,
            pending: None,
            fading: None,
            pending_blend: 0.0,
            blending: None,
            retired: Vec::with_capacity(8),
            swaps: 0,
            blind_swaps: 0,
            repeat: Repeat::new(),
            clock: 0,
            waiting: [None; MAX_WAITING],
            pressed: [None; 256],
            at_bar: [None; MAX_WAITING],
        }
    }

    pub fn generation(&self) -> Generation {
        self.generation
    }
    pub fn swaps(&self) -> u32 {
        self.swaps
    }
    pub fn blind_swaps(&self) -> u32 {
        self.blind_swaps
    }
    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }
    pub fn engine(&self) -> Option<&SongEngine> {
        self.engine.as_deref()
    }
    pub fn engine_mut(&mut self) -> Option<&mut SongEngine> {
        self.engine.as_deref_mut()
    }
    pub fn running(&self) -> bool {
        self.engine.as_ref().is_some_and(|e| e.running())
    }

    /// Engines the player is done with, for dropping elsewhere.
    pub fn take_retired(&mut self) -> Option<Retired> {
        self.retired.pop()
    }

    fn retire(&mut self, engine: Box<SongEngine>, maps: Vec<(Generation, Inherit)>) {
        self.discard(Spent::Engine(engine, maps));
    }

    fn discard(&mut self, spent: Spent) {
        if self.retired.len() < self.retired.capacity() {
            self.retired.push(Retired(spent));
        }
        // Otherwise it drops here. The caller is not draining `take_retired`.
    }

    pub fn apply(&mut self, plan: Plan) -> Applied {
        match plan {
            Plan::Unchanged => Applied::Unchanged,
            Plan::Fast { base, ops } => {
                let target = if self.generation == base {
                    self.engine.as_deref_mut()
                } else {
                    match self.pending.as_mut() {
                        Some((e, g, _)) if *g == base => Some(e.as_mut()),
                        _ => None,
                    }
                };
                let applied = match target {
                    Some(engine) => {
                        // A value from the text: it lets go of whatever a
                        // knob held on the same target, so the edit plays.
                        for op in &ops {
                            engine.release(*op);
                        }
                        Applied::Fast
                    }
                    None => Applied::Stale,
                };
                // The list was allocated by the planner; it is freed with the
                // retired engines, not here.
                self.discard(Spent::Ops(ops));
                applied
            }
            Plan::Control { base, op } => {
                let target = if self.generation == base {
                    self.engine.as_deref_mut()
                } else {
                    match self.pending.as_mut() {
                        Some((e, g, _)) if *g == base => Some(e.as_mut()),
                        _ => None,
                    }
                };
                // A knob: held, so a scene or a restart does not move it back.
                if let Some(engine) = target {
                    engine.hold(op);
                }
                Applied::Control
            }
            Plan::Play { base, op } => {
                if base == self.generation {
                    if let Some(engine) = self.engine.as_deref_mut() {
                        apply_op(engine, op);
                    }
                }
                Applied::Control
            }
            Plan::Quantized { grid, pad, down, inner } => {
                self.quantize(grid, pad, down, inner);
                Applied::Control
            }
            Plan::AtBar { bar, inner } => {
                match self.at_bar.iter_mut().find(|w| w.is_none()) {
                    Some(free) => *free = Some((bar, inner)),
                    None => self.apply_deferred(inner),
                }
                self.schedule_bars();
                Applied::Control
            }
            Plan::Repeat(Some(sixteenths)) => {
                if let Some(e) = self.engine.as_deref() {
                    let step = SAMPLE_RATE * 60.0 / e.tempo().max(1.0) / 4.0;
                    let wait = e.samples_until_grid(sixteenths);
                    if wait != usize::MAX {
                        self.repeat.press(wait, (step * sixteenths) as usize);
                    }
                }
                Applied::Control
            }
            Plan::Repeat(None) => {
                self.repeat.release();
                Applied::Control
            }
            Plan::Swap { engine, generation, inherit, blend_bars } => {
                if let Some((e, _, m)) = self.pending.take() {
                    self.retire(e, m);
                }
                if self.running() {
                    self.pending = Some((engine, generation, inherit));
                    self.pending_blend = blend_bars;
                    Applied::Queued
                } else {
                    if let Some(old) = self.engine.replace(engine) {
                        self.retire(old, inherit);
                    }
                    self.generation = generation;
                    Applied::Loaded
                }
            }
        }
    }

    /// Put a quantized pad action on its line. Press: the next line of the
    /// grid, or the one just passed if it is within the forgiveness window.
    /// Release: the same, but never before the line after its press, so a
    /// tap lasts a division and a release cannot overtake its press.
    fn quantize(&mut self, grid: Quantize, pad: u8, down: bool, inner: Deferred) {
        let Some(e) = self.engine.as_deref().filter(|e| e.running()) else {
            self.apply_deferred(inner);
            return;
        };
        let sixteenths = grid.sixteenths(e.steps_per_bar());
        let div = (SAMPLE_RATE * 60.0 / e.tempo().max(1.0) / 4.0 * sixteenths) as u64;
        let until = e.samples_until_grid(sixteenths) as u64;
        let since = div.saturating_sub(until);
        let (mut at, line) = if until == 0 {
            (self.clock, self.clock)
        } else if (since as f32) <= QUANTIZE_FORGIVENESS * div as f32 {
            (self.clock, self.clock.saturating_sub(since))
        } else {
            (self.clock + until, self.clock + until)
        };
        let slot = &mut self.pressed[pad as usize];
        if down {
            *slot = Some((line, div));
        } else if let Some((pressed, d)) = *slot {
            at = at.max(pressed + d);
        }
        if at <= self.clock {
            self.apply_deferred(inner);
            return;
        }
        match self.waiting.iter_mut().find(|w| w.is_none()) {
            Some(free) => *free = Some((at, inner)),
            // Full: it acts now rather than not at all.
            None => self.apply_deferred(inner),
        }
    }

    fn apply_deferred(&mut self, inner: Deferred) {
        let plan = match inner {
            Deferred::Control { base, op } => Plan::Control { base, op },
            Deferred::Play { base, op } => Plan::Play { base, op },
            Deferred::Set { base, op } => {
                let target = if self.generation == base {
                    self.engine.as_deref_mut()
                } else {
                    match self.pending.as_mut() {
                        Some((e, g, _)) if *g == base => Some(e.as_mut()),
                        _ => None,
                    }
                };
                if let Some(engine) = target {
                    engine.release(op);
                }
                return;
            }
        };
        self.apply(plan);
    }

    /// Move what waits for a bar onto the sample of its line once that line
    /// is the next one; a bar already begun acts at once. Counted where the
    /// playing engine counts, so a tempo ramp cannot put it off the line.
    fn schedule_bars(&mut self) {
        let Some(e) = self.engine.as_deref().filter(|e| e.running()) else {
            // Nothing is playing: there is no line to wait for.
            for i in 0..MAX_WAITING {
                if let Some((_, inner)) = self.at_bar[i].take() {
                    self.apply_deferred(inner);
                }
            }
            return;
        };
        let line = e.next_bar_line();
        let until = e.samples_until_grid(e.steps_per_bar() as f32) as u64;
        for i in 0..MAX_WAITING {
            let Some((bar, inner)) = self.at_bar[i] else { continue };
            if bar > line {
                continue;
            }
            self.at_bar[i] = None;
            let at = if bar < line { self.clock } else { self.clock + until };
            if at <= self.clock {
                self.apply_deferred(inner);
                continue;
            }
            match self.waiting.iter_mut().find(|w| w.is_none()) {
                Some(free) => *free = Some((at, inner)),
                None => self.apply_deferred(inner),
            }
        }
    }

    /// Apply every waiting action whose line has come, in the order queued.
    fn release_due(&mut self) {
        for i in 0..MAX_WAITING {
            if let Some((at, inner)) = self.waiting[i] {
                if at <= self.clock {
                    self.waiting[i] = None;
                    self.apply_deferred(inner);
                }
            }
        }
    }

    /// Quantized pad actions waiting for their line.
    pub fn waiting_pads(&self) -> usize {
        self.waiting.iter().filter(|w| w.is_some()).count()
    }

    pub fn start(&mut self) {
        if let Some(e) = self.engine.as_mut() {
            e.start();
        }
    }

    /// Stop. A queued engine takes over at once, with nothing to inherit.
    pub fn stop(&mut self) {
        self.repeat.state = RepeatState::Off;
        self.waiting = [None; MAX_WAITING];
        self.pressed = [None; 256];
        self.at_bar = [None; MAX_WAITING];
        if let Some(e) = self.engine.as_mut() {
            e.reset();
        }
        if let Some((f, _, m)) = self.fading.take() {
            self.retire(f, m);
        }
        if let Some(b) = self.blending.take() {
            self.retire(b.old, b.maps);
        }
        if let Some((e, g, m)) = self.pending.take() {
            if let Some(old) = self.engine.replace(e) {
                self.retire(old, m);
            }
            self.generation = g;
        }
    }

    /// Render one block of at most `BLOCK_SIZE` frames. Returns the bar a
    /// queued engine took over on, if it happened inside this block. A
    /// quantized pad whose line falls inside the block splits it there, so
    /// it acts on the sample of the line, not at the next block.
    pub fn process(&mut self, out_l: &mut [f32], out_r: &mut [f32]) -> Option<usize> {
        let len = out_l.len().min(out_r.len()).min(BLOCK_SIZE);
        let mut done = 0;
        let mut swapped = None;
        loop {
            if self.at_bar.iter().any(|w| w.is_some()) {
                self.schedule_bars();
            }
            self.release_due();
            if done >= len {
                break;
            }
            let next = self.waiting.iter().flatten().map(|(at, _)| (at - self.clock) as usize).min();
            let chunk = next.map_or(len - done, |n| n.clamp(1, len - done));
            let s = self.process_span(&mut out_l[done..done + chunk], &mut out_r[done..done + chunk]);
            swapped = swapped.or(s);
            self.clock += chunk as u64;
            done += chunk;
        }
        swapped
    }

    fn process_span(&mut self, out_l: &mut [f32], out_r: &mut [f32]) -> Option<usize> {
        let len = out_l.len().min(out_r.len()).min(BLOCK_SIZE);
        let (out_l, out_r) = (&mut out_l[..len], &mut out_r[..len]);
        let Some(engine) = self.engine.as_mut() else {
            out_l.fill(0.0);
            out_r.fill(0.0);
            return None;
        };

        let mut swapped = None;
        if self.pending.is_some() && engine.running() {
            let until = engine.samples_until_bar();
            if until < len {
                swapped = Some(self.swap_now(until));
            }
        }
        let engine = self.engine.as_mut().expect("engine present");
        engine.process_block_stereo(out_l, out_r);
        // What the previous engine still owns fades out on top.
        self.render_fade(out_l, out_r);
        self.render_blend(out_l, out_r);
        self.repeat.process(out_l, out_r);
        swapped
    }

    /// Mix the outgoing engine of a blend under what the new one rendered.
    fn render_blend(&mut self, out_l: &mut [f32], out_r: &mut [f32]) {
        let Some(b) = self.blending.as_mut() else { return };
        let len = out_l.len();
        let tempo = self.engine.as_ref().map_or(0.0, |e| e.tempo());
        if tempo > 0.0 && b.old.tempo() != tempo {
            b.old.set_tempo(tempo);
        }
        let mut ol = [0.0f32; BLOCK_SIZE];
        let mut or = [0.0f32; BLOCK_SIZE];
        b.old.process_block_stereo(&mut ol[..len], &mut or[..len]);
        let k = 1.0 - crate::math::exp(-core::f32::consts::TAU * BLEND_SPLIT_HZ / SAMPLE_RATE);
        let mid = b.len as f32 / 2.0;
        for i in 0..len {
            let pos = (b.done + i) as f32;
            let t = (pos / b.len as f32).min(1.0);
            // Linear, not equal power: two songs playing at full are
            // correlated on the beat, and an equal-power curve summed them
            // 3.5 dB hot, to the edge of clipping, through the middle.
            let (g_new, g_old) = (t, 1.0 - t);
            // The low end belongs to the old engine until the midpoint and
            // to the new one after it.
            let swap = ((pos - mid) / BASS_SWAP_SAMPLES + 0.5).clamp(0.0, 1.0);
            for (ch, (new, old)) in [(&mut out_l[i], ol[i]), (&mut out_r[i], or[i])].into_iter().enumerate() {
                let new_low = b.split_new.low(ch, *new, k);
                let old_low = b.split_old.low(ch, old, k);
                let (new_high, old_high) = (*new - new_low, old - old_low);
                *new = new_high * g_new + new_low * swap + old_high * g_old + old_low * (1.0 - swap);
            }
        }
        b.done += len;
        if b.done >= b.len {
            let b = self.blending.take().expect("blending");
            self.retire(b.old, b.maps);
        }
    }

    fn render_fade(&mut self, out_l: &mut [f32], out_r: &mut [f32]) {
        let Some((old, done, _)) = self.fading.as_mut() else { return };
        let len = out_l.len();
        if len > 0 {
            let mut tl = [0.0f32; BLOCK_SIZE];
            let mut tr = [0.0f32; BLOCK_SIZE];
            old.process_block_stereo(&mut tl[..len], &mut tr[..len]);
            for i in 0..len {
                let k = *done + i;
                if k >= CROSSFADE_SAMPLES {
                    break;
                }
                let g = 1.0 - k as f32 / CROSSFADE_SAMPLES as f32;
                out_l[i] += tl[i] * g;
                out_r[i] += tr[i] * g;
            }
            *done += len;
        }
        if *done >= CROSSFADE_SAMPLES {
            let (f, _, m) = self.fading.take().expect("fading present");
            self.retire(f, m);
        }
    }

    /// The bar line falls `until` samples into the block about to render.
    /// Hand over now; the new engine renders the block and crosses the line.
    fn swap_now(&mut self, until: usize) -> usize {
        let (mut new, generation, maps) = self.pending.take().expect("pending present");
        let mut old = self.engine.take().expect("engine present");
        let bar = old.bar_of_next_step();
        new.start_before_bar(bar);
        if let Some(b) = self.blending.take() {
            self.retire(b.old, b.maps);
        }
        let blend_bars = core::mem::take(&mut self.pending_blend);
        if blend_bars > 0.0 {
            let bar_samples = SAMPLE_RATE * 60.0 / new.tempo() * new.steps_per_bar() as f32 / 4.0;
            let len = (blend_bars * bar_samples) as usize;
            self.blending = Some(Blend {
                old,
                maps,
                done: 0,
                len: len.max(1),
                split_old: Split::default(),
                split_new: Split::default(),
            });
            self.engine = Some(new);
            self.generation = generation;
            self.swaps += 1;
            return bar;
        }
        match maps.iter().find(|(g, _)| *g == self.generation) {
            Some((_, map)) => new.inherit_from(&mut old, map, until),
            None => self.blind_swaps += 1,
        }
        old.coast();
        if let Some((f, _, m)) = self.fading.take() {
            self.retire(f, m);
        }
        self.fading = Some((old, 0, maps));
        self.engine = Some(new);
        self.generation = generation;
        self.swaps += 1;
        bar
    }
}

pub(crate) fn apply_op(engine: &mut SongEngine, op: FastOp) {
    match op {
        FastOp::Tempo(bpm) => engine.set_tempo(bpm),
        FastOp::Swing(s) => engine.set_swing(s),
        FastOp::Humanize { velocity, timing } => engine.set_humanize(velocity, timing),
        FastOp::NodeWet { track, node, wet } => {
            engine.set_node_wet(track, node, wet);
        }
        FastOp::TrackLevel { track, level } => engine.set_track_level(track, level),
        FastOp::TrackPan { track, pan } => engine.set_track_pan(track, pan),
        FastOp::TrackVelocity { track, velocity } => engine.set_track_velocity(track, velocity),
        FastOp::TrackGate { track, gate } => engine.set_track_gate(track, gate),
        FastOp::ModuleParam { instrument, id, value } => {
            engine.set_module_param_id(instrument, id, value);
        }
        FastOp::ReverbMix(mix) => engine.set_reverb_mix(mix),
        FastOp::DelayMix(mix) => engine.set_delay_mix(mix),
        FastOp::ReverbFreeze(on) => engine.set_reverb_freeze(on),
        FastOp::TrackSend { track, reverb, amount } => engine.set_track_send(track, reverb, amount),
        FastOp::TrackMute { track, muted } => engine.set_track_muted(track, muted),
        FastOp::TrackThrow { track, on } => engine.set_track_thrown(track, on),
        FastOp::FreezeHold(on) => engine.set_freeze_pad(on),
        FastOp::NodeParam { track, node, param, value } => {
            engine.set_node_param(track, node, param, value);
        }
        FastOp::NoteOn { track, note, velocity } => engine.live_note_on(track, note, velocity),
        FastOp::NoteOff { track, note } => engine.live_note_off(track, note),
        FastOp::PitchBend { instrument, ratio } => engine.set_pitch_bend(instrument, ratio),
        FastOp::Roll { track, note, velocity, kick } => {
            engine.set_roll(track, (velocity > 0.0).then_some((note, velocity)), kick)
        }
    }
}
