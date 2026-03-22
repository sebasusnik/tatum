import { onMount, onCleanup, createSignal, Show } from "solid-js";
import Transport from "./components/Transport";
import HarmonyBar from "./components/HarmonyBar";
import Canvas from "./components/Canvas";
import CodeView from "./components/CodeView";
import Mixer from "./components/Mixer";
import { togglePlayback, sendSource, dslErrors, dslSource, setDslSource, syncStoreFromDsl } from "./stores/synth";
import CHILLWAVE_SOURCE from "../../examples/chillwave_dream.synth?raw";

type ViewMode = "visual" | "code" | "split";

export default function App() {
  const [viewMode, setViewMode] = createSignal<ViewMode>("code");

  function onKeyDown(e: KeyboardEvent) {
    if ((e.target as HTMLElement)?.closest?.(".cm-editor")) return;
    if (e.key === " ") { e.preventDefault(); togglePlayback(); }
  }

  onMount(() => {
    document.addEventListener("keydown", onKeyDown);
    // Load default preset and sync store signals (BPM, harmony)
    setDslSource(CHILLWAVE_SOURCE);
    syncStoreFromDsl(CHILLWAVE_SOURCE);
  });
  onCleanup(() => document.removeEventListener("keydown", onKeyDown));

  const handleSource = (source: string) => { sendSource(source); };

  return (
    <div class="synth">
      <div class="top-bar">
        <Transport />
        <div class="view-toggle">
          <button classList={{ active: viewMode() === "visual" }} onClick={() => setViewMode("visual")}>Visual</button>
          <button classList={{ active: viewMode() === "code" }} onClick={() => setViewMode("code")}>Code</button>
          <button classList={{ active: viewMode() === "split" }} onClick={() => setViewMode("split")}>Split</button>
        </div>
        <HarmonyBar />
      </div>

      <div class="synth-content">
        <Show when={viewMode() !== "code"}>
          <Canvas />
        </Show>
        <Show when={viewMode() !== "visual"}>
          <div class="code-and-mixer">
            <CodeView
              onSource={handleSource}
              source={dslSource()}
              errors={dslErrors()}
              accentColor="#ff6b35"
            />
            <Mixer />
          </div>
        </Show>
      </div>

      <div class="hints">
        <span class="hint"><kbd>Space</kbd> Play</span>
        <span class="hint"><kbd>Ctrl+Enter</kbd> Evaluate</span>
      </div>
    </div>
  );
}
