extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;

use crate::dsl::ast::*;
use crate::dsl::error::{CompileError, CompileResult};
use crate::graph::{GraphBuilder, GraphTemplate};
use crate::graph::node::NodeSpec;
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

/// A note within a chord.
#[derive(Clone, Copy, Debug, Default)]
pub struct ChordNote {
    pub midi_note: u8,
    pub velocity: f32,
}

/// A compiled step: either a note event, chord, or silence.
#[derive(Clone, Copy, Debug)]
pub enum CompiledStep {
    NoteOn { midi_note: u8, velocity: f32, plock: StepPLock },
    Chord { notes: [ChordNote; MAX_CHORD_NOTES], count: u8, plock: StepPLock },
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
    pub insert_fx: Vec<NodeSpec>,
    pub bus_send: Option<(usize, f32)>, // (bus_idx, amount)
    pub to_master: bool,
    pub delay_send: f32,    // global delay send amount 0.0-1.0
    pub reverb_send: f32,   // global reverb send amount 0.0-1.0
    pub sidechain: Option<f32>, // per-track sidechain override (None = use global)
}

/// A compiled bus.
#[derive(Clone, Debug)]
pub struct CompiledBus {
    pub name: String,
    pub fx_chain: Vec<NodeSpec>,
}

/// A compiled scene snapshot.
#[derive(Clone, Debug)]
pub struct CompiledScene {
    pub name: String,
    pub tempo: Option<f32>,
    pub tracks: Vec<CompiledTrack>,
    pub reverb_mix: Option<f32>,
    pub delay_mix: Option<f32>,
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
    pub fx_chain: Vec<NodeSpec>,
}

/// A compiled instrument — either a graph template or a module preset.
pub enum CompiledInstrumentKind {
    Graph(GraphTemplate),
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
}

// ── Compiler ──

pub fn compile(song: &Song) -> CompileResult<CompiledSong> {
    let mut errors = Vec::new();

    // 1. Compile graph instruments
    let mut instruments: Vec<CompiledInstrumentKind> = Vec::new();
    let mut instrument_names = Vec::new();
    for inst_def in &song.instruments {
        match compile_instrument(inst_def) {
            Ok(template) => {
                instruments.push(CompiledInstrumentKind::Graph(template));
                instrument_names.push(inst_def.name.clone());
            }
            Err(e) => errors.push(e),
        }
    }

    // 1b. Compile module instruments
    for mod_def in &song.module_defs {
        match compile_module_def(mod_def) {
            Ok(kind) => {
                instruments.push(kind);
                instrument_names.push(mod_def.name.clone());
            }
            Err(e) => errors.push(e),
        }
    }

    // 2. Compile patterns (with scale context for degree resolution)
    let (intervals, root_pc) = scale_context(song);
    let mut patterns = Vec::new();
    for pat_def in &song.patterns {
        patterns.push(compile_pattern(pat_def, &intervals, root_pc));
    }

    // 3. Compile buses
    let mut buses = Vec::new();
    for bus_def in &song.buses {
        let chain = song.bus_chains.iter()
            .find(|bc| bc.bus_name == bus_def.name)
            .map(|bc| compile_fx_chain(&bc.chain))
            .unwrap_or_default();
        buses.push(CompiledBus { name: bus_def.name.clone(), fx_chain: chain });
    }

    // 4. Compile tracks
    let mut tracks = Vec::new();
    for track_def in &song.tracks {
        match compile_track(track_def, &instrument_names, &patterns, &buses, None) {
            Ok(t) => tracks.push(t),
            Err(e) => errors.push(e),
        }
    }

    // 5. Compile master
    let master = match &song.master {
        Some(m) => CompiledMaster { fx_chain: compile_fx_chain(&m.chain) },
        None => CompiledMaster { fx_chain: Vec::new() },
    };

    // 6. Compile scenes
    let mut scenes = Vec::new();
    for scene_def in &song.scenes {
        match compile_scene(scene_def, &instrument_names, &patterns, &buses, &tracks) {
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
            errors.push(CompileError {
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
    })
}

// ── Instrument compilation ──

fn compile_instrument(inst: &InstrumentDef) -> Result<GraphTemplate, CompileError> {
    let mut builder = GraphBuilder::new();
    let mut name_to_idx: Vec<(String, u8)> = Vec::new();
    let mut noise_seed = 42u32;
    let mut osc_drift_seed = 1000u32; // unique drift seed per oscillator

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
        let idx = builder.add_node(NodeSpec::Mix);
        name_to_idx.push((String::from("mix"), idx));
    }

    // Create explicit nodes
    for node_def in &inst.nodes {
        let spec = node_def_to_spec(node_def, &mut noise_seed, &mut osc_drift_seed)?;

        if matches!(spec, NodeSpec::Env { .. }) {
            // Envelope+VCA pattern: an envelope in a signal chain means
            // "multiply audio by envelope". We create two nodes:
            //   1. Env node (generates the 0-1 envelope curve)
            //   2. VCA node (multiplies: input[0] * input[1])
            // The alias (e.g., "amp") points to the VCA so that:
            //   - `filter > amp` connects filter → VCA input 1 (audio)
            //   - Env is auto-wired to VCA input 0 (envelope curve)
            //   - VCA output = envelope * audio
            let env_idx = builder.add_node(spec);
            let vca_idx = builder.add_node(NodeSpec::Vca);
            builder.connect(env_idx, vca_idx); // env → VCA input 0
            if let Some(ref alias) = node_def.alias {
                name_to_idx.push((alias.clone(), vca_idx));
            }
        } else {
            let idx = builder.add_node(spec);
            if let Some(ref alias) = node_def.alias {
                name_to_idx.push((alias.clone(), idx));
            }
        }
    }

    // Create "out" as Output node
    let out_idx = builder.add_node(NodeSpec::Output);
    name_to_idx.push((String::from("out"), out_idx));

    // Second pass: create connections
    for conn in &inst.connections {
        let from_idx = find_node(&name_to_idx, &conn.from)
            .ok_or_else(|| CompileError {
                message: format!("instrument '{}': unknown node '{}'", inst.name, conn.from),
            })?;
        let to_idx = find_node(&name_to_idx, &conn.to)
            .ok_or_else(|| CompileError {
                message: format!("instrument '{}': unknown node '{}'", inst.name, conn.to),
            })?;
        builder.connect(from_idx, to_idx);
    }

    let mut template = builder.build();
    template.output_gain = inst.gain.unwrap_or(1.0);
    Ok(template)
}

fn node_def_to_spec(node: &NodeDef, noise_seed: &mut u32, osc_drift_seed: &mut u32) -> Result<NodeSpec, CompileError> {
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
            let res = float_param_at(&node.params, 1).unwrap_or(0.5);
            let (ea, ed, es, er, edepth) = filter_env_params(&node.params);
            Ok(NodeSpec::Biquad { filter_type: FilterType::LowPass, cutoff, resonance: res,
                env_attack: ea, env_decay: ed, env_sustain: es, env_release: er, env_depth: edepth })
        }
        "highpass" => {
            let cutoff = float_param_at(&node.params, 0).unwrap_or(1000.0);
            let res = float_param_at(&node.params, 1).unwrap_or(0.5);
            let (ea, ed, es, er, edepth) = filter_env_params(&node.params);
            Ok(NodeSpec::Biquad { filter_type: FilterType::HighPass, cutoff, resonance: res,
                env_attack: ea, env_decay: ed, env_sustain: es, env_release: er, env_depth: edepth })
        }
        "bandpass" => {
            let cutoff = float_param_at(&node.params, 0).unwrap_or(1000.0);
            let res = float_param_at(&node.params, 1).unwrap_or(0.5);
            let (ea, ed, es, er, edepth) = filter_env_params(&node.params);
            Ok(NodeSpec::Biquad { filter_type: FilterType::BandPass, cutoff, resonance: res,
                env_attack: ea, env_decay: ed, env_sustain: es, env_release: er, env_depth: edepth })
        }
        "ladder" => {
            let cutoff = float_param_at(&node.params, 0).unwrap_or(1000.0);
            let res = float_param_at(&node.params, 1).unwrap_or(0.5);
            let (ea, ed, es, er, edepth) = filter_env_params(&node.params);
            Ok(NodeSpec::Ladder { cutoff, resonance: res,
                env_attack: ea, env_decay: ed, env_sustain: es, env_release: er, env_depth: edepth })
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
        other => Err(CompileError {
            message: format!("unknown node type '{}'", other),
        }),
    }
}

// ── Note resolution ──

/// Resolve a NoteRef (absolute or scale degree) to a MIDI note number.
fn resolve_note(note: &crate::dsl::ast::NoteRef, scale_intervals: &[u8], root_midi: u8) -> u8 {
    match note {
        crate::dsl::ast::NoteRef::Absolute(name) => note_name_to_midi(name),
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
fn scale_context(song: &Song) -> ([u8; 7], u8) {
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

fn compile_pattern(pat: &PatternDef, scale_intervals: &[u8], root_midi: u8) -> CompiledPattern {
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
            let steps: Vec<CompiledStep> = row.iter().map(|step| {
                match step {
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
                        CompiledStep::NoteOn { midi_note: midi, velocity: vel, plock }
                    }
                }
            }).collect();
            lanes.push(CompiledLane { midi_note, steps, swing_override: None, nudge: 0.0 });
        }
        let steps_per_row = lanes.first().map(|l| l.steps.len()).unwrap_or(4);
        return CompiledPattern {
            name: pat.name.clone(),
            steps: Vec::new(),
            steps_per_row,
            lanes,
        };
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
                    steps.push(CompiledStep::NoteOn { midi_note: midi, velocity: vel, plock });
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

    CompiledPattern {
        name: pat.name.clone(),
        steps,
        steps_per_row: if max_row_len > 0 { max_row_len } else { 4 },
        lanes: Vec::new(),
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
        _ => 36, // fallback to kick
    }
}

/// Convert note name (e.g., "A1", "C#4") to MIDI note number.
fn note_name_to_midi(name: &str) -> u8 {
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
) -> Result<CompiledTrack, CompileError> {
    let instrument_idx = inst_names.iter().position(|n| n == &track.using_instrument)
        .ok_or_else(|| CompileError {
            message: format!("track '{}': unknown instrument '{}'", track.name, track.using_instrument),
        })?;

    let pattern_idx = patterns.iter().position(|p| p.name == track.play)
        .ok_or_else(|| CompileError {
            message: format!("track '{}': unknown pattern '{}'", track.name, track.play),
        })?;

    let velocity = track.velocity.unwrap_or_else(|| defaults.map(|d| d.velocity).unwrap_or(0.8));
    let level = track.level.unwrap_or_else(|| defaults.map(|d| d.level).unwrap_or(0.8));
    let pan = track.pan.unwrap_or_else(|| defaults.map(|d| d.pan).unwrap_or(0.0));
    let gate = track.gate.unwrap_or_else(|| defaults.map(|d| d.gate).unwrap_or(0.85));

    // Parse routing chain for insert FX and bus sends.
    // If scene track has no routing but a global default exists, inherit it.
    let mut insert_fx = Vec::new();
    let mut bus_send = None;
    let mut to_master = true;

    if track.routing.is_empty() {
        if let Some(d) = defaults {
            insert_fx = d.insert_fx.clone();
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
                };
                if let Ok(spec) = node_def_to_spec(&node, &mut noise_seed, &mut drift_seed) {
                    insert_fx.push(spec);
                }
            }
        }
    }

    let delay_send = track.delay_send.unwrap_or_else(|| defaults.map(|d| d.delay_send).unwrap_or(0.0));
    let reverb_send = track.reverb_send.unwrap_or_else(|| defaults.map(|d| d.reverb_send).unwrap_or(0.0));

    Ok(CompiledTrack {
        name: track.name.clone(),
        instrument_idx,
        pattern_idx,
        velocity,
        level,
        pan,
        gate,
        insert_fx,
        bus_send,
        to_master,
        delay_send,
        reverb_send,
        sidechain: track.sidechain,
    })
}

// ── Bus/Master FX chain compilation ──

fn compile_fx_chain(chain: &[ChainNode]) -> Vec<NodeSpec> {
    let mut specs = Vec::new();
    let mut noise_seed = 200u32;
    let mut drift_seed = 8000u32;

    for node in chain {
        let node_def = NodeDef {
            kind: node.kind.clone(),
            alias: None,
            params: node.params.clone(),
        };
        if let Ok(spec) = node_def_to_spec(&node_def, &mut noise_seed, &mut drift_seed) {
            specs.push(spec);
        }
    }
    specs
}

// ── Scene compilation ──

fn compile_scene(
    scene: &SceneDef,
    inst_names: &[String],
    patterns: &[CompiledPattern],
    buses: &[CompiledBus],
    global_tracks: &[CompiledTrack],
) -> Result<CompiledScene, CompileError> {
    let mut tracks = Vec::new();
    for track_def in &scene.tracks {
        // Look up matching global track by name for default inheritance
        let defaults = global_tracks.iter().find(|gt| gt.name == track_def.name);
        match compile_track(track_def, inst_names, patterns, buses, defaults) {
            Ok(t) => tracks.push(t),
            Err(e) => return Err(e),
        }
    }

    // Extract effect overrides from scene overrides
    let mut reverb_mix = None;
    let mut delay_mix = None;
    for ovr in &scene.overrides {
        match ovr.target.as_str() {
            "reverb_mix" => reverb_mix = Some(ovr.value),
            "delay_mix" => delay_mix = Some(ovr.value),
            _ => {} // Other overrides not handled yet
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

fn compile_module_def(mod_def: &ModuleDef) -> Result<CompiledInstrumentKind, CompileError> {
    match mod_def.module_type.as_str() {
        "bass" => {
            let mut preset = ModulePreset::default();
            for p in &mod_def.params {
                preset.params.push((p.name.clone(), p.value));
            }
            Ok(CompiledInstrumentKind::Bass(preset))
        }
        "fm" => {
            let mut preset = FmPreset::default();
            for p in &mod_def.params {
                // Check for op envelope shorthand: op0_envelope, op1_envelope, etc.
                if p.name.starts_with("op") && p.name.ends_with("_envelope") {
                    // These are handled via the op_envelopes Vec in the AST
                    continue;
                }
                preset.params.push((p.name.clone(), p.value));
            }
            // Copy operator envelopes from AST
            for env in &mod_def.op_envelopes {
                preset.op_envelopes.push((env.op_index, (env.a, env.d, env.s, env.r)));
            }
            Ok(CompiledInstrumentKind::Fm(preset))
        }
        "keys" => {
            let mut preset = ModulePreset::default();
            for p in &mod_def.params {
                preset.params.push((p.name.clone(), p.value));
            }
            Ok(CompiledInstrumentKind::Keys(preset))
        }
        "beats" => {
            let mut preset = ModulePreset::default();
            for p in &mod_def.params {
                preset.params.push((p.name.clone(), p.value));
            }
            Ok(CompiledInstrumentKind::Beats(preset))
        }
        other => Err(CompileError {
            message: format!("unknown module type '{}' (expected bass, fm, keys, or beats)", other),
        }),
    }
}
