extern crate alloc;
use alloc::boxed::Box;

use crate::graph::{GraphTemplate, MAX_GRAPH_NODES};
use crate::graph::node::{NodeKind, MAX_NODE_INPUTS};
use crate::math;
use crate::{BLOCK_SIZE, MAX_VOICES};

/// A single voice: owns per-voice DSP node state on the heap.
pub struct Voice {
    pub nodes: [Option<Box<NodeKind>>; MAX_GRAPH_NODES],
    pub note: u8,
    pub velocity: f32,
    pub active: bool,
    pub age: u32,
}

impl Voice {
    pub fn new() -> Self {
        Self {
            nodes: core::array::from_fn(|_| None),
            note: 0,
            velocity: 0.0,
            active: false,
            age: 0,
        }
    }

    /// Initialize this voice for a new note.
    pub fn init(&mut self, template: &GraphTemplate, note: u8, velocity: f32) {
        for i in 0..template.node_count as usize {
            self.nodes[i] = Some(Box::new(template.specs[i].instantiate()));
        }
        for i in template.node_count as usize..MAX_GRAPH_NODES {
            self.nodes[i] = None;
        }
        self.note = note;
        self.velocity = velocity;
        self.active = true;
        self.age = 0;

        let freq = math::midi_to_freq(note);
        for i in 0..template.osc_count as usize {
            let idx = template.osc_nodes[i] as usize;
            if let Some(ref mut node) = self.nodes[idx] {
                let offset = template.osc_pitch_offsets[i];
                let f = if offset != 0.0 { freq * math::pow2(offset / 12.0) } else { freq };
                node.set_frequency(f);
            }
        }

        for i in 0..template.env_count as usize {
            let idx = template.env_nodes[i] as usize;
            if let Some(ref mut node) = self.nodes[idx] {
                node.gate_on();
            }
        }

        // Gate on + set velocity for filter envelope nodes
        for i in 0..template.filter_count as usize {
            let idx = template.filter_nodes[i] as usize;
            if let Some(ref mut node) = self.nodes[idx] {
                node.set_velocity(velocity);
                node.gate_on();
            }
        }
    }

    pub fn release(&mut self, template: &GraphTemplate) {
        for i in 0..template.env_count as usize {
            let idx = template.env_nodes[i] as usize;
            if let Some(ref mut node) = self.nodes[idx] {
                node.gate_off();
            }
        }
        for i in 0..template.filter_count as usize {
            let idx = template.filter_nodes[i] as usize;
            if let Some(ref mut node) = self.nodes[idx] {
                node.gate_off();
            }
        }
    }

    pub fn all_envelopes_idle(&self, template: &GraphTemplate) -> bool {
        if template.env_count == 0 {
            return true;
        }
        for i in 0..template.env_count as usize {
            let idx = template.env_nodes[i] as usize;
            if let Some(ref node) = self.nodes[idx] {
                if !node.is_idle() {
                    return false;
                }
            }
        }
        true
    }
}

/// Polyphonic instrument: template + voice pool + shared processing buffers.
pub struct Instrument {
    pub template: GraphTemplate,
    voices: [Voice; MAX_VOICES],
    /// Per-node sample buffer, heap-allocated to avoid stack overflow.
    node_bufs: Box<[[f32; BLOCK_SIZE]; MAX_GRAPH_NODES]>,
    /// Staged p-lock values, applied on next note_on().
    staged_cutoff: Option<f32>,
    staged_env_depth: Option<f32>,
    staged_resonance: Option<f32>,
}

impl Instrument {
    pub fn new(template: GraphTemplate) -> Self {
        Self {
            template,
            voices: core::array::from_fn(|_| Voice::new()),
            node_bufs: Box::new([[0.0; BLOCK_SIZE]; MAX_GRAPH_NODES]),
            staged_cutoff: None,
            staged_env_depth: None,
            staged_resonance: None,
        }
    }

    /// Stage per-step parameter locks to be applied on the next note_on().
    pub fn stage_plock(&mut self, cutoff: Option<f32>, env_depth: Option<f32>, resonance: Option<f32>) {
        self.staged_cutoff = cutoff;
        self.staged_env_depth = env_depth;
        self.staged_resonance = resonance;
    }

    pub fn note_on(&mut self, note: u8, velocity: f32) {
        let mut slot = None;
        for i in 0..MAX_VOICES {
            if !self.voices[i].active {
                slot = Some(i);
                break;
            }
        }

        if slot.is_none() {
            let mut oldest_age = 0u32;
            let mut oldest_idx = 0;
            for i in 0..MAX_VOICES {
                if self.voices[i].age > oldest_age {
                    oldest_age = self.voices[i].age;
                    oldest_idx = i;
                }
            }
            slot = Some(oldest_idx);
        }

        let idx = slot.unwrap();
        self.voices[idx].init(&self.template, note, velocity);

        // Apply staged p-lock values to filter nodes on the newly initialized voice
        let has_plocks = self.staged_cutoff.is_some()
            || self.staged_env_depth.is_some()
            || self.staged_resonance.is_some();

        if has_plocks {
            for i in 0..self.template.filter_count as usize {
                let node_idx = self.template.filter_nodes[i] as usize;
                if let Some(ref mut node) = self.voices[idx].nodes[node_idx] {
                    match node.as_mut() {
                        NodeKind::Ladder(m) => {
                            if let Some(cutoff) = self.staged_cutoff {
                                m.base_cutoff = cutoff;
                            }
                            if let Some(depth) = self.staged_env_depth {
                                m.env_depth = depth;
                            }
                            if let Some(res) = self.staged_resonance {
                                let cutoff = self.staged_cutoff.unwrap_or(m.base_cutoff);
                                m.filter.set_params(cutoff, res);
                            }
                        }
                        NodeKind::Biquad(m) => {
                            if let Some(cutoff) = self.staged_cutoff {
                                m.base_cutoff = cutoff;
                            }
                            if let Some(depth) = self.staged_env_depth {
                                m.env_depth = depth;
                            }
                            if let Some(res) = self.staged_resonance {
                                let cutoff = self.staged_cutoff.unwrap_or(m.base_cutoff);
                                m.filter.set_resonance(cutoff, res);
                            }
                        }
                        _ => {}
                    }
                }
            }

            // Clear staging
            self.staged_cutoff = None;
            self.staged_env_depth = None;
            self.staged_resonance = None;
        }
    }

    pub fn note_off(&mut self, note: u8) {
        for voice in self.voices.iter_mut() {
            if voice.active && voice.note == note {
                voice.release(&self.template);
                break;
            }
        }
    }

    /// Process one block. Sums all active voices into `output`.
    pub fn process_block(&mut self, output: &mut [f32]) {
        let len = output.len().min(BLOCK_SIZE);

        for s in output[..len].iter_mut() {
            *s = 0.0;
        }

        let template = &self.template;

        for voice in self.voices.iter_mut() {
            if !voice.active {
                continue;
            }

            // Process voice sample by sample using shared node buffers
            for s in 0..len {
                for ei in 0..template.exec_len as usize {
                    let node_idx = template.execution_order[ei] as usize;
                    let input_count = template.input_counts[node_idx];

                    let mut input_vals = [0.0f32; MAX_NODE_INPUTS];
                    for inp in 0..input_count as usize {
                        let edge = &template.edges[node_idx][inp];
                        if edge.is_connected() {
                            input_vals[inp] = self.node_bufs[edge.src_node as usize][s];
                        }
                    }

                    if let Some(ref mut node) = voice.nodes[node_idx] {
                        self.node_bufs[node_idx][s] = node.process(&input_vals, input_count);
                    }
                }
            }

            // Mix output into buffer
            let out_idx = template.output_node as usize;
            let vel = voice.velocity * template.output_gain;
            for s in 0..len {
                output[s] += self.node_bufs[out_idx][s] * vel;
            }

            voice.age += 1;

            if voice.all_envelopes_idle(template) {
                voice.active = false;
            }
        }
    }

    pub fn all_notes_off(&mut self) {
        for voice in self.voices.iter_mut() {
            if voice.active {
                voice.release(&self.template);
            }
        }
    }

    pub fn reset(&mut self) {
        for voice in self.voices.iter_mut() {
            voice.active = false;
        }
    }
}
