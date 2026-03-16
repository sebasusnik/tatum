/**
 * Main-thread bridge between the Solid.js UI and the WASM synth
 * running inside an AudioWorklet.
 *
 * Framework-agnostic — receives a step callback, exposes typed methods.
 * All communication with the worklet happens via postMessage.
 */

export type StepCallback = (step: number) => void;

export class SynthAudio {
  private ctx: AudioContext | null = null;
  private node: AudioWorkletNode | null = null;
  private onStep: StepCallback | null = null;
  private _ready = false;
  private readyPromise: Promise<void> | null = null;

  get ready(): boolean {
    return this._ready;
  }

  async init(onStep: StepCallback): Promise<void> {
    if (this.ctx) return; // already initialized

    this.onStep = onStep;
    this.ctx = new AudioContext({ sampleRate: 44100 });

    // Resume if suspended (browser autoplay policy)
    if (this.ctx.state === "suspended") {
      await this.ctx.resume();
    }

    // Load the worklet processor from public/ (served as-is, no Vite transforms)
    await this.ctx.audioWorklet.addModule("/synth-processor.js");

    // Create the worklet node — stereo output, no inputs
    this.node = new AudioWorkletNode(this.ctx, "synth-processor", {
      numberOfInputs: 0,
      numberOfOutputs: 1,
      outputChannelCount: [2],
    });
    this.node.connect(this.ctx.destination);

    // Listen for messages from the worklet
    this.readyPromise = new Promise<void>((resolve) => {
      this.node!.port.onmessage = (e) => {
        const msg = e.data;
        if (msg.type === "ready") {
          this._ready = true;
          resolve();
        } else if (msg.type === "step" && this.onStep) {
          this.onStep(msg.step);
        } else if (msg.type === "error") {
          console.error("[synth-processor]", msg.message);
        }
      };
    });

    // Fetch WASM binary and send raw bytes to worklet.
    // The worklet compiles + instantiates it on the audio thread.
    const wasmUrl = new URL("../wasm-pkg/synth_wasm_bg.wasm", import.meta.url).href;
    const wasmResponse = await fetch(wasmUrl);
    const wasmBytes = await wasmResponse.arrayBuffer();

    console.log("[SynthAudio] sending WASM bytes to worklet...", wasmBytes.byteLength, "bytes");
    this.node.port.postMessage({ type: "init-wasm", bytes: wasmBytes }, [wasmBytes]);

    await this.readyPromise;
    console.log("[SynthAudio] worklet ready!");
  }

  // ── Transport ────────────────────────────────────────

  setActiveModule(module: number): void {
    this.post({ type: "set_active_module", module });
  }

  start(): void {
    this.post({ type: "start" });
  }

  stop(): void {
    this.post({ type: "stop" });
  }

  reset(): void {
    this.post({ type: "reset" });
  }

  setBpm(bpm: number): void {
    this.post({ type: "set_bpm", value: bpm });
  }

  // ── Parameters ───────────────────────────────────────

  setParam(paramId: number, value: number): void {
    this.post({ type: "set_param", paramId, value });
  }

  // ── Notes ────────────────────────────────────────────

  noteOn(module: number, note: number, velocity: number): void {
    this.post({ type: "note_on", module, note, velocity });
  }

  noteOff(module: number, note: number): void {
    this.post({ type: "note_off", module, note });
  }

  // ── Sequencer Steps ──────────────────────────────────

  setStep(idx: number, note: number, velocity: number, gate: number): void {
    this.post({ type: "set_step", idx, note, velocity, gate });
  }

  clearStep(idx: number): void {
    this.post({ type: "clear_step", idx });
  }

  setStepSlide(idx: number, slide: boolean): void {
    this.post({ type: "set_step_slide", idx, slide });
  }

  setStepLockSlide(idx: number, lockSlide: boolean): void {
    this.post({ type: "set_step_lock_slide", idx, lockSlide });
  }

  setStepLock(idx: number, paramId: number, value: number): void {
    this.post({ type: "set_step_lock", idx, paramId, value });
  }

  clearStepLocks(idx: number): void {
    this.post({ type: "clear_step_locks", idx });
  }

  setStepActive(idx: number, active: boolean): void {
    this.post({ type: "set_step_active", idx, active });
  }

  setStepProbability(idx: number, prob: number): void {
    this.post({ type: "set_step_probability", idx, prob });
  }

  setNumSteps(n: number): void {
    this.post({ type: "set_num_steps", n });
  }

  setSwing(value: number): void {
    this.post({ type: "set_swing", value });
  }

  setHumanize(value: number): void {
    this.post({ type: "set_humanize", value });
  }

  // ── Effects ──────────────────────────────────────────

  setDelayTime(value: number): void {
    this.post({ type: "set_delay_time", value });
  }

  setDelayFeedback(value: number): void {
    this.post({ type: "set_delay_feedback", value });
  }

  setDelayFilter(value: number): void {
    this.post({ type: "set_delay_filter", value });
  }

  setReverbSize(value: number): void {
    this.post({ type: "set_reverb_size", value });
  }

  setReverbDamping(value: number): void {
    this.post({ type: "set_reverb_damping", value });
  }

  setReverbMix(value: number): void {
    this.post({ type: "set_reverb_mix", value });
  }

  setEqLow(value: number): void {
    this.post({ type: "set_eq_low", value });
  }

  setEqMid(value: number): void {
    this.post({ type: "set_eq_mid", value });
  }

  setEqHigh(value: number): void {
    this.post({ type: "set_eq_high", value });
  }

  setCompressorThreshold(value: number): void {
    this.post({ type: "set_compressor_threshold", value });
  }

  setCompressorRatio(value: number): void {
    this.post({ type: "set_compressor_ratio", value });
  }

  setSidechain(value: number): void {
    this.post({ type: "set_sidechain", value });
  }

  setPitchBend(value: number): void {
    this.post({ type: "set_pitch_bend", value });
  }

  // ── Harmony ──────────────────────────────────────────

  setHarmony(root: number, scale: number, chordDegree: number): void {
    this.post({ type: "set_harmony", root, scale, chordDegree });
  }

  // ── Lifecycle ────────────────────────────────────────

  destroy(): void {
    this.node?.disconnect();
    this.ctx?.close();
    this.node = null;
    this.ctx = null;
    this._ready = false;
  }

  // ── Internal ─────────────────────────────────────────

  private post(msg: Record<string, unknown>): void {
    this.node?.port.postMessage(msg);
  }
}
