import { createSignal, For } from "solid-js";
import MiniKnob from "./MiniKnob";

const FX_LIST = [
  { id: "delay", label: "Delay", defaultOn: true },
  { id: "reverb", label: "Reverb", defaultOn: true },
  { id: "eq", label: "EQ", defaultOn: false },
  { id: "comp", label: "Comp", defaultOn: false },
];

export default function FxBar() {
  const [fxState, setFxState] = createSignal(
    FX_LIST.map((fx) => ({ ...fx, on: fx.defaultOn }))
  );

  function toggleFx(idx: number) {
    setFxState((prev) =>
      prev.map((fx, i) => (i === idx ? { ...fx, on: !fx.on } : fx))
    );
  }

  return (
    <div class="fx-bar">
      <For each={fxState()}>
        {(fx, i) => (
          <div
            class="fx-chip"
            classList={{ on: fx.on }}
            onClick={() => toggleFx(i())}
          >
            <div class="fx-dot" />
            <span class="fx-name">{fx.label}</span>
            <div class="fx-minis" onClick={(e) => e.stopPropagation()}>
              <MiniKnob />
              <MiniKnob />
            </div>
          </div>
        )}
      </For>
    </div>
  );
}
