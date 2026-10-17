// The app's state: one engine, the loop's view as the engine last gave it,
// where the engine is, and what it last said. Every change goes through
// the engine and comes back as a view; nothing here knows what a voice is.

import { createSignal } from "solid-js";
import type { Chain, ComposeAction, FxNodeEdit, FxSpec, Pattern, PatternEdit, Position, Tatum, View } from "@tatum/engine";
import { Tatum as Engine } from "@tatum/engine";
import { download, shipped, store, stored } from "./library";

export const SESSION = "session.synth";

const [view, setView] = createSignal<View | null>(null);
const [position, setPosition] = createSignal<Position | null>(null);
const [said, setSaid] = createSignal<{ text: string; bad: boolean }>({ text: "press play", bad: false });
const [running, setRunning] = createSignal(false);
const [midi, setMidi] = createSignal<string[]>([]);
const [analyser, setAnalyser] = createSignal<AnalyserNode | null>(null);
const [pattern, setPattern] = createSignal<Pattern | null>(null);
const [chain, setChain] = createSignal<Chain | null>(null);
const [catalog, setCatalog] = createSignal<FxSpec[]>([]);

export { view, position, said, running, midi, analyser, pattern, chain, catalog };

let engine: Tatum | null = null;

function say(text: string, bad = false): void {
  setSaid({ text, bad });
}

/** The engine, made on the first play: a browser wants a gesture first. */
async function boot(): Promise<Tatum> {
  if (engine) return engine;
  const t = await Engine.create({
    wasmUrl: new URL("/engine/pkg/tatum_wasm_bg.wasm", location.href),
    processorUrl: new URL("/engine/src/tatum-processor.js", location.href),
  });
  const a = t.context.createAnalyser();
  a.fftSize = 2048;
  a.smoothingTimeConstant = 0.6;
  t.node.connect(a);
  setAnalyser(a);
  // The engine tells where it has rendered to; the speakers get there
  // later by the output's latency (a lot more over Bluetooth), so the
  // playhead waits that long to show it.
  t.on("position", (p) => {
    const ctx = t.context;
    const late = ((ctx.outputLatency || 0) + (ctx.baseLatency || 0)) * 1000;
    if (late < 5) setPosition(p);
    else setTimeout(() => setPosition(p), late);
  });
  await t.files({ ...shipped(), ...stored() });
  const opened = await t.composeOpen(SESSION);
  if (!opened.ok) throw new Error(opened.error ?? "the loop did not open");
  setView(opened.view);
  store(SESSION, opened.session);
  engine = t;
  const c = await t.fxCatalog();
  if (c.ok) setCatalog(c.catalog);
  await refreshRow();
  return t;
}

/** The chosen row's pattern and chain, from the engine, as the text has them now. */
async function refreshRow(): Promise<void> {
  const v = view();
  const track = v?.rows[v.focus]?.track;
  if (!engine || !track) {
    setPattern(null);
    setChain(null);
    return;
  }
  const [p, f] = await Promise.all([engine.patternGet(track), engine.fxGet(track)]);
  setPattern(p.ok ? p.pattern : null);
  setChain(f.ok ? f.fx : null);
}

/** The chosen row's chain set to `nodes`: one rewrite of its routing. */
export async function setFx(nodes: FxNodeEdit[], what: string): Promise<boolean> {
  const c = chain();
  if (!engine || !c) return false;
  const a = await engine.fxSet(c.track, nodes);
  if (!a.ok) {
    say(a.error ?? "refused", true);
    return false;
  }
  for (const w of a.writes) store(w.path, w.text);
  setChain(a.fx);
  setView(a.view);
  say(`${c.track}: ${what}`);
  return true;
}

/** One step of the chosen row's pattern, as a token. */
export async function setStep(lane: number, step: number, token: string): Promise<void> {
  const p = pattern();
  if (!engine || !p) return;
  const a = await engine.patternSet(p.track, lane, step, token);
  if (!a.ok) {
    say(a.error ?? "refused", true);
    return;
  }
  for (const w of a.writes) store(w.path, w.text);
  setPattern(a.pattern);
  say(`${p.track}: step ${step + 1} is ${token} · on the next bar`);
}

/** Several steps of the chosen row's pattern in one rewrite: a note drawn longer, moved. */
export async function editSteps(edits: PatternEdit[], what: string): Promise<boolean> {
  const p = pattern();
  if (!engine || !p || edits.length === 0) return false;
  const a = await engine.patternEdit(p.track, edits);
  if (!a.ok) {
    say(a.error ?? "refused", true);
    return false;
  }
  for (const w of a.writes) store(w.path, w.text);
  setPattern(a.pattern);
  say(`${p.track}: ${what} · on the next bar`);
  return true;
}

export async function play(): Promise<void> {
  try {
    const t = await boot();
    await t.start();
    setRunning(true);
    say("playing");
  } catch (e) {
    say(String(e), true);
  }
}

export async function stop(): Promise<void> {
  if (!engine) return;
  await engine.stop();
  setRunning(false);
  say("stopped");
}

/** A key or a knob on the loop: the engine answers with the view. */
export async function act(action: ComposeAction): Promise<void> {
  if (!engine) return;
  const a = await engine.compose(action);
  if (!a.ok) {
    say(a.error ?? "refused", true);
    return;
  }
  for (const w of a.writes) {
    store(w.path, w.text);
    if (w.path !== SESSION) download(w.path, w.text);
  }
  setView(a.view);
  say(a.said);
  if (action.action !== "turn" && action.action !== "mix") await refreshRow();
}

export async function focus(row: number): Promise<void> {
  if (!engine) return;
  const a = await engine.composeView(row);
  if (a.ok) setView(a.view);
  await refreshRow();
}

export async function connectMidi(): Promise<void> {
  try {
    const t = await boot();
    const names = await t.connectMidi();
    setMidi(names);
    say(names.length ? `midi: ${names.join(", ")}` : "midi: no inputs");
  } catch (e) {
    say(String(e), true);
  }
}

export function sessionText(): string {
  return stored()[SESSION] ?? "";
}
