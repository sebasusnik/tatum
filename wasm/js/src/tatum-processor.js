// The AudioWorklet: the engine on the audio thread. The page sends the
// module's bytes once (a worklet cannot fetch), then every message of
// `tatum_core::command` as `{type: "command", id, message}`, and gets each
// answer back as `{type: "answer", id, answer}`. Twenty times a second it
// also gets `{type: "position", answer}`, the engine's `position` message,
// so a screen can follow the bar without asking.
//
// The polyfill goes first: wasm-bindgen's glue makes a TextDecoder the
// moment it is imported, and the worklet scope has none.

import "./text-codec.js";
import { initSync, Tatum, wasm_memory } from "../pkg/tatum_wasm.js";

/// The engine renders this many frames a call; a quantum is walked in
/// steps of it, so a context with `renderSizeHint` other than 128 works.
const BLOCK = 128;

class TatumProcessor extends AudioWorkletProcessor {
  constructor() {
    super();
    this.tatum = null;
    this.memory = null;
    this.frames = 0;
    this.port.onmessage = (e) => this.onMessage(e.data);
  }

  onMessage(msg) {
    if (msg.type === "init") {
      try {
        initSync({ module: msg.bytes });
        this.tatum = new Tatum();
        this.memory = wasm_memory();
        this.port.postMessage({ type: "ready" });
      } catch (err) {
        this.port.postMessage({ type: "error", message: String(err) });
      }
      return;
    }
    if (msg.type === "command") {
      const answer = this.tatum
        ? this.tatum.command(msg.message)
        : '{"ok":false,"error":"the engine is not ready"}';
      this.port.postMessage({ type: "answer", id: msg.id, answer });
    }
  }

  process(_inputs, outputs) {
    if (!this.tatum) return true;
    const output = outputs[0];
    if (!output || output.length < 2) return true;
    const [left, right] = output;
    const frames = left.length;
    for (let done = 0; done < frames; done += BLOCK) {
      const n = Math.min(BLOCK, frames - done);
      const ptr = this.tatum.process(n) >>> 0;
      const mem = new Float32Array(this.memory.buffer, ptr, n * 2);
      for (let i = 0; i < n; i++) {
        const l = mem[2 * i];
        const r = mem[2 * i + 1];
        left[done + i] = Number.isFinite(l) ? l : 0;
        right[done + i] = Number.isFinite(r) ? r : 0;
      }
    }
    this.frames += frames;
    if (this.frames >= sampleRate / 20) {
      this.frames = 0;
      this.port.postMessage({ type: "position", answer: this.tatum.command('{"op":"position"}') });
    }
    return true;
  }
}

registerProcessor("tatum-processor", TatumProcessor);
