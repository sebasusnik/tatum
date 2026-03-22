import { For } from "solid-js";
import {
  harmonyRoot, harmonyScale, harmonyDegree,
  setRoot, setScale, setDegree,
} from "../stores/synth";

const ROOT_NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
const SCALE_NAMES = ["Major", "Minor", "Dorian", "Mixo", "PentMin"];
const DEGREE_LABELS = ["I", "II", "III", "IV", "V", "VI", "VII"];

export default function HarmonyBar() {
  function cycleScale() {
    setScale((harmonyScale() + 1) % SCALE_NAMES.length);
  }

  return (
    <div class="harmony-bar">
      <div class="harmony-section">
        <span class="harmony-label">Root</span>
        <For each={ROOT_NAMES}>
          {(name, i) => (
            <button
              class="root-btn"
              classList={{ active: harmonyRoot() === i() }}
              onClick={() => setRoot(i())}
            >
              {name}
            </button>
          )}
        </For>
      </div>

      <div class="harmony-section">
        <button class="scale-btn" onClick={cycleScale}>
          {SCALE_NAMES[harmonyScale()]}
        </button>
      </div>

      <div class="harmony-section">
        <span class="harmony-label">Chord</span>
        <For each={DEGREE_LABELS}>
          {(label, i) => (
            <button
              class="deg-btn"
              classList={{ active: harmonyDegree() === i() }}
              onClick={() => setDegree(i())}
            >
              {label}
            </button>
          )}
        </For>
      </div>
    </div>
  );
}
