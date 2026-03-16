use wasm_bindgen::prelude::*;
use synth_core::engine::Engine;
use synth_core::sequencer::Step;
use synth_core::harmony::{HarmonyContext, Scale};
use synth_core::{Module, BLOCK_SIZE};

#[wasm_bindgen]
pub fn wasm_memory() -> JsValue {
    wasm_bindgen::memory()
}

/// Main synth handle exposed to JavaScript.
/// Create one instance, feed it to an AudioWorklet, call `process()` every frame.
#[wasm_bindgen]
pub struct Synth {
    engine: Engine,
    // Pre-allocated interleaved output buffer
    out_buf: Vec<f32>,
}

#[wasm_bindgen]
impl Synth {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            engine: Engine::new(),
            out_buf: vec![0.0; BLOCK_SIZE * 2],
        }
    }

    // ── Transport ──────────────────────────────────────────

    pub fn set_bpm(&mut self, bpm: f32) { self.engine.set_bpm(bpm); }
    pub fn set_active_module(&mut self, module: u8) { self.engine.set_active_module(module); }
    pub fn start(&mut self) { self.engine.sequencer.start(); }
    pub fn stop(&mut self) {
        self.engine.sequencer.stop();
        self.engine.all_notes_off();
    }
    pub fn reset(&mut self) { self.engine.reset(); }

    // ── Audio ──────────────────────────────────────────────

    /// Process `num_frames` stereo frames (max 128).
    /// Returns a pointer to the internal interleaved [L,R,L,R,...] buffer.
    /// The JS side should read `num_frames * 2` floats from this pointer.
    pub fn process(&mut self, num_frames: usize) -> *const f32 {
        let len = num_frames.min(BLOCK_SIZE);
        let mut out_l = [0.0f32; BLOCK_SIZE];
        let mut out_r = [0.0f32; BLOCK_SIZE];
        self.engine.process_block_stereo(&mut out_l[..len], &mut out_r[..len]);

        // Interleave into pre-allocated buffer
        if self.out_buf.len() < len * 2 {
            self.out_buf.resize(len * 2, 0.0);
        }
        for i in 0..len {
            self.out_buf[i * 2] = out_l[i];
            self.out_buf[i * 2 + 1] = out_r[i];
        }
        self.out_buf.as_ptr()
    }

    /// How many bytes the output buffer occupies (for JS memory view).
    pub fn output_buffer_ptr(&self) -> *const f32 { self.out_buf.as_ptr() }

    // ── Sequencer ──────────────────────────────────────────

    pub fn set_step(&mut self, idx: usize, note: u8, velocity: f32, gate: f32) {
        if idx < 16 {
            self.engine.sequencer.steps[idx] = Step::new(note, velocity, gate);
        }
    }

    pub fn clear_step(&mut self, idx: usize) {
        if idx < 16 {
            self.engine.sequencer.steps[idx] = Step::empty();
        }
    }

    pub fn set_step_slide(&mut self, idx: usize, slide: bool) {
        if idx < 16 { self.engine.sequencer.steps[idx].slide = slide; }
    }

    pub fn set_step_lock_slide(&mut self, idx: usize, lock_slide: bool) {
        if idx < 16 { self.engine.sequencer.steps[idx].lock_slide = lock_slide; }
    }

    pub fn set_step_lock(&mut self, idx: usize, param_id: u8, value: f32) {
        if idx >= 16 { return; }
        let s = &mut self.engine.sequencer.steps[idx];
        for slot in s.locks.iter_mut() {
            if let Some((id, _)) = slot {
                if *id == param_id {
                    *slot = Some((param_id, value));
                    return;
                }
            }
        }
        for slot in s.locks.iter_mut() {
            if slot.is_none() {
                *slot = Some((param_id, value));
                return;
            }
        }
    }

    pub fn clear_step_locks(&mut self, idx: usize) {
        if idx < 16 { self.engine.sequencer.steps[idx].locks = [None; 4]; }
    }

    pub fn set_step_probability(&mut self, idx: usize, prob: f32) {
        if idx < 16 { self.engine.sequencer.steps[idx].probability = prob; }
    }

    pub fn set_step_active(&mut self, idx: usize, active: bool) {
        if idx < 16 { self.engine.sequencer.steps[idx].active = active; }
    }

    pub fn set_num_steps(&mut self, n: usize) {
        self.engine.sequencer.num_steps = n.clamp(1, 16);
    }

    pub fn set_swing(&mut self, swing: f32) { self.engine.sequencer.set_swing(swing); }
    pub fn set_humanize(&mut self, amount: f32) { self.engine.sequencer.set_humanize(amount); }
    pub fn current_step(&self) -> usize { self.engine.sequencer.current_step() }

    // ── Parameters (universal) ─────────────────────────────

    /// Set any synth parameter by global ID. See PARAM_* constants.
    pub fn set_param(&mut self, param_id: u8, value: f32) {
        self.engine.apply_param_lock(param_id, value);
    }

    // ── Direct note control ────────────────────────────────

    pub fn note_on(&mut self, module: u8, note: u8, velocity: f32) {
        match module {
            0 => self.engine.bass.note_on(note, velocity),
            1 => self.engine.keys.note_on(note, velocity),
            2 => self.engine.fm.note_on(note, velocity),
            3 => self.engine.beats.note_on(note, velocity),
            _ => {}
        }
    }

    pub fn note_off(&mut self, module: u8, note: u8) {
        match module {
            0 => self.engine.bass.note_off(note),
            1 => self.engine.keys.note_off(note),
            2 => self.engine.fm.note_off(note),
            3 => self.engine.beats.note_off(note),
            _ => {}
        }
    }

    // ── Effects ────────────────────────────────────────────

    pub fn set_delay_time(&mut self, t: f32) { self.engine.set_delay_time(t); }
    pub fn set_delay_feedback(&mut self, f: f32) { self.engine.set_delay_feedback(f); }
    pub fn set_delay_filter(&mut self, a: f32) { self.engine.set_delay_filter(a); }

    pub fn set_reverb_size(&mut self, s: f32) { self.engine.reverb.set_room_size(s); }
    pub fn set_reverb_damping(&mut self, d: f32) { self.engine.reverb.set_damping(d); }
    pub fn set_reverb_mix(&mut self, m: f32) { self.engine.reverb.set_mix(m); }

    pub fn set_eq_low(&mut self, db: f32) { self.engine.set_eq_low(db); }
    pub fn set_eq_mid(&mut self, db: f32) { self.engine.set_eq_mid(db); }
    pub fn set_eq_high(&mut self, db: f32) { self.engine.set_eq_high(db); }

    pub fn set_compressor_threshold(&mut self, db: f32) { self.engine.set_compressor_threshold(db); }
    pub fn set_compressor_ratio(&mut self, r: f32) { self.engine.set_compressor_ratio(r); }
    pub fn set_sidechain(&mut self, a: f32) { self.engine.set_sidechain_amount(a); }

    pub fn set_pitch_bend(&mut self, b: f32) { self.engine.set_pitch_bend(b); }

    // ── Harmony ────────────────────────────────────────────

    /// Set harmony: root (MIDI note), scale (0=Major,1=Minor,2=Dorian,3=Mixo,4=PentMin), chord_degree (0-6)
    pub fn set_harmony(&mut self, root: u8, scale: u8, chord_degree: u8) {
        let s = match scale {
            0 => Scale::Major,
            1 => Scale::Minor,
            2 => Scale::Dorian,
            3 => Scale::Mixolydian,
            4 => Scale::PentatonicMinor,
            _ => Scale::Minor,
        };
        self.engine.set_harmony(HarmonyContext { root, scale: s, chord_degree });
    }
}
