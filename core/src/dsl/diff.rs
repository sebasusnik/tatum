//! DSL diff engine — compares two Song ASTs and classifies changes.
//!
//! Used by the livecoding system to decide whether to apply runtime
//! mutations (fast path) or fall back to a full hot-swap (slow path).

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use super::ast::Song;
use crate::params::{self, ModuleKind};

/// A classified change between two Song ASTs.
#[derive(Debug, Clone)]
pub enum DslChange {
    /// Tempo changed — can apply via set_tempo()
    TempoChanged(f32),
    /// Swing changed
    SwingChanged(f32),
    /// Humanize (velocity, timing) changed
    HumanizeChanged { velocity: f32, timing: f32 },
    /// A module parameter changed — can apply via set_module_param()
    ModuleParamChanged {
        module_name: String,
        param_name: String,
        value: f32,
    },
    /// A track's level changed — can apply via set_track_level()
    TrackLevelChanged {
        track_name: String,
        level: f32,
    },
    /// A track's pan changed
    TrackPanChanged {
        track_name: String,
        pan: f32,
    },
    /// A track's velocity changed
    TrackVelocityChanged {
        track_name: String,
        velocity: f32,
    },
    /// A track's gate length changed
    TrackGateChanged {
        track_name: String,
        gate: f32,
    },
    /// Something structural changed — requires full hot-swap
    StructuralChange,
}

/// Compare two Song ASTs and return a list of classified changes.
/// If any change is `StructuralChange`, the caller should do a full hot-swap.
pub fn diff(old: &Song, new: &Song) -> Vec<DslChange> {
    let mut changes = Vec::new();

    // ── Globals ──
    if (old.globals.tempo - new.globals.tempo).abs() > 0.001 {
        changes.push(DslChange::TempoChanged(new.globals.tempo));
    }
    if old.globals.swing != new.globals.swing {
        changes.push(DslChange::SwingChanged(new.globals.swing.unwrap_or(0.5)));
    }
    if old.globals.humanize != new.globals.humanize
        || old.globals.humanize_timing != new.globals.humanize_timing
    {
        changes.push(DslChange::HumanizeChanged {
            velocity: new.globals.humanize.unwrap_or(0.0),
            timing: new.globals.humanize_timing.unwrap_or(0.0),
        });
    }
    // Every other global (scale, meter, sidechain, ...) = structural
    {
        let mut o = old.globals.clone();
        let mut n = new.globals.clone();
        o.tempo = 0.0; n.tempo = 0.0;
        o.swing = None; n.swing = None;
        o.humanize = None; n.humanize = None;
        o.humanize_timing = None; n.humanize_timing = None;
        if o != n {
            changes.push(DslChange::StructuralChange);
            return changes;
        }
    }

    // ── Instruments (graph-based) ──
    // Any change = structural (can't hot-swap graph topology)
    if old.instruments != new.instruments {
        changes.push(DslChange::StructuralChange);
        return changes;
    }

    // ── Module definitions ──
    if old.module_defs.len() != new.module_defs.len() {
        changes.push(DslChange::StructuralChange);
        return changes;
    }
    for (old_mod, new_mod) in old.module_defs.iter().zip(new.module_defs.iter()) {
        if old_mod.module_type != new_mod.module_type
            || old_mod.name != new_mod.name
            || old_mod.op_envelopes != new_mod.op_envelopes
        {
            changes.push(DslChange::StructuralChange);
            return changes;
        }
        // Changed or added params
        for new_param in &new_mod.params {
            let old_val = old_mod.params.iter()
                .find(|p| p.name == new_param.name)
                .map(|p| p.value);
            if old_val != Some(new_param.value) {
                changes.push(DslChange::ModuleParamChanged {
                    module_name: new_mod.name.clone(),
                    param_name: new_param.name.clone(),
                    value: new_param.value,
                });
            }
        }
        // Removed params go back to the registry default
        let kind = ModuleKind::from_str(&new_mod.module_type);
        for old_param in &old_mod.params {
            if new_mod.params.iter().any(|p| p.name == old_param.name) { continue; }
            match kind.and_then(|k| params::lookup(k, &old_param.name)) {
                Some(spec) => changes.push(DslChange::ModuleParamChanged {
                    module_name: new_mod.name.clone(),
                    param_name: old_param.name.clone(),
                    value: spec.default,
                }),
                None => {
                    changes.push(DslChange::StructuralChange);
                    return changes;
                }
            }
        }
    }

    // ── Patterns ──
    // Different number of patterns = structural
    if old.patterns.len() != new.patterns.len() {
        changes.push(DslChange::StructuralChange);
        return changes;
    }
    // Pattern name changes = structural
    for (old_pat, new_pat) in old.patterns.iter().zip(new.patterns.iter()) {
        // Any edit to a pattern's steps = structural (quantized hot-swap)
        if old_pat != new_pat {
            changes.push(DslChange::StructuralChange);
            return changes;
        }
    }

    // ── Tracks ──
    if old.tracks.len() != new.tracks.len() {
        changes.push(DslChange::StructuralChange);
        return changes;
    }
    for (old_track, new_track) in old.tracks.iter().zip(new.tracks.iter()) {
        if old_track.name != new_track.name ||
           old_track.using_instrument != new_track.using_instrument {
            changes.push(DslChange::StructuralChange);
            return changes;
        }
        // Routing, sends, sidechain, arp = structural
        if old_track.routing != new_track.routing
            || old_track.delay_send != new_track.delay_send
            || old_track.reverb_send != new_track.reverb_send
            || old_track.sidechain != new_track.sidechain
            || old_track.arp != new_track.arp
        {
            changes.push(DslChange::StructuralChange);
            return changes;
        }
        // Gate
        if old_track.gate != new_track.gate {
            changes.push(DslChange::TrackGateChanged {
                track_name: new_track.name.clone(),
                gate: new_track.gate.unwrap_or(0.85),
            });
        }
        // Which pattern a track plays is quantized to the bar like any other
        // edit to what is played (docs/DSL.md, livecoding semantics). It used
        // to switch instantly and restart the pattern from step 0 mid-bar.
        if old_track.play != new_track.play {
            changes.push(DslChange::StructuralChange);
            return changes;
        }
        // Level
        if old_track.level != new_track.level {
            changes.push(DslChange::TrackLevelChanged {
                track_name: new_track.name.clone(),
                level: new_track.level.unwrap_or(0.8),
            });
        }
        // Pan
        if old_track.pan != new_track.pan {
            changes.push(DslChange::TrackPanChanged {
                track_name: new_track.name.clone(),
                pan: new_track.pan.unwrap_or(0.0),
            });
        }
        // Velocity
        if old_track.velocity != new_track.velocity {
            changes.push(DslChange::TrackVelocityChanged {
                track_name: new_track.name.clone(),
                velocity: new_track.velocity.unwrap_or(0.8),
            });
        }
    }

    // ── Buses, Master, Scenes, Arrangement, Grooves ──
    // Any change in these = structural
    if old.buses != new.buses
        || old.bus_chains != new.bus_chains
        || old.master != new.master
        || old.scenes != new.scenes
        || old.arrangement != new.arrangement
        || old.grooves != new.grooves
    {
        changes.push(DslChange::StructuralChange);
        return changes;
    }

    changes
}

/// Check if any change in the list is structural.
pub fn has_structural_change(changes: &[DslChange]) -> bool {
    changes.iter().any(|c| matches!(c, DslChange::StructuralChange))
}
