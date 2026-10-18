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
  /** The song's next step, from the top. Not where a pattern is: see `tracks`. */
  step: number;
  /**
   * Each track's step in the pattern it plays, the one sounding now. A
   * track's count starts over when a new text takes over on a bar line.
   */
  tracks: Record<string, number>;
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
  /** Muted from the loop's key. */
  muted: boolean;
  /** Alone, from the mixer's solo. */
  soloed: boolean;
  /** The pattern it plays, and every one its voice offers, its own first. */
  play: string;
  patterns: string[];
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
  /** Every row's channel: level, pan, reverb_send, delay_send; `[]` for a row with nothing. */
  mix: Macro[][];
  comparing: boolean;
  knob_ccs: number[];
  /** The song's key as the text has it; `own` when the session sets it over the groove's. */
  key: { root?: string | null; kind?: string | null; own: boolean };
  /** Every parameter of the chosen row's module, by name: the whole instrument. */
  params: Macro[];
  /** Every role the library has voices for, in the order it lists them. */
  roles: string[];
  /** Every scale the language knows, by name. */
  scales: string[];
  /** The song's tempo as the text has it; `own` when the session sets it over the groove's. */
  tempo: { bpm: number; own: boolean };
}

export type ComposeAction =
  | { action: "choose"; row: number; by: number }
  | { action: "turn"; row: number; knob: number; value: number }
  | { action: "mute"; row: number }
  | { action: "solo"; row: number }
  /** A channel of the mixer, held like a knob and written by the row's save. */
  | { action: "mix"; row: number; what: "level" | "pan" | "reverb_send" | "delay_send"; value: number }
  | { action: "reset"; row: number }
  | { action: "compare" }
  | { action: "save"; row: number; name: string }
  /** The song in `root kind`, or without a root, the groove's own scale again. */
  | { action: "scale"; root?: string; kind?: string }
  /** The song at `bpm`, or without one, the groove's own tempo again. */
  | { action: "tempo"; bpm?: number }
  /** A row more for `role`, with the first of its voices not in the loop yet. */
  | { action: "add"; role: string }
  /** The row out of the loop; not the groove's. */
  | { action: "remove"; row: number }
  /** The next (or previous) pattern of the row's voice. */
  | { action: "vary"; row: number; by: number }
  /** Any parameter of the row's module, held like a knob and written by the row's save. */
  | { action: "set"; row: number; param: string; value: number }
  /** A voice from nothing: `kind` is bass, keys, fm or beats; a row for it. */
  | { action: "new"; role: string; kind: string; name: string };

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
  /** The MIDI notes it sounds: one for a note, several for a chord or a run. */
  midi?: number[];
  /** Each note as written, for a note, a chord or a run inside the step. */
  notes?: PatternNote[];
  /** Locked on the step, `null` where the text leaves it: the chance it
   * sounds (0-1), the gate, the cutoff, the resonance, the envelope depth. */
  prob?: number | null;
  gate?: number | null;
  cutoff?: number | null;
  res?: number | null;
  edepth?: number | null;
  /** A drum step split inside itself: each hit's velocity, 0 a rest. */
  hits?: number[];
}

export interface PatternNote {
  midi: number;
  /** As the pattern writes it: `1.2`, `C#3`. */
  text: string;
  slide: boolean;
  velocity: number | null;
}

/** A lane of a whole pattern to put: its drum, and a token a step. */
export interface LaneTokens {
  label?: string;
  tokens: string[];
}

/** The song's scale: the root's pitch class (0 for C) and its seven intervals. */
export interface PatternScale {
  root: number;
  intervals: number[];
  /** As the song writes it, `A minor`; absent when the song has none (C major). */
  name?: string | null;
}

/** One argument of an effect, in the unit its doc names. */
export interface FxArg {
  name: string;
  value: number;
  min: number;
  max: number;
  default: number;
  doc: string;
  /** Written `name=value`; otherwise by position. */
  named: boolean;
  /** The text writes it (a named one left out takes its default). */
  set: boolean;
}

/** An effect the language has, from `fxCatalog`. */
export interface FxSpec {
  name: string;
  category: "filter" | "effect" | "util" | string;
  doc: string;
  /** It also takes words: a division (`1/8`) or a mode (`saw`, `a`). */
  words: boolean;
  positional: FxArg[];
  named: FxArg[];
}

/** One effect of a chain, as `fxGet` gives it: every argument, `wet` last. */
export interface FxNode {
  kind: string;
  label?: string | null;
  doc: string;
  category: string;
  words: string[];
  args: FxArg[];
}

export interface Chain {
  track: string;
  nodes: FxNode[];
  /** Where it ends, kept as it is: `master`, a bus. */
  to: string[];
}

/** An effect for `fxSet`: what is left out takes its default. */
export interface FxNodeEdit {
  kind: string;
  label?: string | null;
  words?: string[];
  args?: { name: string; value: number; named?: boolean; set?: boolean }[];
}

export interface FxAnswer extends Answer {
  writes: { path: string; text: string }[];
  loaded: Loaded;
  fx: Chain;
  view: View;
}

/** One step of a pattern set to a token. */
export interface PatternEdit {
  lane: number;
  step: number;
  token: string;
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
  /** The lane names a drum pattern can take. */
  drum_names: string[];
  length: number;
  scale: PatternScale;
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

  /** A file the engine was handed, taken back. */
  forget(path: string): Promise<Answer> {
    return this.command({ op: "forget", path });
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
  /**
   * The loop in `root`. `library` is the directory its grooves and voices
   * are in when not beside it: `""` for a loop of a set,
   * `sets/jungle/01-intro.synth`. Opening another loop while one plays
   * swaps to it on the next bar.
   */
  composeOpen(
    root: string,
    options: { knobCcs?: number[]; library?: string } = {},
  ): Promise<Answer & { loaded: Loaded; view: View; session: string }> {
    return this.command({
      op: "compose_open",
      root,
      ...(options.knobCcs ? { knob_ccs: options.knobCcs } : {}),
      ...(options.library !== undefined ? { library: options.library } : {}),
    });
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

  /** Every effect a chain takes, with its arguments, for a picker. */
  fxCatalog(): Promise<Answer & { catalog: FxSpec[] }> {
    return this.command({ op: "fx_catalog" });
  }

  /** A track's chain: its effects in order, and where it ends. */
  fxGet(track: string): Promise<Answer & { fx: Chain }> {
    return this.command({ op: "fx_get", track });
  }

  /**
   * A track's chain set to `nodes`, rewritten in the file that defines the
   * track (where it ends kept) and reloaded. Refused whole, nothing
   * changed, when a node is not an effect or the file would not compile.
   */
  fxSet(track: string, nodes: FxNodeEdit[]): Promise<FxAnswer> {
    return this.command<FxAnswer>({ op: "fx_set", track, nodes });
  }

  /**
   * Several steps at once, in one rewrite and one reload: a note made
   * longer, a note moved. Taken whole or refused whole.
   */
  patternEdit(track: string, edits: PatternEdit[]): Promise<PatternAnswer> {
    return this.command<PatternAnswer>({ op: "pattern_set", track, edits });
  }

  /** The whole pattern as `lanes`: another length, a drum lane more, a step undone. */
  patternPut(track: string, lanes: LaneTokens[]): Promise<PatternAnswer> {
    return this.command<PatternAnswer>({ op: "pattern_put", track, lanes });
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
