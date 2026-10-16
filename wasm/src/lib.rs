//! The browser shell. All livecoding logic (diff, fast path, bar-quantized
//! swap with state inheritance) lives in `tatum_core::live`, and the live
//! session as a whole -- files, `use`, knobs by name, the loop of `compose`
//! -- in `tatum_core::command`, shared with the native CLI; this file only
//! carries strings and buffers across the WASM boundary. The exported API
//! is what `js/src/tatum-processor.js` calls: [`Tatum::command`] and
//! [`Tatum::process`], plus the older per-value exports.

extern crate alloc;

use tatum_core::command::Live;
use tatum_core::BLOCK_SIZE;
use wasm_bindgen::prelude::*;

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

/// Main engine handle: the live session on the worklet thread.
#[wasm_bindgen]
pub struct Tatum {
    live: Box<Live>,
    out_buf: Vec<f32>,
    result_buf: Vec<u8>,
}

// Plain impl: wasm_bindgen exports the constructor, not this.
impl Default for Tatum {
    fn default() -> Self {
        Self::new()
    }
}

#[wasm_bindgen]
impl Tatum {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self { live: tatum_core::command::boxed(), out_buf: vec![0.0; BLOCK_SIZE * 2], result_buf: Vec::new() }
    }

    // ═══════════════════════════════════════════════════════
    //  Messages: everything the live session does
    // ═══════════════════════════════════════════════════════

    /// One JSON message in (`{"op":"try_knob","target":"bass.cutoff",
    /// "value":96}`), one JSON answer out. The operations and what each
    /// takes are `tatum_core::command`'s.
    pub fn command(&mut self, message: &str) -> String {
        self.live.command(message)
    }

    // ═══════════════════════════════════════════════════════
    //  DSL source loading
    // ═══════════════════════════════════════════════════════

    /// Re-evaluate a .synth source. Always parsed and compiled, so the same
    /// text is rejected here and in `tatum check`. Value edits apply at once;
    /// anything else takes over on the next bar line, keeping every piece of
    /// state whose definition did not change. Not playing: replaced at once.
    /// # Safety
    /// `source_ptr`/`source_len` must describe valid UTF-8 inside this module's
    /// memory. The JS glue hands over a slice it just wrote there.
    pub unsafe fn load_source(&mut self, source_ptr: *const u8, source_len: usize) -> *const u8 {
        let source = unsafe {
            let slice = core::slice::from_raw_parts(source_ptr, source_len);
            core::str::from_utf8_unchecked(slice)
        };
        let message = tatum_core::json::Obj::new().str("op", "text").str("source", source).finish();
        self.result_buf = self.live.command(&message).into_bytes();
        self.result_buf.as_ptr()
    }

    /// Length of the last result JSON buffer.
    pub fn result_len(&self) -> usize {
        self.result_buf.len()
    }

    // ═══════════════════════════════════════════════════════
    //  Audio processing
    // ═══════════════════════════════════════════════════════

    pub fn process(&mut self, num_frames: usize) -> *const f32 {
        let len = num_frames.min(BLOCK_SIZE);
        let mut out_l = [0.0f32; BLOCK_SIZE];
        let mut out_r = [0.0f32; BLOCK_SIZE];
        self.live.process(&mut out_l[..len], &mut out_r[..len]);
        self.interleave(&out_l, &out_r, len);
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
        self.live.player_mut().start();
    }

    pub fn stop(&mut self) {
        self.live.player_mut().stop();
    }

    pub fn reset(&mut self) {
        self.live.player_mut().stop();
    }

    // ═══════════════════════════════════════════════════════
    //  UI sync
    // ═══════════════════════════════════════════════════════

    pub fn current_step(&self) -> usize {
        self.live.player().engine().map_or(0, |e| e.global_step())
    }

    pub fn running(&self) -> bool {
        self.live.player().running()
    }

    // ═══════════════════════════════════════════════════════
    //  Real-time track control from the UI (not from the text)
    // ═══════════════════════════════════════════════════════

    pub fn track_count(&self) -> usize {
        self.live.player().engine().map_or(0, |e| e.track_count())
    }

    pub fn track_info(&mut self) -> *const u8 {
        let engine = match self.live.player().engine() {
            Some(e) => e,
            None => {
                self.result_buf = b"[]".to_vec();
                return self.result_buf.as_ptr();
            }
        };
        let mut json = String::from("[");
        for i in 0..engine.track_count() {
            if i > 0 {
                json.push(',');
            }
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
        if let Some(engine) = self.live.player_mut().engine_mut() {
            engine.set_track_level(idx, level);
        }
    }

    pub fn set_track_pan(&mut self, idx: usize, pan: f32) {
        if let Some(engine) = self.live.player_mut().engine_mut() {
            engine.set_track_pan(idx, pan);
        }
    }

    pub fn set_tempo(&mut self, bpm: f32) {
        if let Some(engine) = self.live.player_mut().engine_mut() {
            engine.set_tempo(bpm);
        }
    }

    pub fn set_track_pattern(&mut self, track_idx: usize, pattern_idx: usize) {
        if let Some(engine) = self.live.player_mut().engine_mut() {
            engine.set_track_pattern(track_idx, pattern_idx);
        }
    }

    pub fn set_track_velocity(&mut self, track_idx: usize, velocity: f32) {
        if let Some(engine) = self.live.player_mut().engine_mut() {
            engine.set_track_velocity(track_idx, velocity);
        }
    }

    /// Returns false if the instrument or parameter name does not exist.
    /// # Safety
    /// `name_ptr`/`name_len` must describe valid UTF-8 inside this module's
    /// memory. The JS glue hands over a slice it just wrote there.
    pub unsafe fn set_module_param(
        &mut self,
        inst_idx: usize,
        name_ptr: *const u8,
        name_len: usize,
        value: f32,
    ) -> bool {
        let name = unsafe {
            let slice = core::slice::from_raw_parts(name_ptr, name_len);
            core::str::from_utf8_unchecked(slice)
        };
        match self.live.player_mut().engine_mut() {
            Some(engine) => engine.set_module_param(inst_idx, name, value),
            None => false,
        }
    }
}
