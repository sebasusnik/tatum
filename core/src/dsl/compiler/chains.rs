//! Effect chains on buses, the master and the send returns, and scenes: the
//! snapshots of tracks and send levels the arrangement plays in turn.

use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;

use crate::dsl::ast::*;
use crate::dsl::error::CompileError;
use crate::graph::node::{ChainStep, NodeSpec};

use super::compiled::{CompiledAutomation, CompiledBus, CompiledPattern, CompiledScene, CompiledTrack};
use super::graph::node_def_to_spec;
use super::params::named_param;
use super::track::compile_track;

// ── Bus/Master FX chain compilation ──

/// Two nodes in one chain with the same name make `auto track.name wet` mean
/// two things, so it is an error rather than a silent first-match-wins.
pub(super) fn check_unique_labels(owner: &str, labels: &[Option<String>]) -> Result<(), CompileError> {
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

/// A chain is a line of effects, each working on what the one before it
/// hands over. A node that makes its own signal reads none of that: put in a
/// chain it compiled, and what came out was the node alone, the track's own
/// sound gone without a word. An envelope there is never struck by a note, so
/// it stayed shut and the chain was silent.
pub(super) fn takes_what_comes_in(owner: &str, kind: &str, spec: &NodeSpec) -> Result<(), CompileError> {
    if !crate::nodes::takes_input(kind) {
        return Err(CompileError::new(format!(
            "{}: `{}` makes its own signal and takes nothing in, so in a chain it would replace everything before it; a chain only holds effects. Build the sound in an `instrument {{ }}` and play it on a track of its own",
            owner, kind
        )));
    }
    if matches!(spec, NodeSpec::Env { .. }) {
        return Err(CompileError::new(format!(
            "{}: `{}` is an envelope, and in a chain no note ever strikes it: it would stay shut and silence everything before it. An envelope belongs inside an `instrument {{ }}`",
            owner, kind
        )));
    }
    Ok(())
}

pub(super) fn compile_fx_chain(
    owner: &str,
    chain: &[ChainNode],
    samples_per_bar: f32,
) -> Result<Vec<ChainStep>, CompileError> {
    check_unique_labels(owner, &chain.iter().map(|n| n.label.clone()).collect::<Vec<_>>())?;
    let mut specs = Vec::new();
    let mut noise_seed = 200u32;
    let mut drift_seed = 8000u32;

    for node in chain {
        let node_def =
            NodeDef { kind: node.kind.clone(), alias: None, params: node.params.clone(), line: Line::default() };
        let wet = named_param(&node.params, crate::nodes::WET).unwrap_or(1.0);
        match node_def_to_spec(&node_def, &mut noise_seed, &mut drift_seed, samples_per_bar) {
            Ok(spec) => {
                takes_what_comes_in(owner, &node.kind, &spec)?;
                specs.push(ChainStep { spec, wet })
            }
            Err(e) => return Err(CompileError::new(format!("{}: {}", owner, e.message))),
        }
    }
    Ok(specs)
}

// ── Scene compilation ──

#[allow(clippy::too_many_arguments)]
pub(super) fn compile_scene(
    scene: &SceneDef,
    inst_names: &[String],
    patterns: &mut Vec<CompiledPattern>,
    plays: &mut Vec<super::compiled::PlayPlan>,
    ctx: &super::transform::Ctx,
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
        let t = compile_track(track_def, inst_names, patterns, plays, ctx, buses, defaults, samples_per_bar)?;
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
            other => {
                return Err(CompileError::new(format!(
                    "scene '{}': unknown override '{}' (expected reverb_mix, delay_mix or reverb_freeze)",
                    scene.name, other
                )))
            }
        }
    }

    // Compile automation definitions
    let automations: Vec<CompiledAutomation> = scene
        .automations
        .iter()
        .map(|a| CompiledAutomation { target: a.target.clone(), keyframes: a.keyframes.clone(), over: None })
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

/// The master chain as it is built: without the `limiter` the engine
/// replaces with its own.
pub(super) fn without_limiter(chain: &[ChainNode]) -> Vec<ChainNode> {
    chain.iter().filter(|n| n.kind != "limiter").cloned().collect()
}
