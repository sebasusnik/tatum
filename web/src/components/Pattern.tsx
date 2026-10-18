// The chosen row's pattern. A voice's is a piano roll (`PianoRoll`); a
// groove's is a lane per drum, sixteen cells a bar: a click cycles a cell
// (rest, hit, accent, ghost), the wheel moves its velocity, a double click
// opens the token itself (rolls, runs), and a right click rests the step.
// Over both: the pattern's length in bars, twice what it has, undo and redo
// (ctrl+Z, ctrl+shift+Z), and for drums a lane more or one taken out.
// Every change is sent to the engine, which rewrites the pattern in the
// file that defines it.

import { For, Index, Show, onCleanup, onMount } from "solid-js";
import type { LaneTokens, PatternStep } from "@tatum/engine";
import { canUndo, lanesOf, pattern, position, putPattern, setStep, undo } from "../store";
import { hitStep, locksOf } from "../steps";
import { PianoRoll } from "./PianoRoll";

function nextHit(s: PatternStep): string {
  if (s.kind === "rest") return "x";
  const v = s.velocity ?? 0.8;
  if (s.kind === "hit" && Math.abs(v - 0.8) < 0.001) return hitStep(1, locksOf(s), s.roll ?? 1).token;
  if (s.kind === "hit" && Math.abs(v - 1) < 0.001) return hitStep(0.35, locksOf(s), s.roll ?? 1).token;
  return "-";
}

/** `tokens` at `n` steps: cut, or filled with rests. A tie cannot open a pattern. */
function resized(tokens: string[], n: number): string[] {
  const out = tokens.slice(0, n);
  while (out.length < n) out.push("-");
  return out;
}

export function Pattern() {
  const playhead = () => {
    const p = pattern();
    const pos = position();
    const at = p && pos?.running ? pos.tracks?.[p.track] : undefined;
    return p && at !== undefined ? at % Math.max(1, p.length) : -1;
  };
  const wheel = (e: WheelEvent, lane: number, step: number, s: PatternStep) => {
    e.preventDefault();
    if (s.kind === "hit") void setStep(lane, step, hitStep((s.velocity ?? 0.8) + (e.deltaY < 0 ? 0.1 : -0.1), locksOf(s), s.roll ?? 1).token);
  };
  const edit = (lane: number, step: number, s: PatternStep) => {
    const token = prompt("the step, as the pattern writes it: x  X  o  x:0.6  x*3 (a roll)  x?0.5 (a chance)  <x - x>  -", s.token);
    if (token !== null && token.trim()) void setStep(lane, step, token.trim());
  };

  const bars = () => (pattern()?.length ?? 16) / 16;
  const setBars = (n: number) => {
    const p = pattern();
    if (!p) return;
    const lanes: LaneTokens[] = lanesOf(p).map((l) => ({ ...l, tokens: resized(l.tokens, n * 16) }));
    void putPattern(lanes, `${n} bar${n > 1 ? "s" : ""}`);
  };
  const twice = () => {
    const p = pattern();
    if (!p) return;
    void putPattern(lanesOf(p).map((l) => ({ ...l, tokens: [...l.tokens, ...l.tokens] })), "twice as long");
  };
  const addLane = (label: string) => {
    const p = pattern();
    if (!p || !label) return;
    void putPattern([...lanesOf(p), { label, tokens: Array.from({ length: p.length }, () => "-") }], `a ${label} lane`);
  };
  const removeLane = (i: number) => {
    const p = pattern();
    if (!p || p.lanes.length < 2) return;
    void putPattern(lanesOf(p).filter((_, k) => k !== i), `the ${p.lanes[i].label} lane out`);
  };

  const keydown = (e: KeyboardEvent) => {
    if ((e.target as HTMLElement).closest("input, textarea, select")) return;
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "z") {
      e.preventDefault();
      void undo(e.shiftKey ? 1 : -1);
    }
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "y") {
      e.preventDefault();
      void undo(1);
    }
  };
  onMount(() => window.addEventListener("keydown", keydown));
  onCleanup(() => window.removeEventListener("keydown", keydown));

  return (
    <Show when={pattern()}>
      {(p) => (
        <section class="panel pattern">
          <header class="panel-head">
            <span class="label">PATTERN</span>
            <span class="heading">
              {p().track} · {p().name}
            </span>
            <span class="hint">
              {p().drums
                ? "click cycles · wheel: velocity · double click: the token · right click: rest"
                : "click: a note, or a chord where one starts · drag: select · drag a note: move · its edge: length · right click: the step's menu"}
            </span>
          </header>
          <div class="pattern-tools">
            <span class="tool-label">BARS</span>
            <For each={[1, 2, 4, 8]}>
              {(n) => (
                <button class={`key small ${bars() === n ? "lit" : ""}`} onClick={() => setBars(n)} title={`the pattern at ${n} bar${n > 1 ? "s" : ""}`}>
                  {n}
                </button>
              )}
            </For>
            <button class="key small" onClick={twice} title="what it has, twice: change only the second half after">
              ×2
            </button>
            <span class="tool-gap" />
            <button class="key small" disabled={!canUndo().back} onClick={() => void undo(-1)} title="undo (ctrl+Z)">
              ↶ UNDO
            </button>
            <button class="key small" disabled={!canUndo().ahead} onClick={() => void undo(1)} title="redo (ctrl+shift+Z)">
              REDO ↷
            </button>
          </div>
          <Show when={p().drums} fallback={<PianoRoll />}>
            <div class="lanes">
              <Index each={p().lanes}>
                {(lane, li) => (
                  <div class="lane">
                    <span class="label">
                      <Show when={p().lanes.length > 1}>
                        <button class="row-out" onClick={() => removeLane(li)} title="the lane out of the pattern">
                          ×
                        </button>
                      </Show>
                      {lane().label}
                    </span>
                    <div class="steps">
                      <Index each={lane().steps}>
                        {(s, si) => (
                          <>
                            <Show when={si > 0 && si % 4 === 0}>
                              <i class={si % 16 === 0 ? "sep bar" : "sep"} />
                            </Show>
                            <div
                              class={`cell ${s().kind} ${s().kind === "hit" && (s().velocity ?? 0) >= 0.99 ? "acc" : ""} ${si === playhead() ? "now" : ""}`}
                              style={{ "--v": String(s().kind === "rest" ? 0 : (s().velocity ?? 0.8)) }}
                              title={s().token}
                              onClick={() => void setStep(li, si, nextHit(s()))}
                              onDblClick={() => edit(li, si, s())}
                              onContextMenu={(e) => (e.preventDefault(), setStep(li, si, "-"))}
                              onWheel={(e) => wheel(e, li, si, s())}
                            >
                              {s().roll && s().roll! > 1 ? `×${s().roll}` : s().kind === "drumsub" ? "< >" : s().prob != null ? "?" : ""}
                            </div>
                          </>
                        )}
                      </Index>
                    </div>
                  </div>
                )}
              </Index>
              <div class="lane add-lane">
                <select
                  class="lane-pick"
                  onChange={(e) => {
                    addLane(e.currentTarget.value);
                    e.currentTarget.value = "";
                  }}
                  title="a drum more in the pattern"
                >
                  <option value="">+ PIEZA</option>
                  <For each={p().drum_names.filter((d) => !p().lanes.some((l) => l.label === d))}>
                    {(d) => <option value={d}>{d.toUpperCase()}</option>}
                  </For>
                </select>
              </div>
            </div>
          </Show>
        </section>
      )}
    </Show>
  );
}
