//! Tracks: which instrument plays which pattern, at what level, through
//! which insert chain and sends, with the arpeggiator if it has one. A scene
//! track compiles here too, filling what it leaves out from the top level.

use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;

use crate::dsl::ast::*;
use crate::dsl::error::CompileError;
use crate::math;
use crate::graph::node::ChainStep;

use super::chains::check_unique_labels;
use super::compiled::{ArpConfig, CompiledBus, CompiledPattern, CompiledTrack};
use super::graph::node_def_to_spec;
use super::params::named_param;


// ── Track compilation ──

pub(super) fn compile_track(
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
