import { Show } from "solid-js";
import { Rows } from "./components/Rows";
import { Knobs } from "./components/Knobs";
import { Pattern } from "./components/Pattern";
import { Display, Top } from "./components/Transport";
import { play, view } from "./store";

export function App() {
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
            <Pattern />
            <Knobs view={v()} />
          </>
        )}
      </Show>
    </div>
  );
}
