// The app's state: one engine, the loop's view as the engine last gave it,
// where the engine is, and what it last said. Every change goes through
// the engine and comes back as a view; nothing here knows what a voice is.

import { createSignal } from "solid-js";
import type { ComposeAction, Pattern, Position, Tatum, View } from "@tatum/engine";
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

export { view, position, said, running, midi, analyser, pattern };

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
  t.on("position", setPosition);
  await t.files({ ...shipped(), ...stored() });
  const opened = await t.composeOpen(SESSION);
  if (!opened.ok) throw new Error(opened.error ?? "the loop did not open");
  setView(opened.view);
  store(SESSION, opened.session);
  engine = t;
  await refreshPattern();
  return t;
}

/** The chosen row's pattern, from the engine, as the text has it now. */
async function refreshPattern(): Promise<void> {
  const v = view();
  const track = v?.rows[v.focus]?.track;
  if (!engine || !track) {
    setPattern(null);
    return;
  }
  const a = await engine.patternGet(track);
  setPattern(a.ok ? a.pattern : null);
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
  if (action.action !== "turn") await refreshPattern();
}

export async function focus(row: number): Promise<void> {
  if (!engine) return;
  const a = await engine.composeView(row);
  if (a.ok) setView(a.view);
  await refreshPattern();
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
