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

use crate::dsl::ast::{ChainNode, Song};
use crate::dsl::compiler::CompiledSong;
use crate::dsl::diff::{self, DslChange};
use crate::params::{self, ModuleKind, ParamId};
use crate::song_engine::{DslError, SongEngine};
use crate::BLOCK_SIZE;

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
    Humanize { velocity: f32, timing: f32 },
    TrackLevel { track: usize, level: f32 },
    TrackPan { track: usize, pan: f32 },
    TrackVelocity { track: usize, velocity: f32 },
    TrackGate { track: usize, gate: f32 },
    ModuleParam { instrument: usize, id: ParamId, value: f32 },
    /// Switch one node of a track's insert chain in or out, or anywhere
    /// between. The node is already built; only how much of it is heard moves.
    NodeWet { track: usize, node: usize, wet: f32 },
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
        }
    }

    pub fn is_complete(&self) -> bool {
        self.sends && self.master
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
    },
}

impl Plan {
    pub fn describe(&self) -> &'static str {
        match self {
            Plan::Unchanged => "unchanged",
            Plan::Fast { .. } => "fast",
            Plan::Swap { .. } => "swap",
        }
    }
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
}

impl Known {
    fn new(generation: Generation, ast: Song, engine: &SongEngine) -> Self {
        Self {
            generation,
            ast,
            instruments: (0..engine.instrument_count()).map(|i| String::from(engine.instrument_name(i))).collect(),
            track_instruments: (0..engine.track_count()).map(|i| engine.track_instrument(i).unwrap_or(usize::MAX)).collect(),
            tracks: (0..engine.track_count()).map(|i| String::from(engine.track_name(i))).collect(),
            buses: (0..engine.bus_count()).map(|i| String::from(engine.bus_name(i))).collect(),
        }
    }
}

/// Control-thread half. Owns the ASTs of what plays and what is queued.
pub struct LivePlanner {
    running: Option<Known>,
    pending: Option<Known>,
    next_generation: Generation,
}

impl Default for LivePlanner {
    fn default() -> Self { Self::new() }
}

impl LivePlanner {
    pub fn new() -> Self {
        Self { running: None, pending: None, next_generation: 1 }
    }

    /// Plan how to get the player from what it has to `source`. `playing` is
    /// the generation the player reports right now ([`LivePlayer::generation`]);
    /// it tells the planner whether a queued swap has landed.
    pub fn plan(&mut self, source: &str, playing: Generation) -> Result<Plan, DslError> {
        if self.pending.as_ref().is_some_and(|p| p.generation == playing) {
            self.running = self.pending.take();
        }
        let ast = crate::dsl::parse(source).map_err(DslError::Parse)?;
        let compiled = crate::dsl::compiler::compile(&ast).map_err(DslError::Compile)?;

        let latest = self.pending.as_mut().or(self.running.as_mut());
        if let Some(latest) = latest {
            if latest.ast == ast {
                return Ok(Plan::Unchanged);
            }
            let changes = diff::diff(&latest.ast, &ast);
            if !diff::has_structural_change(&changes) {
                if let Some(ops) = resolve_fast(&changes, &ast, &compiled, &latest.instruments) {
                    latest.ast = ast;
                    return Ok(Plan::Fast { base: latest.generation, ops });
                }
            }
        }

        let generation = self.next_generation;
        self.next_generation += 1;
        let engine = Box::new(SongEngine::from_compiled(compiled));
        let known = Known::new(generation, ast, &engine);
        let mut inherit = Vec::with_capacity(2);
        for old in self.running.iter().chain(self.pending.iter()) {
            inherit.push((old.generation, inherit_map(old, &known)));
        }
        if self.running.is_none() {
            self.running = Some(known);
        } else {
            self.pending = Some(known);
        }
        Ok(Plan::Swap { engine, generation, inherit })
    }
}

/// Resolve non-structural changes to indices in the new song. `None` when a
/// name does not resolve, which makes the caller swap instead. `instruments`
/// are the engine's names, one per copy: a module edit reaches every copy.
fn resolve_fast(changes: &[DslChange], ast: &Song, compiled: &CompiledSong, instruments: &[String]) -> Option<Vec<FastOp>> {
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
                if !any { return None; }
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

/// What the new song can take over from `old`, by name and definition.
fn inherit_map(old: &Known, known: &Known) -> Inherit {
    let o = &old.ast;
    let new = &known.ast;
    let sends = o.globals.send_delay == new.globals.send_delay
        && o.globals.send_reverb == new.globals.send_reverb
        && chain_of(o, "reverb_return") == chain_of(new, "reverb_return")
        && chain_of(o, "delay_return") == chain_of(new, "delay_return");
    let master = o.master == new.master;

    let buses = known.buses.iter().map(|name| {
        let j = old.buses.iter().position(|n| n == name)?;
        (chain_of(o, name) == chain_of(new, name)).then_some(j)
    }).collect::<Vec<_>>();

    let instruments = (0..known.instruments.len()).map(|i| {
        let j = nth_same(&old.instruments, &known.instruments, i)?;
        same_instrument(o, new, &known.instruments[i]).then_some(j)
    }).collect::<Vec<_>>();

    // A track continues only where everything that decides what it plays is
    // the same: its own definition, its pattern, its instrument, the scale and
    // meter the pattern compiles against, the grooves, and the scenes that may
    // override it.
    let composition_same = o.scenes == new.scenes
        && o.arrangement == new.arrangement
        && o.grooves == new.grooves
        && o.globals.scale == new.globals.scale
        && o.globals.meter == new.globals.meter;
    let tracks = known.tracks.iter().enumerate().map(|(i, name)| {
        if !composition_same { return None; }
        let j = old.tracks.iter().position(|n| n == name)?;
        let od = o.tracks.iter().find(|d| &d.name == name)?;
        let nd = new.tracks.iter().find(|d| &d.name == name)?;
        if od != nd { return None; }
        let op = o.patterns.iter().find(|p| p.name == nd.play)?;
        let np = new.patterns.iter().find(|p| p.name == nd.play)?;
        if op != np { return None; }
        // The copy this track plays must be the one inherited from the copy
        // the old track played.
        let inst = *known.track_instruments.get(i)?;
        let old_inst = *old.track_instruments.get(j)?;
        (instruments.get(inst).copied().flatten() == Some(old_inst)).then_some(j)
    }).collect::<Vec<_>>();

    Inherit { sends, master, buses, instruments, tracks }
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
}

/// The inherit maps an engine carries, each tagged with the generation it was
/// built for.
type InheritMaps = Vec<(Generation, Inherit)>;

/// An engine the player is done with. Drop it off the audio thread.
pub struct Retired {
    pub engine: Box<SongEngine>,
    /// Never read: held so its memory is freed with the engine, off-thread.
    #[allow(dead_code)]
    maps: InheritMaps,
}

/// Audio-thread half. Nothing in here allocates once constructed, except
/// `apply` when handed more retired engines than it has room for, which then
/// drops one in place rather than lose it.
pub struct LivePlayer {
    engine: Option<Box<SongEngine>>,
    generation: Generation,
    pending: Option<(Box<SongEngine>, Generation, InheritMaps)>,
    /// The engine that just handed over, how far its fade-out has run, and
    /// the inherit maps of that handover, which retire with it.
    fading: Option<(Box<SongEngine>, usize, InheritMaps)>,
    retired: Vec<Retired>,
    swaps: u32,
    /// Swaps that found no inherit map for the running generation. Bookkeeping
    /// makes this impossible; it is counted so a regression is visible.
    blind_swaps: u32,
}

impl Default for LivePlayer {
    fn default() -> Self { Self::new() }
}

impl LivePlayer {
    pub fn new() -> Self {
        Self {
            engine: None,
            generation: 0,
            pending: None,
            fading: None,
            retired: Vec::with_capacity(8),
            swaps: 0,
            blind_swaps: 0,
        }
    }

    pub fn generation(&self) -> Generation { self.generation }
    pub fn swaps(&self) -> u32 { self.swaps }
    pub fn blind_swaps(&self) -> u32 { self.blind_swaps }
    pub fn has_pending(&self) -> bool { self.pending.is_some() }
    pub fn engine(&self) -> Option<&SongEngine> { self.engine.as_deref() }
    pub fn engine_mut(&mut self) -> Option<&mut SongEngine> { self.engine.as_deref_mut() }
    pub fn running(&self) -> bool { self.engine.as_ref().is_some_and(|e| e.running()) }

    /// Engines the player is done with, for dropping elsewhere.
    pub fn take_retired(&mut self) -> Option<Retired> { self.retired.pop() }

    fn retire(&mut self, engine: Box<SongEngine>, maps: Vec<(Generation, Inherit)>) {
        if self.retired.len() < self.retired.capacity() {
            self.retired.push(Retired { engine, maps });
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
                match target {
                    Some(engine) => {
                        for op in &ops { apply_op(engine, *op); }
                        Applied::Fast
                    }
                    None => Applied::Stale,
                }
            }
            Plan::Swap { engine, generation, inherit } => {
                if let Some((e, _, m)) = self.pending.take() {
                    self.retire(e, m);
                }
                if self.running() {
                    self.pending = Some((engine, generation, inherit));
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

    pub fn start(&mut self) {
        if let Some(e) = self.engine.as_mut() { e.start(); }
    }

    /// Stop. A queued engine takes over at once, with nothing to inherit.
    pub fn stop(&mut self) {
        if let Some(e) = self.engine.as_mut() { e.reset(); }
        if let Some((f, _, m)) = self.fading.take() { self.retire(f, m); }
        if let Some((e, g, m)) = self.pending.take() {
            if let Some(old) = self.engine.replace(e) { self.retire(old, m); }
            self.generation = g;
        }
    }

    /// Render one block of at most `BLOCK_SIZE` frames. Returns the bar a
    /// queued engine took over on, if it happened inside this block.
    pub fn process(&mut self, out_l: &mut [f32], out_r: &mut [f32]) -> Option<usize> {
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
        swapped
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
                if k >= CROSSFADE_SAMPLES { break; }
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

fn apply_op(engine: &mut SongEngine, op: FastOp) {
    match op {
        FastOp::Tempo(bpm) => engine.set_tempo(bpm),
        FastOp::Swing(s) => engine.set_swing(s),
        FastOp::Humanize { velocity, timing } => engine.set_humanize(velocity, timing),
        FastOp::NodeWet { track, node, wet } => { engine.set_node_wet(track, node, wet); }
        FastOp::TrackLevel { track, level } => engine.set_track_level(track, level),
        FastOp::TrackPan { track, pan } => engine.set_track_pan(track, pan),
        FastOp::TrackVelocity { track, velocity } => engine.set_track_velocity(track, velocity),
        FastOp::TrackGate { track, gate } => engine.set_track_gate(track, gate),
        FastOp::ModuleParam { instrument, id, value } => {
            engine.set_module_param_id(instrument, id, value);
        }
    }
}
