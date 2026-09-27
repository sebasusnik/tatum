//! Values a performer's hands put on the engine. A knob or fader that has
//! been moved holds its target where it was left: a scene that starts, an
//! `auto` lane on the same thing, or a save that rebuilds the engine does not
//! move it back. Editing the value in the text lets go of it, so the edit
//! plays: the last hand wins.

use super::automation::AutoTarget;
use super::SongEngine;
use crate::live::FastOp;

/// How many different things the hands can hold at once. A KeyLab has
/// eighteen knobs and faders; a macro moves a few things each.
pub const MAX_HELD: usize = 64;

/// Whether two ops set the same thing, whatever the value.
fn same_target(a: &FastOp, b: &FastOp) -> bool {
    use FastOp::*;
    match (a, b) {
        (Tempo(_), Tempo(_)) | (Swing(_), Swing(_)) | (Humanize { .. }, Humanize { .. }) => true,
        (ReverbMix(_), ReverbMix(_)) | (DelayMix(_), DelayMix(_)) | (ReverbFreeze(_), ReverbFreeze(_)) => true,
        (TrackLevel { track: x, .. }, TrackLevel { track: y, .. })
        | (TrackPan { track: x, .. }, TrackPan { track: y, .. })
        | (TrackVelocity { track: x, .. }, TrackVelocity { track: y, .. })
        | (TrackGate { track: x, .. }, TrackGate { track: y, .. }) => x == y,
        (NodeWet { track: t, node: n, .. }, NodeWet { track: u, node: m, .. }) => t == u && n == m,
        (TrackSend { track: t, reverb: r, .. }, TrackSend { track: u, reverb: s, .. }) => t == u && r == s,
        (NodeParam { track: t, node: n, param: p, .. }, NodeParam { track: u, node: m, param: q, .. }) => {
            t == u && n == m && p == q
        }
        (ModuleParam { instrument: i, id: p, .. }, ModuleParam { instrument: j, id: q, .. }) => i == j && p == q,
        _ => false,
    }
}

/// Notes and the pitch strip are events and positions of the keys, not
/// values a scene would overwrite.
fn holdable(op: &FastOp) -> bool {
    !matches!(op, FastOp::NoteOn { .. } | FastOp::NoteOff { .. } | FastOp::PitchBend { .. })
}

impl SongEngine {
    /// Apply `op` and keep it: it is put back whenever a scene or a restart
    /// would move its target.
    pub fn hold(&mut self, op: FastOp) {
        if holdable(&op) {
            let slot = self.held[..self.held_count].iter().position(|h| same_target(h, &op));
            match slot {
                Some(i) => self.held[i] = op,
                None if self.held_count < MAX_HELD => {
                    self.held[self.held_count] = op;
                    self.held_count += 1;
                }
                // Full: it still applies, it just is not held.
                None => {}
            }
        }
        crate::live::apply_op(self, op);
    }

    /// Let go of whatever holds the target of `op`, and apply it. A value
    /// written in the text arrives this way.
    pub fn release(&mut self, op: FastOp) {
        if let Some(i) = self.held[..self.held_count].iter().position(|h| same_target(h, &op)) {
            self.held.copy_within(i + 1..self.held_count, i);
            self.held_count -= 1;
        }
        crate::live::apply_op(self, op);
    }

    /// Put every held value back, after something that sets them from the
    /// text: a scene starting, the engine starting mid-song.
    pub(super) fn reassert_held(&mut self) {
        for i in 0..self.held_count {
            crate::live::apply_op(self, self.held[i]);
        }
    }

    /// Whether the hands hold what an `auto` lane would move. A held knob
    /// wins: the lane stays quiet on it for the rest of the scene.
    pub(super) fn holds_auto(&self, target: &AutoTarget) -> bool {
        self.held[..self.held_count].iter().any(|h| match (target, h) {
            (AutoTarget::TrackLevel { track_idx }, FastOp::TrackLevel { track, .. }) => track_idx == track,
            (AutoTarget::TrackNodeWet { track_idx, node_idx }, FastOp::NodeWet { track, node, .. }) => {
                track_idx == track && node_idx == node
            }
            (AutoTarget::ReverbMix, FastOp::ReverbMix(_)) => true,
            (AutoTarget::MasterParam { param_name }, FastOp::NodeParam { track: None, param, .. }) => {
                param_name.as_str() == *param
            }
            (AutoTarget::DelayMix, FastOp::DelayMix(_)) => true,
            (AutoTarget::ReverbFreeze, FastOp::ReverbFreeze(_)) => true,
            (
                AutoTarget::InstrumentParam { instrument_idx, param_name },
                FastOp::ModuleParam { instrument, id, .. },
            ) => {
                let same_module = self.instrument_names.get(*instrument_idx) == self.instrument_names.get(*instrument);
                same_module && self.param_id_of(*instrument, param_name.as_str()) == Some(*id)
            }
            _ => false,
        })
    }

    fn param_id_of(&self, instrument: usize, name: &str) -> Option<crate::params::ParamId> {
        self.instruments.get(instrument)?.param_id(name)
    }

    /// The whole list of what the hands hold, oldest first.
    pub fn held(&self) -> &[FastOp] {
        &self.held[..self.held_count]
    }
}
