extern crate alloc;

use wasm_bindgen::prelude::*;
use synth_core::song_engine::SongEngine;
use synth_core::dsl::ast::Song;
use synth_core::dsl::diff::{self, DslChange};
use synth_core::BLOCK_SIZE;

#[wasm_bindgen]
pub fn wasm_memory() -> JsValue {
    wasm_bindgen::memory()
}

/// Allocate `len` bytes in WASM memory. Used by the AudioWorklet to write
/// the DSL source string before calling load_source.
#[wasm_bindgen]
pub fn alloc(len: usize) -> *mut u8 {
    let mut buf = Vec::with_capacity(len);
    let ptr = buf.as_mut_ptr();
    core::mem::forget(buf);
    ptr
}

static OK_JSON: &[u8] = br#"{"ok":true}"#;

/// Crossfade state for seamless engine swaps.
const CROSSFADE_SAMPLES: usize = 512; // ~12ms at 44.1kHz

struct Crossfade {
    old_l: [f32; CROSSFADE_SAMPLES],
    old_r: [f32; CROSSFADE_SAMPLES],
    pos: usize,       // current position in crossfade
    remaining: usize,  // samples left (0 = inactive)
}

impl Crossfade {
    fn new() -> Self {
        Self {
            old_l: [0.0; CROSSFADE_SAMPLES],
            old_r: [0.0; CROSSFADE_SAMPLES],
            pos: 0,
            remaining: 0,
        }
    }

    fn start(&mut self, old_engine: &mut SongEngine) {
        // Render the tail of the old engine into the crossfade buffer
        let mut l = [0.0f32; CROSSFADE_SAMPLES];
        let mut r = [0.0f32; CROSSFADE_SAMPLES];
        // Render in blocks
        let mut offset = 0;
        while offset < CROSSFADE_SAMPLES {
            let chunk = (CROSSFADE_SAMPLES - offset).min(BLOCK_SIZE);
            old_engine.process_block_stereo(&mut l[offset..offset+chunk], &mut r[offset..offset+chunk]);
            offset += chunk;
        }
        self.old_l = l;
        self.old_r = r;
        self.pos = 0;
        self.remaining = CROSSFADE_SAMPLES;
    }

    fn is_active(&self) -> bool {
        self.remaining > 0
    }

    /// Apply crossfade: blend old tail (fade out) with new output (fade in)
    fn apply(&mut self, out_l: &mut [f32], out_r: &mut [f32]) {
        let len = out_l.len().min(self.remaining);
        for i in 0..len {
            let t = (self.pos + i) as f32 / CROSSFADE_SAMPLES as f32;
            let fade_in = t;
            let fade_out = 1.0 - t;
            out_l[i] = out_l[i] * fade_in + self.old_l[self.pos + i] * fade_out;
            out_r[i] = out_r[i] * fade_in + self.old_r[self.pos + i] * fade_out;
        }
        self.pos += len;
        self.remaining -= len;
    }
}

/// Apply a single DSL change to the running engine (Phase 3 fast path).
fn apply_change(engine: &mut SongEngine, change: &DslChange) {
    match change {
        DslChange::TempoChanged(bpm) => engine.set_tempo(*bpm),
        DslChange::TrackLevelChanged { track_name, level } => {
            if let Some(idx) = find_track(engine, track_name) {
                engine.set_track_level(idx, *level);
            }
        }
        DslChange::TrackPanChanged { track_name, pan } => {
            if let Some(idx) = find_track(engine, track_name) {
                engine.set_track_pan(idx, *pan);
            }
        }
        DslChange::TrackVelocityChanged { track_name, velocity } => {
            if let Some(idx) = find_track(engine, track_name) {
                engine.set_track_velocity(idx, *velocity);
            }
        }
        DslChange::TrackPatternSwapped { track_name, new_pattern } => {
            if let Some(track_idx) = find_track(engine, track_name) {
                for pi in 0..engine.pattern_count() {
                    if engine.pattern_name(pi) == new_pattern.as_str() {
                        engine.set_track_pattern(track_idx, pi);
                        break;
                    }
                }
            }
        }
        DslChange::ModuleParamChanged { module_name, param_name, value } => {
            if let Some(idx) = engine.instrument_index(module_name) {
                engine.set_module_param(idx, param_name, *value);
            }
        }
        DslChange::SwingChanged(swing) => engine.set_swing(*swing),
        DslChange::HumanizeChanged(velocity) => {
            let (_, timing) = engine.humanize();
            engine.set_humanize(*velocity, timing);
        }
        DslChange::StructuralChange => {}
    }
}

fn find_track(engine: &SongEngine, name: &str) -> Option<usize> {
    (0..engine.track_count()).find(|&i| engine.track_name(i) == name)
}

/// Main synth handle — dual-engine with quantized hot-swap for livecoding.
#[wasm_bindgen]
pub struct Synth {
    engine: Option<SongEngine>,
    pending: Option<SongEngine>,  // compiled, waiting for bar boundary
    last_ast: Option<Song>,       // for incremental diffing (Phase 3)
    crossfade: Crossfade,
    out_buf: Vec<f32>,
    result_buf: Vec<u8>,
    last_bar: usize,              // track bar changes for swap detection
}

#[wasm_bindgen]
impl Synth {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            engine: None,
            pending: None,
            last_ast: None,
            crossfade: Crossfade::new(),
            out_buf: vec![0.0; BLOCK_SIZE * 2],
            result_buf: Vec::new(),
            last_bar: 0,
        }
    }

    // ═══════════════════════════════════════════════════════
    //  DSL source loading — compiles to pending engine
    // ═══════════════════════════════════════════════════════

    /// Compile a .synth DSL source. Uses smart diffing to choose the fastest path:
    /// - Non-structural changes (params, levels, tempo) → runtime mutations (instant)
    /// - Structural changes → quantized hot-swap on next bar boundary
    /// - Not playing → immediate swap
    pub fn load_source(&mut self, source_ptr: *const u8, source_len: usize) -> *const u8 {
        let source = unsafe {
            let slice = core::slice::from_raw_parts(source_ptr, source_len);
            core::str::from_utf8_unchecked(slice)
        };

        // Parse the new source
        let new_ast = match synth_core::dsl::parse(source) {
            Ok(ast) => ast,
            Err(errs) => {
                let err = synth_core::song_engine::DslError::Parse(errs);
                self.result_buf = err.to_json().into_bytes();
                return self.result_buf.as_ptr();
            }
        };

        let is_running = self.engine.as_ref().map_or(false, |e| e.running());

        // Phase 3: Try incremental apply if we have a previous AST and engine is running
        if is_running {
            if let Some(ref old_ast) = self.last_ast {
                let changes = diff::diff(old_ast, &new_ast);

                if changes.is_empty() {
                    // Nothing changed — skip entirely
                    self.last_ast = Some(new_ast);
                    self.result_buf.clear();
                    self.result_buf.extend_from_slice(OK_JSON);
                    return self.result_buf.as_ptr();
                }

                if !diff::has_structural_change(&changes) {
                    // Fast path: apply mutations to running engine
                    if let Some(ref mut engine) = self.engine {
                        for change in &changes {
                            apply_change(engine, change);
                        }
                    }
                    self.last_ast = Some(new_ast);
                    self.result_buf.clear();
                    self.result_buf.extend_from_slice(OK_JSON);
                    return self.result_buf.as_ptr();
                }
                // Structural change → fall through to hot-swap
            }
        }

        // Full compile path (structural change, first load, or no previous AST)
        match synth_core::dsl::compiler::compile(&new_ast) {
            Ok(compiled) => {
                let new_engine = SongEngine::from_compiled(compiled);
                if is_running {
                    self.pending = Some(new_engine);
                } else {
                    self.engine = Some(new_engine);
                }
                self.last_ast = Some(new_ast);
                self.result_buf.clear();
                self.result_buf.extend_from_slice(OK_JSON);
            }
            Err(errs) => {
                let err = synth_core::song_engine::DslError::Compile(errs);
                self.result_buf = err.to_json().into_bytes();
            }
        }
        self.result_buf.as_ptr()
    }

    /// Length of the last result JSON buffer.
    pub fn result_len(&self) -> usize {
        self.result_buf.len()
    }

    // ═══════════════════════════════════════════════════════
    //  Audio processing with hot-swap
    // ═══════════════════════════════════════════════════════

    pub fn process(&mut self, num_frames: usize) -> *const f32 {
        let len = num_frames.min(BLOCK_SIZE);

        if let Some(ref mut engine) = self.engine {
            // Render current engine
            let mut out_l = [0.0f32; BLOCK_SIZE];
            let mut out_r = [0.0f32; BLOCK_SIZE];
            engine.process_block_stereo(&mut out_l[..len], &mut out_r[..len]);

            // Continue crossfade if active from a previous swap
            if self.crossfade.is_active() {
                self.crossfade.apply(&mut out_l[..len], &mut out_r[..len]);
            }

            // Check for bar boundary AFTER rendering (process_block_stereo
            // is what advances current_bar, so we must check after)
            let current_bar = engine.current_bar();
            if self.pending.is_some() && current_bar != self.last_bar && engine.running() {
                // Bar boundary detected — swap to pending engine.
                // Capture the old engine's tail for crossfade.
                self.crossfade.start(engine);
                let mut new_engine = self.pending.take().unwrap();
                new_engine.start_from_bar(current_bar);
                self.engine = Some(new_engine);
            }

            self.last_bar = current_bar;
            self.interleave(&out_l, &out_r, len);
        } else {
            for i in 0..len * 2 {
                self.out_buf[i] = 0.0;
            }
        }
        self.out_buf.as_ptr()
    }

    fn interleave(&mut self, out_l: &[f32], out_r: &[f32], len: usize) {
        if self.out_buf.len() < len * 2 {
            self.out_buf.resize(len * 2, 0.0);
        }
        for i in 0..len {
            self.out_buf[i * 2] = out_l[i];
            self.out_buf[i * 2 + 1] = out_r[i];
        }
    }

    // ═══════════════════════════════════════════════════════
    //  Transport
    // ═══════════════════════════════════════════════════════

    pub fn start(&mut self) {
        if let Some(ref mut engine) = self.engine {
            engine.start();
            self.last_bar = 0;
        }
    }

    pub fn stop(&mut self) {
        if let Some(ref mut engine) = self.engine {
            engine.reset();
        }
        // If there's a pending engine, apply it immediately on stop
        if let Some(pending) = self.pending.take() {
            self.engine = Some(pending);
        }
    }

    pub fn reset(&mut self) {
        if let Some(ref mut engine) = self.engine {
            engine.reset();
        }
        if let Some(pending) = self.pending.take() {
            self.engine = Some(pending);
        }
    }

    // ═══════════════════════════════════════════════════════
    //  UI sync
    // ═══════════════════════════════════════════════════════

    pub fn current_step(&self) -> usize {
        self.engine.as_ref().map_or(0, |e| e.global_step())
    }

    pub fn running(&self) -> bool {
        self.engine.as_ref().map_or(false, |e| e.running())
    }

    // ═══════════════════════════════════════════════════════
    //  Real-time track control (no recompile)
    // ═══════════════════════════════════════════════════════

    pub fn track_count(&self) -> usize {
        self.engine.as_ref().map_or(0, |e| e.track_count())
    }

    pub fn track_info(&mut self) -> *const u8 {
        let engine = match self.engine.as_ref() {
            Some(e) => e,
            None => {
                self.result_buf = b"[]".to_vec();
                return self.result_buf.as_ptr();
            }
        };
        let mut json = String::from("[");
        for i in 0..engine.track_count() {
            if i > 0 { json.push(','); }
            json.push_str(&alloc::format!(
                r#"{{"name":"{}","kind":"{}","level":{:.3},"pan":{:.3}}}"#,
                engine.track_name(i),
                engine.track_kind(i),
                engine.track_level(i),
                engine.track_pan(i),
            ));
        }
        json.push(']');
        self.result_buf = json.into_bytes();
        self.result_buf.as_ptr()
    }

    pub fn set_track_level(&mut self, idx: usize, level: f32) {
        if let Some(ref mut engine) = self.engine {
            engine.set_track_level(idx, level);
        }
    }

    pub fn set_track_pan(&mut self, idx: usize, pan: f32) {
        if let Some(ref mut engine) = self.engine {
            engine.set_track_pan(idx, pan);
        }
    }

    // ═══════════════════════════════════════════════════════
    //  Phase 2: Runtime mutations (no recompile)
    // ═══════════════════════════════════════════════════════

    pub fn set_tempo(&mut self, bpm: f32) {
        if let Some(ref mut engine) = self.engine {
            engine.set_tempo(bpm);
        }
    }

    pub fn set_track_pattern(&mut self, track_idx: usize, pattern_idx: usize) {
        if let Some(ref mut engine) = self.engine {
            engine.set_track_pattern(track_idx, pattern_idx);
        }
    }

    pub fn set_track_velocity(&mut self, track_idx: usize, velocity: f32) {
        if let Some(ref mut engine) = self.engine {
            engine.set_track_velocity(track_idx, velocity);
        }
    }

    /// Returns false if the instrument or parameter name does not exist.
    pub fn set_module_param(&mut self, inst_idx: usize, name_ptr: *const u8, name_len: usize, value: f32) -> bool {
        let name = unsafe {
            let slice = core::slice::from_raw_parts(name_ptr, name_len);
            core::str::from_utf8_unchecked(slice)
        };
        match self.engine {
            Some(ref mut engine) => engine.set_module_param(inst_idx, name, value),
            None => false,
        }
    }
}
