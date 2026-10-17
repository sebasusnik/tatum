// The mixer: a channel strip per row with something in it. Pan and the two
// sends as small knobs, the level on a fader, mute and solo. Every move is
// held on the engine like a knob, and the row's save writes it into the
// voice's track.

import { Index, Show } from "solid-js";
import type { View } from "@tatum/engine";
import { act, focus } from "../store";
import { Fader } from "./Fader";
import { Knob } from "./Knob";
import { shapeOf } from "./Rows";

type What = "level" | "pan" | "reverb_send" | "delay_send";
const SENDS: { what: What; label: string; at: number; tone: "white" | "black" | "blue" }[] = [
  { what: "pan", label: "PAN", at: 1, tone: "white" },
  { what: "reverb_send", label: "REVERB", at: 2, tone: "blue" },
  { what: "delay_send", label: "DELAY", at: 3, tone: "black" },
];

export function Mixer(props: { view: View }) {
  return (
    <section class="panel mixer">
      <header class="panel-head">
        <span class="label">MIX</span>
        <span class="hint">drag a fader or a knob · M mutes · S plays the row alone · save on a row writes its channel</span>
      </header>
      <div class="strips">
        <Index each={props.view.rows}>
          {(row, i) => (
            <Show when={(props.view.mix[i] ?? []).length === 4}>
              <div
                class={`strip ${i === props.view.focus ? "focused" : ""} ${row().muted ? "muted" : ""}`}
                onClick={() => focus(i)}
              >
                <div class="strip-head">
                  <i class={`shape ${shapeOf(row().role, i)}`} />
                  <span class="role">{row().role.toUpperCase()}</span>
                </div>
                <span class="strip-name">{row().name}</span>
                <Index each={SENDS}>
                  {(s) => {
                    const m = () => props.view.mix[i][s().at];
                    return (
                      <div class={`send ${m().held ? "held" : ""}`} onClick={(e) => e.stopPropagation()}>
                        <Knob
                          value={m().position}
                          tone={s().tone}
                          onChange={(value) => act({ action: "mix", row: i, what: s().what, value })}
                        />
                        <span class="label">{s().label}</span>
                        <span class="reading">{m().reading}</span>
                      </div>
                    );
                  }}
                </Index>
                <div class={`level ${props.view.mix[i][0].held ? "held" : ""}`} onClick={(e) => e.stopPropagation()}>
                  <Fader value={props.view.mix[i][0].position} onChange={(value) => act({ action: "mix", row: i, what: "level", value })} />
                  <span class="reading">{props.view.mix[i][0].reading}</span>
                </div>
                <div class="strip-keys">
                  <button
                    class={`key small mute ${row().muted ? "lit" : ""}`}
                    onClick={(e) => (e.stopPropagation(), act({ action: "mute", row: i }))}
                    title="mute the track, or bring it back"
                  >
                    M
                  </button>
                  <button
                    class={`key small solo ${row().soloed ? "lit" : ""}`}
                    onClick={(e) => (e.stopPropagation(), act({ action: "solo", row: i }))}
                    title="the track alone, or back with the rest"
                  >
                    S
                  </button>
                </div>
              </div>
            </Show>
          )}
        </Index>
      </div>
    </section>
  );
}
