use wasm_bindgen::prelude::*;
use synth_core::engine::Engine;
use synth_core::command::{Command, CommandBus};
use synth_core::BLOCK_SIZE;

#[wasm_bindgen]
pub fn wasm_memory() -> JsValue {
    wasm_bindgen::memory()
}

/// Main synth handle exposed to JavaScript.
/// All interaction goes through the CommandBus — one universal protocol.
#[wasm_bindgen]
pub struct Synth {
    engine: Engine,
    bus: CommandBus,
    out_buf: Vec<f32>,
}

#[wasm_bindgen]
impl Synth {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            engine: Engine::new(),
            bus: CommandBus::new(),
            out_buf: vec![0.0; BLOCK_SIZE * 2],
        }
    }

    // ═══════════════════════════════════════════════════════
    //  PRIMARY API: CommandBus
    // ═══════════════════════════════════════════════════════

    /// Push a command using flat encoding: (tag, a, b, c, d, e).
    /// This is the single entry point for all engine actions from JS.
    pub fn cmd(&mut self, tag: u8, a: f32, b: f32, c: f32, d: f32, e: f32) {
        if let Some(command) = Command::from_flat(tag, a, b, c, d, e) {
            self.bus.push(command);
        }
    }

    /// Process `num_frames` stereo frames (max 128).
    /// Drains the command bus first, then runs the audio engine.
    /// Returns a pointer to the interleaved [L,R,L,R,...] output buffer.
    pub fn process(&mut self, num_frames: usize) -> *const f32 {
        // Drain all pending commands before processing audio
        self.bus.drain(&mut self.engine);

        let len = num_frames.min(BLOCK_SIZE);
        let mut out_l = [0.0f32; BLOCK_SIZE];
        let mut out_r = [0.0f32; BLOCK_SIZE];
        self.engine.process_block_stereo(&mut out_l[..len], &mut out_r[..len]);

        if self.out_buf.len() < len * 2 {
            self.out_buf.resize(len * 2, 0.0);
        }
        for i in 0..len {
            self.out_buf[i * 2] = out_l[i];
            self.out_buf[i * 2 + 1] = out_r[i];
        }
        self.out_buf.as_ptr()
    }

    /// Current sequencer step (for UI sync).
    pub fn current_step(&self) -> usize {
        self.engine.sequencer.current_step()
    }

    // ═══════════════════════════════════════════════════════
    //  DEBUG: built-in test pattern
    // ═══════════════════════════════════════════════════════

    /// Load the funk pattern entirely via CommandBus (no JS messages needed).
    /// Call this then cmd(0, ...) [Start] to test audio without the message bridge.
    pub fn load_test_pattern(&mut self) {
        use synth_core::command::Command::*;

        let cmds: &[Command] = &[
            SetBpm(112.0),
            SetHarmony { root: 57, scale: 1, degree: 0 },

            // Bass params (default ADSR: A=0, D=0.2, S=0.8, R=0.15 — same as funk test)
            SetParam { id: 0, value: 0.534 },  // Cutoff (~800Hz with exp scaling)
            SetParam { id: 2, value: 0.75 },   // Reso
            SetParam { id: 1, value: 0.94 },   // EnvMod (~5600Hz sweep with exp scaling)
            SetParam { id: 4, value: 0.00 },   // Attack = 0 (snappy)
            SetParam { id: 3, value: 0.15 },   // Glide
            SetParam { id: 10, value: 0.25 },  // Osc2 sub
            SetParam { id: 12, value: 0.00 },  // Saw
            SetParam { id: 15, value: 0.50 },  // Keytrack

            // FM clav
            SetParam { id: 64, value: 0.29 },  // Algorithm 2
            SetParam { id: 65, value: 0.87 },  // ModIndex → ~2.5 (exp scaled)
            SetParam { id: 71, value: 0.40 },  // Feedback
            // FM ADSR: logarithmic scaling 0.001 * 2000^v
            // Funk test wants: A=0.001, D=0.12, S=0.05, R=0.08
            SetParam { id: 76, value: 0.0 },    // Attack: 0.001s (v=0)
            SetParam { id: 77, value: 0.63 },   // Decay:  ~0.12s
            SetParam { id: 78, value: 0.05 },   // Sustain (no scaling)
            SetParam { id: 79, value: 0.58 },   // Release: ~0.08s

            // Mute arp
            SetParam { id: 131, value: 0.0 },

            // Effects
            SetDelaySync(5),          // Sixteenth note sync
            SetDelayFeedback(0.25),
            SetDelayMix(0.10),        // subtle — funk is dry
            SetDelayFilter(0.6),
            SetTiltEq(0.0),           // neutral tilt
            SetReverbSize(0.3),
            SetReverbDamping(0.6),
            SetReverbMix(0.10),
            SetEqLow(4.0),
            SetEqMid(2.0),
            SetEqHigh(1.0),
            SetCompThreshold(-9.0),
            SetCompRatio(5.0),
            SetCompAttack(8.0),
            SetCompRelease(80.0),
            SetCompMakeup(3.0),
            SetSidechain(0.4),

            // Bass steps — gate ~0.90 to match funk test (note plays until next step)
            // Bass steps — gate ~0.90, env locks use exp scaling
            SetStep { track: 0, idx: 0,  note: 21, velocity: 0.90, gate: 0.90 },
            SetStepLock { track: 0, idx: 0,  param_id: 1, value: 0.96 },
            SetStepLock { track: 0, idx: 0,  param_id: 2, value: 0.80 },
            SetStep { track: 0, idx: 3,  note: 21, velocity: 0.55, gate: 0.85 },
            SetStepLock { track: 0, idx: 3,  param_id: 1, value: 0.88 },
            SetStepLock { track: 0, idx: 3,  param_id: 2, value: 0.65 },
            SetStep { track: 0, idx: 5,  note: 19, velocity: 0.70, gate: 0.85 },
            SetStepLock { track: 0, idx: 5,  param_id: 1, value: 0.93 },
            SetStepLock { track: 0, idx: 5,  param_id: 2, value: 0.75 },
            SetStep { track: 0, idx: 8,  note: 21, velocity: 0.80, gate: 0.90 },
            SetStepLock { track: 0, idx: 8,  param_id: 1, value: 0.94 },
            SetStepLock { track: 0, idx: 8,  param_id: 2, value: 0.78 },
            SetStep { track: 0, idx: 10, note: 21, velocity: 0.40, gate: 0.75 },
            SetStepLock { track: 0, idx: 10, param_id: 1, value: 0.83 },
            SetStepLock { track: 0, idx: 10, param_id: 2, value: 0.55 },
            SetStep { track: 0, idx: 12, note: 24, velocity: 0.50, gate: 0.85 },
            SetStepLock { track: 0, idx: 12, param_id: 1, value: 0.89 },
            SetStepLock { track: 0, idx: 12, param_id: 2, value: 0.70 },
            SetStep { track: 0, idx: 14, note: 21, velocity: 0.65, gate: 0.85 },
            SetStepLock { track: 0, idx: 14, param_id: 1, value: 0.91 },
            SetStepLock { track: 0, idx: 14, param_id: 2, value: 0.72 },

            // FM clav steps
            SetStep { track: 2, idx: 2,  note: 60, velocity: 0.50, gate: 0.30 },
            SetStep { track: 2, idx: 5,  note: 64, velocity: 0.55, gate: 0.30 },
            SetStep { track: 2, idx: 6,  note: 57, velocity: 0.35, gate: 0.20 },
            SetStep { track: 2, idx: 9,  note: 67, velocity: 0.50, gate: 0.30 },
            SetStep { track: 2, idx: 11, note: 64, velocity: 0.40, gate: 0.25 },
            SetStep { track: 2, idx: 13, note: 60, velocity: 0.55, gate: 0.30 },
            SetStep { track: 2, idx: 15, note: 57, velocity: 0.30, gate: 0.20 },

            // Drums — matching funk test variation 0
            // Kick: steps 0, 7, 8
            SetDrumHit { lane: 0, step: 0,  on: true, velocity: 0.95 },
            SetDrumHit { lane: 0, step: 7,  on: true, velocity: 0.70 },
            SetDrumHit { lane: 0, step: 8,  on: true, velocity: 0.85 },
            // Snare: steps 4, 12
            SetDrumHit { lane: 1, step: 4,  on: true, velocity: 0.85 },
            SetDrumHit { lane: 1, step: 12, on: true, velocity: 0.80 },
        ];

        // Hihat pattern — NO hats on kick/snare/open-hat steps (matching funk test)
        // Steps with other drums: 0(kick), 4(snare), 6(open hat), 7(kick), 8(kick),
        //                          12(snare), 14(open hat)
        // Open hats (lane 2, note 46) on steps 6, 14
        // Closed hats on remaining steps: 1,2,3,5,9,10,11,13,15
        let hh_pattern: [(u8, bool, f32); 16] = [
            (0,  false, 0.0),  // kick
            (1,  true,  0.35), // closed hat
            (2,  true,  0.25),
            (3,  true,  0.35),
            (4,  false, 0.0),  // snare
            (5,  true,  0.35),
            (6,  false, 0.0),  // open hat (separate)
            (7,  false, 0.0),  // kick
            (8,  false, 0.0),  // kick
            (9,  true,  0.35),
            (10, true,  0.35),
            (11, true,  0.30),
            (12, false, 0.0),  // snare
            (13, true,  0.25),
            (14, false, 0.0),  // open hat (separate)
            (15, true,  0.35),
        ];

        for &c in cmds {
            self.bus.push(c);
        }
        for &(step, on, vel) in &hh_pattern {
            self.bus.push(SetDrumHit { lane: 2, step, on, velocity: vel });
        }
        // Open hats — need lane with MIDI 46 (currently lane 2 is closed hat 42)
        // Use lane 4 (tom) repurposed as open hat
        self.bus.push(SetDrumLaneNote { lane: 4, note: 46 });
        self.bus.push(SetDrumHit { lane: 4, step: 6,  on: true, velocity: 0.45 });
        self.bus.push(SetDrumHit { lane: 4, step: 14, on: true, velocity: 0.40 });

        // Drain immediately so pattern is loaded
        self.bus.drain(&mut self.engine);
    }
}
