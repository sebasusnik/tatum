import { Index } from "solid-js";
import type { View } from "@tatum/engine";
import { act, focus } from "../store";

export function Rows(props: { view: View }) {
  return (
    <section class="rows">
      <Index each={props.view.rows}>
        {(row, i) => (
          <div class={`row ${i === props.view.focus ? "focused" : ""} ${row().name ? "" : "empty"}`} onClick={() => focus(i)}>
            <span class="role">{row().role}</span>
            <button class="arrow" onClick={(e) => (e.stopPropagation(), act({ action: "choose", row: i, by: -1 }))} title="the voice before">
              ‹
            </button>
            <span class="name">{row().name ?? "—"}</span>
            <button class="arrow" onClick={(e) => (e.stopPropagation(), act({ action: "choose", row: i, by: 1 }))} title="the next voice">
              ›
            </button>
            <span class="doc">
              {row().name ? (
                <>
                  <em>{row().kind}</em> {row().doc}
                </>
              ) : (
                `nothing · ‹ › picks one of ${row().count}`
              )}
            </span>
            <span class="which">{row().index ? `${row().index}/${row().count}` : ""}</span>
            <span class={`held ${row().held ? "on" : ""}`} title="knobs turned, not saved">
              ●
            </span>
            <button
              class="mute"
              disabled={!row().name}
              onClick={(e) => (e.stopPropagation(), act({ action: "mute", row: i }))}
              title="mute the row's track, or bring it back"
            >
              m
            </button>
          </div>
        )}
      </Index>
    </section>
  );
}
