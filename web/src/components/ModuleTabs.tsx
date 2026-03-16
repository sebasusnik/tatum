import { For } from "solid-js";
import { currentModule, switchModule, type ModuleId } from "../stores/synth";

const TABS: { id: ModuleId; icon: string; label: string }[] = [
  { id: "bass", icon: "~", label: "Bass" },
  { id: "keys", icon: "\u266D", label: "Keys" },
  { id: "fm", icon: "\u221E", label: "FM" },
  { id: "beats", icon: "\u25C9", label: "Beats" },
];

export default function ModuleTabs() {
  return (
    <div class="module-tabs">
      <For each={TABS}>
        {(tab) => (
          <button
            class="mod-tab"
            classList={{ active: currentModule() === tab.id }}
            data-m={tab.id}
            onClick={() => switchModule(tab.id)}
          >
            <span class="tab-icon">{tab.icon}</span>
            {tab.label}
          </button>
        )}
      </For>
    </div>
  );
}
