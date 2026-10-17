// The rows of the loop as a strip of cards, one per role: its shape, the
// voice it plays with keys for the one before and the next, and its mute.
// A click on a card chooses the row the pattern and the knobs are for.

import { Index } from "solid-js";
import type { View } from "@tatum/engine";
import { act, focus } from "../store";

/** A shape and a colour for each role, Bauhaus primaries; others cycle. */
const SHAPES = ["circle", "square", "triangle", "half", "bar"];
const ROLE_SHAPE: Record<string, string> = { drums: "circle", bass: "square", lead: "triangle", pad: "half", perc: "bar" };

export function shapeOf(role: string, i: number): string {
  return ROLE_SHAPE[role] ?? SHAPES[i % SHAPES.length];
}

export function Rows(props: { view: View }) {
  return (
    <section class="rows">
      <Index each={props.view.rows}>
        {(row, i) => (
          <div
            class={`row ${i === props.view.focus ? "focused" : ""} ${row().name ? "" : "empty"} ${row().muted ? "muted" : ""}`}
            onClick={() => focus(i)}
            title={row().doc ?? ""}
          >
            <div class="row-head">
              <i class={`shape ${shapeOf(row().role, i)}`} />
              <span class="role">{row().role.toUpperCase()}</span>
              <span class={`held ${row().held ? "on" : ""}`} title="knobs turned, not saved" />
            </div>
            <div class="row-voice">
              <button class="key small dark arrow" onClick={(e) => (e.stopPropagation(), act({ action: "choose", row: i, by: -1 }))} title="the voice before">
                ◀
              </button>
              <span class="name">{row().name ?? "—"}</span>
              <button class="key small dark arrow" onClick={(e) => (e.stopPropagation(), act({ action: "choose", row: i, by: 1 }))} title="the next voice">
                ▶
              </button>
            </div>
            <div class="row-foot">
              <span class="kind">{row().name ? row().kind : `${row().count} voices`}</span>
              <span class="which">{row().index ? `${row().index}/${row().count}` : ""}</span>
              <button
                class={`key small mute ${row().muted ? "lit" : ""}`}
                disabled={!row().name}
                onClick={(e) => (e.stopPropagation(), act({ action: "mute", row: i }))}
                title="mute the row's track, or bring it back"
              >
                M
              </button>
            </div>
          </div>
        )}
      </Index>
    </section>
  );
}
