import { Match, Show, Switch, createSignal } from "solid-js";
import { Rows } from "./components/Rows";
import { Knobs } from "./components/Knobs";
import { Fx } from "./components/Fx";
import { Mixer } from "./components/Mixer";
import { Pattern } from "./components/Pattern";
import { Display, Top } from "./components/Transport";
import { play, view } from "./store";

type Mode = "edit" | "fx" | "mix";
const MODES: { mode: Mode; label: string; title: string }[] = [
  { mode: "edit", label: "EDIT", title: "the chosen row's pattern and sound" },
  { mode: "fx", label: "FX", title: "the chosen row's effects: add, take out, reorder, turn" },
  { mode: "mix", label: "MIX", title: "every row's level, pan and sends" },
];

export function App() {
  const [mode, setMode] = createSignal<Mode>("edit");
  return (
    <div class="device">
      <Top />
      <Display />
      <Show
        when={view()}
        fallback={
          <section class="panel start">
            <button class="key dark play big" onClick={play}>
              <span>▶ PLAY</span>
            </button>
            <p class="hint">
              A loop to try voices in. Play starts the engine; ◀ ▶ choose a voice, draw its notes, turn a knob, save what sounds right.
            </p>
          </section>
        }
      >
        {(v) => (
          <>
            <Rows view={v()} />
            <nav class="modes">
              {MODES.map((m) => (
                <button class={`key mode ${mode() === m.mode ? "lit" : ""}`} onClick={() => setMode(m.mode)} title={m.title}>
                  <i class="led" />
                  {m.label}
                </button>
              ))}
            </nav>
            <Switch>
              <Match when={mode() === "edit"}>
                <Pattern />
                <Knobs view={v()} />
              </Match>
              <Match when={mode() === "fx"}>
                <Fx />
                <Knobs view={v()} />
              </Match>
              <Match when={mode() === "mix"}>
                <Mixer view={v()} />
              </Match>
            </Switch>
          </>
        )}
      </Show>
    </div>
  );
}
