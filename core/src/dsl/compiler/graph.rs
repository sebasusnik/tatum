//! Graph instruments: `instrument { ... }` blocks become graph templates,
//! one node spec per node, and every node in an effect chain goes through
//! the same `node_def_to_spec`.

use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;

use crate::dsl::ast::*;
use crate::dsl::error::CompileError;
use crate::graph::{GraphBuilder, GraphError, GraphTemplate};
use crate::graph::node::NodeSpec;
use crate::primitives::oscillator::Waveform;
use crate::primitives::filter::FilterType;

use super::params::{filter_env_params, filter_lfo_params, first_float_param, float_param_at, named_param, rhythm_div_param};


// ── Instrument compilation ──

pub(super) fn compile_instrument(inst: &InstrumentDef, samples_per_bar: f32) -> Result<GraphTemplate, CompileError> {
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

pub(super) fn node_def_to_spec(node: &NodeDef, noise_seed: &mut u32, osc_drift_seed: &mut u32, samples_per_bar: f32) -> Result<NodeSpec, CompileError> {
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

fn find_node(names: &[(String, u8)], name: &str) -> Option<u8> {
    names.iter().find(|(n, _)| n == name).map(|(_, idx)| *idx)
}
