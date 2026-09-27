extern crate alloc;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;

use crate::dsl::ast::*;
use crate::dsl::error::{CompileError, CompileResult};
use crate::params::{self, ModuleKind};
use crate::math;
use crate::graph::{GraphBuilder, GraphError, GraphTemplate};
use crate::graph::node::{ChainStep, NodeSpec};
use crate::primitives::oscillator::Waveform;
use crate::primitives::filter::FilterType;

// ── Compiled output types ──

/// Per-step parameter lock data (compiled, Copy-able).
#[derive(Clone, Copy, Debug, Default)]
pub struct StepPLock {
    pub cutoff: Option<f32>,
    pub env_depth: Option<f32>,
    pub resonance: Option<f32>,
    pub gate: Option<f32>,
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
    NoteOn { midi_note: u8, velocity: f32, plock: StepPLock, slide: bool },
    Chord { notes: [ChordNote; MAX_CHORD_NOTES], count: u8, plock: StepPLock },
    /// `<B4 C#5 D5>`: notes played in sequence inside one step, evenly spaced
    /// across whatever that step's real duration turns out to be, so swing and
    /// humanize carry through instead of being bypassed.
    Subdiv { notes: [SubNote; MAX_SUBDIV], count: u8, plock: StepPLock },
    DrumHit { velocity: f32, probability: f32, roll: u8, plock: StepPLock },
    Rest,
    Tie,
}

/// A compiled drum lane: MIDI note + step sequence for one drum.
#[derive(Clone, Debug)]
pub struct CompiledLane {
    pub midi_note: u8,
    pub steps: Vec<CompiledStep>,
    pub swing_override: Option<f32>,  // per-lane swing from groove block
    pub nudge: f32,                   // per-lane timing offset from groove block
}

/// A compiled pattern: flat array of steps.
#[derive(Clone, Debug)]
pub struct CompiledPattern {
    pub name: String,
    pub steps: Vec<CompiledStep>,
    pub steps_per_row: usize,
    pub lanes: Vec<CompiledLane>,  // empty = sequential, non-empty = parallel drum pattern
}

/// A compiled track.
#[derive(Clone, Debug)]
pub struct CompiledTrack {
    pub name: String,
    pub instrument_idx: usize,
    pub pattern_idx: usize,
    pub velocity: f32,
    pub level: f32,         // output level 0.0-1.0 (default 0.8)
    pub pan: f32,           // stereo pan -1.0 (L) to 1.0 (R), 0.0 = center
    pub gate: f32,          // gate length as fraction of step (default 0.85)
    pub insert_fx: Vec<ChainStep>,
    /// `as <name>` per insert node, so `auto <track>.<name> wet` can find it.
    /// Parallel to `insert_fx`; `None` where a node was left unnamed.
    pub insert_fx_labels: Vec<Option<String>>,
    pub bus_send: Option<(usize, f32)>, // (bus_idx, amount)
    pub to_master: bool,
    pub delay_send: f32,    // global delay send amount 0.0-1.0
    pub reverb_send: f32,   // global reverb send amount 0.0-1.0
    pub sidechain: Option<f32>, // per-track sidechain override (None = use global)
    /// Track whose level ducks this one. `None` = whatever the song's global
    /// source is, which is the kick unless `sidechain ... from=` says otherwise.
    pub sidechain_source: Option<String>,
    pub arp: Option<ArpConfig>, // arpeggiator driven by the pattern's held notes
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
}

/// Master FX chain.
#[derive(Clone, Debug)]
pub struct CompiledMaster {
    pub fx_chain: Vec<ChainStep>,
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
    pub arrangement: Vec<(usize, u32)>, // (scene_idx, repeat_count)
    pub grooves: Vec<CompiledGroove>,
    /// Insert chains on the global send returns: `reverb_return { in > ... > out }`.
    pub reverb_return: Vec<ChainStep>,
    pub delay_return: Vec<ChainStep>,
}

// ── Compiler ──

pub fn compile(song: &Song) -> CompileResult<CompiledSong> {
    let mut errors = Vec::new();

    // A `capture` window is sized in samples at compile time so the buffer can
    // be allocated when the chain is built. Use the slowest tempo the song ever
    // reaches, since a scene can override it and a slower bar is a longer one.
    let slowest_tempo = song.scenes.iter()
        .filter_map(|s| s.tempo)
        .fold(song.globals.tempo, f32::min)
        .max(20.0);
    let samples_per_bar = crate::SAMPLE_RATE * 60.0 / slowest_tempo * song.globals.meter.0 as f32;

    // 1. Compile graph instruments
    let mut instruments: Vec<CompiledInstrumentKind> = Vec::new();
    let mut instrument_names = Vec::new();
    for inst_def in &song.instruments {
        match compile_instrument(inst_def, samples_per_bar) {
            Ok(template) => {
                instruments.push(CompiledInstrumentKind::Graph(Box::new(template)));
                instrument_names.push(inst_def.name.clone());
            }
            Err(e) => {
                errors.push(e);
                // Keep the name, as for modules below: one broken instrument
                // is one error, not one more for every track that plays it.
                instruments.push(CompiledInstrumentKind::Bass(ModulePreset::default()));
                instrument_names.push(inst_def.name.clone());
            }
        }
    }

    // 1b. Compile module instruments
    for mod_def in &song.module_defs {
        match compile_module_def(mod_def) {
            Ok(kind) => {
                instruments.push(kind);
                instrument_names.push(mod_def.name.clone());
            }
            Err(errs) => {
                errors.extend(errs);
                // Keep the name registered so tracks and scenes that reference
                // this module do not produce a cascade of "unknown instrument" errors.
                instruments.push(CompiledInstrumentKind::Bass(ModulePreset::default()));
                instrument_names.push(mod_def.name.clone());
            }
        }
    }

    // 1c. Validate automation targets against the registry
    errors.extend(validate_automations(song));
    errors.extend(validate_midi(song));

    // 2. Compile patterns (with scale context for degree resolution)
    let (intervals, root_pc) = scale_context(song);
    let mut patterns = Vec::new();
    for pat_def in &song.patterns {
        match compile_pattern(pat_def, &intervals, root_pc) {
            Ok(p) => patterns.push(p),
            Err(e) => errors.push(e),
        }
    }

    // 3. Compile buses
    let mut buses = Vec::new();
    for bus_def in &song.buses {
        let chain = match song.bus_chains.iter().find(|bc| bc.bus_name == bus_def.name) {
            Some(bc) => match compile_fx_chain(&format!("bus '{}'", bus_def.name), &bc.chain, samples_per_bar) {
                Ok(c) => c,
                Err(e) => { errors.push(e); Vec::new() }
            },
            None => Vec::new(),
        };
        buses.push(CompiledBus { name: bus_def.name.clone(), fx_chain: chain });
    }

    // 3b. Return chains on the global sends, and chains that belong to nothing
    let mut reverb_return = Vec::new();
    let mut delay_return = Vec::new();
    for bc in &song.bus_chains {
        match bc.bus_name.as_str() {
            "reverb_return" => match compile_fx_chain("reverb_return", &bc.chain, samples_per_bar) {
                Ok(c) => reverb_return = c,
                Err(e) => errors.push(e),
            },
            "delay_return" => match compile_fx_chain("delay_return", &bc.chain, samples_per_bar) {
                Ok(c) => delay_return = c,
                Err(e) => errors.push(e),
            },
            name if !song.buses.iter().any(|b| b.name == name) => errors.push(CompileError::new(format!(
                "chain '{}' has no `bus {}` declaration (or use reverb_return / delay_return for the global sends)", name, name
            ))),
            // Names that do match a declared bus were already compiled in step 3.
            _ => {}
        }
    }

    // 4. Compile tracks
    let mut tracks = Vec::new();
    for track_def in &song.tracks {
        match compile_track(track_def, &instrument_names, &patterns, &buses, None, samples_per_bar) {
            Ok(t) => tracks.push(t),
            Err(e) => errors.push(e),
        }
    }

    // 4b. A sidechain source has to name something that exists, and a track
    // cannot duck against itself. Getting this wrong used to be impossible
    // because there was nothing to get wrong: everything ducked the kick.
    {
        let known = |name: &str| {
            song.tracks.iter().any(|t| t.name == name)
                || instrument_names.iter().any(|n| n == name)
        };
        let mut check = |amount: Option<f32>, source: &Option<String>, owner: &str| {
            let Some(name) = source else { return };
            if !known(name) {
                errors.push(CompileError::new(format!(
                    "{}: sidechain from='{}' names no track or module", owner, name
                )));
            } else if owner == name {
                errors.push(CompileError::new(format!(
                    "{}: sidechain from='{}' would duck the track against itself", owner, name
                )));
            } else if amount.unwrap_or(song.globals.sidechain) <= 0.0 {
                errors.push(CompileError::new(format!(
                    "{}: sidechain from='{}' has no amount, so it does nothing. Write `sidechain 0.4 from={}`",
                    owner, name, name
                )));
            }
        };
        check(Some(song.globals.sidechain), &song.globals.sidechain_source, "song");
        for t in &song.tracks {
            check(t.sidechain, &t.sidechain_source, &t.name);
        }
        for scene in &song.scenes {
            for t in &scene.tracks {
                check(t.sidechain, &t.sidechain_source, &t.name);
            }
        }
    }

    // 5. Compile master. A `limiter` in it is left out: the engine's output
    // stage limits every song on its true peak, after it and after the gain
    // that levels the song, so one here could only limit a level that is
    // about to change.
    let master = match &song.master {
        Some(m) => match compile_fx_chain("master", &without_limiter(&m.chain), samples_per_bar) {
            Ok(c) => CompiledMaster { fx_chain: c },
            Err(e) => { errors.push(e); CompiledMaster { fx_chain: Vec::new() } }
        },
        None => CompiledMaster { fx_chain: Vec::new() },
    };

    // 6. Compile scenes
    let mut scenes = Vec::new();
    for scene_def in &song.scenes {
        match compile_scene(scene_def, &instrument_names, &patterns, &buses, &tracks, samples_per_bar) {
            Ok(s) => scenes.push(s),
            Err(e) => errors.push(e),
        }
    }

    // 7. Compile arrangement
    let scene_names: Vec<&str> = scenes.iter().map(|s| s.name.as_str()).collect();
    let mut arrangement = Vec::new();
    for entry in &song.arrangement {
        if let Some(idx) = scene_names.iter().position(|&n| n == entry.scene_name) {
            arrangement.push((idx, entry.repeat));
        } else {
            errors.push(CompileError { line: 0,
                message: format!("unknown scene '{}' in arrangement", entry.scene_name),
            });
        }
    }

    if !errors.is_empty() {
        return Err(errors);
    }

    // Compile groove blocks
    let grooves: Vec<CompiledGroove> = song.grooves.iter().map(|g| {
        let lanes = g.lanes.iter().map(|l| {
            CompiledGrooveLane {
                midi_note: drum_name_to_midi(&l.drum_name),
                swing_override: l.swing,
                nudge: l.nudge.unwrap_or(0.0),
            }
        }).collect();
        CompiledGroove { name: g.name.clone(), lanes }
    }).collect();

    // Apply first groove block to all drum patterns (simple model: one active groove)
    if let Some(groove) = grooves.first() {
        for pat in &mut patterns {
            for lane in &mut pat.lanes {
                for gl in &groove.lanes {
                    if gl.midi_note == lane.midi_note {
                        lane.swing_override = gl.swing_override;
                        lane.nudge = gl.nudge;
                    }
                }
            }
        }
    }

    Ok(CompiledSong {
        globals: song.globals.clone(),
        instruments,
        instrument_names,
        patterns,
        tracks,
        buses,
        master,
        scenes,
        arrangement,
        grooves,
        reverb_return,
        delay_return,
    })
}

// ── Instrument compilation ──

fn compile_instrument(inst: &InstrumentDef, samples_per_bar: f32) -> Result<GraphTemplate, CompileError> {
    let mut builder = GraphBuilder::new();
    let mut name_to_idx: Vec<(String, u8)> = Vec::new();
    let mut noise_seed = 42u32;
    let mut osc_drift_seed = 1000u32; // unique drift seed per oscillator
    let full = |line: Line| CompileError::at(line.0, format!(
        "instrument '{}': more than {} nodes (an envelope counts twice, it brings its own VCA); split it into two instruments, or share a filter through `mix`",
        inst.name, crate::graph::MAX_GRAPH_NODES
    ));

    // First pass: ensure built-in nodes exist
    // "out" and "mix" are always available as implicit nodes
    let mut has_out = false;
    let mut has_mix = false;

    for node_def in &inst.nodes {
        if node_def.alias.as_deref() == Some("out") || node_def.kind == "out" {
            has_out = true;
        }
        if node_def.alias.as_deref() == Some("mix") || node_def.kind == "mix" {
            has_mix = true;
        }
    }
    for conn in &inst.connections {
        if conn.to == "out" && !has_out {
            has_out = true;
        }
        if (conn.to == "mix" || conn.from == "mix") && !has_mix {
            has_mix = true;
        }
    }

    // Create implicit "mix" node if referenced
    if has_mix {
        let idx = builder.try_add_node(NodeSpec::Mix).map_err(|_| full(inst.line))?;
        name_to_idx.push((String::from("mix"), idx));
    }

    // Create explicit nodes
    for node_def in &inst.nodes {
        let spec = node_def_to_spec(node_def, &mut noise_seed, &mut osc_drift_seed, samples_per_bar)
            .map_err(|e| if e.line == 0 { CompileError::at(node_def.line.0, e.message) } else { e })?;

        if matches!(spec, NodeSpec::Env { .. }) {
            // Envelope+VCA pattern: an envelope in a signal chain means
            // "multiply audio by envelope". We create two nodes:
            //   1. Env node (generates the 0-1 envelope curve)
            //   2. VCA node (multiplies: input[0] * input[1])
            // The alias (e.g., "amp") points to the VCA so that:
            //   - `filter > amp` connects filter → VCA input 1 (audio)
            //   - Env is auto-wired to VCA input 0 (envelope curve)
            //   - VCA output = envelope * audio
            let env_idx = builder.try_add_node(spec).map_err(|_| full(node_def.line))?;
            let vca_idx = builder.try_add_node(NodeSpec::Vca).map_err(|_| full(node_def.line))?;
            builder.connect(env_idx, vca_idx); // env → VCA input 0, its first
            if let Some(ref alias) = node_def.alias {
                name_to_idx.push((alias.clone(), vca_idx));
            }
        } else {
            let idx = builder.try_add_node(spec).map_err(|_| full(node_def.line))?;
            if let Some(ref alias) = node_def.alias {
                name_to_idx.push((alias.clone(), idx));
            }
        }
    }

    // Create "out" as Output node
    let out_idx = builder.try_add_node(NodeSpec::Output).map_err(|_| full(inst.line))?;
    name_to_idx.push((String::from("out"), out_idx));

    // Second pass: create connections
    for conn in &inst.connections {
        let from_idx = find_node(&name_to_idx, &conn.from)
            .ok_or_else(|| CompileError::at(conn.line.0,
                format!("instrument '{}': unknown node '{}'", inst.name, conn.from),
            ))?;
        let to_idx = find_node(&name_to_idx, &conn.to)
            .ok_or_else(|| CompileError::at(conn.line.0,
                format!("instrument '{}': unknown node '{}'", inst.name, conn.to),
            ))?;
        builder.try_connect(from_idx, to_idx).map_err(|_| CompileError::at(conn.line.0, format!(
            "instrument '{}': more than {} connections into '{}'; gather them in a `mix` first",
            inst.name, crate::graph::node::MAX_NODE_INPUTS, conn.to
        )))?;
    }

    let mut template = builder.try_build().map_err(|e| match e {
        GraphError::Cycle { stuck } => {
            // `stuck` is the loop and everything downstream of it. Peel off
            // the nodes that feed no other stuck node until only the loop is
            // left, so the message names the nodes to rewire and not `out`.
            let edges: Vec<(u8, u8)> = inst.connections.iter()
                .filter_map(|c| Some((find_node(&name_to_idx, &c.from)?, find_node(&name_to_idx, &c.to)?)))
                .collect();
            let mut stuck = stuck;
            loop {
                let tail = (0..32u8).find(|&i| stuck & (1 << i) != 0
                    && !edges.iter().any(|&(a, b)| a == i && stuck & (1 << b) != 0));
                match tail {
                    Some(i) => stuck &= !(1 << i),
                    None => break,
                }
            }
            let names: Vec<&str> = name_to_idx.iter()
                .filter(|(n, i)| stuck & (1 << i) != 0 && !n.starts_with("_anon_"))
                .map(|(n, _)| n.as_str())
                .collect();
            // The first connection between two stuck nodes is where the loop
            // shows in the file.
            let line = inst.connections.iter()
                .find(|c| [&c.from, &c.to].iter().all(|n| find_node(&name_to_idx, n).is_some_and(|i| stuck & (1 << i) != 0)))
                .map_or(inst.line, |c| c.line);
            CompileError::at(line.0, format!(
                "instrument '{}': the connections loop back on themselves ({} feed each other); a node cannot take its own output as input",
                inst.name, names.join(", ")
            ))
        }
        _ => CompileError::at(inst.line.0, format!("instrument '{}': {:?}", inst.name, e)),
    })?;
    template.output_gain = inst.gain.unwrap_or(1.0);
    Ok(template)
}

fn node_def_to_spec(node: &NodeDef, noise_seed: &mut u32, osc_drift_seed: &mut u32, samples_per_bar: f32) -> Result<NodeSpec, CompileError> {
    let problems = crate::nodes::validate(&node.kind, &node.params);
    if !problems.is_empty() {
        return Err(CompileError::new(problems.join("; ")));
    }
    match node.kind.as_str() {
        "osc" => {
            let waveform = node.params.iter()
                .find_map(|p| if let Param::Waveform(w) = p { Some(w.as_str()) } else { None })
                .unwrap_or("sine");
            let wf = match waveform {
                "sine" => Waveform::Sine,
                "saw" => Waveform::Saw,
                "square" => Waveform::Square,
                "triangle" => Waveform::Triangle,
                _ => Waveform::Sine,
            };
            let freq = first_float_param(&node.params).unwrap_or(440.0);
            let seed = *osc_drift_seed;
            *osc_drift_seed += 1;
            let pitch_semitones = named_param(&node.params, "pitch").unwrap_or(0.0);
            Ok(NodeSpec::Osc { waveform: wf, freq, drift_seed: seed, fixed: false, pitch_semitones })
        }
        "fixosc" => {
            let waveform = node.params.iter()
                .find_map(|p| if let Param::Waveform(w) = p { Some(w.as_str()) } else { None })
                .unwrap_or("sine");
            let wf = match waveform {
                "sine" => Waveform::Sine,
                "saw" => Waveform::Saw,
                "square" => Waveform::Square,
                "triangle" => Waveform::Triangle,
                _ => Waveform::Sine,
            };
            let freq = first_float_param(&node.params).unwrap_or(440.0);
            let seed = *osc_drift_seed;
            *osc_drift_seed += 1;
            Ok(NodeSpec::Osc { waveform: wf, freq, drift_seed: seed, fixed: true, pitch_semitones: 0.0 })
        }
        "pitch_osc" => {
            let waveform = node.params.iter()
                .find_map(|p| if let Param::Waveform(w) = p { Some(w.as_str()) } else { None })
                .unwrap_or("sine");
            let wf = match waveform {
                "sine" => Waveform::Sine,
                "saw" => Waveform::Saw,
                "square" => Waveform::Square,
                "triangle" => Waveform::Triangle,
                _ => Waveform::Sine,
            };
            let start_freq = float_param_at(&node.params, 0).unwrap_or(300.0);
            let end_freq = float_param_at(&node.params, 1).unwrap_or(55.0);
            let decay = named_param(&node.params, "decay")
                .or_else(|| float_param_at(&node.params, 2))
                .unwrap_or(0.995);
            Ok(NodeSpec::PitchOsc { waveform: wf, start_freq, end_freq, decay })
        }
        "noise" => {
            *noise_seed += 1;
            Ok(NodeSpec::Noise { seed: *noise_seed })
        }
        "lfo" => {
            let rate = float_param_at(&node.params, 0).unwrap_or(1.0);
            let depth = float_param_at(&node.params, 1).unwrap_or(0.5);
            Ok(NodeSpec::Lfo { rate, depth })
        }
        "adsr" => {
            let a = float_param_at(&node.params, 0).unwrap_or(0.01);
            let d = float_param_at(&node.params, 1).unwrap_or(0.1);
            let s = float_param_at(&node.params, 2).unwrap_or(0.7);
            let r = float_param_at(&node.params, 3).unwrap_or(0.3);
            Ok(NodeSpec::Env { a, d, s, r })
        }
        "perc" => {
            let a = float_param_at(&node.params, 0).unwrap_or(0.001);
            let d = float_param_at(&node.params, 1).unwrap_or(0.3);
            Ok(NodeSpec::Env { a, d, s: 0.0, r: 0.01 })
        }
        "lowpass" => {
            let cutoff = float_param_at(&node.params, 0).unwrap_or(1000.0);
            let res = float_param_at(&node.params, 1).unwrap_or(0.01);
            let (ea, ed, es, er, edepth) = filter_env_params(&node.params);
            Ok(NodeSpec::Biquad { filter_type: FilterType::LowPass, cutoff, resonance: res,
                env_attack: ea, env_decay: ed, env_sustain: es, env_release: er, env_depth: edepth,
                lfo: filter_lfo_params(&node.params) })
        }
        "highpass" => {
            let cutoff = float_param_at(&node.params, 0).unwrap_or(1000.0);
            let res = float_param_at(&node.params, 1).unwrap_or(0.01);
            let (ea, ed, es, er, edepth) = filter_env_params(&node.params);
            Ok(NodeSpec::Biquad { filter_type: FilterType::HighPass, cutoff, resonance: res,
                env_attack: ea, env_decay: ed, env_sustain: es, env_release: er, env_depth: edepth,
                lfo: filter_lfo_params(&node.params) })
        }
        "bandpass" => {
            let cutoff = float_param_at(&node.params, 0).unwrap_or(1000.0);
            let res = float_param_at(&node.params, 1).unwrap_or(0.5);
            let (ea, ed, es, er, edepth) = filter_env_params(&node.params);
            Ok(NodeSpec::Biquad { filter_type: FilterType::BandPass, cutoff, resonance: res,
                env_attack: ea, env_decay: ed, env_sustain: es, env_release: er, env_depth: edepth,
                lfo: filter_lfo_params(&node.params) })
        }
        "ladder" => {
            let cutoff = float_param_at(&node.params, 0).unwrap_or(1000.0);
            let res = float_param_at(&node.params, 1).unwrap_or(0.5);
            let (ea, ed, es, er, edepth) = filter_env_params(&node.params);
            Ok(NodeSpec::Ladder { cutoff, resonance: res,
                env_attack: ea, env_decay: ed, env_sustain: es, env_release: er, env_depth: edepth,
                lfo: filter_lfo_params(&node.params) })
        }
        "mix" => Ok(NodeSpec::Mix),
        "gain" => {
            let amount = float_param_at(&node.params, 0).unwrap_or(1.0);
            Ok(NodeSpec::Gain { amount })
        }
        "saturate" | "drive" => {
            let drive = float_param_at(&node.params, 0).unwrap_or(1.0);
            Ok(NodeSpec::Saturator { drive })
        }
        "chorus" => {
            let mix = float_param_at(&node.params, 0).unwrap_or(0.3);
            Ok(NodeSpec::Chorus { mix })
        }
        "bitcrush" => {
            let bits = float_param_at(&node.params, 0).unwrap_or(8.0);
            let rate = float_param_at(&node.params, 1).unwrap_or(0.0);
            Ok(NodeSpec::Bitcrusher { bits, rate })
        }
        "compressor" => {
            let threshold = float_param_at(&node.params, 0).unwrap_or(-3.0);
            let ratio = named_param(&node.params, "ratio").unwrap_or(3.0);
            let attack_ms = named_param(&node.params, "attack").unwrap_or(20.0);
            let release_ms = named_param(&node.params, "release").unwrap_or(100.0);
            let makeup = named_param(&node.params, "makeup").unwrap_or(1.0);
            Ok(NodeSpec::Compressor { threshold_db: threshold, ratio, attack_ms, release_ms, makeup })
        }
        "limiter" => {
            let threshold = float_param_at(&node.params, 0).unwrap_or(0.95);
            Ok(NodeSpec::Limiter { threshold })
        }
        "tilt" => {
            let amount = float_param_at(&node.params, 0).unwrap_or(0.0);
            Ok(NodeSpec::TiltEq { amount })
        }
        "eq" => {
            let low = named_param(&node.params, "low").unwrap_or(0.0);
            let mid = named_param(&node.params, "mid").unwrap_or(0.0);
            let high = named_param(&node.params, "high").unwrap_or(0.0);
            Ok(NodeSpec::ThreeBandEq { low, mid, high })
        }
        "delay" => {
            let sync_div = rhythm_div_param(&node.params).unwrap_or(0.25);
            // feedback is the first float param (rhythm div is parsed separately)
            let feedback = float_param_at(&node.params, 0).unwrap_or(0.3);
            Ok(NodeSpec::Delay { sync_div, feedback })
        }
        "reverb" => {
            let room_size = float_param_at(&node.params, 0).unwrap_or(0.5);
            Ok(NodeSpec::Reverb { room_size })
        }
        "phaser" => {
            let mix = float_param_at(&node.params, 0).unwrap_or(0.5);
            let bars = named_param(&node.params, "bars").unwrap_or(0.0);
            let hz = named_param(&node.params, "hz").unwrap_or(if bars > 0.0 { 0.0 } else { 0.3 });
            let stages = named_param(&node.params, "stages").unwrap_or(6.0) as u8;
            let feedback = named_param(&node.params, "feedback").unwrap_or(0.4);
            let depth = named_param(&node.params, "depth").unwrap_or(1.0);
            Ok(NodeSpec::Phaser { mix, hz, bars, stages, feedback, depth })
        }
        "vowel" => {
            let words: Vec<&str> = node.params.iter()
                .filter_map(|p| if let Param::Waveform(w) = p { Some(w.as_str()) } else { None })
                .collect();
            let mut idx = Vec::new();
            for w in &words {
                match crate::effects::formant::vowel_index(w) {
                    Some(i) => idx.push(i),
                    None => return Err(CompileError::new(format!("vowel: '{}' is not a vowel (a, e, i, o, u)", w))),
                }
            }
            if idx.is_empty() {
                return Err(CompileError::new(String::from("vowel: needs one or two vowels, e.g. vowel(a, o, bars=2)")));
            }
            let from = idx[0];
            let to = *idx.get(1).unwrap_or(&from);
            let bars = named_param(&node.params, "bars").unwrap_or(0.0);
            let hz = named_param(&node.params, "hz").unwrap_or(if bars > 0.0 { 0.0 } else { 0.25 });
            let mix = named_param(&node.params, "mix").unwrap_or(1.0);
            Ok(NodeSpec::Vowel { from, to, hz, bars, mix })
        }
        "capture" => {
            let bars = float_param_at(&node.params, 0)
                .or_else(|| named_param(&node.params, "bars"))
                .unwrap_or(2.0);
            let start = named_param(&node.params, "start").unwrap_or(0.0);
            let speed = named_param(&node.params, "speed").unwrap_or(1.0);
            let reverse = named_param(&node.params, "reverse").unwrap_or(0.0) >= 0.5;
            let mix = named_param(&node.params, "mix").unwrap_or(1.0);
            Ok(NodeSpec::Capture {
                samples: (bars * samples_per_bar) as u32,
                start_samples: (start * samples_per_bar) as u32,
                speed,
                reverse,
                mix,
            })
        }
        "autowah" | "envfilter" => {
            let sens = float_param_at(&node.params, 0).unwrap_or(0.7);
            let base = named_param(&node.params, "base").unwrap_or(300.0);
            let range = named_param(&node.params, "range").unwrap_or(2200.0);
            let q = named_param(&node.params, "peak").unwrap_or(0.75);
            let attack_ms = named_param(&node.params, "attack").unwrap_or(8.0);
            let release_ms = named_param(&node.params, "release").unwrap_or(200.0);
            let down = named_param(&node.params, "down").unwrap_or(0.0) > 0.5;
            let wobble = named_param(&node.params, "wobble").unwrap_or(0.0);
            let wobble_hz = named_param(&node.params, "wobble_hz").unwrap_or(5.0);
            let mut mode = 0u8;
            for p in node.params.iter() {
                if let Param::Waveform(w) = p {
                    mode = match w.as_str() {
                        "lowpass" | "lp" => 0,
                        "bandpass" | "bp" => 1,
                        "highpass" | "hp" => 2,
                        other => return Err(CompileError::new(format!(
                            "autowah: '{}' is not a mode (lowpass, bandpass, highpass)", other))),
                    };
                }
            }
            Ok(NodeSpec::AutoWah { sens, base, range, q, attack_ms, release_ms, mode, down,
                                   wobble, wobble_hz })
        }
        "panenv" => {
            let depth = float_param_at(&node.params, 0).unwrap_or(0.6);
            let attack_ms = named_param(&node.params, "attack").unwrap_or(5.0);
            let release_ms = named_param(&node.params, "release").unwrap_or(300.0);
            Ok(NodeSpec::PanEnv { depth, attack_ms, release_ms })
        }
        "autopan" => {
            let depth = float_param_at(&node.params, 0).unwrap_or(0.5);
            let bars = named_param(&node.params, "bars").unwrap_or(0.0);
            let hz = named_param(&node.params, "hz").unwrap_or(if bars > 0.0 { 0.0 } else { 0.25 });
            Ok(NodeSpec::AutoPan { hz, bars, depth })
        }
        other => Err(CompileError { line: 0,
            message: format!("unknown node type '{}'", other),
        }),
    }
}

// ── Note resolution ──

/// Resolve a NoteRef (absolute or scale degree) to a MIDI note number.
pub fn resolve_note(note: &crate::dsl::ast::NoteRef, scale_intervals: &[u8], root_midi: u8) -> u8 {
    match note {
        crate::dsl::ast::NoteRef::Absolute(name) => note_name_to_midi(name),
        crate::dsl::ast::NoteRef::Midi(m) => *m,
        crate::dsl::ast::NoteRef::Degree(degree, octave) => {
            // degree is 1-7, map to 0-indexed
            let deg_idx = (*degree as usize).saturating_sub(1) % scale_intervals.len();
            let semitones = scale_intervals[deg_idx];
            // Root MIDI is in octave 0 context — degree 1 octave 3 means root note in octave 3
            // root_midi is the root at octave 0 (just the pitch class, 0-11)
            let midi = ((*octave as i16 + 1) * 12) + root_midi as i16 + semitones as i16;
            midi.clamp(0, 127) as u8
        }
    }
}

/// Get scale intervals and root pitch class from the song's scale definition.
pub fn scale_context(song: &Song) -> ([u8; 7], u8) {
    if let Some(ref scale_def) = song.globals.scale {
        let intervals = match scale_def.kind.as_str() {
            "major" => [0, 2, 4, 5, 7, 9, 11],
            "minor" => [0, 2, 3, 5, 7, 8, 10],
            "dorian" => [0, 2, 3, 5, 7, 9, 10],
            "mixolydian" => [0, 2, 4, 5, 7, 9, 10],
            "phrygian" => [0, 1, 3, 5, 7, 8, 10],
            "lydian" => [0, 2, 4, 6, 7, 9, 11],
            "locrian" => [0, 1, 3, 5, 6, 8, 10],
            _ => [0, 2, 4, 5, 7, 9, 11], // default major
        };
        // Root pitch class (0=C, 2=D, 4=E, 5=F, 7=G, 9=A, 11=B)
        let root_pc = match scale_def.root.as_str() {
            "C" => 0, "C#" | "Db" => 1, "D" => 2, "D#" | "Eb" => 3,
            "E" => 4, "F" => 5, "F#" | "Gb" => 6, "G" => 7,
            "G#" | "Ab" => 8, "A" => 9, "A#" | "Bb" => 10, "B" => 11,
            _ => 0,
        };
        (intervals, root_pc)
    } else {
        // Default: C major
        ([0, 2, 4, 5, 7, 9, 11], 0)
    }
}

// ── Pattern compilation ──

fn compile_pattern(pat: &PatternDef, scale_intervals: &[u8], root_midi: u8) -> Result<CompiledPattern, CompileError> {
    // Multi-lane drum pattern
    if !pat.lane_labels.is_empty() {
        let mut lanes = Vec::new();
        for (i, row) in pat.rows.iter().enumerate() {
            let label = if i < pat.lane_labels.len() {
                &pat.lane_labels[i]
            } else {
                continue;
            };
            let midi_note = drum_name_to_midi(label);
            if midi_note == 0 {
                return Err(CompileError::new(format!(
                    "pattern '{}': unknown drum lane '{}' (kick, snare, clap, hat, openhat, tom, tom2, tom3, crash)",
                    pat.name, label
                )));
            }
            let steps: Vec<CompiledStep> = row.iter().map(|step| {
                match step {
                    Step::Subdiv(_) => CompiledStep::Rest,
                    Step::DrumHit(ds) => {
                        let plock = StepPLock {
                            cutoff: ds.plock.cutoff,
                            env_depth: ds.plock.env_depth,
                            resonance: ds.plock.resonance,
                            gate: ds.plock.gate,
                        };
                        CompiledStep::DrumHit {
                            velocity: ds.velocity,
                            probability: ds.probability,
                            roll: ds.roll,
                            plock,
                        }
                    }
                    Step::Rest => CompiledStep::Rest,
                    Step::Tie => CompiledStep::Tie,
                    Step::Chord(_) => CompiledStep::Rest, // chords not supported in drum lanes
                    Step::Note(ns) => {
                        let midi = resolve_note(&ns.note, scale_intervals, root_midi);
                        let vel = ns.velocity.unwrap_or(0.8);
                        let plock = StepPLock {
                            cutoff: ns.plock.cutoff,
                            env_depth: ns.plock.env_depth,
                            resonance: ns.plock.resonance,
                            gate: ns.plock.gate,
                        };
                        CompiledStep::NoteOn { midi_note: midi, velocity: vel, plock, slide: ns.slide }
                    }
                }
            }).collect();
            lanes.push(CompiledLane { midi_note, steps, swing_override: None, nudge: 0.0 });
        }
        let steps_per_row = lanes.first().map(|l| l.steps.len()).unwrap_or(4);
        return Ok(CompiledPattern {
            name: pat.name.clone(),
            steps: Vec::new(),
            steps_per_row,
            lanes,
        });
    }

    // Sequential pattern (existing behavior)
    let mut steps = Vec::new();
    let mut max_row_len = 0;

    for row in &pat.rows {
        let row_len = row.len();
        if row_len > max_row_len {
            max_row_len = row_len;
        }
        for step in row {
            match step {
                Step::Note(ns) => {
                    let midi = resolve_note(&ns.note, scale_intervals, root_midi);
                    let vel = ns.velocity.unwrap_or(0.8);
                    let plock = StepPLock {
                        cutoff: ns.plock.cutoff,
                        env_depth: ns.plock.env_depth,
                        resonance: ns.plock.resonance,
                        gate: ns.plock.gate,
                    };
                    steps.push(CompiledStep::NoteOn { midi_note: midi, velocity: vel, plock, slide: ns.slide });
                }
                Step::Chord(cs) => {
                    let mut chord_notes = [ChordNote::default(); MAX_CHORD_NOTES];
                    let shared_vel = cs.velocity.unwrap_or(0.8);
                    let count = cs.notes.len().min(MAX_CHORD_NOTES);
                    for (i, ns) in cs.notes.iter().take(MAX_CHORD_NOTES).enumerate() {
                        chord_notes[i] = ChordNote {
                            midi_note: resolve_note(&ns.note, scale_intervals, root_midi),
                            velocity: ns.velocity.unwrap_or(shared_vel),
                        };
                    }
                    let plock = StepPLock {
                        cutoff: cs.plock.cutoff,
                        env_depth: cs.plock.env_depth,
                        resonance: cs.plock.resonance,
                        gate: cs.plock.gate,
                    };
                    steps.push(CompiledStep::Chord { notes: chord_notes, count: count as u8, plock });
                }
                Step::Subdiv(subs) => {
                    let mut notes = [SubNote::default(); MAX_SUBDIV];
                    let count = subs.len().min(MAX_SUBDIV);
                    for (i, ns) in subs.iter().take(MAX_SUBDIV).enumerate() {
                        notes[i] = SubNote {
                            midi_note: resolve_note(&ns.note, scale_intervals, root_midi),
                            velocity: ns.velocity.unwrap_or(0.8),
                            slide: ns.slide,
                        };
                    }
                    let plock = subs.first().map(|ns| StepPLock {
                        cutoff: ns.plock.cutoff,
                        env_depth: ns.plock.env_depth,
                        resonance: ns.plock.resonance,
                        gate: ns.plock.gate,
                    }).unwrap_or_default();
                    steps.push(CompiledStep::Subdiv { notes, count: count as u8, plock });
                }
                Step::DrumHit(ds) => {
                    let plock = StepPLock {
                        cutoff: ds.plock.cutoff,
                        env_depth: ds.plock.env_depth,
                        resonance: ds.plock.resonance,
                        gate: ds.plock.gate,
                    };
                    steps.push(CompiledStep::DrumHit {
                        velocity: ds.velocity,
                        probability: ds.probability,
                        roll: ds.roll,
                        plock,
                    });
                }
                Step::Rest => {
                    steps.push(CompiledStep::Rest);
                }
                Step::Tie => {
                    steps.push(CompiledStep::Tie);
                }
            }
        }
    }

    Ok(CompiledPattern {
        name: pat.name.clone(),
        steps,
        steps_per_row: if max_row_len > 0 { max_row_len } else { 4 },
        lanes: Vec::new(),
    })
}

/// The note a drum lane plays on a `beats` module, `None` for a name that is
/// not a drum. What a pad hits, and what a lane label compiles to.
pub fn drum_note(name: &str) -> Option<u8> {
    match drum_name_to_midi(name) {
        0 => None,
        n => Some(n),
    }
}

/// Map drum lane label to MIDI note number.
fn drum_name_to_midi(name: &str) -> u8 {
    match name {
        "kick" | "bd" => 36,
        "snare" | "sd" => 38,
        "clap" | "cp" => 39,
        "hat" | "hh" | "hihat" => 42,
        "tom" | "lt" => 43,
        "tom2" | "mt" => 45,
        "tom3" | "ht" => 47,
        "openhat" | "oh" => 46,
        "crash" | "cr" => 49,
        _ => 0, // unknown: caller reports an error
    }
}

/// Convert note name (e.g., "A1", "C#4") to MIDI note number.
pub fn note_name_to_midi(name: &str) -> u8 {
    let chars: Vec<char> = name.chars().collect();
    if chars.is_empty() { return 60; } // default C4

    let base = match chars[0].to_ascii_uppercase() {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => 0,
    };

    let mut i = 1;
    let accidental = if i < chars.len() && chars[i] == '#' {
        i += 1;
        1i8
    } else if i < chars.len() && chars[i] == 'b' {
        i += 1;
        -1i8
    } else {
        0
    };

    let octave: i8 = if i < chars.len() {
        let oct_str: String = chars[i..].iter().collect();
        oct_str.parse().unwrap_or(4)
    } else {
        4
    };

    // MIDI: C4 = 60
    let midi = (octave as i16 + 1) * 12 + base as i16 + accidental as i16;
    midi.clamp(0, 127) as u8
}

// ── Track compilation ──

fn compile_track(
    track: &TrackDef,
    inst_names: &[String],
    patterns: &[CompiledPattern],
    buses: &[CompiledBus],
    defaults: Option<&CompiledTrack>,
    samples_per_bar: f32,
) -> Result<CompiledTrack, CompileError> {
    let instrument_idx = inst_names.iter().position(|n| n == &track.using_instrument)
        .ok_or_else(|| CompileError { line: 0,
            message: format!("track '{}': unknown instrument '{}'", track.name, track.using_instrument),
        })?;

    let pattern_idx = patterns.iter().position(|p| p.name == track.play)
        .ok_or_else(|| CompileError { line: 0,
            message: format!("track '{}': unknown pattern '{}'", track.name, track.play),
        })?;

    let velocity = track.velocity.unwrap_or_else(|| defaults.map(|d| d.velocity).unwrap_or(0.8));
    let level = track.level.unwrap_or_else(|| defaults.map(|d| d.level).unwrap_or(0.8));
    let pan = track.pan.unwrap_or_else(|| defaults.map(|d| d.pan).unwrap_or(0.0));
    let gate = track.gate.unwrap_or_else(|| defaults.map(|d| d.gate).unwrap_or(0.85));

    // Parse routing chain for insert FX and bus sends.
    // If scene track has no routing but a global default exists, inherit it.
    let mut insert_fx = Vec::new();
    let mut insert_fx_labels: Vec<Option<String>> = Vec::new();
    let mut bus_send = None;
    let mut to_master = true;

    if track.routing.is_empty() {
        if let Some(d) = defaults {
            insert_fx = d.insert_fx.clone();
            insert_fx_labels = d.insert_fx_labels.clone();
            bus_send = d.bus_send;
            to_master = d.to_master;
        }
    } else {
        for rnode in &track.routing {
            if let Some(bus_idx) = buses.iter().position(|b| b.name == rnode.kind) {
                bus_send = Some((bus_idx, 1.0));
                to_master = false;
            } else if rnode.kind == "master" {
                to_master = true;
            } else {
                let mut noise_seed = 100u32;
                let mut drift_seed = 9000u32;
                let node = NodeDef {
                    kind: rnode.kind.clone(),
                    alias: None,
                    params: rnode.params.clone(),
                    line: Line::default(),
                };
                let wet = named_param(&rnode.params, crate::nodes::WET).unwrap_or(1.0);
                match node_def_to_spec(&node, &mut noise_seed, &mut drift_seed, samples_per_bar) {
                    Ok(spec) => {
                        insert_fx.push(ChainStep { spec, wet });
                        insert_fx_labels.push(rnode.label.clone());
                    }
                    Err(e) => return Err(CompileError::new(format!(
                        "track '{}': {} (or declare `bus {}` if it is a bus)", track.name, e.message, rnode.kind
                    ))),
                }
            }
        }
    }

    check_unique_labels(&format!("track '{}'", track.name), &insert_fx_labels)?;

    let delay_send = track.delay_send.unwrap_or_else(|| defaults.map(|d| d.delay_send).unwrap_or(0.0));
    let reverb_send = track.reverb_send.unwrap_or_else(|| defaults.map(|d| d.reverb_send).unwrap_or(0.0));

    let arp = match &track.arp {
        Some(def) => compile_arp(&track.name, def)?,
        None => defaults.and_then(|d| d.arp),
    };

    Ok(CompiledTrack {
        name: track.name.clone(),
        instrument_idx,
        pattern_idx,
        velocity,
        level,
        pan,
        gate,
        insert_fx,
        insert_fx_labels,
        bus_send,
        to_master,
        delay_send,
        reverb_send,
        sidechain: track.sidechain.or_else(|| defaults.and_then(|d| d.sidechain)),
        sidechain_source: track.sidechain_source.clone()
            .or_else(|| defaults.and_then(|d| d.sidechain_source.clone())),
        arp,
    })
}

/// Validate and compile an `arp` clause. `arp off` yields `None`.
fn compile_arp(track_name: &str, def: &ArpDef) -> Result<Option<ArpConfig>, CompileError> {
    let pattern = match def.mode.as_str() {
        "off" => return Ok(None),
        "up" => 0.0,
        "down" => 0.5,
        "updown" => 1.0,
        other => return Err(CompileError::new(format!(
            "track '{}': arp mode '{}' — expected up, down, updown or off", track_name, other
        ))),
    };
    let rate = def.rate.unwrap_or(16.0);
    if ![4.0, 8.0, 16.0, 32.0].contains(&rate) {
        return Err(CompileError::new(format!(
            "track '{}': arp rate {} — expected 4, 8, 16 or 32 (notes per bar)", track_name, rate
        )));
    }
    let gate = def.gate.unwrap_or(0.6);
    if !(0.1..=1.0).contains(&gate) {
        return Err(CompileError::new(format!(
            "track '{}': arp gate {} — expected 0.1..1.0", track_name, gate
        )));
    }
    let octaves = def.octaves.unwrap_or(1.0);
    if !(1.0..=4.0).contains(&octaves) || octaves != math::floor(octaves) {
        return Err(CompileError::new(format!(
            "track '{}': arp octaves {} — expected 1, 2, 3 or 4", track_name, octaves
        )));
    }
    Ok(Some(ArpConfig {
        pattern,
        rate_mult: rate / 16.0,
        gate,
        octaves: octaves as u8,
    }))
}

// ── Bus/Master FX chain compilation ──

/// Two nodes in one chain with the same name make `auto track.name wet` mean
/// two things, so it is an error rather than a silent first-match-wins.
fn check_unique_labels(owner: &str, labels: &[Option<String>]) -> Result<(), CompileError> {
    for (i, l) in labels.iter().enumerate() {
        let Some(name) = l else { continue };
        if labels[..i].iter().flatten().any(|prev| prev == name) {
            return Err(CompileError::new(format!(
                "{}: two nodes are both named '{}'; a name has to pick out one node",
                owner, name
            )));
        }
    }
    Ok(())
}

fn compile_fx_chain(owner: &str, chain: &[ChainNode], samples_per_bar: f32) -> Result<Vec<ChainStep>, CompileError> {
    check_unique_labels(owner, &chain.iter().map(|n| n.label.clone()).collect::<Vec<_>>())?;
    let mut specs = Vec::new();
    let mut noise_seed = 200u32;
    let mut drift_seed = 8000u32;

    for node in chain {
        let node_def = NodeDef {
            kind: node.kind.clone(),
            alias: None,
            params: node.params.clone(),
            line: Line::default(),
        };
        let wet = named_param(&node.params, crate::nodes::WET).unwrap_or(1.0);
        match node_def_to_spec(&node_def, &mut noise_seed, &mut drift_seed, samples_per_bar) {
            Ok(spec) => specs.push(ChainStep { spec, wet }),
            Err(e) => return Err(CompileError::new(format!("{}: {}", owner, e.message))),
        }
    }
    Ok(specs)
}

// ── Scene compilation ──

fn compile_scene(
    scene: &SceneDef,
    inst_names: &[String],
    patterns: &[CompiledPattern],
    buses: &[CompiledBus],
    global_tracks: &[CompiledTrack],
    samples_per_bar: f32,
) -> Result<CompiledScene, CompileError> {
    let mut tracks = Vec::new();
    for track_def in &scene.tracks {
        // Look up matching global track by name for default inheritance
        let defaults = global_tracks.iter().find(|gt| gt.name == track_def.name);
        if defaults.is_none() {
            return Err(CompileError::new(format!(
                "scene '{}': track '{}' is not declared at top level; scenes can only reconfigure existing tracks",
                scene.name, track_def.name
            )));
        }
        let t = compile_track(track_def, inst_names, patterns, buses, defaults, samples_per_bar)?;
        // A scene track's own `out > ...` parses and compiles and is
        // then never applied: insert chains are built once per track
        // and the engine does not rebuild them on a scene change.
        // Restating the same chain is harmless; changing it is not, and
        // it used to change nothing in silence.
        if let Some(d) = defaults {
            if !track_def.routing.is_empty() && t.insert_fx != d.insert_fx {
                return Err(CompileError::new(format!(
                    "scene '{}': track '{}' cannot change its `out > ...` chain. Insert chains are fixed per track for the whole song; move the chain to the top-level `track {}` block, or add a second track with the other chain and swap which one plays.",
                    scene.name, track_def.name, track_def.name
                )));
            }
        }
        tracks.push(t);
    }

    // Extract effect overrides from scene overrides
    let mut reverb_mix = None;
    let mut delay_mix = None;
    let mut reverb_freeze = None;
    for ovr in &scene.overrides {
        match ovr.target.as_str() {
            "reverb_mix" => reverb_mix = Some(ovr.value),
            "delay_mix" => delay_mix = Some(ovr.value),
            "reverb_freeze" => reverb_freeze = Some(ovr.value >= 0.5),
            other => return Err(CompileError::new(format!(
                "scene '{}': unknown override '{}' (expected reverb_mix, delay_mix or reverb_freeze)", scene.name, other
            ))),
        }
    }

    // Compile automation definitions
    let automations: Vec<CompiledAutomation> = scene.automations.iter()
        .map(|a| CompiledAutomation {
            target: a.target.clone(),
            keyframes: a.keyframes.clone(),
        })
        .collect();

    Ok(CompiledScene {
        name: scene.name.clone(),
        tempo: scene.tempo,
        tracks,
        reverb_mix,
        delay_mix,
        reverb_freeze,
        automations,
    })
}

// ── Param helpers ──

fn find_node(names: &[(String, u8)], name: &str) -> Option<u8> {
    names.iter().find(|(n, _)| n == name).map(|(_, idx)| *idx)
}

fn first_float_param(params: &[Param]) -> Option<f32> {
    for p in params {
        match p {
            Param::Float(v) => return Some(*v),
            Param::Expr(e) => return Some(e.eval()),
            _ => {}
        }
    }
    None
}

fn float_param_at(params: &[Param], index: usize) -> Option<f32> {
    let mut count = 0;
    for p in params {
        match p {
            Param::Float(v) => {
                if count == index { return Some(*v); }
                count += 1;
            }
            Param::Expr(e) => {
                if count == index { return Some(e.eval()); }
                count += 1;
            }
            Param::Waveform(_) => {} // skip waveforms in positional count
            Param::Named(_, _) => {} // skip named
            Param::RhythmDiv(_, _) => {
                if count == index {
                    // Convert rhythm div to float (not directly useful as freq)
                    return None;
                }
                count += 1;
            }
        }
    }
    None
}

fn named_param(params: &[Param], name: &str) -> Option<f32> {
    for p in params {
        if let Param::Named(n, v) = p {
            if n == name { return Some(*v); }
        }
    }
    None
}

fn rhythm_div_param(params: &[Param]) -> Option<f32> {
    for p in params {
        if let Param::RhythmDiv(num, den) = p {
            return Some(*num as f32 / *den as f32);
        }
    }
    None
}

/// Filter LFO options: lfo_hz / lfo_bars (cycle length) and lfo_depth (Hz).
fn filter_lfo_params(params: &[Param]) -> crate::graph::node::FilterLfo {
    let bars = named_param(params, "lfo_bars").unwrap_or(0.0);
    let hz = named_param(params, "lfo_hz").unwrap_or(0.0);
    let depth = named_param(params, "lfo_depth").unwrap_or(0.0);
    crate::graph::node::FilterLfo { hz, bars, depth }
}

/// Extract filter envelope named params: ea, ed, es, er, edepth.
/// Returns (attack, decay, sustain, release, depth) with defaults.
fn filter_env_params(params: &[Param]) -> (f32, f32, f32, f32, f32) {
    let ea = named_param(params, "ea").unwrap_or(0.005);
    let ed = named_param(params, "ed").unwrap_or(0.2);
    let es = named_param(params, "es").unwrap_or(0.0);
    let er = named_param(params, "er").unwrap_or(0.1);
    let edepth = named_param(params, "edepth").unwrap_or(0.0);
    (ea, ed, es, er, edepth)
}

// ── Module compilation ──

fn compile_module_def(mod_def: &ModuleDef) -> Result<CompiledInstrumentKind, Vec<CompileError>> {
    let kind = match ModuleKind::from_str(&mod_def.module_type) {
        Some(k) => k,
        None => return Err(vec![CompileError::new(format!(
            "module '{}': unknown module type '{}' (expected bass, fm, keys, or beats)",
            mod_def.name, mod_def.module_type
        ))]),
    };

    let mut errors = Vec::new();
    let mut valid: Vec<(String, f32)> = Vec::new();
    for p in &mod_def.params {
        match params::lookup(kind, &p.name) {
            Some(spec) => match spec.validate(p.value) {
                Ok(()) => valid.push((p.name.clone(), p.value)),
                Err(msg) => errors.push(CompileError::at(p.line, format!("module '{}': {}", mod_def.name, msg))),
            },
            None => errors.push(CompileError::at(p.line, unknown_param_message(kind, &mod_def.name, &p.name))),
        }
    }
    for env in &mod_def.op_envelopes {
        if kind != ModuleKind::Fm {
            errors.push(CompileError::new(format!(
                "module '{}': op{}_envelope is only valid on fm modules", mod_def.name, env.op_index
            )));
        } else if env.op_index > 3 {
            errors.push(CompileError::new(format!(
                "module '{}': op{}_envelope — operators are op0..op3", mod_def.name, env.op_index
            )));
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }

    Ok(match kind {
        ModuleKind::Bass => {
            let preset = ModulePreset { params: valid };
            CompiledInstrumentKind::Bass(preset)
        }
        ModuleKind::Keys => {
            let preset = ModulePreset { params: valid };
            CompiledInstrumentKind::Keys(preset)
        }
        ModuleKind::Beats => {
            let preset = ModulePreset { params: valid };
            CompiledInstrumentKind::Beats(preset)
        }
        ModuleKind::Fm => {
            let mut preset = FmPreset { params: valid, ..Default::default() };
            for env in &mod_def.op_envelopes {
                preset.op_envelopes.push((env.op_index, (env.a, env.d, env.s, env.r)));
            }
            CompiledInstrumentKind::Fm(preset)
        }
    })
}

/// Explain an unknown parameter name: typo suggestion or wrong module type.
fn unknown_param_message(kind: ModuleKind, module_name: &str, param: &str) -> String {
    let base = format!("module '{}' ({}): unknown parameter '{}'", module_name, kind.as_str(), param);
    if let Some(sugg) = params::suggest(kind, param) {
        return format!("{}. Did you mean '{}'?", base, sugg);
    }
    let others = params::kinds_with_param(param);
    if !others.is_empty() {
        let names: Vec<&str> = others.iter().map(|k| k.as_str()).collect();
        return format!("{}. It exists on: {}", base, names.join(", "));
    }
    format!("{}. Run `tatum params {}` for the list", base, kind.as_str())
}

/// The master chain as it is built: without the `limiter` the engine
/// replaces with its own.
fn without_limiter(chain: &[ChainNode]) -> Vec<ChainNode> {
    chain.iter().filter(|n| n.kind != "limiter").cloned().collect()
}

/// Master-chain parameters that `auto master <param>` can move.
pub const MASTER_AUTO_PARAMS: &[&str] = &["tilt", "eq_low", "eq_mid", "eq_high", "drive", "gain", "cutoff", "comp_threshold"];

/// Node kinds that carry a given master automation parameter.
pub fn master_auto_node_kinds(param: &str) -> &'static [&'static str] {
    match param {
        "tilt" => &["tilt"],
        "eq_low" | "eq_mid" | "eq_high" => &["eq"],
        "drive" => &["saturate", "drive"],
        "gain" => &["gain"],
        "cutoff" => &["lowpass", "highpass", "bandpass", "ladder"],
        "comp_threshold" => &["compressor"],
        _ => &[],
    }
}

/// What a knob can be put on, for error messages.
const MIDI_TARGETS: &str = "<module> <param>, <track> level, <track> pan, <track> <node> wet, reverb_mix, delay_mix or reverb_freeze";

/// Validate `midi { }` against the song. A knob, a keyboard or a pad on
/// something that does not exist is an error here rather than a control that
/// silently does nothing on stage.
fn validate_midi(song: &Song) -> Vec<CompileError> {
    let mut errors = Vec::new();
    // The module a track plays, as written: its name and its kind.
    let module_of = |track: &str| -> Option<(&str, Option<&str>)> {
        let t = song.tracks.iter().find(|t| t.name == track)?;
        let kind = song.module_defs.iter().find(|m| m.name == t.using_instrument).map(|m| m.module_type.as_str());
        Some((t.using_instrument.as_str(), kind))
    };
    for map in &song.midi {
        let words: Vec<&str> = map.target.split('.').collect();
        let source = match map.source {
            MidiSource::Cc(cc) => format!("cc {}", cc),
            MidiSource::Keys => String::from("keys"),
            MidiSource::Pad(n) => format!("pad {}", n),
        };
        let err = |msg: String| CompileError::at(map.line, format!("midi: {} > {}: {}", source, words.join(" "), msg));
        match map.source {
            MidiSource::Keys => match words.as_slice() {
                [track] => match module_of(track) {
                    None => errors.push(err(format!("no track named '{}'", track))),
                    Some((_, Some("beats"))) => errors.push(err(format!(
                        "'{}' plays drums, which the pads play: `pad 36 > {} kick`", track, track))),
                    Some(_) => {}
                },
                _ => errors.push(err(String::from("the keys play one track: `keys > solo`"))),
            },
            MidiSource::Pad(_) => match words.as_slice() {
                [track, drum] => match module_of(track) {
                    None => errors.push(err(format!("no track named '{}'", track))),
                    Some((_, Some("beats"))) if drum_note(drum).is_some() => {}
                    Some((_, Some("beats"))) => errors.push(err(format!(
                        "'{}' is not a drum (kick, snare, clap, hat, openhat, tom, tom2, tom3, crash)", drum))),
                    Some((module, _)) => errors.push(err(format!(
                        "track '{}' plays '{}', which is not a drum kit; a pad hits a `beats` module", track, module))),
                },
                _ => errors.push(err(String::from("a pad hits one drum of a track: `pad 36 > kick kick`"))),
            },
            MidiSource::Cc(_) => match words.as_slice() {
                ["reverb_mix"] | ["delay_mix"] | ["reverb_freeze"] => {}
                ["master", _] => errors.push(err(String::from(
                    "the master chain cannot be put on a knob yet; `auto master` can sweep it"))),
                [name, "level"] | [name, "pan"] => {
                    let is_track = song.tracks.iter().any(|t| &t.name == name);
                    let is_module = song.module_defs.iter().any(|m| &m.name == name)
                        || song.instruments.iter().any(|i| &i.name == name);
                    if !is_track && !is_module {
                        errors.push(err(format!("no track or module named '{}'", name)));
                    }
                }
                [track, node, "wet"] => match song.tracks.iter().find(|t| &t.name == track) {
                    None => errors.push(err(format!("no track named '{}'", track))),
                    Some(t) if t.routing.iter().any(|n| n.label.as_deref() == Some(node)) => {}
                    Some(t) => {
                        let named: Vec<&str> = t.routing.iter().filter_map(|n| n.label.as_deref()).collect();
                        errors.push(err(format!("track '{}' has no node named '{}'{}", track, node,
                            if named.is_empty() {
                                String::from("; name one with `> effect(...) as <name>`")
                            } else {
                                format!(" (it has {})", named.join(", "))
                            })));
                    }
                },
                [module, param] => match song.module_defs.iter().find(|m| &m.name == module) {
                    Some(m) => {
                        if let Some(kind) = ModuleKind::from_str(&m.module_type) {
                            if params::lookup(kind, param).is_none() {
                                errors.push(err(unknown_param_message(kind, module, param)));
                            }
                        }
                    }
                    None if song.instruments.iter().any(|i| &i.name == module) => errors.push(err(String::from(
                        "graph instruments have no named params (only level and pan)"))),
                    None => errors.push(err(format!("no module named '{}'", module))),
                },
                _ => errors.push(err(format!("a knob moves {}", MIDI_TARGETS))),
            },
        }
    }
    errors
}

/// Validate `auto <target> ...` lanes against tracks, instruments and the param registry.
fn validate_automations(song: &Song) -> Vec<CompileError> {
    let mut errors = Vec::new();
    for scene in &song.scenes {
        for auto in &scene.automations {
            let target = auto.target.as_str();
            if target == "reverb_mix" || target == "delay_mix" || target == "reverb_freeze" {
                continue;
            }
            let (name, param) = match target.find('.') {
                Some(i) => (&target[..i], &target[i + 1..]),
                None => {
                    errors.push(CompileError::new(format!(
                        "scene '{}': automation target '{}' must be reverb_mix, delay_mix, <track> level or <module> <param>",
                        scene.name, target
                    )));
                    continue;
                }
            };
            if name == "master" {
                let needed = master_auto_node_kinds(param);
                if needed.is_empty() {
                    errors.push(CompileError::new(format!(
                        "scene '{}': automation — master has no parameter '{}' (expected {})",
                        scene.name, param, MASTER_AUTO_PARAMS.join(", ")
                    )));
                } else if !song.master.as_ref().is_some_and(|m| m.chain.iter().any(|n| needed.contains(&n.kind.as_str()))) {
                    errors.push(CompileError::new(format!(
                        "scene '{}': automation — `auto master {}` needs a {} node in the master chain",
                        scene.name, param, needed.join(" or ")
                    )));
                }
                continue;
            }
            // `<track>.<node> wet`: sweeping an effect in or out over a scene.
            // Checked here so a misspelled node name is an error at compile
            // time rather than an automation that silently does nothing.
            if let Some(node) = param.strip_suffix(".wet") {
                // A scene track inherits the top-level chain when it does not
                // write one, so an empty scene routing is not "no chain".
                let routing = scene.tracks.iter()
                    .find(|t| t.name == name && !t.routing.is_empty())
                    .or_else(|| song.tracks.iter().find(|t| t.name == name))
                    .map(|t| &t.routing);
                match routing {
                    Some(r) if r.iter().any(|n| n.label.as_deref() == Some(node)) => continue,
                    Some(_) => {
                        let named: Vec<&str> = routing.into_iter().flatten()
                            .filter_map(|n| n.label.as_deref())
                            .collect();
                        errors.push(CompileError::new(format!(
                            "scene '{}': automation target '{}' — track '{}' has no node named '{}'{}",
                            scene.name, target, name, node,
                            if named.is_empty() {
                                String::from("; name one with `> effect(...) as <name>`")
                            } else {
                                format!(" (it has {})", named.join(", "))
                            }
                        )));
                        continue;
                    }
                    None => {
                        errors.push(CompileError::new(format!(
                            "scene '{}': automation target '{}' — no track named '{}'",
                            scene.name, target, name
                        )));
                        continue;
                    }
                }
            }
            let is_track = scene.tracks.iter().chain(song.tracks.iter()).any(|t| t.name == name);
            let module = song.module_defs.iter().find(|m| m.name == name);
            let is_graph = song.instruments.iter().any(|i| i.name == name);
            if param == "level" && (is_track || module.is_some() || is_graph) {
                continue;
            }
            match module {
                Some(m) => {
                    if let Some(kind) = ModuleKind::from_str(&m.module_type) {
                        if params::lookup(kind, param).is_none() {
                            errors.push(CompileError::new(format!(
                                "scene '{}': automation — {}", scene.name, unknown_param_message(kind, name, param)
                            )));
                        }
                    }
                }
                None if is_graph => errors.push(CompileError::new(format!(
                    "scene '{}': automation target '{}' — graph instruments have no named params (only level)",
                    scene.name, target
                ))),
                None => errors.push(CompileError::new(format!(
                    "scene '{}': automation target '{}' — no module or track named '{}'",
                    scene.name, target, name
                ))),
            }
        }
    }
    errors
}
