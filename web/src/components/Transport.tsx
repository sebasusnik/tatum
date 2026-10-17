// The top of the device and its display. The top is a strip of small keys
// (the controller, the session file, forgetting) over the name; the display
// is the black band: where the loop is, its tempo and scale, what the
// engine last said, and the mix's spectrum.

import { Show } from "solid-js";
import { connectMidi, midi, pattern, position, running, said, sessionText, view } from "../store";
import { download, forgetStored } from "../library";
import { Spectrum } from "./Spectrum";

export function Top() {
  return (
    <header class="top">
      <div class="tabs">
        <button class={`tab ${midi().length ? "lit" : ""}`} onClick={connectMidi} title="the controller's knobs on the chosen row (Chrome, Edge)">
          <i class="led" />
          {midi().length ? `MIDI ${midi().length}` : "MIDI"}
        </button>
        <Show when={view()}>
          <button class="tab" onClick={() => download("session.synth", sessionText())} title="the session file, as the terminal would have it">
            SESSION ↓
          </button>
          <button
            class="tab"
            onClick={() => {
              if (confirm("Forget every voice and session this browser saved?")) {
                forgetStored();
                location.reload();
              }
            }}
            title="back to the shipped library"
          >
            FORGET
          </button>
        </Show>
      </div>
      <div class="nameplate">
        <div>
          <h1>
            TATUM <span class="mark">C</span>
          </h1>
          <p class="kana">コンポーザー</p>
          <p class="model">LOOP COMPOSER · VOICES · PATTERNS</p>
        </div>
        <div class="grille" aria-hidden="true" />
      </div>
    </header>
  );
}

export function Display() {
  const beat = () => {
    const p = position();
    return p ? Math.floor(((p.step + 15) % 16) / 4) : 0;
  };
  const bar = () => {
    const p = position();
    return p ? String(p.bar + 1).padStart(3, "0") : "001";
  };
  return (
    <section class="display">
      <div class="lcd-left">
        <span class="lcd-tag">BAR</span>
        <span class="digits position">
          {bar()}
          <span class="dot">.</span>
          {beat() + 1}
        </span>
        <span class={`glyph ${running() ? "on" : ""}`}>{running() ? "▶" : "■"}</span>
        <span class="beats">
          {[0, 1, 2, 3].map((b) => (
            <i class={b === beat() && running() ? "on" : ""} />
          ))}
        </span>
      </div>
      <div class="lcd-mid">
        <div class="lcd-row">
          <span class="lcd-tag">BPM</span>
          <span class="lcd-value">{position() ? Math.round(position()!.tempo) : "---"}</span>
          <span class="lcd-tag">KEY</span>
          <span class="lcd-value">{pattern()?.scale.name?.toUpperCase() ?? "---"}</span>
        </div>
        <p class={`said ${said().bad ? "bad" : ""}`}>{said().text}</p>
      </div>
      <div class="lcd-right">
        <Spectrum />
      </div>
    </section>
  );
}
