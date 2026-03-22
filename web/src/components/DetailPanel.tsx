import { For } from "solid-js";
import Knob from "./Knob";
import {
  mod, page, currentPage, setCurrentPage,
  pageCount, setParamValue, accentColor,
} from "../stores/synth";

export default function DetailPanel() {
  return (
    <div class="detail-panel">
      <div class="detail-header">
        <div>
          <div class="detail-title">{mod().label}</div>
          <div class="detail-sub">{mod().sub}</div>
        </div>
        <div class="detail-pages">
          <For each={Array.from({ length: pageCount() })}>
            {(_, i) => (
              <div
                class="detail-dot"
                classList={{ active: currentPage() === i() }}
                onClick={() => setCurrentPage(i())}
              />
            )}
          </For>
        </div>
      </div>

      <div class="detail-knobs">
        <For each={page().keys}>
          {(key, i) => (
            <div class="detail-knob-wrap">
              <Knob
                value={page().values[i()]}
                label={key}
                color={accentColor()}
                onChange={(v) => setParamValue(i(), v)}
              />
              <span class="detail-knob-value">{Math.round(page().values[i()])}</span>
            </div>
          )}
        </For>
      </div>
    </div>
  );
}
