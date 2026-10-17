// A voice's pattern as a piano roll: a row per degree of the song's scale,
// a column per step, every note a bar as long as it is held. A click on an
// empty cell draws a note there (drag right while holding to make it
// longer), a drag on a note moves it, a drag on its right edge sets its
// length, a right click deletes it, shift-click slides into it, alt-click
// adds the row to the step as a chord, and a double click opens the token
// itself for what the grid does not draw. The lane under it is velocity:
// drag across it to paint.
//
// Every gesture is drawn on a copy of the steps while the pointer is down
// and sent to the engine once, as the steps that changed, when it is up.

import { For, Index, Show, createMemo, createSignal } from "solid-js";
import type { Pattern, PatternEdit, PatternScale, PatternStep } from "@tatum/engine";
import { editSteps, pattern, position, setStep } from "../store";

const ROW = 18;
const PAD = 3;
const MIN_ROWS = 15;
const NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

const rest = (): PatternStep => ({ token: "-", kind: "rest" });
const tie = (): PatternStep => ({ token: "..", kind: "tie" });

function nameOf(midi: number): string {
  return `${NAMES[midi % 12]}${Math.floor(midi / 12) - 1}`;
}

/** The degree of the scale a MIDI note is, if it is one. */
function degreeOf(midi: number, scale: PatternScale): { degree: number; octave: number } | null {
  const above = (((midi - scale.root) % 12) + 12) % 12;
  const d = scale.intervals.indexOf(above);
  if (d < 0) return null;
  const octave = (midi - scale.root - scale.intervals[d]) / 12 - 1;
  return octave >= 0 && octave <= 9 ? { degree: d + 1, octave } : null;
}

/** How a MIDI note is written in the pattern: its degree, or its name when off the scale. */
function noteText(midi: number, scale: PatternScale): string {
  const d = degreeOf(midi, scale);
  return d ? `${d.degree}.${d.octave}` : nameOf(midi);
}

function noteStep(midis: number[], velocity: number | null | undefined, slide: boolean, scale: PatternScale): PatternStep {
  const sorted = [...new Set(midis)].sort((a, b) => a - b);
  const names = sorted.map((m) => noteText(m, scale));
  const v = velocity == null ? "" : `:${Math.round(velocity * 100) / 100}`;
  const one = sorted.length === 1;
  return {
    token: one ? `${slide ? "~" : ""}${names[0]}${v}` : `[${names.join(" ")}]${v}`,
    kind: one ? "note" : "chord",
    note: names.join(" "),
    midi: sorted,
    velocity: velocity ?? null,
    slide: one && slide,
  };
}

interface Bar {
  start: number;
  length: number;
  midi: number;
  step: PatternStep;
}

/** The notes the steps play, a bar per pitch, held through the ties after it. */
function barsOf(steps: PatternStep[]): Bar[] {
  const bars: Bar[] = [];
  for (let i = 0; i < steps.length; i++) {
    const s = steps[i];
    if (!s.midi || s.midi.length === 0) continue;
    let length = 1;
    while (i + length < steps.length && steps[i + length].kind === "tie") length++;
    const midis = s.kind === "sub" ? [s.midi[0]] : s.midi;
    for (const midi of midis) bars.push({ start: i, length, midi, step: s });
  }
  return bars;
}

/** The step that sounds at `at`: the note it starts or the one it is held from. */
function startOf(steps: PatternStep[], at: number): number | null {
  for (let i = at; i >= 0; i--) {
    if (steps[i].kind === "tie") continue;
    return steps[i].kind === "rest" ? null : i;
  }
  return null;
}

function lengthAt(steps: PatternStep[], start: number): number {
  let n = 1;
  while (start + n < steps.length && steps[start + n].kind === "tie") n++;
  return n;
}

/** `step` at `at`, held for `length`; what was held past it before stops. */
function put(steps: PatternStep[], at: number, step: PatternStep, length: number): void {
  const end = Math.min(steps.length, at + length);
  steps[at] = step;
  for (let i = at + 1; i < end; i++) steps[i] = tie();
  for (let i = end; i < steps.length && steps[i].kind === "tie"; i++) steps[i] = rest();
}

function clear(steps: PatternStep[], start: number): void {
  const n = lengthAt(steps, start);
  for (let i = start; i < start + n; i++) steps[i] = rest();
}

/** The steps whose token changed, as edits for the engine. */
function diff(before: PatternStep[], after: PatternStep[]): PatternEdit[] {
  const edits: PatternEdit[] = [];
  after.forEach((s, i) => {
    if (s.token !== before[i]?.token) edits.push({ lane: 0, step: i, token: s.token });
  });
  return edits;
}

type Gesture =
  | { kind: "draw"; start: number; midi: number; base: PatternStep[]; step: PatternStep }
  | { kind: "move"; start: number; length: number; midi: number; base: PatternStep[]; col: number; row: number; moved: boolean }
  | { kind: "resize"; start: number; base: PatternStep[] }
  | { kind: "velocity"; base: PatternStep[] };

export function PianoRoll() {
  const [draft, setDraft] = createSignal<PatternStep[] | null>(null);
  const [more, setMore] = createSignal({ key: "", up: 0, down: 0 });
  const extra = () => (more().key === range().key ? more() : { key: range().key, up: 0, down: 0 });
  const grow = (up: number, down: number) => setMore({ ...extra(), up: extra().up + up, down: extra().down + down });
  let gesture: Gesture | null = null;
  let area!: HTMLDivElement;
  let lane!: HTMLDivElement;

  const p = () => pattern() as Pattern;
  const scale = () => p().scale;
  const length = () => p().length;
  const base = () => p().lanes[0]?.steps ?? [];
  const steps = () => draft() ?? base();
  const bars = createMemo(() => barsOf(steps()));

  // The range of the rows: a little under the lowest note the pattern
  // plays to a little over its highest, taken when the pattern is first
  // shown and kept while it is edited, so the rows do not slide under the
  // pointer; the buttons add an octave either way. Never under degree 1 of
  // octave 0, the lowest a degree can be written.
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
    const key = `${p().track}/${p().name}`;
    if (prev && prev.key === key) return prev;
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
    return { key, lo, hi };
  });

  // The rows, high at the top: the scale's notes in the range, and any
  // off-scale note the pattern plays there.
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

  const playhead = () => {
    const pos = position();
    const at = pos?.running ? pos.tracks?.[p().track] : undefined;
    return at !== undefined ? at % Math.max(1, length()) : -1;
  };

  const cellAt = (e: PointerEvent) => {
    const r = area.getBoundingClientRect();
    const col = Math.max(0, Math.min(length() - 1, Math.floor(((e.clientX - r.left) / r.width) * length())));
    const row = Math.max(0, Math.min(rows().length - 1, Math.floor((e.clientY - r.top) / ROW)));
    const fraction = ((e.clientX - r.left) / r.width) * length() - col;
    return { col, row, fraction };
  };

  const commit = async (what: string) => {
    const g = gesture;
    gesture = null;
    const after = draft();
    if (!g || !after) return;
    const edits = diff(g.base, after);
    if (edits.length) await editSteps(edits, what);
    setDraft(null);
  };

  const down = (e: PointerEvent) => {
    if (e.button !== 0) return;
    const { col, row, fraction } = cellAt(e);
    const midi = rows()[row];
    const now = [...base()];
    const sounding = startOf(now, col);
    const hit = bars().find((b) => b.midi === midi && col >= b.start && col < b.start + b.length);
    area.setPointerCapture(e.pointerId);

    if (hit && e.shiftKey && hit.step.kind === "note") {
      const s = hit.step;
      now[hit.start] = noteStep(s.midi ?? [midi], s.velocity, !s.slide, scale());
      gesture = { kind: "velocity", base: base() };
      setDraft(now);
      void commit(s.slide ? "no slide" : "slides in");
      return;
    }
    if (hit) {
      const last = hit.start + hit.length - 1;
      if (col === last && fraction > 0.6 && hit.step.kind !== "sub") {
        gesture = { kind: "resize", start: hit.start, base: base() };
      } else {
        gesture = { kind: "move", start: hit.start, length: hit.length, midi, base: base(), col, row, moved: false };
      }
      setDraft(now);
      return;
    }
    // An empty cell: a chord tone with alt on a step that sounds, a new
    // note otherwise, cutting whatever was held through it.
    if (e.altKey && sounding !== null && now[sounding].midi) {
      const s = now[sounding];
      now[sounding] = noteStep([...(s.midi ?? []), midi], s.velocity, false, scale());
      gesture = { kind: "velocity", base: base() };
      setDraft(now);
      void commit("a chord");
      return;
    }
    const step = noteStep([midi], null, false, scale());
    put(now, col, step, 1);
    gesture = { kind: "draw", start: col, midi, base: base(), step };
    setDraft(now);
  };

  const move = (e: PointerEvent) => {
    const g = gesture;
    if (!g || g.kind === "velocity") return;
    const { col, row } = cellAt(e);
    const now = [...g.base];
    if (g.kind === "draw") {
      put(now, g.start, g.step, Math.max(1, col - g.start + 1));
    } else if (g.kind === "resize") {
      put(now, g.start, now[g.start], Math.max(1, col - g.start + 1));
    } else {
      const by = col - g.col;
      const rowBy = row - g.row;
      if (by === 0 && rowBy === 0 && !g.moved) return;
      g.moved = true;
      const s = g.base[g.start];
      const to = Math.max(0, Math.min(length() - g.length, g.start + by));
      const midi = rows()[Math.max(0, Math.min(rows().length - 1, rowOf(g.midi) + rowBy))];
      const midis = (s.midi ?? []).map((m) => (m === g.midi ? midi : m));
      clear(now, g.start);
      put(now, to, s.kind === "sub" ? s : noteStep(midis, s.velocity, !!s.slide, scale()), g.length);
    }
    setDraft(now);
  };

  const up = () => {
    const g = gesture;
    if (!g) return;
    if (g.kind === "move" && !g.moved) {
      gesture = null;
      setDraft(null);
      return;
    }
    void commit(g.kind === "draw" ? "a note" : g.kind === "resize" ? "a longer note" : "a note moved");
  };

  const context = (e: MouseEvent) => {
    e.preventDefault();
    const { col, row } = cellAt(e as PointerEvent);
    const midi = rows()[row];
    const hit = bars().find((b) => b.midi === midi && col >= b.start && col < b.start + b.length);
    if (!hit) return;
    const now = [...base()];
    const left = (hit.step.midi ?? []).filter((m) => m !== midi);
    if (hit.step.kind === "chord" && left.length) {
      now[hit.start] = noteStep(left, hit.step.velocity, false, scale());
    } else {
      clear(now, hit.start);
    }
    gesture = { kind: "velocity", base: base() };
    setDraft(now);
    void commit("a note deleted");
  };

  const dblclick = (e: MouseEvent) => {
    const { col } = cellAt(e as PointerEvent);
    const at = startOf(base(), col) ?? col;
    const token = prompt(
      "the step, as the pattern writes it: 1.2:0.9  ~5.2  [1.3 5.3 2.4]:0.8  <1.1 1.1 1.1>  ..  -",
      base()[at]?.token ?? "-",
    );
    if (token !== null && token.trim()) void setStep(0, at, token.trim());
  };

  // Velocity: the height of the bar at a note's start, painted by a drag.
  const paint = (e: PointerEvent) => {
    const g = gesture;
    if (!g || g.kind !== "velocity") return;
    const r = lane.getBoundingClientRect();
    const col = Math.max(0, Math.min(length() - 1, Math.floor(((e.clientX - r.left) / r.width) * length())));
    const v = Math.max(0.05, Math.min(1, Math.round((1 - (e.clientY - r.top) / r.height) * 20) / 20));
    const now = [...(draft() ?? g.base)];
    const s = now[col];
    if (!s.midi || s.kind === "sub") return;
    now[col] = noteStep(s.midi, v, !!s.slide, scale());
    setDraft(now);
  };
  const laneDown = (e: PointerEvent) => {
    if (e.button !== 0) return;
    lane.setPointerCapture(e.pointerId);
    gesture = { kind: "velocity", base: base() };
    setDraft([...base()]);
    paint(e);
  };

  return (
    <div class="roll">
      <div class="roll-head">
        <span class="scale">{scale().name ?? "C major"}</span>
        <button class="range" title="one more octave on top" onClick={() => grow(1, 0)}>
          +▲
        </button>
        <button class="range" title="one more octave under" onClick={() => grow(0, 1)}>
          +▼
        </button>
      </div>
      <div class="roll-body">
        <div class="keys">
          <Index each={rows()}>
            {(m) => {
              const d = () => degreeOf(m(), scale());
              return (
                <div class={`key ${d()?.degree === 1 ? "root" : ""} ${d() ? "" : "off"}`} style={{ height: `${ROW}px` }}>
                  <span class="degree">{d() ? `${d()!.degree}.${d()!.octave}` : "·"}</span>
                  <span class="pitch">{nameOf(m())}</span>
                </div>
              );
            }}
          </Index>
          <div class="key vel-key">vel</div>
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
              onDblClick={dblclick}
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
                      class={`bar-note ${b.step.kind} ${b.step.slide ? "slide" : ""}`}
                      style={{
                        left: `${(b.start / length()) * 100}%`,
                        width: `${(b.length / length()) * 100}%`,
                        top: `${rowOf(b.midi) * ROW + 1}px`,
                        height: `${ROW - 2}px`,
                        "--v": String(b.step.velocity ?? 0.8),
                      }}
                      title={b.step.token}
                    >
                      <span>{b.step.kind === "sub" ? b.step.token : `${b.step.slide ? "~" : ""}${noteText(b.midi, scale())}`}</span>
                    </div>
                  </Show>
                )}
              </For>
            </div>
            <div
              ref={lane}
              class="vel-lane"
              onPointerDown={laneDown}
              onPointerMove={paint}
              onPointerUp={() => void commit("velocity")}
              onPointerCancel={() => void commit("velocity")}
            >
              <Index each={steps()}>
                {(s, i) => (
                  <Show when={s().midi && s().kind !== "sub"}>
                    <div
                      class="vel"
                      style={{
                        left: `${(i / length()) * 100}%`,
                        width: `${100 / length()}%`,
                        height: `${(s().velocity ?? 0.8) * 100}%`,
                      }}
                      title={s().velocity == null ? "default velocity" : String(s().velocity)}
                    />
                  </Show>
                )}
              </Index>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
