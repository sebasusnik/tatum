/**
 * Main-thread bridge to the WASM synth running in an AudioWorklet.
 *
 * Sends .synth DSL source text to the worklet for parsing/compilation.
 * Transport (start/stop/reset) uses lightweight messages.
 */

export type StepCallback = (step: number) => void;

export interface DslError {
  line: number;
  col: number;
  msg: string;
}

export interface DslResult {
  ok: boolean;
  errors?: DslError[];
}

export type DslResultCallback = (result: DslResult) => void;

export interface TrackInfoData {
  name: string;
  kind: string;
  level: number;
  pan: number;
}
export type TrackInfoCallback = (tracks: TrackInfoData[]) => void;

export class SynthAudio {
  private ctx: AudioContext | null = null;
  private node: AudioWorkletNode | null = null;
  private onStep: StepCallback | null = null;
  private onDslResult: DslResultCallback | null = null;
  private onTrackInfo: TrackInfoCallback | null = null;
  private _ready = false;
  private readyPromise: Promise<void> | null = null;

  get ready(): boolean { return this._ready; }

  async init(onStep: StepCallback, onDslResult?: DslResultCallback): Promise<void> {
    if (this.ctx) return;
    this.onStep = onStep;
    this.onDslResult = onDslResult ?? null;
    this.ctx = new AudioContext({ sampleRate: 44100 });

    if (this.ctx.state === "suspended") {
      await this.ctx.resume();
    }

    await this.ctx.audioWorklet.addModule("/synth-processor.js");

    this.node = new AudioWorkletNode(this.ctx, "synth-processor", {
      numberOfInputs: 0,
      numberOfOutputs: 1,
      outputChannelCount: [2],
    });
    this.node.connect(this.ctx.destination);

    this.readyPromise = new Promise<void>((resolve) => {
      this.node!.port.onmessage = (e) => {
        const msg = e.data;
        if (msg.type === "ready") {
          this._ready = true;
          resolve();
        } else if (msg.type === "step" && this.onStep) {
          this.onStep(msg.step);
        } else if (msg.type === "dsl-result" && this.onDslResult) {
          this.onDslResult(msg.result);
        } else if (msg.type === "track-info" && this.onTrackInfo) {
          this.onTrackInfo(msg.tracks);
        } else if (msg.type === "error") {
          console.error("[synth-processor]", msg.message);
        }
      };
    });

    const wasmUrl = new URL("../wasm-pkg/synth_wasm_bg.wasm", import.meta.url).href;
    const wasmResponse = await fetch(wasmUrl);
    const wasmBytes = await wasmResponse.arrayBuffer();
    this.node.port.postMessage({ type: "init-wasm", bytes: wasmBytes }, [wasmBytes]);

    await this.readyPromise;
  }

  // ═══════════════════════════════════════════════════════
  //  DSL source loading
  // ═══════════════════════════════════════════════════════

  /** Send .synth DSL source text for parsing/compilation/loading. */
  loadSource(source: string): void {
    // Encode on the main thread — TextEncoder is not available in AudioWorklet scope
    const bytes = new TextEncoder().encode(source);
    this.node?.port.postMessage({ type: "load-source", bytes }, [bytes.buffer]);
  }

  /** Update the DSL result callback. */
  setDslResultCallback(cb: DslResultCallback): void {
    this.onDslResult = cb;
  }

  // ═══════════════════════════════════════════════════════
  //  Transport
  // ═══════════════════════════════════════════════════════

  start(): void {
    this.node?.port.postMessage({ type: "transport", action: "start" });
  }

  stop(): void {
    this.node?.port.postMessage({ type: "transport", action: "stop" });
  }

  reset(): void {
    this.node?.port.postMessage({ type: "transport", action: "reset" });
  }

  // ═══════════════════════════════════════════════════════
  //  Real-time track control (no recompile)
  // ═══════════════════════════════════════════════════════

  setTrackLevel(idx: number, level: number): void {
    this.node?.port.postMessage({ type: "set-track-level", idx, value: level });
  }

  setTrackPan(idx: number, pan: number): void {
    this.node?.port.postMessage({ type: "set-track-pan", idx, value: pan });
  }

  // ═══════════════════════════════════════════════════════
  //  Runtime mutations (no recompile)
  // ═══════════════════════════════════════════════════════

  setTempo(bpm: number): void {
    this.node?.port.postMessage({ type: "set-tempo", value: bpm });
  }

  setTrackPattern(trackIdx: number, patternIdx: number): void {
    this.node?.port.postMessage({ type: "set-track-pattern", idx: trackIdx, patternIdx });
  }

  setTrackVelocity(trackIdx: number, velocity: number): void {
    this.node?.port.postMessage({ type: "set-track-velocity", idx: trackIdx, value: velocity });
  }

  setModuleParam(instIdx: number, paramName: string, value: number): void {
    const nameBytes = new TextEncoder().encode(paramName);
    this.node?.port.postMessage({ type: "set-module-param", instIdx, nameBytes, value }, [nameBytes.buffer]);
  }

  /** Request track info from the engine. Result arrives via onTrackInfo callback. */
  requestTrackInfo(cb: TrackInfoCallback): void {
    this.onTrackInfo = cb;
    this.node?.port.postMessage({ type: "get-track-info" });
  }

  // ── Lifecycle ──
  destroy(): void {
    this.node?.disconnect();
    this.ctx?.close();
    this.node = null;
    this.ctx = null;
    this._ready = false;
  }
}
