import { For, Show } from "solid-js";
import type { View } from "@tatum/engine";
import { act } from "../store";
import { Knob } from "./Knob";

export function Knobs(props: { view: View }) {
  const row = () => props.view.rows[props.view.focus];
  const save = () => {
    const r = row();
    if (!r?.name) return;
    const name = prompt(`Save ${r.role} as it sounds now. The same name writes over ${r.name}; another is a new voice.`, r.name);
    if (name) void act({ action: "save", row: props.view.focus, name });
  };
  return (
    <section class="knobs">
      <header>
        <span class="heading">
          {row()?.name ? `${row().role} · ${row().name}` : "nothing chosen"}
        </span>
        <span class="ccs">knobs 1–{props.view.knob_ccs.length} on cc {props.view.knob_ccs.join(" ")}</span>
        <span class="spacer" />
        <button onClick={() => act({ action: "reset", row: props.view.focus })} title="every knob of the row back to the text">
          reset
        </button>
        <button class="save" disabled={!row()?.name} onClick={save} title="save the voice as it sounds, in units">
          save
        </button>
      </header>
      <Show when={props.view.macros.length} fallback={<p class="hint">choose a voice first</p>}>
        <div class="grid">
          <For each={props.view.macros}>
            {(m, i) => (
              <div class={`macro ${m.held ? "held" : ""}`}>
                <span class="number">{i() < 9 ? i() + 1 : ""}</span>
                <Knob value={m.position} onChange={(value) => act({ action: "turn", row: props.view.focus, knob: i(), value })} />
                <span class="label">{m.label}</span>
                <span class="reading">{m.reading}</span>
              </div>
            )}
          </For>
        </div>
      </Show>
    </section>
  );
}
