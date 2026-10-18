// A voice's pattern as a piano roll: a row per degree of the song's scale,
// a column per step, every note a bar as long as it is held.
//
// - A click on an empty cell puts a note there; on a column where a note
//   or a chord starts, it adds the row to it, a chord.
// - A drag from an empty cell draws a box and selects what it touches;
//   shift adds to the selection. A click on a note selects it.
// - A drag on a note moves the selection; its right edge sets its length.
// - Delete, the arrows, ctrl+C / ctrl+V, ctrl+A work on the selection.
// - A right click opens the step's menu: delete, slide, split the step
//   into 2, 3 or 4 (a run inside it, triplets with 3), join, the token.
// - The lane under it paints velocity, length, brightness or chance.
//
// Every gesture is drawn on a copy of the steps while the pointer is down
// and sent to the engine once, as the steps that changed, when it is up.

import { For, Index, Show, createEffect, createMemo, createSignal, on, onCleanup, onMount } from "solid-js";
import type { Pattern, PatternEdit, PatternStep } from "@tatum/engine";
import { editSteps, pattern, position, setStep } from "../store";
import { chordStep, degreeOf, nameOf, noteText, notesOf, rest, runStep, tie, withValues } from "../steps";

const ROW = 18;
const PAD = 3;
const MIN_ROWS = 15;

/** A note as the roll draws it: where it starts, which of a run it is, its pitch, how long. */
interface Bar {
  id: string;
  start: number;
  /** Its place in a run inside the step, and how many the run has. */
  sub?: number;
  of?: number;
  length: number;
  midi: number;
  step: PatternStep;
}

function barsOf(steps: PatternStep[]): Bar[] {
  const bars: Bar[] = [];
  for (let i = 0; i < steps.length; i++) {
    const s = steps[i];
    const notes = notesOf(s);
    if (notes.length === 0) continue;
    if (s.kind === "sub") {
      notes.forEach((n, k) => bars.push({ id: `${i}:${k}:${n.midi}`, start: i, sub: k, of: notes.length, length: 1, midi: n.midi, step: s }));
      continue;
    }
    const length = lengthAt(steps, i);
    for (const n of notes) bars.push({ id: `${i}::${n.midi}`, start: i, length, midi: n.midi, step: s });
  }
  return bars;
}

function lengthAt(steps: PatternStep[], start: number): number {
  let n = 1;
  while (start + n < steps.length && steps[start + n].kind === "tie") n++;
  return n;
}

/** The step that sounds at `at`: the one it starts, or the one held through it. */
function startOf(steps: PatternStep[], at: number): number | null {
  for (let i = at; i >= 0; i--) {
    if (steps[i].kind === "tie") continue;
    return steps[i].kind === "rest" ? null : i;
  }
  return null;
}

/** `step` at `at`, held for `length`; what was held past it before stops. */
function put(steps: PatternStep[], at: number, step: PatternStep, length: number): void {
  // A note cut short where `at` was held from an earlier one.
  const end = Math.min(steps.length, at + length);
  steps[at] = step;
  for (let i = at + 1; i < end; i++) steps[i] = tie();
  for (let i = end; i < steps.length && steps[i].kind === "tie"; i++) steps[i] = rest();
}

function clear(steps: PatternStep[], start: number): void {
  const n = lengthAt(steps, start);
  for (let i = start; i < start + n; i++) steps[i] = rest();
}

function diff(before: PatternStep[], after: PatternStep[]): PatternEdit[] {
  const edits: PatternEdit[] = [];
  after.forEach((s, i) => {
    if (s.token !== before[i]?.token) edits.push({ lane: 0, step: i, token: s.token });
  });
  return edits;
}

/** What a selection lifts: per step it touches, the notes chosen and how the step was. */
interface Lifted {
  start: number;
  length: number;
  midis: number[];
  step: PatternStep;
  /** The whole step goes with it: a run, or every note of a chord. */
  whole: boolean;
}

type Lane = "vel" | "gate" | "cutoff" | "prob";
const LANES: { lane: Lane; label: string; title: string; rest: number }[] = [
  { lane: "vel", label: "FUERZA", title: "how hard each note is played", rest: 0.8 },
  { lane: "gate", label: "LARGO", title: "how much of its length each note really sounds: short is staccato", rest: 1 },
  { lane: "cutoff", label: "BRILLO", title: "the filter opened or closed on that note alone", rest: 0.5 },
  { lane: "prob", label: "CHANCE", title: "the chance each note sounds", rest: 1 },
];

type Gesture =
  | { kind: "box"; x0: number; y0: number; x1: number; y1: number; add: boolean; moved: boolean; col: number; row: number }
  | { kind: "move"; col: number; row: number; base: PatternStep[]; lifted: Lifted[]; moved: boolean }
  | { kind: "resize"; start: number; base: PatternStep[] }
  | { kind: "paint"; base: PatternStep[] };

export function PianoRoll() {
  const [draft, setDraft] = createSignal<PatternStep[] | null>(null);
  const [selected, setSelected] = createSignal<Set<string>>(new Set());
  const [box, setBox] = createSignal<{ x: number; y: number; w: number; h: number } | null>(null);
  const [menu, setMenu] = createSignal<{ x: number; y: number; col: number; midi: number; bar: Bar | null } | null>(null);
  const [laneKind, setLaneKind] = createSignal<Lane>("vel");
  const [more, setMore] = createSignal({ key: "", up: 0, down: 0 });
  let clipboard: { at: number; length: number; midis: number[]; step: PatternStep; whole: boolean }[] = [];
  let cursor = 0;
  let gesture: Gesture | null = null;
  let area!: HTMLDivElement;
  let lane!: HTMLDivElement;

  const p = () => pattern() as Pattern;
  const scale = () => p().scale;
  const length = () => p().length;
  const base = () => p().lanes[0]?.steps ?? [];
  const steps = () => draft() ?? base();
  const bars = createMemo(() => barsOf(steps()));
  const key = () => `${p().track}/${p().name}`;
  createEffect(on(key, () => setSelected(new Set<string>())));

  const extra = () => (more().key === range().key ? more() : { key: range().key, up: 0, down: 0 });
  const grow = (up: number, down: number) => setMore({ ...extra(), up: extra().up + up, down: extra().down + down });

  // The range of the rows, taken when the pattern is first shown and kept
  // while it is edited, so the rows do not slide under the pointer.
  const inScale = (m: number) => scale().intervals.includes((((m - scale().root) % 12) + 12) % 12);
  const stepDown = (m: number) => {
    const floor = 12 + scale().root;
    do m--;
    while (m > floor && !inScale(m));
    return Math.max(floor, m);
  };
  const stepUp = (m: number) => {
    do m++;
    while (m < 127 && !inScale(m));
    return Math.min(127, m);
  };
  const range = createMemo<{ key: string; lo: number; hi: number }>((prev) => {
    if (prev && prev.key === key()) return prev;
    const played = base().flatMap((s) => s.midi ?? []);
    const center = played.length ? played[0] : 36 + scale().root;
    let lo = Math.max(12 + scale().root, played.length ? Math.min(...played) : center);
    let hi = played.length ? Math.max(...played) : center;
    if (!inScale(lo)) lo = stepDown(lo);
    if (!inScale(hi)) hi = stepUp(hi);
    for (let i = 0; i < PAD; i++) lo = stepDown(lo);
    for (let i = 0; i < PAD; i++) hi = stepUp(hi);
    let rows = 0;
    for (let m = lo; m <= hi; m++) if (inScale(m)) rows++;
    for (; rows < MIN_ROWS && hi < 127; rows++) hi = stepUp(hi);
    return { key: key(), lo, hi };
  });
  const rows = createMemo(() => {
    let { lo, hi } = range();
    for (let i = 0; i < extra().down * 7; i++) lo = stepDown(lo);
    for (let i = 0; i < extra().up * 7; i++) hi = stepUp(hi);
    const played = steps().flatMap((s) => s.midi ?? []);
    const out: number[] = [];
    for (let m = hi; m >= lo; m--) if (inScale(m) || played.includes(m)) out.push(m);
    return out;
  });
  const rowOf = (midi: number) => rows().indexOf(midi);
  const shift = (midi: number, by: number) => rows()[Math.max(0, Math.min(rows().length - 1, rowOf(midi) + by))] ?? midi;

  const playhead = () => {
    const pos = position();
    const at = pos?.running ? pos.tracks?.[p().track] : undefined;
    return at !== undefined ? at % Math.max(1, length()) : -1;
  };

  const point = (e: { clientX: number; clientY: number }) => {
    const r = area.getBoundingClientRect();
    const x = ((e.clientX - r.left) / r.width) * length();
    const col = Math.max(0, Math.min(length() - 1, Math.floor(x)));
    const row = Math.max(0, Math.min(rows().length - 1, Math.floor((e.clientY - r.top) / ROW)));
    return { col, row, x, y: e.clientY - r.top };
  };
  /** The note under a point: a run's notes split the step in parts. */
  const barAt = (col: number, row: number, x: number) =>
    bars().find((b) => {
      if (b.midi !== rows()[row]) return false;
      if (b.of) return col === b.start && Math.floor((x - col) * b.of) === b.sub;
      return col >= b.start && col < b.start + b.length;
    }) ?? null;

  const commit = async (after: PatternStep[], what: string) => {
    const edits = diff(base(), after);
    if (edits.length) await editSteps(edits, what);
    setDraft(null);
  };

  // ── the selection lifted out of the steps and put down elsewhere ──

  const lift = (from: PatternStep[], ids: Set<string>): { left: PatternStep[]; lifted: Lifted[] } => {
    const left = [...from];
    const lifted: Lifted[] = [];
    const chosen = barsOf(from).filter((b) => ids.has(b.id));
    const starts = [...new Set(chosen.map((b) => b.start))];
    for (const start of starts) {
      const s = from[start];
      const here = chosen.filter((b) => b.start === start);
      const all = notesOf(s).map((n) => n.midi);
      const whole = s.kind === "sub" || here.length >= all.length;
      const midis = whole ? all : here.map((b) => b.midi);
      lifted.push({ start, length: lengthAt(from, start), midis, step: s, whole });
      if (whole) clear(left, start);
      else left[start] = chordStep(all.filter((m) => !midis.includes(m)), s, scale());
    }
    return { left, lifted };
  };

  /** `l` put at `at`, its notes `by` rows lower; onto a chord that starts there, they join it. */
  const place = (into: PatternStep[], l: { length: number; midis: number[]; step: PatternStep; whole: boolean }, at: number, by: number) => {
    if (at < 0 || at >= into.length) return;
    const len = Math.min(l.length, into.length - at);
    if (l.step.kind === "sub") {
      const notes = notesOf(l.step).map((n) => ({ ...n, midi: shift(n.midi, by) }));
      put(into, at, runStep(notes, scale()), 1);
      return;
    }
    const midis = l.midis.map((m) => shift(m, by));
    const there = into[at];
    if ((there.kind === "note" || there.kind === "chord") && !l.whole) {
      into[at] = chordStep([...(there.midi ?? []), ...midis], there, scale());
      return;
    }
    put(into, at, chordStep(midis, l.step, scale(), midis.length === 1 && !!l.step.slide), len);
  };

  const idsAt = (after: PatternStep[], spots: { at: number; midis: number[] }[]) => {
    const out = new Set<string>();
    for (const b of barsOf(after)) if (spots.some((s) => s.at === b.start && s.midis.includes(b.midi))) out.add(b.id);
    return out;
  };

  /** The selection moved `cols` right and `by` rows down. */
  const moveSelection = (cols: number, by: number, what: string) => {
    if (selected().size === 0) return;
    const { left, lifted } = lift(base(), selected());
    const lo = Math.min(...lifted.map((l) => l.start));
    const hi = Math.max(...lifted.map((l) => l.start + l.length));
    const dc = Math.max(-lo, Math.min(length() - hi, cols));
    for (const l of lifted) place(left, l, l.start + dc, by);
    setSelected(idsAt(left, lifted.map((l) => ({ at: l.start + dc, midis: l.midis.map((m) => shift(m, by)) }))));
    void commit(left, what);
  };

  const deleteSelection = () => {
    if (selected().size === 0) return;
    const { left } = lift(base(), selected());
    setSelected(new Set<string>());
    void commit(left, "notes deleted");
  };

  const copy = () => {
    const { lifted } = lift(base(), selected());
    if (!lifted.length) return;
    const lo = Math.min(...lifted.map((l) => l.start));
    clipboard = lifted.map((l) => ({ at: l.start - lo, length: l.length, midis: l.midis, step: l.step, whole: l.whole }));
    cursor = Math.max(...lifted.map((l) => l.start + l.length));
  };

  const paste = () => {
    if (!clipboard.length) return;
    const now = [...base()];
    for (const c of clipboard) place(now, { ...c, whole: true }, cursor + c.at, 0);
    setSelected(idsAt(now, clipboard.map((c) => ({ at: cursor + c.at, midis: c.midis }))));
    const width = Math.max(...clipboard.map((c) => c.at + c.length));
    cursor = Math.min(length() - 1, cursor + width);
    void commit(now, "pasted");
  };

  // ── pointer ──

  const down = (e: PointerEvent) => {
    setMenu(null);
    if (e.button !== 0) return;
    const { col, row, x, y } = point(e);
    cursor = col;
    area.setPointerCapture(e.pointerId);
    const edge = (e.target as HTMLElement).closest(".edge");
    const hit = barAt(col, row, x);
    if (edge && hit && !hit.of) {
      gesture = { kind: "resize", start: hit.start, base: base() };
      setDraft([...base()]);
      return;
    }
    if (hit) {
      const sel = new Set(selected());
      if (e.shiftKey) {
        if (sel.has(hit.id)) sel.delete(hit.id);
        else sel.add(hit.id);
        setSelected(sel);
        return;
      }
      if (!sel.has(hit.id)) setSelected(new Set([hit.id]));
      const { left, lifted } = lift(base(), selected());
      gesture = { kind: "move", col, row, base: left, lifted, moved: false };
      return;
    }
    const px = (x / length()) * area.clientWidth;
    gesture = { kind: "box", x0: px, y0: y, x1: px, y1: y, add: e.shiftKey, moved: false, col, row };
  };

  const move = (e: PointerEvent) => {
    const g = gesture;
    if (!g) return;
    const { col, row, x, y } = point(e);
    if (g.kind === "box") {
      g.x1 = (x / length()) * area.clientWidth;
      g.y1 = y;
      if (!g.moved && Math.abs(g.x1 - g.x0) + Math.abs(g.y1 - g.y0) < 5) return;
      g.moved = true;
      setBox({ x: Math.min(g.x0, g.x1), y: Math.min(g.y0, g.y1), w: Math.abs(g.x1 - g.x0), h: Math.abs(g.y1 - g.y0) });
    } else if (g.kind === "resize") {
      const now = [...g.base];
      put(now, g.start, now[g.start], Math.max(1, col - g.start + 1));
      setDraft(now);
    } else if (g.kind === "move") {
      const cols = col - g.col;
      const by = row - g.row;
      if (!g.moved && cols === 0 && by === 0) return;
      g.moved = true;
      const lo = Math.min(...g.lifted.map((l) => l.start));
      const hi = Math.max(...g.lifted.map((l) => l.start + l.length));
      const dc = Math.max(-lo, Math.min(length() - hi, cols));
      const now = [...g.base];
      for (const l of g.lifted) place(now, l, l.start + dc, by);
      setDraft(now);
    }
  };

  const up = (e: PointerEvent) => {
    const g = gesture;
    gesture = null;
    if (!g) return;
    if (g.kind === "box") {
      setBox(null);
      if (g.moved) {
        const r = area.getBoundingClientRect();
        const [x0, x1] = [Math.min(g.x0, g.x1), Math.max(g.x0, g.x1)].map((px) => (px / r.width) * length());
        const [r0, r1] = [Math.min(g.y0, g.y1), Math.max(g.y0, g.y1)].map((py) => Math.floor(py / ROW));
        const sel = g.add ? new Set(selected()) : new Set<string>();
        for (const b of bars()) {
          const row = rowOf(b.midi);
          const from = b.of ? b.start + b.sub! / b.of : b.start;
          const to = b.of ? from + 1 / b.of : b.start + b.length;
          if (row >= r0 && row <= r1 && to > x0 && from < x1) sel.add(b.id);
        }
        setSelected(sel);
        return;
      }
      clickEmpty(g.col, g.row, e.shiftKey);
      return;
    }
    if (g.kind === "move") {
      if (!g.moved) return;
      const now = draft();
      const { col, row } = point(e);
      const lo = Math.min(...g.lifted.map((l) => l.start));
      const hi = Math.max(...g.lifted.map((l) => l.start + l.length));
      const dc = Math.max(-lo, Math.min(length() - hi, col - g.col));
      const by = row - g.row;
      if (now) {
        setSelected(idsAt(now, g.lifted.map((l) => ({ at: l.start + dc, midis: l.midis.map((m) => shift(m, by)) }))));
        void commit(now, g.lifted.length > 1 ? "notes moved" : "a note moved");
      }
      return;
    }
    const now = draft();
    if (now) void commit(now, g.kind === "resize" ? "a note's length" : "painted");
  };

  /** A click where there is no note: onto the chord that starts there, or a note of its own. */
  const clickEmpty = (col: number, row: number, keep: boolean) => {
    const midi = rows()[row];
    const now = [...base()];
    const s = now[col];
    if (s.kind === "note" || s.kind === "chord") {
      now[col] = chordStep([...(s.midi ?? []), midi], s, scale());
      void commit(now, "a chord");
    } else if (s.kind === "sub") {
      return;
    } else {
      put(now, col, chordStep([midi], null, scale()), 1);
      void commit(now, "a note");
    }
    const id = `${col}::${midi}`;
    setSelected(keep ? new Set([...selected(), id]) : new Set([id]));
  };

  // ── the step's menu ──

  const context = (e: MouseEvent) => {
    e.preventDefault();
    const { col, row, x } = point(e);
    const bar = barAt(col, row, x);
    const r = area.closest(".roll")!.getBoundingClientRect();
    setMenu({ x: e.clientX - r.left, y: e.clientY - r.top, col: bar?.start ?? col, midi: rows()[row], bar });
  };

  const split = (col: number, midi: number, n: number) => {
    const now = [...base()];
    const s = now[col];
    const at = startOf(now, col) === col ? col : col;
    const pitch = notesOf(s)[0]?.midi ?? midi;
    put(now, at, runStep(Array.from({ length: n }, () => ({ midi: pitch, text: "", slide: false, velocity: null })), scale()), 1);
    void commit(now, n === 3 ? "a triplet" : `split in ${n}`);
  };

  const join = (col: number) => {
    const now = [...base()];
    const first = notesOf(now[col])[0];
    if (!first) return;
    put(now, col, chordStep([first.midi], null, scale(), first.slide), 1);
    void commit(now, "joined");
  };

  const slide = (bar: Bar) => {
    const now = [...base()];
    const s = now[bar.start];
    if (s.kind !== "note") return;
    now[bar.start] = chordStep(s.midi ?? [], s, scale(), !s.slide);
    void commit(now, s.slide ? "no slide" : "slides in from the note before");
  };

  const removeOne = (bar: Bar) => {
    const now = [...base()];
    const s = now[bar.start];
    if (bar.of) {
      const notes = notesOf(s).filter((_, k) => k !== bar.sub);
      if (notes.length >= 2) now[bar.start] = runStep(notes, scale());
      else if (notes.length === 1) put(now, bar.start, chordStep([notes[0].midi], null, scale()), 1);
      else clear(now, bar.start);
    } else {
      const left = (s.midi ?? []).filter((m) => m !== bar.midi);
      if (left.length) now[bar.start] = chordStep(left, s, scale());
      else clear(now, bar.start);
    }
    void commit(now, "a note deleted");
  };

  const token = (col: number) => {
    const at = startOf(base(), col) ?? col;
    const t = prompt("the step, as the pattern writes it: 1.2:0.9  ~5.2  [1.3 5.3 2.4]:0.8  <1.1 1.1 1.1>  ..  -", base()[at]?.token ?? "-");
    if (t !== null && t.trim()) void setStep(0, at, t.trim());
  };

  // ── the lane under the roll ──

  const laneValue = (s: PatternStep): number | null => {
    switch (laneKind()) {
      case "vel":
        return s.velocity ?? null;
      case "gate":
        return s.gate ?? null;
      case "cutoff":
        return s.cutoff ?? null;
      case "prob":
        return s.prob ?? null;
    }
  };
  const laneRest = () => LANES.find((l) => l.lane === laneKind())!.rest;
  const valued = (s: PatternStep, v: number | null): PatternStep => {
    const k = laneKind();
    return withValues(s, scale(), k === "vel" ? { velocity: v } : { [k]: v });
  };
  const paint = (e: PointerEvent, clearIt = false) => {
    const g = gesture;
    if (!g || g.kind !== "paint") return;
    const r = lane.getBoundingClientRect();
    const col = Math.max(0, Math.min(length() - 1, Math.floor(((e.clientX - r.left) / r.width) * length())));
    const v = Math.max(0.05, Math.min(1, Math.round((1 - (e.clientY - r.top) / r.height) * 20) / 20));
    const now = [...(draft() ?? g.base)];
    const s = now[col];
    if (s.kind !== "note" && s.kind !== "chord") return;
    now[col] = valued(s, clearIt ? null : v);
    setDraft(now);
  };
  const laneDown = (e: PointerEvent) => {
    if (e.button !== 0 && e.button !== 2) return;
    lane.setPointerCapture(e.pointerId);
    gesture = { kind: "paint", base: base() };
    setDraft([...base()]);
    paint(e, e.button === 2);
  };

  // ── keys ──

  const keydown = (e: KeyboardEvent) => {
    if ((e.target as HTMLElement).closest("input, textarea, select")) return;
    const mod = e.ctrlKey || e.metaKey;
    if (mod && e.key.toLowerCase() === "c") return copy();
    if (mod && e.key.toLowerCase() === "v") return (e.preventDefault(), paste());
    if (mod && e.key.toLowerCase() === "a") {
      e.preventDefault();
      return setSelected(new Set(bars().map((b) => b.id)));
    }
    if (selected().size === 0) return;
    if (e.key === "Delete" || e.key === "Backspace") return (e.preventDefault(), deleteSelection());
    if (e.key === "Escape") return setSelected(new Set<string>());
    const arrows: Record<string, [number, number]> = { ArrowUp: [0, -1], ArrowDown: [0, 1], ArrowLeft: [-1, 0], ArrowRight: [1, 0] };
    const a = arrows[e.key];
    if (a) {
      e.preventDefault();
      const big = e.shiftKey ? (a[0] ? 4 : 7) : 1;
      moveSelection(a[0] * big, a[1] * big, "moved");
    }
  };
  onMount(() => window.addEventListener("keydown", keydown));
  onCleanup(() => window.removeEventListener("keydown", keydown));

  return (
    <div class="roll" onPointerDown={(e) => !(e.target as HTMLElement).closest(".menu") && e.button === 0 && menu() && setMenu(null)}>
      <div class="roll-head">
        <span class="scale">{scale().name ?? "C major"}</span>
        <button class="key small range" title="one more octave on top" onClick={() => grow(1, 0)}>
          +▲
        </button>
        <button class="key small range" title="one more octave under" onClick={() => grow(0, 1)}>
          +▼
        </button>
        <Show when={selected().size}>
          <span class="selection">{selected().size} selected · arrows move · delete · ctrl+C / ctrl+V</span>
        </Show>
      </div>
      <div class="roll-body">
        <div class="keys">
          <Index each={rows()}>
            {(m) => {
              const d = () => degreeOf(m(), scale());
              return (
                <div class={`rowkey ${d()?.degree === 1 ? "root" : ""} ${d() ? "" : "off"}`} style={{ height: `${ROW}px` }}>
                  <span class="degree">{d() ? `${d()!.degree}.${d()!.octave}` : "·"}</span>
                  <span class="pitch">{nameOf(m())}</span>
                </div>
              );
            }}
          </Index>
          <div class="lane-tabs">
            <For each={LANES}>
              {(l) => (
                <button class={`lane-tab ${laneKind() === l.lane ? "on" : ""}`} onClick={() => setLaneKind(l.lane)} title={l.title}>
                  {l.label}
                </button>
              )}
            </For>
          </div>
        </div>
        <div class="roll-scroll">
          <div class="roll-inner" style={{ "min-width": `${length() * 18}px` }}>
            <div
              ref={area}
              class="area"
              style={{ height: `${rows().length * ROW}px` }}
              onPointerDown={down}
              onPointerMove={move}
              onPointerUp={up}
              onPointerCancel={up}
              onContextMenu={context}
              onDblClick={(e) => token(point(e).col)}
            >
              <Index each={rows()}>
                {(m, i) => (
                  <div
                    class={`stripe ${degreeOf(m(), scale())?.degree === 1 ? "root" : ""} ${degreeOf(m(), scale()) ? "" : "off"}`}
                    style={{ top: `${i * ROW}px`, height: `${ROW}px` }}
                  />
                )}
              </Index>
              <Index each={Array.from({ length: length() }, (_, i) => i)}>
                {(c) => (
                  <div
                    class={`column ${c() % 16 === 0 ? "bar" : c() % 4 === 0 ? "beat" : ""}`}
                    style={{ left: `${(c() / length()) * 100}%`, width: `${100 / length()}%` }}
                  />
                )}
              </Index>
              <Show when={playhead() >= 0}>
                <div class="playhead" style={{ left: `${(playhead() / length()) * 100}%`, width: `${100 / length()}%` }} />
              </Show>
              <For each={bars()}>
                {(b) => (
                  <Show when={rowOf(b.midi) >= 0}>
                    <div
                      class={`bar-note ${b.step.kind} ${notesOf(b.step)[b.sub ?? 0]?.slide ? "slide" : ""} ${selected().has(b.id) ? "selected" : ""}`}
                      style={{
                        left: `${((b.start + (b.of ? b.sub! / b.of : 0)) / length()) * 100}%`,
                        width: `${((b.of ? 1 / b.of : b.length) / length()) * 100}%`,
                        top: `${rowOf(b.midi) * ROW + 1}px`,
                        height: `${ROW - 2}px`,
                        "--v": String(b.step.velocity ?? notesOf(b.step)[b.sub ?? 0]?.velocity ?? 0.8),
                      }}
                      title={b.step.token}
                    >
                      <span>{`${notesOf(b.step)[b.sub ?? 0]?.slide ? "~" : ""}${noteText(b.midi, scale())}`}</span>
                      <Show when={!b.of}>
                        <i class="edge" title="drag: its length" />
                      </Show>
                    </div>
                  </Show>
                )}
              </For>
              <Show when={box()}>
                {(r) => <div class="band" style={{ left: `${r().x}px`, top: `${r().y}px`, width: `${r().w}px`, height: `${r().h}px` }} />}
              </Show>
            </div>
            <div
              ref={lane}
              class="vel-lane"
              onPointerDown={laneDown}
              onPointerMove={(e) => paint(e, e.buttons === 2)}
              onPointerUp={up}
              onPointerCancel={up}
              onContextMenu={(e) => e.preventDefault()}
              title="drag to paint · right drag: back to the default"
            >
              <Index each={steps()}>
                {(s, i) => (
                  <Show when={s().kind === "note" || s().kind === "chord"}>
                    <div
                      class={`vel ${laneValue(s()) == null ? "unset" : ""}`}
                      style={{
                        left: `${(i / length()) * 100}%`,
                        width: `${100 / length()}%`,
                        height: `${(laneValue(s()) ?? laneRest()) * 100}%`,
                      }}
                      title={laneValue(s()) == null ? "as the track has it" : String(laneValue(s()))}
                    />
                  </Show>
                )}
              </Index>
            </div>
          </div>
        </div>
      </div>
      <Show when={menu()}>
        {(m) => (
          <div class="menu" style={{ left: `${m().x}px`, top: `${m().y}px` }} onPointerDown={(e) => e.stopPropagation()}>
            <Show when={m().bar}>
              <button onClick={() => (removeOne(m().bar!), setMenu(null))}>Borrar la nota</button>
              <Show when={m().bar!.step.kind === "note"}>
                <button onClick={() => (slide(m().bar!), setMenu(null))}>
                  {m().bar!.step.slide ? "Sin ligado" : "Ligar desde la anterior (~)"}
                </button>
              </Show>
            </Show>
            <Show when={base()[m().col]?.kind !== "sub"}>
              <button onClick={() => (split(m().col, m().midi, 2), setMenu(null))}>Partir en 2</button>
              <button onClick={() => (split(m().col, m().midi, 3), setMenu(null))}>Partir en 3 (tresillo)</button>
              <button onClick={() => (split(m().col, m().midi, 4), setMenu(null))}>Partir en 4</button>
            </Show>
            <Show when={base()[m().col]?.kind === "sub"}>
              <button onClick={() => (join(m().col), setMenu(null))}>Unir en una nota</button>
            </Show>
            <button onClick={() => (token(m().col), setMenu(null))}>Texto del paso…</button>
          </div>
        )}
      </Show>
    </div>
  );
}
