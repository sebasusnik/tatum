// The chosen row's sound: its macros as round knobs, numbered as the
// controller's are, with their readings in units, and the keys beside
// them: reset, A/B against the text, save, and play.

import { Index, Show } from "solid-js";
import type { View } from "@tatum/engine";
import { act, play, running, stop } from "../store";
import { Knob, type Tone } from "./Knob";

const TONES: Tone[] = ["white", "black", "orange", "black", "white", "black", "blue", "black"];

export function Knobs(props: { view: View }) {
  const row = () => props.view.rows[props.view.focus];
  const save = () => {
    const r = row();
    if (!r?.name) return;
    const name = prompt(`Save ${r.role} as it sounds now. The same name writes over ${r.name}; another is a new voice.`, r.name);
    if (name) void act({ action: "save", row: props.view.focus, name });
  };
  return (
    <section class="panel knobs">
      <header class="panel-head">
        <span class="label">SOUND</span>
        <span class="heading">{row()?.name ? `${row().role} · ${row().name}` : "nothing chosen"}</span>
        <span class="spacer" />
        <span class="ccs">CC {props.view.knob_ccs.join(" ")}</span>
      </header>
      <div class="sound">
        <Show when={props.view.macros.length} fallback={<p class="hint">choose a voice first</p>}>
          {/* By index, not by object: every answer is a new view, and a knob
              mid-drag has to stay the same element to keep the pointer. */}
          <div class="grid">
            <Index each={props.view.macros}>
              {(m, i) => (
                <div class={`macro ${m().held ? "held" : ""}`}>
                  <span class="number">
                    <i class="led" />
                    {i < 9 ? i + 1 : ""}
                  </span>
                  <Knob
                    value={m().position}
                    tone={TONES[i % TONES.length]}
                    onChange={(value) => act({ action: "turn", row: props.view.focus, knob: i, value })}
                  />
                  <span class="label">{m().label.toUpperCase()}</span>
                  <span class="reading">{m().reading}</span>
                </div>
              )}
            </Index>
          </div>
        </Show>
        <div class="cluster">
          <button class="key" onClick={() => act({ action: "reset", row: props.view.focus })} title="every knob of the row back to the text">
            <span>RESET</span>
          </button>
          <button
            class={`key ${props.view.comparing ? "lit" : ""}`}
            onClick={() => act({ action: "compare" })}
            title="A/B: every knob at the text's value, then back"
          >
            <span>{props.view.comparing ? "A · TEXT" : "A / B"}</span>
          </button>
          <button class="key orange save" disabled={!row()?.name} onClick={save} title="save the voice as it sounds, in units">
            <span>SAVE</span>
          </button>
          <button class="key dark play" onClick={() => (running() ? stop() : play())}>
            <span>{running() ? "■ STOP" : "▶ PLAY"}</span>
          </button>
        </div>
      </div>
    </section>
  );
}
