// The chosen row's effects as a row of pedals, from the voice into where
// the chain ends. A + between two opens the language's effects to put one
// there; on a pedal, ◀ ▶ move it, × takes it out, its knobs turn its
// arguments (MORE shows the named ones) and WET blends it. A knob is drawn
// at once and sent a moment after it stops; a pedal added, moved or taken
// out is sent at once. Every change rewrites the track's routing in its
// file, and the engine takes it on the next bar.

import { For, Index, Show, createSignal } from "solid-js";
import type { FxArg, FxNode, FxNodeEdit, FxSpec } from "@tatum/engine";
import { catalog, chain, setFx } from "../store";
import { Knob, type Tone } from "./Knob";

/** Where an argument sits on a knob: by ratio for the wide ranges (Hz, ms), straight otherwise. */
function isLog(a: FxArg): boolean {
  return a.min > 0 && a.max / a.min >= 50;
}

function toKnob(a: FxArg): number {
  const v = Math.max(a.min, Math.min(a.max, a.value));
  const t = isLog(a) ? Math.log(v / a.min) / Math.log(a.max / a.min) : (v - a.min) / (a.max - a.min || 1);
  return Math.round(t * 127);
}

function fromKnob(a: FxArg, k: number): number {
  const t = k / 127;
  const v = isLog(a) ? a.min * Math.pow(a.max / a.min, t) : a.min + t * (a.max - a.min);
  const step = Math.abs(v) >= 100 ? 1 : Math.abs(v) >= 10 ? 0.1 : 0.01;
  return Math.round(v / step) * step;
}

function reading(a: FxArg): string {
  const v = a.value;
  const plain = (x: number) => String(Math.round(x * 100) / 100);
  if (a.doc.startsWith("Hz")) return v >= 1000 ? `${plain(v / 1000)}khz` : `${Math.round(v)}hz`;
  if (a.doc.startsWith("ms")) return `${plain(v)}ms`;
  if (a.doc.startsWith("dB")) return `${plain(v)}db`;
  if (a.doc.startsWith("linear gain")) return `${plain(20 * Math.log10(Math.max(1e-6, v)))}db`;
  return plain(v);
}

/** A colour for each kind of effect, Bauhaus primaries. */
function toneOf(category: string): string {
  return category === "filter" ? "blue" : category === "util" ? "yellow" : "orange";
}

const KNOB_TONES: Tone[] = ["white", "black", "white", "black"];

/** Words a new pedal needs to compile; others take none. */
const FIRST_WORDS: Record<string, string[]> = { vowel: ["a"] };

function fresh(spec: FxSpec): FxNode {
  return {
    kind: spec.name,
    label: null,
    doc: spec.doc,
    category: spec.category,
    words: FIRST_WORDS[spec.name] ?? [],
    args: [
      ...spec.positional.map((a) => ({ ...a, set: true })),
      ...spec.named.map((a) => ({ ...a, set: false })),
      { name: "wet", value: 1, min: 0, max: 1, default: 1, doc: "dry to wet", named: true, set: false },
    ],
  };
}

function edit(nodes: FxNode[]): FxNodeEdit[] {
  return nodes.map((n) => ({
    kind: n.kind,
    label: n.label ?? null,
    words: n.words,
    args: n.args.map((a) => ({ name: a.name, value: a.value, named: a.named, set: a.set })),
  }));
}

export function Fx() {
  const [draft, setDraft] = createSignal<FxNode[] | null>(null);
  const [picking, setPicking] = createSignal<number | null>(null);
  const [open, setOpen] = createSignal<Set<number>>(new Set());
  const nodes = () => draft() ?? chain()?.nodes ?? [];
  let timer: ReturnType<typeof setTimeout> | undefined;
  let version = 0;

  const send = async (next: FxNode[], what: string) => {
    const mine = ++version;
    setDraft(next);
    await setFx(edit(next), what);
    if (mine === version) setDraft(null);
  };
  /** A knob: drawn now, sent when it has been still for a moment. */
  const turn = (i: number, name: string, k: number) => {
    const next = nodes().map((n, j) =>
      j !== i ? n : { ...n, args: n.args.map((a) => (a.name !== name ? a : { ...a, value: fromKnob(a, k), set: true })) },
    );
    setDraft(next);
    version++;
    clearTimeout(timer);
    const what = `${next[i].kind} ${name} ${reading(next[i].args.find((a) => a.name === name)!)}`;
    timer = setTimeout(() => void send(next, what), 160);
  };
  const add = (at: number, spec: FxSpec) => {
    setPicking(null);
    const next = [...nodes()];
    next.splice(at, 0, fresh(spec));
    void send(next, `${spec.name} in`);
  };
  const remove = (i: number) => {
    const next = nodes().filter((_, j) => j !== i);
    void send(next, `${nodes()[i].kind} out`);
  };
  const shift = (i: number, by: number) => {
    const j = i + by;
    if (j < 0 || j >= nodes().length) return;
    const next = [...nodes()];
    [next[i], next[j]] = [next[j], next[i]];
    void send(next, `${next[j].kind} moved`);
  };
  const rename = (i: number) => {
    const n = nodes()[i];
    const label = prompt("A name for this pedal, so a knob or an `auto` can reach it (letters, digits, _). Empty takes it off.", n.label ?? "");
    if (label === null) return;
    const next = nodes().map((m, j) => (j === i ? { ...m, label: label.trim() || null } : m));
    void send(next, `${n.kind} ${label.trim() ? `as ${label.trim()}` : "unnamed"}`);
  };
  const words = (i: number) => {
    const n = nodes()[i];
    const got = prompt(`Words for ${n.kind}, by commas (a division like 1/8, a mode, a vowel):`, n.words.join(", "));
    if (got === null) return;
    const next = nodes().map((m, j) => (j === i ? { ...m, words: got.split(",").map((w) => w.trim()).filter(Boolean) } : m));
    void send(next, `${n.kind} ${got}`);
  };
  const toggle = (i: number) => {
    const s = new Set(open());
    if (s.has(i)) s.delete(i);
    else s.add(i);
    setOpen(s);
  };
  const groups = () => {
    const by: Record<string, FxSpec[]> = {};
    for (const s of catalog()) (by[s.category] ??= []).push(s);
    return Object.entries(by);
  };

  const Gap = (props: { at: number }) => (
    <button class={`gap ${picking() === props.at ? "lit" : ""}`} onClick={() => setPicking(picking() === props.at ? null : props.at)} title="put an effect here">
      +
    </button>
  );

  return (
    <Show when={chain()} fallback={<section class="panel fx"><p class="hint">choose a row with a voice</p></section>}>
      <section class="panel fx">
        <header class="panel-head">
          <span class="label">FX</span>
          <span class="heading">{chain()!.track}</span>
          <span class="hint">+ puts an effect in · ◀ ▶ move it · × takes it out · its name is what a knob reaches</span>
        </header>
        <div class="chain">
          <div class="jack">IN</div>
          <Index each={nodes()}>
            {(n, i) => (
              <>
                <Gap at={i} />
                <div class={`pedal ${toneOf(n().category)}`} title={n().doc}>
                  <div class="pedal-head">
                    <span class="pedal-kind">{n().kind.toUpperCase()}</span>
                    <span class="spacer" />
                    <button class="key small" onClick={() => shift(i, -1)} title="earlier in the chain" disabled={i === 0}>
                      ◀
                    </button>
                    <button class="key small" onClick={() => shift(i, 1)} title="later in the chain" disabled={i === nodes().length - 1}>
                      ▶
                    </button>
                    <button class="key small dark" onClick={() => remove(i)} title="take it out">
                      ×
                    </button>
                  </div>
                  <div class="pedal-sub">
                    <button class="tag" onClick={() => rename(i)} title="its name">
                      {n().label ? `AS ${n().label!.toUpperCase()}` : "NAME…"}
                    </button>
                    <Show when={n().words.length || (catalog().find((s) => s.name === n().kind)?.words ?? false)}>
                      <button class="tag" onClick={() => words(i)} title="its words">
                        {n().words.length ? n().words.join(" ") : "WORDS…"}
                      </button>
                    </Show>
                  </div>
                  <div class="pedal-knobs">
                    <Index each={n().args.filter((a) => !a.named || a.name === "wet" || open().has(i))}>
                      {(a, k) => (
                        <div class={`arg ${a().set && a().named ? "set" : ""}`} title={a().doc}>
                          <Knob value={toKnob(a())} tone={a().name === "wet" ? "orange" : KNOB_TONES[k % 4]} onChange={(v) => turn(i, a().name, v)} />
                          <span class="label">{a().name.replace(/_/g, " ").toUpperCase()}</span>
                          <span class="reading">{reading(a())}</span>
                        </div>
                      )}
                    </Index>
                  </div>
                  <Show when={n().args.some((a) => a.named && a.name !== "wet")}>
                    <button class="more" onClick={() => toggle(i)}>
                      {open().has(i) ? "LESS" : "MORE"}
                    </button>
                  </Show>
                </div>
              </>
            )}
          </Index>
          <Gap at={nodes().length} />
          <div class="jack out">{chain()!.to.join(" ").toUpperCase() || "MASTER"}</div>
        </div>
        <Show when={picking() !== null}>
          <div class="picker">
            <For each={groups()}>
              {([category, specs]) => (
                <div class="picker-group">
                  <span class={`picker-cat ${toneOf(category)}`}>{category.toUpperCase()}</span>
                  <div class="picker-keys">
                    <For each={specs}>
                      {(s) => (
                        <button class="key small" onClick={() => add(picking()!, s)} title={s.doc}>
                          {s.name}
                        </button>
                      )}
                    </For>
                  </div>
                </div>
              )}
            </For>
          </div>
        </Show>
      </section>
    </Show>
  );
}
