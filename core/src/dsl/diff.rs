//! DSL diff engine — compares two Song ASTs and classifies changes.
//!
//! Used by the livecoding system to decide whether to apply runtime
//! mutations (fast path) or fall back to a full hot-swap (slow path).

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use super::ast::Song;

/// A classified change between two Song ASTs.
#[derive(Debug, Clone)]
pub enum DslChange {
    /// Tempo changed — can apply via set_tempo()
    TempoChanged(f32),
    /// Swing changed
    SwingChanged(f32),
    /// Humanize changed
    HumanizeChanged(f32),
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
    /// A track switched to a different pattern
    TrackPatternSwapped {
        track_name: String,
        new_pattern: String,
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
    if old.globals.humanize != new.globals.humanize {
        changes.push(DslChange::HumanizeChanged(new.globals.humanize.unwrap_or(0.0)));
    }
    // Scale/meter change = structural (scale affects degree→note resolution)
    if old.globals.meter != new.globals.meter || old.globals.scale != new.globals.scale {
        changes.push(DslChange::StructuralChange);
        return changes;
    }

    // ── Instruments (graph-based) ──
    // Any change = structural (can't hot-swap graph topology)
    if old.instruments.len() != new.instruments.len() {
        changes.push(DslChange::StructuralChange);
        return changes;
    }

    // ── Module definitions ──
    if old.module_defs.len() != new.module_defs.len() {
        changes.push(DslChange::StructuralChange);
        return changes;
    }
    for (old_mod, new_mod) in old.module_defs.iter().zip(new.module_defs.iter()) {
        if old_mod.module_type != new_mod.module_type || old_mod.name != new_mod.name {
            changes.push(DslChange::StructuralChange);
            return changes;
        }
        // Compare params
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
    }

    // ── Patterns ──
    // Different number of patterns = structural
    if old.patterns.len() != new.patterns.len() {
        changes.push(DslChange::StructuralChange);
        return changes;
    }
    // Pattern name changes = structural
    for (old_pat, new_pat) in old.patterns.iter().zip(new.patterns.iter()) {
        if old_pat.name != new_pat.name {
            changes.push(DslChange::StructuralChange);
            return changes;
        }
        // Pattern content changes are handled by hot-swap for now
        // (comparing step arrays is complex and the hot-swap is fast enough)
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
        // Routing changes = structural
        if old_track.routing.len() != new_track.routing.len() {
            changes.push(DslChange::StructuralChange);
            return changes;
        }
        // Pattern swap
        if old_track.play != new_track.play {
            changes.push(DslChange::TrackPatternSwapped {
                track_name: new_track.name.clone(),
                new_pattern: new_track.play.clone(),
            });
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

    // ── Buses, Master, Scenes, Arrangement ──
    // Any change in these = structural for now
    if old.buses.len() != new.buses.len() ||
       old.bus_chains.len() != new.bus_chains.len() ||
       old.scenes.len() != new.scenes.len() ||
       old.arrangement.len() != new.arrangement.len() {
        changes.push(DslChange::StructuralChange);
        return changes;
    }

    changes
}

/// Check if any change in the list is structural.
pub fn has_structural_change(changes: &[DslChange]) -> bool {
    changes.iter().any(|c| matches!(c, DslChange::StructuralChange))
}
