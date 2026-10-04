//! A chain of effects in series, with a dry/wet per node: what a track's
//! inserts, a bus, the master and the send returns are all made of.

use alloc::vec::Vec;

use crate::graph::node::{ChainStep, NodeKind};

/// FX chain processing helper: a chain of NodeKind applied in series.
pub(super) struct FxChain {
    pub(super) nodes: Vec<NodeKind>,
    /// Dry/wet per node. 0 bypasses: the node is not processed at all, which
    /// is what makes a rig of switched-off effects free rather than merely
    /// silent. Kept beside the nodes rather than inside them so every node
    /// type gets it without knowing about it.
    wet: Vec<f32>,
    /// True while a node is bypassed, so entering bypass can clear it once.
    /// Without this a delay or reverb resumes with whatever it was holding
    /// when it was switched out, which arrives as a stale burst.
    bypassed: Vec<bool>,
}

impl FxChain {
    pub(super) fn new(steps: &[ChainStep]) -> Self {
        let nodes = steps.iter().map(|s| s.spec.instantiate()).collect();
        let wet = steps.iter().map(|s| s.wet).collect();
        let bypassed = steps.iter().map(|s| s.wet <= 0.0).collect();
        Self { nodes, wet, bypassed }
    }

    /// Set one node's dry/wet by position. Returns false if there is no such
    /// node, so a caller can tell a no-op from a real change.
    pub(super) fn set_wet(&mut self, idx: usize, value: f32) -> bool {
        let Some(slot) = self.wet.get_mut(idx) else { return false };
        *slot = value.clamp(0.0, 1.0);
        let now_off = *slot <= 0.0;
        if now_off && !self.bypassed[idx] {
            self.nodes[idx].reset();
        }
        self.bypassed[idx] = now_off;
        true
    }

    /// Process a stereo pair through the chain (for master bus).
    #[inline]
    pub(super) fn process_stereo(&mut self, l: f32, r: f32) -> (f32, f32) {
        let mut vl = l;
        let mut vr = r;
        for (i, node) in self.nodes.iter_mut().enumerate() {
            let wet = self.wet[i];
            // Bypassed: not processed at all. This is the whole point -- an
            // effect that is switched off has to cost nothing, or a rig with a
            // full chain on every voice is unaffordable.
            if wet <= 0.0 {
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
