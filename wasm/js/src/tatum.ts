/**
 * The page side of the engine: an AudioContext, the worklet, and every
 * message of `tatum_core::command` as a method that returns its answer.
 *
 *   const t = await Tatum.create({ wasmUrl, processorUrl });
 *   await t.files({ "session.synth": text, "grooves/dub.synth": ... });
 *   await t.load("session.synth");
 *   t.start();
 *   await t.tryKnob("bass.cutoff", 96);
 *   t.on("position", (p) => bar.textContent = String(p.bar + 1));
 */

/** Every answer: `ok`, and `error` when it is not. */
export interface Answer {
  ok: boolean;
  error?: string;
  [key: string]: unknown;
}

export interface Position {
  bar: number;
  step: number;
  tempo: number;
  generation: number;
  running: boolean;
  swaps: number;
}

export interface Loaded extends Answer {
  plan?: string;
  applied?: string;
  files?: string[];
  errors?: { line: number; col: number; msg: string }[];
  lines?: string[];
}

export interface KnobTarget {
  words: string;
  target: string;
  doc: string;
}

export interface Track {
  name: string;
  instrument: string;
  play: string;
  level?: number;
  pan?: number;
  muted?: boolean;
  knobs: KnobTarget[];
}

export interface Song extends Answer {
  tempo: number;
  tracks: Track[];
  bindings: { cc: number; target: string }[];
  patterns: string[];
  scenes: string[];
}

export interface Row {
  role: string;
  name: string | null;
  kind: string;
  doc: string;
  track: string;
  index: number | null;
  count: number;
  held: boolean;
}

export interface Macro {
  label: string;
  position: number;
  reading: string;
  held: boolean;
}

export interface View {
  focus: number;
  rows: Row[];
  macros: Macro[];
  comparing: boolean;
  knob_ccs: number[];
}

export type ComposeAction =
  | { action: "choose"; row: number; by: number }
  | { action: "turn"; row: number; knob: number; value: number }
  | { action: "mute"; row: number }
  | { action: "reset"; row: number }
  | { action: "compare" }
  | { action: "save"; row: number; name: string };

export interface ComposeAnswer extends Answer {
  said: string;
  writes: { path: string; text: string }[];
  loaded: Loaded | null;
  view: View;
}

export interface PatternStep {
  /** As the pattern writes it: `1.2:0.9`, `x`, `..`, `[1.3 5.3]`. */
  token: string;
  kind: "rest" | "tie" | "note" | "chord" | "hit" | "sub" | "drumsub";
  note?: string;
  velocity?: number | null;
  slide?: boolean;
  roll?: number;
}

export interface PatternLane {
  /** The drum, or `null` for a melodic pattern's one lane. */
  label: string | null;
  steps: PatternStep[];
}

export interface Pattern {
  track: string;
  name: string;
  drums: boolean;
  steps_per_bar: number;
  length: number;
  lanes: PatternLane[];
}

export interface PatternAnswer extends Answer {
  writes: { path: string; text: string }[];
  loaded: Loaded;
  pattern: Pattern;
}

export interface CreateOptions {
  /** `tatum_wasm_bg.wasm`, fetched here and handed to the worklet. */
  wasmUrl: string | URL;
  /** `tatum-processor.js`, served beside `pkg/`. */
  processorUrl: string | URL;
  /** The engine renders at 44.1 kHz; another rate is resampled by the browser. */
  sampleRate?: number;
  /** An AudioContext of your own, else one is made. */
  context?: AudioContext;
}

type Listener = (position: Position) => void;

export class Tatum {
  private next = 1;
  private pending = new Map<number, (answer: string) => void>();
  private listeners = new Set<Listener>();
  private midiInputs: { close(): void }[] = [];

  private constructor(
    public readonly context: AudioContext,
    public readonly node: AudioWorkletNode,
  ) {
    node.port.onmessage = (e) => this.onMessage(e.data);
  }

  static async create(options: CreateOptions): Promise<Tatum> {
    const context = options.context ?? new AudioContext({ sampleRate: options.sampleRate ?? 44100 });
    await context.audioWorklet.addModule(options.processorUrl);
    const node = new AudioWorkletNode(context, "tatum-processor", {
      numberOfInputs: 0,
      numberOfOutputs: 1,
      outputChannelCount: [2],
    });
    node.connect(context.destination);
    const tatum = new Tatum(context, node);
    const bytes = await (await fetch(options.wasmUrl)).arrayBuffer();
    await new Promise<void>((resolve, reject) => {
      const onReady = (e: MessageEvent) => {
        if (e.data?.type === "ready") resolve();
        else if (e.data?.type === "error") reject(new Error(e.data.message));
        else return;
        tatum.node.port.removeEventListener("message", onReady);
      };
      tatum.node.port.addEventListener("message", onReady);
      tatum.node.port.postMessage({ type: "init", bytes }, [bytes]);
    });
    return tatum;
  }

  private onMessage(msg: { type: string; id?: number; answer?: string; message?: string }) {
    if (msg.type === "answer" && msg.id !== undefined) {
      const resolve = this.pending.get(msg.id);
      this.pending.delete(msg.id);
      resolve?.(msg.answer ?? "");
    } else if (msg.type === "position" && msg.answer) {
      const position = JSON.parse(msg.answer) as Position & Answer;
      if (position.ok) for (const l of this.listeners) l(position);
    }
  }

  /** Any message of `tatum_core::command`, by its `op`. */
  command<T extends Answer = Answer>(message: Record<string, unknown>): Promise<T> {
    const id = this.next++;
    return new Promise<T>((resolve) => {
      this.pending.set(id, (answer) => resolve(JSON.parse(answer) as T));
      this.node.port.postMessage({ type: "command", id, message: JSON.stringify(message) });
    });
  }

  /** Files handed over, added to or replacing what was: `{"session.synth": text}`. */
  files(files: Record<string, string>): Promise<Answer> {
    return this.command({ op: "files", files });
  }

  /** The song to play, its `use` lines resolved over the files. */
  load(root: string): Promise<Loaded> {
    return this.command<Loaded>({ op: "load", root });
  }

  /** The same root again, after its files changed. */
  reload(): Promise<Loaded> {
    return this.command<Loaded>({ op: "reload" });
  }

  /** A whole text, no `use` lines. */
  text(source: string): Promise<Loaded> {
    return this.command<Loaded>({ op: "text", source });
  }

  start(): Promise<Answer> {
    if (this.context.state === "suspended") void this.context.resume();
    return this.command({ op: "start" });
  }

  stop(): Promise<Answer> {
    return this.command({ op: "stop" });
  }

  /** One message from a controller, as its bytes. */
  midi(bytes: ArrayLike<number>): Promise<Answer & { readings: string[] }> {
    return this.command({ op: "midi", bytes: Array.from(bytes) });
  }

  knob(cc: number, value: number): Promise<Answer & { readings: string[] }> {
    return this.command({ op: "knob", cc, value });
  }

  /** A controller value on a target by name, as if a `midi` line mapped it. */
  tryKnob(target: string, value: number): Promise<Answer & { readings: string[] }> {
    return this.command({ op: "try_knob", target, value });
  }

  /** Where the text leaves a target (0..127) and what it reads there. */
  probeKnob(target: string): Promise<Answer & { found: boolean; rest: number | null; reading: string | null }> {
    return this.command({ op: "probe_knob", target });
  }

  mute(track: string): Promise<Answer> {
    return this.command({ op: "mute", track });
  }

  solo(track: string): Promise<Answer & { soloed: string[] }> {
    return this.command({ op: "solo", track });
  }

  resetKnobs(): Promise<Answer> {
    return this.command({ op: "reset_knobs" });
  }

  scene(name: string): Promise<Answer & { scene: string }> {
    return this.command({ op: "scene", name });
  }

  scenes(): Promise<Answer & { scenes: string[] }> {
    return this.command({ op: "scenes" });
  }

  pages(): Promise<Answer & { pages: { name: string; on: boolean; alive: boolean }[] }> {
    return this.command({ op: "pages" });
  }

  voice(to: "next" | "prev" | string): Promise<Answer & { said: string }> {
    return this.command({ op: "voice", to });
  }

  position(): Promise<Position & Answer> {
    return this.command({ op: "position" });
  }

  song(): Promise<Song> {
    return this.command<Song>({ op: "song" });
  }

  /** The loop: the library beside `root` among the files, the session its text or new (`session` is that text). */
  composeOpen(root: string, knobCcs?: number[]): Promise<Answer & { loaded: Loaded; view: View; session: string }> {
    return this.command({ op: "compose_open", root, ...(knobCcs ? { knob_ccs: knobCcs } : {}) });
  }

  compose(action: ComposeAction): Promise<ComposeAnswer> {
    return this.command<ComposeAnswer>({ op: "compose", ...action });
  }

  /** The pattern a track plays, as a grid. */
  patternGet(track: string): Promise<Answer & { pattern: Pattern }> {
    return this.command({ op: "pattern_get", track });
  }

  /**
   * One step of a track's pattern set to `token` (`x`, `1.2:0.8`, `..`,
   * `~5.2`, `[1.3 5.3]`), rewritten in the file that defines the pattern
   * and reloaded on the next bar. Refused, with nothing changed, when the
   * token is not one step or the file would not compile.
   */
  patternSet(track: string, lane: number, step: number, token: string): Promise<PatternAnswer> {
    return this.command<PatternAnswer>({ op: "pattern_set", track, lane, step, token });
  }

  /** The rows and the knobs of the row chosen; `focus` chooses it. */
  composeView(focus?: number): Promise<Answer & { view: View }> {
    return this.command({ op: "compose_view", ...(focus === undefined ? {} : { focus }) });
  }

  /** The engine's position, twenty times a second. Returns what stops listening. */
  on(event: "position", listener: Listener): () => void {
    if (event !== "position") throw new Error(`no event called ${event}`);
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  /**
   * Every MIDI input the browser has (Chrome, Edge; Firefox with the
   * permission), or those whose name `accept` takes, forwarded to the
   * engine as the terminal's `--midi` would. Returns their names.
   */
  async connectMidi(accept: (name: string) => boolean = () => true): Promise<string[]> {
    if (!("requestMIDIAccess" in navigator)) throw new Error("this browser has no WebMIDI");
    const access = await navigator.requestMIDIAccess();
    const names: string[] = [];
    for (const input of access.inputs.values()) {
      const name = input.name ?? "?";
      if (!accept(name)) continue;
      input.onmidimessage = (e: MIDIMessageEvent) => {
        if (e.data) void this.midi(e.data);
      };
      this.midiInputs.push({ close: () => void input.close() });
      names.push(name);
    }
    return names;
  }

  destroy(): void {
    for (const i of this.midiInputs) i.close();
    this.midiInputs = [];
    this.node.disconnect();
    void this.context.close();
  }
}
