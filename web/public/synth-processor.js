/**
 * AudioWorklet processor — runs the WASM synth engine on the audio thread.
 *
 * Receives a compiled WebAssembly.Module from the main thread,
 * instantiates it directly (no glue code), and calls process()
 * every render quantum to produce stereo audio.
 */

class SynthProcessor extends AudioWorkletProcessor {
  constructor() {
    super();
    this.wasm = null;
    this.synthPtr = 0;
    this.ready = false;
    this.frameCount = 0;
    this.port.onmessage = (e) => this.handleMessage(e.data);
  }

  handleMessage(msg) {
    if (msg.type === "init-wasm") {
      console.log("[synth-processor] received init-wasm, bytes:", msg.bytes?.byteLength);
      try {
        // Compile + instantiate the WASM module directly on the audio thread.
        // The imports mirror what wasm-pack generates in __wbg_get_imports().
        let wasm;
        const imports = {
          "./synth_wasm_bg.js": {
            __wbg___wbindgen_memory_edb3f01e3930bbf6: () => wasm.memory,
            __wbg___wbindgen_throw_6ddd609b62940d55: (ptr, len) => {
              const bytes = new Uint8Array(wasm.memory.buffer, ptr, len);
              throw new Error(new TextDecoder().decode(bytes));
            },
            __wbindgen_init_externref_table: () => {
              const table = wasm.__wbindgen_externrefs;
              const offset = table.grow(4);
              table.set(0, undefined);
              table.set(offset + 0, undefined);
              table.set(offset + 1, null);
              table.set(offset + 2, true);
              table.set(offset + 3, false);
            },
          },
        };

        const wasmModule = new WebAssembly.Module(msg.bytes);
        console.log("[synth-processor] module compiled");
        const instance = new WebAssembly.Instance(wasmModule, imports);
        wasm = instance.exports;
        this.wasm = wasm;
        console.log("[synth-processor] instance created");

        // Run wasm-bindgen start function
        wasm.__wbindgen_start();

        // Create the synth instance
        this.synthPtr = wasm.synth_new() >>> 0;
        this.ready = true;
        console.log("[synth-processor] WASM init OK, synthPtr =", this.synthPtr);
        this.port.postMessage({ type: "ready" });
      } catch (err) {
        console.error("[synth-processor] init failed:", err);
        this.port.postMessage({ type: "error", message: String(err) });
      }
      return;
    }

    if (!this.ready) return;
    const w = this.wasm;
    const p = this.synthPtr;

    switch (msg.type) {
      // Module selection
      case "set_active_module": w.synth_set_active_module(p, msg.module); break;

      // Transport
      case "start":            w.synth_start(p); break;
      case "stop":             w.synth_stop(p); break;
      case "reset":            w.synth_reset(p); break;
      case "set_bpm":          w.synth_set_bpm(p, msg.value); break;

      // Universal param
      case "set_param":        w.synth_set_param(p, msg.paramId, msg.value); break;

      // Notes
      case "note_on":          w.synth_note_on(p, msg.module, msg.note, msg.velocity); break;
      case "note_off":         w.synth_note_off(p, msg.module, msg.note); break;

      // Sequencer steps
      case "set_step":         w.synth_set_step(p, msg.idx, msg.note, msg.velocity, msg.gate); break;
      case "clear_step":       w.synth_clear_step(p, msg.idx); break;
      case "set_step_slide":   w.synth_set_step_slide(p, msg.idx, msg.slide); break;
      case "set_step_lock_slide": w.synth_set_step_lock_slide(p, msg.idx, msg.lockSlide); break;
      case "set_step_lock":    w.synth_set_step_lock(p, msg.idx, msg.paramId, msg.value); break;
      case "clear_step_locks": w.synth_clear_step_locks(p, msg.idx); break;
      case "set_step_active":  w.synth_set_step_active(p, msg.idx, msg.active); break;
      case "set_step_probability": w.synth_set_step_probability(p, msg.idx, msg.prob); break;
      case "set_num_steps":    w.synth_set_num_steps(p, msg.n); break;
      case "set_swing":        w.synth_set_swing(p, msg.value); break;
      case "set_humanize":     w.synth_set_humanize(p, msg.value); break;

      // Effects
      case "set_delay_time":        w.synth_set_delay_time(p, msg.value); break;
      case "set_delay_feedback":    w.synth_set_delay_feedback(p, msg.value); break;
      case "set_delay_filter":      w.synth_set_delay_filter(p, msg.value); break;
      case "set_reverb_size":       w.synth_set_reverb_size(p, msg.value); break;
      case "set_reverb_damping":    w.synth_set_reverb_damping(p, msg.value); break;
      case "set_reverb_mix":        w.synth_set_reverb_mix(p, msg.value); break;
      case "set_eq_low":            w.synth_set_eq_low(p, msg.value); break;
      case "set_eq_mid":            w.synth_set_eq_mid(p, msg.value); break;
      case "set_eq_high":           w.synth_set_eq_high(p, msg.value); break;
      case "set_compressor_threshold": w.synth_set_compressor_threshold(p, msg.value); break;
      case "set_compressor_ratio":  w.synth_set_compressor_ratio(p, msg.value); break;
      case "set_sidechain":         w.synth_set_sidechain(p, msg.value); break;
      case "set_pitch_bend":        w.synth_set_pitch_bend(p, msg.value); break;

      // Harmony
      case "set_harmony":      w.synth_set_harmony(p, msg.root, msg.scale, msg.chordDegree); break;
    }
  }

  process(_inputs, outputs) {
    if (!this.ready || !this.wasm) return true;

    const output = outputs[0];
    if (!output || !output[0] || !output[1]) return true;

    const left = output[0];
    const right = output[1];
    const frames = left.length; // 128

    // Call WASM process — returns pointer to interleaved stereo buffer
    const ptr = this.wasm.synth_process(this.synthPtr, frames) >>> 0;

    // Read from WASM linear memory
    const mem = new Float32Array(this.wasm.memory.buffer, ptr, frames * 2);
    for (let i = 0; i < frames; i++) {
      left[i] = mem[i * 2];
      right[i] = mem[i * 2 + 1];
    }

    // Debug: log first few frames to verify audio output
    this.frameCount++;
    if (this.frameCount <= 3) {
      let maxL = 0, maxR = 0;
      for (let i = 0; i < frames; i++) {
        maxL = Math.max(maxL, Math.abs(left[i]));
        maxR = Math.max(maxR, Math.abs(right[i]));
      }
      console.log(`[synth-processor] frame ${this.frameCount}: maxL=${maxL.toFixed(6)} maxR=${maxR.toFixed(6)}`);
    }

    // Send step position to main thread (~every 23ms)
    if (this.frameCount % 8 === 0) {
      this.port.postMessage({
        type: "step",
        step: this.wasm.synth_current_step(this.synthPtr) >>> 0,
      });
    }

    return true;
  }
}

registerProcessor("synth-processor", SynthProcessor);
