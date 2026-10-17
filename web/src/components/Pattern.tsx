// The chosen row's pattern. A voice's is a piano roll (`PianoRoll`); a
// groove's is a lane per drum, sixteen cells a bar: a click cycles a cell
// (rest, hit, accent, ghost), the wheel moves its velocity, a double click
// opens the token itself (rolls, runs), and a right click rests the step.
// Every change is sent to the engine, which rewrites the pattern in the
// file that defines it.

import { Index, Show } from "solid-js";
import type { PatternStep } from "@tatum/engine";
import { pattern, position, setStep } from "../store";
import { PianoRoll } from "./PianoRoll";

function hitToken(velocity: number): string {
  const v = Math.max(0.05, Math.min(1, Math.round(velocity * 20) / 20));
  if (Math.abs(v - 1) < 0.001) return "X";
  if (Math.abs(v - 0.8) < 0.001) return "x";
  if (Math.abs(v - 0.35) < 0.001) return "o";
  return `x:${v}`;
}

function nextHit(s: PatternStep): string {
  if (s.kind === "rest") return "x";
  const v = s.velocity ?? 0.8;
  if (s.kind === "hit" && Math.abs(v - 0.8) < 0.001) return "X";
  if (s.kind === "hit" && Math.abs(v - 1) < 0.001) return "o";
  return "-";
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
    if (s.kind === "hit") void setStep(lane, step, hitToken((s.velocity ?? 0.8) + (e.deltaY < 0 ? 0.1 : -0.1)));
  };
  const edit = (lane: number, step: number, s: PatternStep) => {
    const token = prompt("the step, as the pattern writes it: x  X  o  x:0.6  x*3  -", s.token);
    if (token !== null && token.trim()) void setStep(lane, step, token.trim());
  };

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
                : "click draws (drag to hold) · drag a note to move · its edge: length · right click deletes · shift: slide · alt: chord · double click: the token"}
            </span>
          </header>
          <Show when={p().drums} fallback={<PianoRoll />}>
            <div class="lanes">
              <Index each={p().lanes}>
                {(lane, li) => (
                  <div class="lane">
                    <span class="label">{lane().label}</span>
                    <div class="steps" style={{ "grid-template-columns": `repeat(${p().length}, minmax(14px, 1fr))` }}>
                      <Index each={lane().steps}>
                        {(s, si) => (
                          <div
                            class={`cell ${s().kind} ${s().kind === "hit" && (s().velocity ?? 0) >= 0.99 ? "acc" : ""} ${si === playhead() ? "now" : ""} ${si % 16 === 0 ? "bar" : si % 4 === 0 ? "beat" : ""}`}
                            style={{ "--v": String(s().kind === "rest" ? 0 : (s().velocity ?? 0.8)) }}
                            title={s().token}
                            onClick={() => void setStep(li, si, nextHit(s()))}
                            onDblClick={() => edit(li, si, s())}
                            onContextMenu={(e) => (e.preventDefault(), setStep(li, si, "-"))}
                            onWheel={(e) => wheel(e, li, si, s())}
                          >
                            {s().roll && s().roll! > 1 ? `×${s().roll}` : s().kind === "drumsub" ? "< >" : ""}
                          </div>
                        )}
                      </Index>
                    </div>
                  </div>
                )}
              </Index>
            </div>
          </Show>
        </section>
      )}
    </Show>
  );
}
