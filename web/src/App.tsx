import { Show } from "solid-js";
import { Rows } from "./components/Rows";
import { Knobs } from "./components/Knobs";
import { Pattern } from "./components/Pattern";
import { Transport } from "./components/Transport";
import { Spectrum } from "./components/Spectrum";
import { said, view } from "./store";

export function App() {
  return (
    <div class="app">
      <Transport />
      <Show when={view()} fallback={<p class="hint">A loop to try voices in. Play starts the engine; ← → choose a voice, drag a knob, save what sounds right.</p>}>
        {(v) => (
          <>
            <Rows view={v()} />
            <Pattern />
            <Knobs view={v()} />
          </>
        )}
      </Show>
      <Spectrum />
      <p class={`said ${said().bad ? "bad" : ""}`}>{said().text}</p>
    </div>
  );
}
