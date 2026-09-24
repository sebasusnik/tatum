/**
 * AudioWorklet processor — runs the WASM synth engine on the audio thread.
 * (Dev copy — identical to public/tatum-processor.js)
 *
 * Communication protocol:
 *   - "load-source": Send .synth DSL text → WASM parses/compiles/loads
 *   - "transport":   start / stop / reset
 *   - "dsl-result":  Response from load-source with {ok, errors?}
 *   - "step":        Current sequencer step (sent to main thread for UI sync)
 */

class TatumProcessor extends AudioWorkletProcessor {
  constructor() {
    super();
    this.wasm = null;
    this.tatumPtr = 0;
    this.ready = false;
    this.frameCount = 0;
    this.port.onmessage = (e) => this.handleMessage(e.data);
  }

  handleMessage(msg) {
    if (msg.type === "init-wasm") {
      try {
        let wasm;
        const imports = {
          "./tatum_wasm_bg.js": {
            __wbg___wbindgen_memory_edb3f01e3930bbf6: () => wasm.memory,
            __wbg___wbindgen_throw_6ddd609b62940d55: (ptr, len) => {
              const bytes = new Uint8Array(wasm.memory.buffer, ptr, len);
              let msg = "";
              for (let i = 0; i < len; i++) msg += String.fromCharCode(bytes[i]);
              throw new Error(msg);
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
        const instance = new WebAssembly.Instance(wasmModule, imports);
        wasm = instance.exports;
        this.wasm = wasm;
        wasm.__wbindgen_start();
        this.tatumPtr = wasm.tatum_new() >>> 0;
        this.ready = true;
        this.port.postMessage({ type: "ready" });
      } catch (err) {
        console.error("[tatum-processor] init failed:", err);
        this.port.postMessage({ type: "error", message: String(err) });
      }
      return;
    }

    if (!this.ready) return;

    // Load DSL source (bytes pre-encoded on main thread)
    if (msg.type === "load-source") {
      try {
        const bytes = new Uint8Array(msg.bytes);

        // Allocate buffer in WASM memory and copy source bytes
        const ptr = this.wasm.alloc(bytes.length) >>> 0;
        new Uint8Array(this.wasm.memory.buffer, ptr, bytes.length).set(bytes);

        // Call load_source — parses, compiles, swaps engine on success
        const resultPtr = this.wasm.tatum_load_source(this.tatumPtr, ptr, bytes.length) >>> 0;
        const resultLen = this.wasm.tatum_result_len(this.tatumPtr) >>> 0;
        const resultBytes = new Uint8Array(this.wasm.memory.buffer, resultPtr, resultLen);
        let resultJson = "";
        for (let i = 0; i < resultLen; i++) resultJson += String.fromCharCode(resultBytes[i]);
        const result = JSON.parse(resultJson);

        this.port.postMessage({ type: "dsl-result", result });
      } catch (err) {
        this.port.postMessage({
          type: "dsl-result",
          result: { ok: false, errors: [{ line: 0, col: 0, msg: String(err) }] },
        });
      }
      return;
    }

    // Transport: start / stop / reset
    if (msg.type === "transport") {
      if (msg.action === "start") this.wasm.tatum_start(this.tatumPtr);
      else if (msg.action === "stop") this.wasm.tatum_stop(this.tatumPtr);
      else if (msg.action === "reset") this.wasm.tatum_reset(this.tatumPtr);
      return;
    }

    // Real-time track params (no recompile)
    if (msg.type === "set-track-level") {
      this.wasm.tatum_set_track_level(this.tatumPtr, msg.idx, msg.value);
      return;
    }
    if (msg.type === "set-track-pan") {
      this.wasm.tatum_set_track_pan(this.tatumPtr, msg.idx, msg.value);
      return;
    }

    // Runtime mutations (no recompile)
    if (msg.type === "set-tempo") {
      this.wasm.tatum_set_tempo(this.tatumPtr, msg.value);
      return;
    }
    if (msg.type === "set-track-pattern") {
      this.wasm.tatum_set_track_pattern(this.tatumPtr, msg.idx, msg.patternIdx);
      return;
    }
    if (msg.type === "set-track-velocity") {
      this.wasm.tatum_set_track_velocity(this.tatumPtr, msg.idx, msg.value);
      return;
    }
    if (msg.type === "set-module-param") {
      const bytes = new Uint8Array(msg.nameBytes);
      const ptr = this.wasm.alloc(bytes.length) >>> 0;
      new Uint8Array(this.wasm.memory.buffer, ptr, bytes.length).set(bytes);
      this.wasm.tatum_set_module_param(this.tatumPtr, msg.instIdx, ptr, bytes.length, msg.value);
      return;
    }

    // Track info request
    if (msg.type === "get-track-info") {
      const resultPtr = this.wasm.tatum_track_info(this.tatumPtr) >>> 0;
      const resultLen = this.wasm.tatum_result_len(this.tatumPtr) >>> 0;
      const resultBytes = new Uint8Array(this.wasm.memory.buffer, resultPtr, resultLen);
      let json = "";
      for (let i = 0; i < resultLen; i++) json += String.fromCharCode(resultBytes[i]);
      this.port.postMessage({ type: "track-info", tracks: JSON.parse(json) });
      return;
    }
  }

  process(_inputs, outputs) {
    if (!this.ready || !this.wasm) return true;

    const output = outputs[0];
    if (!output || !output[0] || !output[1]) return true;

    const left = output[0];
    const right = output[1];
    const frames = left.length;

    try {
      const ptr = this.wasm.tatum_process(this.tatumPtr, frames) >>> 0;
      const buf = this.wasm.memory.buffer;
      const mem = new Float32Array(buf, ptr, frames * 2);

      for (let i = 0; i < frames; i++) {
        const l = mem[i * 2];
        const r = mem[i * 2 + 1];
        left[i] = (l === l && l !== Infinity && l !== -Infinity) ? l : 0.0;
        right[i] = (r === r && r !== Infinity && r !== -Infinity) ? r : 0.0;
      }
    } catch (e) {
      for (let i = 0; i < frames; i++) {
        left[i] = 0.0;
        right[i] = 0.0;
      }
    }

    this.frameCount++;
    if (this.frameCount % 8 === 0) {
      const step = this.wasm.tatum_current_step(this.tatumPtr) >>> 0;
      this.port.postMessage({ type: "step", step });
    }

    return true;
  }
}

registerProcessor("tatum-processor", TatumProcessor);
