extern crate alloc;
use alloc::boxed::Box;

use crate::graph::{GraphTemplate, MAX_GRAPH_NODES};
use crate::graph::node::NodeKind;
use crate::math;
use crate::{BLOCK_SIZE, MAX_VOICES};

/// A single voice: owns per-voice DSP node state on the heap.
pub struct Voice {
    pub nodes: [Option<Box<NodeKind>>; MAX_GRAPH_NODES],
    pub note: u8,
    pub velocity: f32,
    pub active: bool,
    pub age: u32,
    /// Between its note-on and its note-off: a voice that is only ringing
    /// out is nobody's to slide.
    held: bool,
    /// The pitch the oscillators are at, in Hz, and the slide towards the
    /// next: the ratio it is multiplied by each sample, how many samples are
    /// left, and where it lands.
    freq: f32,
    glide_ratio: f32,
    glide_left: u32,
    glide_to: f32,
}

impl Default for Voice {
    fn default() -> Self {
        Self::new()
    }
}

impl Voice {
    pub fn new() -> Self {
        Self {
            nodes: core::array::from_fn(|_| None),
            note: 0,
            velocity: 0.0,
            active: false,
            age: 0,
            held: false,
            freq: 0.0,
            glide_ratio: 1.0,
            glide_left: 0,
            glide_to: 0.0,
        }
    }

    /// The voice's nodes, allocated once for `template`: a note renews them
    /// in place, so playing never allocates. Each note used to box a fresh
    /// node per node of the graph, running the allocator on the audio thread
    /// thousands of times a minute.
    pub fn allocate(&mut self, template: &GraphTemplate) {
        for i in 0..MAX_GRAPH_NODES {
            self.nodes[i] = (i < template.node_count as usize).then(|| Box::new(template.specs[i].instantiate()));
        }
    }

    /// Initialize this voice for a new note.
    pub fn init(&mut self, template: &GraphTemplate, note: u8, velocity: f32) {
        for i in 0..template.node_count as usize {
            match self.nodes[i] {
                Some(ref mut node) => node.renew(&template.specs[i]),
                // A voice that was never allocated for this template.
                None => self.nodes[i] = Some(Box::new(template.specs[i].instantiate())),
            }
        }
        for node in self.nodes[template.node_count as usize..].iter_mut() {
            *node = None;
        }
        self.note = note;
        self.velocity = velocity;
        self.active = true;
        self.age = 0;
        self.held = true;

        self.glide_left = 0;
        self.tune(template, math::midi_to_freq(note));

        for &n in &template.env_nodes[..template.env_count as usize] {
            let idx = n as usize;
            if let Some(ref mut node) = self.nodes[idx] {
                node.gate_on();
            }
        }

        // Gate on + set velocity for filter envelope nodes
        for &n in &template.filter_nodes[..template.filter_count as usize] {
            let idx = n as usize;
            if let Some(ref mut node) = self.nodes[idx] {
                node.set_velocity(velocity);
                node.gate_on();
            }
        }
    }

    /// Put every oscillator on `freq`, each at its own offset from it.
    fn tune(&mut self, template: &GraphTemplate, freq: f32) {
        self.freq = freq;
        for i in 0..template.osc_count as usize {
            let idx = template.osc_nodes[i] as usize;
            if let Some(ref mut node) = self.nodes[idx] {
                let offset = template.osc_pitch_offsets[i];
                let f = if offset != 0.0 { freq * math::pow2(offset / 12.0) } else { freq };
                node.set_frequency(f);
            }
        }
    }

    /// Slide to `note` over the template's glide, in a straight line in
    /// pitch, with nothing started again: the envelopes carry on.
    fn slide(&mut self, template: &GraphTemplate, note: u8) {
        self.note = note;
        self.glide_to = math::midi_to_freq(note);
        self.glide_left = template.glide_samples.max(1);
        self.glide_ratio = math::pow(self.glide_to / self.freq.max(1.0), 1.0 / self.glide_left as f32);
    }

    /// One sample of a slide.
    #[inline]
    fn glide(&mut self, template: &GraphTemplate) {
        self.glide_left -= 1;
        let freq = if self.glide_left == 0 { self.glide_to } else { self.freq * self.glide_ratio };
        self.tune(template, freq);
    }

    /// Stop at once and forget everything: every node as it was built, with
    /// none of what the note left in it. No allocation, as for a new note.
    pub fn clear(&mut self, template: &GraphTemplate) {
        for (node, spec) in self.nodes.iter_mut().zip(template.specs[..template.node_count as usize].iter()) {
            if let Some(node) = node {
                node.renew(spec);
            }
        }
        self.active = false;
        self.held = false;
    }

    pub fn release(&mut self, template: &GraphTemplate) {
        self.held = false;
        for &n in &template.env_nodes[..template.env_count as usize] {
            let idx = n as usize;
            if let Some(ref mut node) = self.nodes[idx] {
                node.gate_off();
            }
        }
        for &n in &template.filter_nodes[..template.filter_count as usize] {
            let idx = n as usize;
            if let Some(ref mut node) = self.nodes[idx] {
                node.gate_off();
            }
        }
    }

    pub fn all_envelopes_idle(&self, template: &GraphTemplate) -> bool {
        if template.env_count == 0 {
            return true;
        }
        for &n in &template.env_nodes[..template.env_count as usize] {
            let idx = n as usize;
            if template.hold_mask & (1 << n) == 0 {
                continue;
            }
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
    /// Notes started so far: the hit number a drum node with `vary=` sways
    /// by, and a `rand` or a `dust` draws from.
    hits: u32,
    /// Samples since the start: where a `wander` is on its curve. The song
    /// engine sets it to the song's own count, so it runs on while the
    /// track is not rendered; alone, an instrument counts what it plays.
    clock: u32,
}

impl Instrument {
    /// True when nothing is sounding and nothing is still releasing. A track
    /// whose level is 0 and whose instrument is idle is skipped whole by the
    /// song engine: no voices, no insert chain, no mix. That is what makes a
    /// rig of muted voices waiting to be brought in affordable, since `level`
    /// is a fast-path edit while scene membership is a structural one.
    pub fn is_idle(&self) -> bool {
        !self.voices.iter().any(|v| v.active)
    }
    pub fn new(template: GraphTemplate) -> Self {
        let mut voices: [Voice; MAX_VOICES] = core::array::from_fn(|_| Voice::new());
        for v in voices.iter_mut() {
            v.allocate(&template);
        }
        Self {
            template,
            voices,
            node_bufs: Box::new([[0.0; BLOCK_SIZE]; MAX_GRAPH_NODES]),
            staged_cutoff: None,
            staged_env_depth: None,
            staged_resonance: None,
            hits: 0,
            clock: 0,
        }
    }

    /// Stage per-step parameter locks to be applied on the next note_on().
    pub fn stage_plock(&mut self, cutoff: Option<f32>, env_depth: Option<f32>, resonance: Option<f32>) {
        self.staged_cutoff = cutoff;
        self.staged_env_depth = env_depth;
        self.staged_resonance = resonance;
    }

    pub fn note_on(&mut self, note: u8, velocity: f32) {
        // A free voice, else the oldest: the first of the oldest, as before.
        let idx = self.voices.iter().position(|v| !v.active).unwrap_or_else(|| {
            let mut oldest = (0usize, 0u32);
            for (i, v) in self.voices.iter().enumerate() {
                if v.age > oldest.1 {
                    oldest = (i, v.age);
                }
            }
            oldest.0
        });
        self.voices[idx].init(&self.template, note, velocity);
        let hit = self.hits;
        self.hits = self.hits.wrapping_add(1);
        // A drum node that sways is made again for its hit number, in its own box.
        for i in 0..self.template.node_count as usize {
            let spec = &self.template.specs[i];
            if spec.sways() {
                if let Some(node) = self.voices[idx].nodes[i].as_deref_mut() {
                    node.renew_hit(spec, hit);
                }
            }
        }

        // A random node is made again for this note's number and moment.
        let random = self.template.random_mask;
        if random != 0 {
            for i in 0..self.template.node_count as usize {
                if random & (1 << i) != 0 {
                    if let Some(node) = self.voices[idx].nodes[i].as_deref_mut() {
                        node.strike(&self.template.specs[i], hit, self.clock);
                    }
                }
            }
        }

        // Apply staged p-lock values to filter nodes on the newly initialized voice
        let has_plocks =
            self.staged_cutoff.is_some() || self.staged_env_depth.is_some() || self.staged_resonance.is_some();

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
                        NodeKind::Svf(m) => {
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

    /// Slide to `note` instead of starting one: what `~note` asks for.
    /// `held` is what the track holds, in the order it started them. A slide
    /// is one line, so the last of them slides -- the newest voice still held
    /// on it -- and the others are released: after a chord, one note travels
    /// and the rest end, as on a `bass`, which only ever had the last. False
    /// when the instrument has no `glide=` or no voice holds that note, and
    /// the caller starts a note as it always did.
    pub fn slide_to(&mut self, held: &[u8], note: u8) -> bool {
        if self.template.glide_samples == 0 {
            return false;
        }
        let Some((&from, others)) = held.split_last() else {
            return false;
        };
        let mut newest: Option<usize> = None;
        for (i, v) in self.voices.iter().enumerate() {
            if v.active && v.held && v.note == from && newest.is_none_or(|n| v.age < self.voices[n].age) {
                newest = Some(i);
            }
        }
        let Some(slides) = newest else {
            return false;
        };
        for &n in others {
            let other =
                self.voices.iter().enumerate().position(|(i, v)| i != slides && v.active && v.held && v.note == n);
            if let Some(i) = other {
                self.voices[i].release(&self.template);
            }
        }
        self.voices[slides].slide(&self.template, note);
        true
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
        let now = self.clock;
        self.clock = self.clock.wrapping_add(len as u32);

        for voice in self.voices.iter_mut() {
            if !voice.active {
                continue;
            }
            // A voice that was not run for a while finds the curve where
            // the clock is, not where it stopped.
            if template.random_mask != 0 {
                for i in 0..template.node_count as usize {
                    if template.random_mask & (1 << i) != 0 {
                        if let Some(node) = voice.nodes[i].as_deref_mut() {
                            node.keep_time(&template.specs[i], now);
                        }
                    }
                }
            }

            // Process voice sample by sample using shared node buffers
            for s in 0..len {
                if voice.glide_left > 0 {
                    voice.glide(template);
                }
                template.run_frame(&mut self.node_bufs, s, &mut voice.nodes);
            }

            let out_idx = template.output_node as usize;

            // Nothing that is not a number leaves a voice. An infinity or a
            // NaN let through would settle in whatever keeps state after it
            // (the track's inserts, the sends, the master) and silence the
            // song until it is rebuilt. Times zero every finite sample is
            // zero and anything else is NaN, so the block is checked with
            // one multiply and add a sample and no branch. A voice that
            // fails is dropped, its block with it, and its nodes made new.
            let mut bad = 0.0f32;
            for &b in self.node_bufs[out_idx][..len].iter() {
                bad += b * 0.0;
            }
            if bad.is_nan() {
                voice.clear(template);
                continue;
            }

            // Mix output into buffer
            let vel = voice.velocity * template.output_gain;
            for (o, &b) in output[..len].iter_mut().zip(self.node_bufs[out_idx][..len].iter()) {
                *o += b * vel;
            }

            voice.age += 1;

            if voice.all_envelopes_idle(template) {
                voice.active = false;
            }
        }
    }

    /// Where the clock is, from whoever keeps the time: the song engine,
    /// for every sample of the song whether or not this instrument played
    /// it, so a `wander` is a function of the song's position alone.
    pub fn set_clock(&mut self, clock: u32) {
        self.clock = clock;
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
        // The next play counts its hits from the top, as a render does.
        self.hits = 0;
        self.clock = 0;
    }
}
