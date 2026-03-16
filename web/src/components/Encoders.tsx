import { For } from "solid-js";
import Knob from "./Knob";
import { page, setParamValue, currentPage, pageCount } from "../stores/synth";

const KNOB_COLORS = ["var(--orange)", "var(--teal)", "var(--yellow)", "var(--pink)"];

export default function Encoders() {
  return (
    <>
      <div class="encoders">
        <For each={page().keys}>
          {(label, i) => (
            <Knob
              label={label}
              value={page().values[i()]}
              color={KNOB_COLORS[i()]}
              onChange={(v) => setParamValue(i(), v)}
            />
          )}
        </For>
      </div>
      <div class="pages">
        <For each={Array.from({ length: pageCount() })}>
          {(_, i) => (
            <div
              class="pg"
              classList={{ active: currentPage() === i() }}
            />
          )}
        </For>
      </div>
    </>
  );
}
