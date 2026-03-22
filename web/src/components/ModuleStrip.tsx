import { For } from "solid-js";
import type { ModuleId } from "../stores/synth";
import {
  modules, selectedModule, switchModule,
  muted, soloed, toggleMute, toggleSolo,
} from "../stores/synth";

const MODULE_IDS: ModuleId[] = ["bass", "keys", "fm", "beats"];

export default function ModuleStrip() {
  return (
    <div class="module-strip">
      <For each={MODULE_IDS}>
        {(id) => {
          const mod = () => modules[id];
          const firstPage = () => mod().pages[0];

          return (
            <div
              class="mod-card"
              classList={{ selected: selectedModule() === id }}
              data-m={id}
              onClick={() => switchModule(id)}
            >
              <div class="mod-card-label">{mod().label}</div>
              <div class="mod-card-btns">
                <button
                  class="ms-btn"
                  classList={{ muted: muted()[id] }}
                  onClick={(e) => { e.stopPropagation(); toggleMute(id); }}
                >
                  M
                </button>
                <button
                  class="ms-btn"
                  classList={{ soloed: soloed()[id] }}
                  onClick={(e) => { e.stopPropagation(); toggleSolo(id); }}
                >
                  S
                </button>
              </div>
              <div class="mod-card-knobs">
                <For each={firstPage().keys}>
                  {(_, ki) => {
                    const val = () => firstPage().values[ki()];
                    const angle = () => -135 + (val() / 100) * 270;
                    return (
                      <div class="mini-knob-display">
                        <div class="mini-ring">
                          <div
                            class="mini-ring-indicator"
                            style={{ "--mini-angle": `${angle()}deg` }}
                          >
                            <div class="mini-ring-dot" />
                          </div>
                        </div>
                        <span class="mini-val">{val()}</span>
                      </div>
                    );
                  }}
                </For>
              </div>
            </div>
          );
        }}
      </For>
    </div>
  );
}
