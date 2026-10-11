//! Compiling a parsed song into what the engine plays: one stage per file,
//! called in order from [`compile`]. Instruments are built into graph
//! templates or module presets, patterns into steps, tracks and scenes into
//! their compiled form, and the `midi { }` and `auto` lines are checked
//! against all of it, so a name that points at nothing is an error here and
//! not a control that silently does nothing.
extern crate alloc;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;

use crate::dsl::ast::*;
use crate::dsl::error::{CompileError, CompileResult};

mod chains;
mod compiled;
mod graph;
mod module;
mod notes;
mod params;
mod pattern;
mod track;
pub mod transform;
mod validate;

pub use compiled::{
    ArpConfig, ChordNote, CompiledAutomation, CompiledBus, CompiledGroove, CompiledGrooveLane, CompiledInstrumentKind,
    CompiledLane, CompiledMaster, CompiledPattern, CompiledScene, CompiledSong, CompiledStep, CompiledTrack, FmPreset,
    ModulePreset, StepPLock, SubNote, MAX_CHORD_NOTES, MAX_SUBDIV, PlayPlan, LoopCondition,
};
pub use notes::{drum_note, note_in_midi_range, note_name_to_midi, resolve_note, scale_context};
pub use validate::{master_auto_node_kinds, MASTER_AUTO_PARAMS};

use chains::{compile_fx_chain, compile_scene, without_limiter};
use graph::compile_instrument;
use module::compile_module_def;
use notes::drum_name_to_midi;
use pattern::compile_pattern;
use track::compile_track;
use validate::{validate_automations, validate_midi, validate_perform, validate_scale};

// ── Compiler ──

pub fn compile(song: &Song) -> CompileResult<CompiledSong> {
    let mut errors = Vec::new();

    // A `capture` window is sized in samples at compile time so the buffer can
    // be allocated when the chain is built. Use the slowest tempo the song ever
    // reaches, since a scene can override it and a slower bar is a longer one.
    let slowest_tempo = song.scenes.iter().filter_map(|s| s.tempo).fold(song.globals.tempo, f32::min).max(20.0);
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
    errors.extend(validate_scale(song));
    errors.extend(validate_perform(song));

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
                Err(e) => {
                    errors.push(e);
                    Vec::new()
                }
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
                "chain '{}' has no `bus {}` declaration (or use reverb_return / delay_return for the global sends)",
                name, name
            ))),
            // Names that do match a declared bus were already compiled in step 3.
            _ => {}
        }
    }

    // 4. Compile tracks. A `play` line that transforms its pattern adds the
    // transformed versions to `patterns`, and a plan to `plays` when loops
    // differ.
    let ctx = transform::Ctx { intervals: &intervals, root: root_pc };
    let mut plays = Vec::new();
    let mut tracks = Vec::new();
    for track_def in &song.tracks {
        match compile_track(
            track_def,
            &instrument_names,
            &mut patterns,
            &mut plays,
            &ctx,
            &buses,
            None,
            samples_per_bar,
        ) {
            Ok(t) => tracks.push(t),
            Err(e) => errors.push(e),
        }
    }

    // 4b. A sidechain source has to name something that exists, and a track
    // cannot duck against itself. Getting this wrong used to be impossible
    // because there was nothing to get wrong: everything ducked the kick.
    {
        let known =
            |name: &str| song.tracks.iter().any(|t| t.name == name) || instrument_names.iter().any(|n| n == name);
        let mut check = |amount: Option<f32>, source: &Option<String>, owner: &str| {
            let Some(name) = source else { return };
            if !known(name) {
                errors
                    .push(CompileError::new(format!("{}: sidechain from='{}' names no track or module", owner, name)));
            } else if owner == name {
                errors.push(CompileError::new(format!(
                    "{}: sidechain from='{}' would duck the track against itself",
                    owner, name
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
        Some(m) => {
            let chain = without_limiter(&m.chain);
            let labels = chain.iter().map(|n| n.label.clone()).collect();
            match compile_fx_chain("master", &chain, samples_per_bar) {
                Ok(c) => CompiledMaster { fx_chain: c, labels },
                Err(e) => {
                    errors.push(e);
                    CompiledMaster { fx_chain: Vec::new(), labels: Vec::new() }
                }
            }
        }
        None => CompiledMaster { fx_chain: Vec::new(), labels: Vec::new() },
    };

    // 6. Compile scenes
    let mut scenes = Vec::new();
    for scene_def in &song.scenes {
        match compile_scene(
            scene_def,
            &instrument_names,
            &mut patterns,
            &mut plays,
            &ctx,
            &buses,
            &tracks,
            samples_per_bar,
        ) {
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
                line: 0,
                message: format!("unknown scene '{}' in arrangement", entry.scene_name),
            });
        }
    }

    if !errors.is_empty() {
        return Err(errors);
    }

    // Compile groove blocks
    let grooves: Vec<CompiledGroove> = song
        .grooves
        .iter()
        .map(|g| {
            let lanes = g
                .lanes
                .iter()
                .map(|l| CompiledGrooveLane {
                    midi_note: drum_name_to_midi(&l.drum_name),
                    swing_override: l.swing,
                    nudge: l.nudge.unwrap_or(0.0),
                })
                .collect();
            CompiledGroove { name: g.name.clone(), lanes }
        })
        .collect();

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
        plays,
        arrangement,
        grooves,
        reverb_return,
        delay_return,
        automations: song
            .automations
            .iter()
            .map(|a| CompiledAutomation { target: a.target.clone(), keyframes: a.keyframes.clone(), over: a.over })
            .collect(),
    })
}
