import { Show } from "solid-js";
import { act, connectMidi, midi, play, position, running, sessionText, stop, view } from "../store";
import { download, forgetStored } from "../library";

export function Transport() {
  const beat = () => {
    const p = position();
    return p ? Math.floor((p.step % 16) / 4) : 0;
  };
  return (
    <header class="transport">
      <span class="brand">◆ TATUM</span>
      <span class="title">compose</span>
      <button class="play" onClick={() => (running() ? stop() : play())}>
        {running() ? "■ stop" : "▶ play"}
      </button>
      <Show when={position()}>
        {(p) => (
          <span class="position">
            {Math.round(p().tempo)} BPM · bar <b>{p().bar + 1}</b>{" "}
            {[0, 1, 2, 3].map((b) => (
              <span class={b === beat() ? "beat on" : "beat"}>{b === beat() ? "●" : "○"}</span>
            ))}
          </span>
        )}
      </Show>
      <span class="spacer" />
      <Show when={view()}>
        <button onClick={() => act({ action: "compare" })} title="A/B: every knob at the text's value, then back">
          {view()?.comparing ? "A: text" : "a/b"}
        </button>
        <button onClick={() => download("session.synth", sessionText())} title="the session file, as the terminal would have it">
          session ↓
        </button>
        <button
          onClick={() => {
            if (confirm("Forget every voice and session this browser saved?")) {
              forgetStored();
              location.reload();
            }
          }}
          title="back to the shipped library"
        >
          forget
        </button>
      </Show>
      <button onClick={connectMidi} title="the controller's knobs on the chosen row (Chrome, Edge)">
        {midi().length ? `midi: ${midi().length}` : "midi"}
      </button>
    </header>
  );
}
