// The top of the device and its display. The top is a strip of small keys
// (the controller, the session file, forgetting) over the name; the display
// is the black band: where the loop is, its tempo and scale, what the
// engine last said, and the mix's spectrum.

import { Show } from "solid-js";
import { act, connectMidi, midi, pattern, position, running, said, sessionText, view } from "../store";
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

const ROOTS = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
const FLATS: Record<string, string> = { Db: "C#", Eb: "D#", Gb: "F#", Ab: "G#", Bb: "A#", Cb: "B", Fb: "E" };

/**
 * The song's key on the display: the root a semitone at a time, the kind
 * from every scale the language has, and, when the session set its own,
 * a key to give the groove's back. Degrees follow it on the next bar.
 */
function Key() {
  const key = () => view()?.key;
  const root = () => {
    const r = key()?.root ?? "C";
    const name = r.length > 1 ? r[0].toUpperCase() + r.slice(1) : r.toUpperCase();
    return FLATS[name] ?? name;
  };
  const kind = () => key()?.kind ?? "major";
  const shift = (by: number) => {
    const i = ROOTS.indexOf(root());
    const next = ROOTS[(((i < 0 ? 0 : i) + by) % 12 + 12) % 12];
    void act({ action: "scale", root: next, kind: kind() });
  };
  return (
    <Show when={view()} fallback={<span class="lcd-value">{pattern()?.scale.name?.toUpperCase() ?? "---"}</span>}>
      <span class={`lcd-tag ${key()?.own ? "own" : ""}`} title={key()?.own ? "the session's key" : "the groove's key"}>
        KEY
      </span>
      <button class="lcd-step" onClick={() => shift(-1)} title="a semitone down">
        ◀
      </button>
      <span class="lcd-value root">{root()}</span>
      <button class="lcd-step" onClick={() => shift(1)} title="a semitone up">
        ▶
      </button>
      <select class="lcd-select" value={kind()} onChange={(e) => void act({ action: "scale", root: root(), kind: e.currentTarget.value })}>
        {(view()?.scales ?? []).map((s) => (
          <option value={s} selected={s === kind()}>
            {s.replace(/_/g, " ").toUpperCase()}
          </option>
        ))}
      </select>
      <Show when={key()?.own}>
        <button class="lcd-step text" onClick={() => void act({ action: "scale" })} title="the groove's own key again">
          GROOVE
        </button>
      </Show>
    </Show>
  );
}

/**
 * The song's tempo on the display: typed, or a BPM at a time (ten with
 * shift), and, when the session set its own, a key to give the groove's back.
 */
function Tempo() {
  const tempo = () => view()?.tempo;
  const bpm = () => tempo()?.bpm ?? position()?.tempo ?? 0;
  const set = (to: number) => {
    const bpm10 = Math.round(to * 10) / 10;
    if (!Number.isFinite(bpm10) || bpm10 < 20 || bpm10 > 400 || bpm10 === bpm()) return false;
    void act({ action: "tempo", bpm: bpm10 });
    return true;
  };
  const shown = () => String(Math.round(bpm() * 10) / 10);
  return (
    <Show when={view()} fallback={<span class="lcd-value">{position() ? Math.round(position()!.tempo) : "---"}</span>}>
      <span class="lcd-group">
      <button class="lcd-step" onClick={(e) => set(bpm() - (e.shiftKey ? 10 : 1))} title="a BPM slower, ten with shift">
        ◀
      </button>
      <input
        class="lcd-value lcd-input"
        type="text"
        inputmode="decimal"
        value={shown()}
        onFocus={(e) => e.currentTarget.select()}
        onKeyDown={(e) => {
          if (e.key === "Enter") e.currentTarget.blur();
          if (e.key === "Escape") (e.currentTarget.value = shown(), e.currentTarget.blur());
        }}
        onChange={(e) => {
          if (!set(parseFloat(e.currentTarget.value))) e.currentTarget.value = shown();
        }}
        title="type a tempo from 20 to 400, enter to set it"
      />
      <button class="lcd-step" onClick={(e) => set(bpm() + (e.shiftKey ? 10 : 1))} title="a BPM faster, ten with shift">
        ▶
      </button>
      <Show when={tempo()?.own}>
        <button class="lcd-step text" onClick={() => void act({ action: "tempo" })} title="the groove's own tempo again">
          GROOVE
        </button>
      </Show>
      </span>
    </Show>
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
          <span class={`lcd-tag ${view()?.tempo.own ? "own" : ""}`}>BPM</span>
          <Tempo />
          <Key />
        </div>
        <p class={`said ${said().bad ? "bad" : ""}`}>{said().text}</p>
      </div>
      <div class="lcd-right">
        <Spectrum />
      </div>
    </section>
  );
}
