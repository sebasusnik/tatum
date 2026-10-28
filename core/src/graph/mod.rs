pub mod node;
pub mod voice;

use node::{port, NodeKind, NodeSpec, MAX_NODE_INPUTS};
use crate::BLOCK_SIZE;

/// Maximum nodes in a single instrument graph.
pub const MAX_GRAPH_NODES: usize = 32;
// `GraphError::Cycle` keeps one bit per node.
const _: () = assert!(MAX_GRAPH_NODES <= 32);

/// Why an instrument graph cannot be built. The compiler turns each into an
/// error at the line that caused it; the panicking builder methods are for
/// graphs written in code, where any of these is a bug.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphError {
    /// More than `MAX_GRAPH_NODES` nodes.
    TooManyNodes,
    /// More than `MAX_NODE_INPUTS` connections into this node.
    TooManyInputs { node: u8 },
    /// Some nodes feed themselves through the others. Bit `i` is set for
    /// each node that could not be ordered: the loop and whatever it feeds.
    Cycle { stuck: u32 },
    /// No `out` node.
    NoOutput,
}

/// An edge: the source node, and the [`port`] it lands on at the other end.
#[derive(Clone, Copy)]
pub struct Edge {
    pub src_node: u8,
    pub port: u8,
}

impl Edge {
    pub const NONE: Self = Edge { src_node: 255, port: port::AUDIO };

    #[inline]
    pub fn is_connected(&self) -> bool {
        self.src_node != 255
    }
}

/// Graph topology + node creation specs. Shared across all voices.
/// No runtime DSP state — just the blueprint.
#[derive(Clone)]
pub struct GraphTemplate {
    pub specs: [NodeSpec; MAX_GRAPH_NODES],
    pub node_count: u8,
    pub edges: [[Edge; MAX_NODE_INPUTS]; MAX_GRAPH_NODES],
    pub input_counts: [u8; MAX_GRAPH_NODES],
    pub execution_order: [u8; MAX_GRAPH_NODES],
    pub exec_len: u8,
    pub output_node: u8,
    pub osc_nodes: [u8; MAX_GRAPH_NODES],
    pub osc_pitch_offsets: [f32; MAX_GRAPH_NODES], // semitones offset per osc
    pub osc_count: u8,
    pub env_nodes: [u8; MAX_GRAPH_NODES],
    pub env_count: u8,
    pub filter_nodes: [u8; MAX_GRAPH_NODES],
    pub filter_count: u8,
    pub output_gain: f32,
    /// Per node, one bit per [`port`] something is wired into; 0 for a node
    /// that only takes audio, which is then processed exactly as before ports.
    pub mod_masks: [u8; MAX_GRAPH_NODES],
}

impl GraphTemplate {
    /// Gather what flows into `node` at sample `s`: its audio inputs in the
    /// order they were wired, which is what the node reads, and the sum
    /// arriving at each modulation port. Returns how many audio inputs.
    #[inline]
    pub fn gather(
        &self,
        node: usize,
        bufs: &[[f32; BLOCK_SIZE]; MAX_GRAPH_NODES],
        s: usize,
        audio: &mut [f32; MAX_NODE_INPUTS],
        mods: &mut [f32; port::COUNT],
    ) -> u8 {
        let mut n = 0usize;
        for edge in self.edges[node][..self.input_counts[node] as usize].iter() {
            if !edge.is_connected() {
                continue;
            }
            let v = bufs[edge.src_node as usize][s];
            if edge.port == port::AUDIO {
                audio[n] = v;
                n += 1;
            } else {
                mods[edge.port as usize] += v;
            }
        }
        n as u8
    }

    /// Run every node once, for sample `s`, writing each one's output into
    /// `bufs`.
    #[inline]
    pub fn run_frame<N: AsMut<NodeKind>>(
        &self,
        bufs: &mut [[f32; BLOCK_SIZE]; MAX_GRAPH_NODES],
        s: usize,
        nodes: &mut [Option<N>; MAX_GRAPH_NODES],
    ) {
        for &idx in &self.execution_order[..self.exec_len as usize] {
            let node_idx = idx as usize;
            let mut audio = [0.0f32; MAX_NODE_INPUTS];
            let mut mods = [0.0f32; port::COUNT];
            let n = self.gather(node_idx, bufs, s, &mut audio, &mut mods);
            if let Some(node) = nodes[node_idx].as_mut() {
                let node = node.as_mut();
                let mask = self.mod_masks[node_idx];
                if mask != 0 {
                    node.modulate(&mods, mask);
                }
                bufs[node_idx][s] = node.process(&audio, n);
            }
        }
    }

    /// Instantiate fresh runtime nodes for a voice.
    pub fn create_nodes(&self) -> [Option<NodeKind>; MAX_GRAPH_NODES] {
        let mut nodes: [Option<NodeKind>; MAX_GRAPH_NODES] = core::array::from_fn(|_| None);
        let n = self.node_count as usize;
        for (node, spec) in nodes[..n].iter_mut().zip(self.specs[..n].iter()) {
            *node = Some(spec.instantiate());
        }
        nodes
    }
}

/// Runtime graph: per-voice node state.
pub struct Graph {
    pub nodes: [Option<NodeKind>; MAX_GRAPH_NODES],
    pub buffers: [[f32; BLOCK_SIZE]; MAX_GRAPH_NODES],
}

impl Default for Graph {
    fn default() -> Self {
        Self::new()
    }
}

impl Graph {
    pub fn new() -> Self {
        Self { nodes: core::array::from_fn(|_| None), buffers: [[0.0; BLOCK_SIZE]; MAX_GRAPH_NODES] }
    }

    /// Create from a template with fresh node instances.
    pub fn from_template(template: &GraphTemplate) -> Self {
        Self { nodes: template.create_nodes(), buffers: [[0.0; BLOCK_SIZE]; MAX_GRAPH_NODES] }
    }

    /// Re-initialize nodes from template (for voice reuse).
    pub fn reinit(&mut self, template: &GraphTemplate) {
        self.nodes = template.create_nodes();
    }

    /// Process one block. Writes output to `output`.
    pub fn process_block(&mut self, template: &GraphTemplate, output: &mut [f32]) {
        let len = output.len().min(BLOCK_SIZE);

        for s in 0..len {
            template.run_frame(&mut self.buffers, s, &mut self.nodes);
        }

        // Copy output
        let out_idx = template.output_node as usize;
        output[..len].copy_from_slice(&self.buffers[out_idx][..len]);
    }

    pub fn gate_on(&mut self, template: &GraphTemplate) {
        for i in 0..template.env_count as usize {
            let idx = template.env_nodes[i] as usize;
            if let Some(ref mut node) = self.nodes[idx] {
                node.gate_on();
            }
        }
        for i in 0..template.filter_count as usize {
            let idx = template.filter_nodes[i] as usize;
            if let Some(ref mut node) = self.nodes[idx] {
                node.gate_on();
            }
        }
    }

    pub fn gate_off(&mut self, template: &GraphTemplate) {
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

    pub fn set_frequency(&mut self, template: &GraphTemplate, freq: f32) {
        for i in 0..template.osc_count as usize {
            let idx = template.osc_nodes[i] as usize;
            if let Some(ref mut node) = self.nodes[idx] {
                let offset = template.osc_pitch_offsets[i];
                let f = if offset != 0.0 { freq * crate::math::pow2(offset / 12.0) } else { freq };
                node.set_frequency(f);
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

    pub fn reset(&mut self) {
        for n in self.nodes.iter_mut().flatten() {
            n.reset();
        }
    }
}

// ── Graph Builder ──

pub struct GraphBuilder {
    specs: [NodeSpec; MAX_GRAPH_NODES],
    edges: [[Edge; MAX_NODE_INPUTS]; MAX_GRAPH_NODES],
    input_counts: [u8; MAX_GRAPH_NODES],
    node_count: u8,
    output_node: Option<u8>,
}

impl Default for GraphBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl GraphBuilder {
    pub fn new() -> Self {
        Self {
            specs: [NodeSpec::Output; MAX_GRAPH_NODES], // dummy default
            edges: [[Edge::NONE; MAX_NODE_INPUTS]; MAX_GRAPH_NODES],
            input_counts: [0; MAX_GRAPH_NODES],
            node_count: 0,
            output_node: None,
        }
    }

    /// Add a node, returns its index.
    pub fn add_node(&mut self, spec: NodeSpec) -> u8 {
        self.try_add_node(spec).expect("graph full")
    }

    /// Add a node, or say the graph is full.
    pub fn try_add_node(&mut self, spec: NodeSpec) -> Result<u8, GraphError> {
        let idx = self.node_count;
        if idx as usize >= MAX_GRAPH_NODES {
            return Err(GraphError::TooManyNodes);
        }
        if matches!(spec, NodeSpec::Output) {
            self.output_node = Some(idx);
        }
        self.specs[idx as usize] = spec;
        self.node_count += 1;
        Ok(idx)
    }

    /// Connect src output -> dst's next available input.
    pub fn connect(&mut self, src: u8, dst: u8) {
        self.try_connect(src, dst).expect("too many inputs on a node")
    }

    /// Connect, or say `dst` has no input left.
    pub fn try_connect(&mut self, src: u8, dst: u8) -> Result<(), GraphError> {
        self.try_connect_port(src, dst, port::AUDIO)
    }

    /// Connect `src` into one of `dst`'s [`port`]s.
    pub fn try_connect_port(&mut self, src: u8, dst: u8, port: u8) -> Result<(), GraphError> {
        let d = dst as usize;
        let inp = self.input_counts[d] as usize;
        if inp >= MAX_NODE_INPUTS {
            return Err(GraphError::TooManyInputs { node: dst });
        }
        self.edges[d][inp] = Edge { src_node: src, port };
        self.input_counts[d] += 1;
        Ok(())
    }

    /// Build the GraphTemplate with topological sort.
    pub fn build(self) -> GraphTemplate {
        match self.try_build() {
            Ok(t) => t,
            Err(e) => panic!("cannot build graph: {:?}", e),
        }
    }

    /// Build, or say why the graph cannot run: no output, or a loop.
    pub fn try_build(self) -> Result<GraphTemplate, GraphError> {
        let output_node = self.output_node.ok_or(GraphError::NoOutput)?;
        let node_count = self.node_count;

        // Identify osc/env/filter nodes
        let mut osc_nodes = [0u8; MAX_GRAPH_NODES];
        let mut osc_pitch_offsets = [0.0f32; MAX_GRAPH_NODES];
        let mut osc_count = 0u8;
        let mut env_nodes = [0u8; MAX_GRAPH_NODES];
        let mut env_count = 0u8;
        let mut filter_nodes = [0u8; MAX_GRAPH_NODES];
        let mut filter_count = 0u8;

        for i in 0..node_count as usize {
            if self.specs[i].is_oscillator() {
                let offset =
                    if let NodeSpec::Osc { pitch_semitones, .. } = self.specs[i] { pitch_semitones } else { 0.0 };
                osc_nodes[osc_count as usize] = i as u8;
                osc_pitch_offsets[osc_count as usize] = offset;
                osc_count += 1;
            }
            if self.specs[i].is_envelope() {
                env_nodes[env_count as usize] = i as u8;
                env_count += 1;
            }
            if self.specs[i].has_filter_env() {
                filter_nodes[filter_count as usize] = i as u8;
                filter_count += 1;
            }
        }

        // Topological sort (Kahn's algorithm)
        let mut in_degree = [0u16; MAX_GRAPH_NODES];
        for (dst, degree) in in_degree[..node_count as usize].iter_mut().enumerate() {
            for inp in 0..self.input_counts[dst] as usize {
                if self.edges[dst][inp].is_connected() {
                    *degree += 1;
                }
            }
        }

        // Adjacency: src -> [dependents]
        let mut dep_list: [[u8; MAX_GRAPH_NODES]; MAX_GRAPH_NODES] = [[0; MAX_GRAPH_NODES]; MAX_GRAPH_NODES];
        let mut dep_counts = [0u8; MAX_GRAPH_NODES];

        for dst in 0..node_count as usize {
            for inp in 0..self.input_counts[dst] as usize {
                let edge = &self.edges[dst][inp];
                if edge.is_connected() {
                    let src = edge.src_node as usize;
                    let dc = dep_counts[src] as usize;
                    dep_list[src][dc] = dst as u8;
                    dep_counts[src] += 1;
                }
            }
        }

        let mut queue = [0u8; MAX_GRAPH_NODES];
        let mut q_head = 0usize;
        let mut q_tail = 0usize;

        for (i, &degree) in in_degree[..node_count as usize].iter().enumerate() {
            if degree == 0 {
                queue[q_tail] = i as u8;
                q_tail += 1;
            }
        }

        let mut execution_order = [0u8; MAX_GRAPH_NODES];
        let mut exec_len = 0u8;

        while q_head < q_tail {
            let node = queue[q_head];
            q_head += 1;
            execution_order[exec_len as usize] = node;
            exec_len += 1;

            for &d in dep_list[node as usize][..dep_counts[node as usize] as usize].iter() {
                let dep = d as usize;
                in_degree[dep] -= 1;
                if in_degree[dep] == 0 {
                    queue[q_tail] = dep as u8;
                    q_tail += 1;
                }
            }
        }

        if exec_len != node_count {
            // One bit per node; there is at least the output, and at most 32.
            let mut stuck = u32::MAX >> (32 - node_count as u32);
            for &n in &execution_order[..exec_len as usize] {
                stuck &= !(1 << n);
            }
            return Err(GraphError::Cycle { stuck });
        }

        let mut mod_masks = [0u8; MAX_GRAPH_NODES];
        for (mask, (edges, &count)) in mod_masks.iter_mut().zip(self.edges.iter().zip(self.input_counts.iter())) {
            for e in &edges[..count as usize] {
                if e.port != port::AUDIO {
                    *mask |= port::bit(e.port);
                }
            }
        }

        Ok(GraphTemplate {
            specs: self.specs,
            node_count,
            edges: self.edges,
            input_counts: self.input_counts,
            execution_order,
            exec_len,
            output_node,
            osc_nodes,
            osc_pitch_offsets,
            osc_count,
            env_nodes,
            env_count,
            filter_nodes,
            filter_count,
            output_gain: 1.0,
            mod_masks,
        })
    }
}
