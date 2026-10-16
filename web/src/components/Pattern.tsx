// The chosen row's pattern as a grid: a lane per drum, or one lane of
// notes; sixteen steps a bar. A click cycles a cell (rest → hit → accent →
// ghost on a drum; rest → note → tie on a voice), a drag up or down moves a
// note by scale degrees, the wheel moves the velocity, a double click opens
// the token itself for anything the language has (chords, slides, runs),
// and a right click rests the step. Every change is one token sent to the
// engine, which rewrites the pattern in the voice's file.

import { Index, Show, createSignal } from "solid-js";
import type { PatternStep } from "@tatum/engine";
import { pattern, position, setStep } from "../store";

const NOTE_DEFAULT = "1.2";

/** The velocity a step sounds at, for how bright its cell is. */
function velocityOf(s: PatternStep): number {
  if (s.kind === "hit") return s.velocity ?? 0.8;
  if (s.kind === "rest" || s.kind === "tie") return 0;
  return s.velocity ?? 0.8;
}

function hitToken(velocity: number): string {
  const v = Math.max(0.05, Math.min(1, Math.round(velocity * 20) / 20));
  if (Math.abs(v - 1) < 0.001) return "X";
  if (Math.abs(v - 0.8) < 0.001) return "x";
  if (Math.abs(v - 0.35) < 0.001) return "o";
  return `x:${v}`;
}

function noteToken(note: string, velocity: number | null | undefined, slide: boolean | undefined): string {
  const v = velocity == null ? "" : `:${Math.round(velocity * 100) / 100}`;
  return `${slide ? "~" : ""}${note}${v}`;
}

/** A degree or a note name moved by `by` scale steps (degrees) or semitones (names). */
function moved(note: string, by: number): string {
  const degree = /^(\d)\.(\d)$/.exec(note);
  if (degree) {
    let d = Number(degree[1]) - 1 + by;
    let o = Number(degree[2]);
    while (d < 0) {
      d += 7;
      o -= 1;
    }
    while (d > 6) {
      d -= 7;
      o += 1;
    }
    return `${d + 1}.${Math.max(0, Math.min(9, o))}`;
  }
  const name = /^([A-Ga-g])(#|b)?(-?\d+)$/.exec(note);
  if (name) {
    const names = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    const base = { C: 0, D: 2, E: 4, F: 5, G: 7, A: 9, B: 11 }[name[1].toUpperCase()] ?? 0;
    const acc = name[2] === "#" ? 1 : name[2] === "b" ? -1 : 0;
    const midi = Math.max(0, Math.min(127, (Number(name[3]) + 1) * 12 + base + acc + by));
    return `${names[midi % 12]}${Math.floor(midi / 12) - 1}`;
  }
  return note;
}

export function Pattern() {
  const [drag, setDrag] = createSignal<{ lane: number; step: number; y: number; note: string; moved: number } | null>(null);
  const playhead = () => {
    const p = pattern();
    const pos = position();
    return p && pos ? pos.step % Math.max(1, p.length) : -1;
  };
  const lastNote = (lane: number): string => {
    const steps = pattern()?.lanes[lane]?.steps ?? [];
    for (let i = steps.length - 1; i >= 0; i--) {
      const n = steps[i].note;
      if (steps[i].kind === "note" && n) return n;
    }
    return NOTE_DEFAULT;
  };

  // A drag ends with a click on the same cell; that click is not a cycle.
  let dragged = false;
  const click = (lane: number, step: number, s: PatternStep) => {
    const p = pattern();
    if (!p) return;
    if (dragged) {
      dragged = false;
      return;
    }
    if (p.drums) {
      const next = s.kind === "rest" ? "x" : s.kind === "hit" && Math.abs((s.velocity ?? 0.8) - 0.8) < 0.001 ? "X" : s.kind === "hit" && Math.abs((s.velocity ?? 0) - 1) < 0.001 ? "o" : "-";
      void setStep(lane, step, next);
    } else {
      const next = s.kind === "rest" ? noteToken(lastNote(lane), null, false) : s.kind === "note" ? ".." : "-";
      void setStep(lane, step, next);
    }
  };
  const wheel = (e: WheelEvent, lane: number, step: number, s: PatternStep) => {
    e.preventDefault();
    const by = e.deltaY < 0 ? 0.1 : -0.1;
    if (s.kind === "hit") void setStep(lane, step, hitToken((s.velocity ?? 0.8) + by));
    else if (s.kind === "note" && s.note) void setStep(lane, step, noteToken(s.note, Math.max(0.05, Math.min(1, (s.velocity ?? 0.8) + by)), s.slide));
  };
  const edit = (lane: number, step: number, s: PatternStep) => {
    const token = prompt("the step, as the pattern writes it: 1.2:0.9  ~5.2  [1.3 5.3 2.4]:0.8  <1.1 1.1 1.1>  x:0.6  ..  -", s.token);
    if (token !== null && token.trim()) void setStep(lane, step, token.trim());
  };
  const down = (e: PointerEvent, lane: number, step: number, s: PatternStep) => {
    if (e.button !== 0 || s.kind !== "note" || !s.note) return;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    setDrag({ lane, step, y: e.clientY, note: s.note, moved: 0 });
  };
  const move = (e: PointerEvent, s: PatternStep) => {
    const d = drag();
    if (!d) return;
    const steps = Math.round((d.y - e.clientY) / 14);
    if (steps !== d.moved) {
      setDrag({ ...d, moved: steps });
      void setStep(d.lane, d.step, noteToken(moved(d.note, steps), s.velocity, s.slide));
    }
  };
  const up = () => {
    const d = drag();
    dragged = d !== null && d.moved !== 0;
    setDrag(null);
  };

  return (
    <Show when={pattern()}>
      {(p) => (
        <section class="pattern">
          <header>
            <span class="heading">
              {p().track} · {p().name}
            </span>
            <span class="hint">
              click cycles · drag a note up or down · wheel: velocity · double click: the token · right click: rest
            </span>
          </header>
          <div class="lanes">
            <Index each={p().lanes}>
              {(lane, li) => (
                <div class="lane">
                  <span class="label">{lane().label ?? "notes"}</span>
                  <div class="steps" style={{ "grid-template-columns": `repeat(${p().length}, minmax(14px, 1fr))` }}>
                    <Index each={lane().steps}>
                      {(s, si) => (
                        <div
                          class={`cell ${s().kind} ${si === playhead() ? "now" : ""} ${si % 16 === 0 ? "bar" : si % 4 === 0 ? "beat" : ""}`}
                          style={{ "--v": String(velocityOf(s())) }}
                          title={s().token}
                          onClick={() => click(li, si, s())}
                          onDblClick={() => edit(li, si, s())}
                          onContextMenu={(e) => (e.preventDefault(), setStep(li, si, "-"))}
                          onWheel={(e) => wheel(e, li, si, s())}
                          onPointerDown={(e) => down(e, li, si, s())}
                          onPointerMove={(e) => move(e, s())}
                          onPointerUp={up}
                          onPointerCancel={up}
                        >
                          {cellText(s())}
                        </div>
                      )}
                    </Index>
                  </div>
                </div>
              )}
            </Index>
          </div>
        </section>
      )}
    </Show>
  );
}

function cellText(s: PatternStep): string {
  switch (s.kind) {
    case "rest":
      return "";
    case "tie":
      return "·";
    case "hit":
      return s.roll && s.roll > 1 ? `×${s.roll}` : "";
    case "note":
      return `${s.slide ? "~" : ""}${s.note ?? ""}`;
    case "chord":
      return "[ ]";
    case "sub":
    case "drumsub":
      return "< >";
    default:
      return "";
  }
}
