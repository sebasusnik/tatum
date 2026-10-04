//! A chain of effects in series, with a dry/wet per node: what a track's
//! inserts, a bus, the master and the send returns are all made of.

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::graph::node::{ChainStep, NodeKind};

/// How far a wet moves in a sample: across its whole travel in 10 ms.
const WET_STEP: f32 = 1.0 / (0.010 * crate::SAMPLE_RATE);

/// FX chain processing helper: a chain of NodeKind applied in series.
pub(super) struct FxChain {
    pub(super) nodes: Vec<Box<NodeKind>>,
    /// Dry/wet per node. 0 bypasses: the node is not processed at all, which
    /// is what makes a rig of switched-off effects free rather than merely
    /// silent. Kept beside the nodes rather than inside them so every node
    /// type gets it without knowing about it.
    wet: Vec<f32>,
    /// Where each wet is going: a knob or the wheel sets this, and `wet`
    /// follows it over a few milliseconds, so a fast sweep does not step.
    target: Vec<f32>,
    /// True while a node is bypassed, so entering bypass can clear it once.
    /// Without this a delay or reverb resumes with whatever it was holding
    /// when it was switched out, which arrives as a stale burst.
    bypassed: Vec<bool>,
}

impl FxChain {
    pub(super) fn new(steps: &[ChainStep]) -> Self {
        let nodes = steps.iter().map(|s| Box::new(s.spec.instantiate())).collect();
        let wet: Vec<f32> = steps.iter().map(|s| s.wet).collect();
        let bypassed = steps.iter().map(|s| s.wet <= 0.0).collect();
        Self { nodes, target: wet.clone(), wet, bypassed }
    }

    /// Set one node's dry/wet by position. Returns false if there is no such
    /// node, so a caller can tell a no-op from a real change.
    pub(super) fn set_wet(&mut self, idx: usize, value: f32) -> bool {
        let Some(slot) = self.target.get_mut(idx) else { return false };
        *slot = value.clamp(0.0, 1.0);
        // Coming out of bypass: the node was cleared going in, and the wet
        // ramps up from nothing.
        if *slot > 0.0 && self.bypassed[idx] {
            self.bypassed[idx] = false;
            self.wet[idx] = 0.0;
        }
        true
    }

    /// Process a stereo pair through the chain (for master bus).
    #[inline]
    pub(super) fn process_stereo(&mut self, l: f32, r: f32) -> (f32, f32) {
        let mut vl = l;
        let mut vr = r;
        for (i, node) in self.nodes.iter_mut().enumerate() {
            let target = self.target[i];
            if self.wet[i] != target {
                let d = target - self.wet[i];
                self.wet[i] = if d.abs() <= WET_STEP { target } else { self.wet[i] + WET_STEP * d.signum() };
            }
            let wet = self.wet[i];
            // Bypassed: not processed at all. This is the whole point -- an
            // effect that is switched off has to cost nothing, or a rig with a
            // full chain on every voice is unaffordable. Arriving there, it
            // is cleared, so it does not resume with a stale burst.
            if wet <= 0.0 {
                if target <= 0.0 && !self.bypassed[i] {
                    node.reset();
                    self.bypassed[i] = true;
                }
                continue;
            }
            let (wl, wr) = node.process_stereo(vl, vr);
            if wet >= 1.0 {
                (vl, vr) = (wl, wr);
            } else {
                vl += (wl - vl) * wet;
                vr += (wr - vr) * wet;
            }
        }
        (vl, vr)
    }

    pub(super) fn reset(&mut self) {
        for node in self.nodes.iter_mut() {
            node.reset();
        }
    }

    pub(super) fn set_bpm(&mut self, bpm: f32) {
        for node in self.nodes.iter_mut() {
            node.set_bpm(bpm);
        }
    }

    /// Set a named parameter on the node at `idx`. False if there is no such
    /// node, or it has no such parameter.
    pub(super) fn set_node_param(&mut self, idx: usize, name: &str, value: f32) -> bool {
        self.nodes.get_mut(idx).is_some_and(|n| n.glide_named(name, value))
    }

    /// Set a named parameter on the first node that has it.
    pub(super) fn set_param(&mut self, name: &str, value: f32) -> bool {
        self.nodes.iter_mut().any(|n| n.set_named(name, value))
    }
}
