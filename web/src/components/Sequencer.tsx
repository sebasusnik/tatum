import { For } from "solid-js";
import {
  mod, currentStep, playing, toggleStep,
  activePattern, setActivePattern,
} from "../stores/synth";

export default function Sequencer() {
  return (
    <div class="seq">
      <div class="seq-top">
        <div class="seq-modes">
          <button class="seq-mode" id="mSlide">Slide</button>
          <button class="seq-mode" id="mLockSlide">Lock&thinsp;~</button>
          <button class="seq-mode active" id="mSnap">Snap</button>
        </div>
        <div class="pattern-pills">
          <For each={["A", "B", "C", "D"]}>
            {(label, i) => (
              <button
                class="pat"
                classList={{ active: activePattern() === i() }}
                onClick={() => setActivePattern(i())}
              >
                {label}
              </button>
            )}
          </For>
        </div>
      </div>

      <div class="steps">
        <For each={mod().on}>
          {(on, i) => {
            const m = mod();
            return (
              <div
                class="stp"
                classList={{
                  on: on,
                  now: playing() && currentStep() === i(),
                  "slide-on": m.slides[i()],
                  "lock-slide-on": on && m.lockSlides[i()],
                  "has-lock": m.locks[i()],
                }}
                onClick={() => toggleStep(i())}
              >
                <div class="stp-lock" />
                <span class="stp-inner">
                  {on ? m.notes[i()] : ""}
                </span>
                <div class="stp-fill" />
              </div>
            );
          }}
        </For>
      </div>

      <div class="beats-row">
        <For each={Array(16)}>
          {(_, i) => (
            <span class="bm" classList={{ db: i() % 4 === 0 }}>
              {i() % 4 === 0 ? `${i() / 4 + 1}` : "\u00b7"}
            </span>
          )}
        </For>
      </div>
    </div>
  );
}
