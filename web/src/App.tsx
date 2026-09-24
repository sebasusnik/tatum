import { onMount, onCleanup, createSignal, Show, For } from "solid-js";
import Transport from "./components/Transport";
import HarmonyBar from "./components/HarmonyBar";
import SceneBar from "./components/SceneBar";
import Canvas from "./components/Canvas";
import CodeView from "./components/CodeView";
import Mixer from "./components/Mixer";
import TrackCard from "./components/TrackCard";
import MasterPanel from "./components/MasterPanel";
import DetailDrawer from "./components/DetailDrawer";
import { togglePlayback, sendSource, dslErrors, dslSource, setDslSource, syncStoreFromDsl, dslScenes, trackInfos } from "./stores/tatum";
import CHILLWAVE_SOURCE from "../../examples/chillwave_dream.synth?raw";

type ViewMode = "visual" | "code" | "split";

export default function App() {
  const [viewMode, setViewMode] = createSignal<ViewMode>("code");
  const [selectedTrack, setSelectedTrack] = createSignal<string | null>(null);

  function onKeyDown(e: KeyboardEvent) {
    if ((e.target as HTMLElement)?.closest?.(".cm-editor")) return;
    if (e.key === " ") { e.preventDefault(); togglePlayback(); }
  }

  onMount(() => {
    document.addEventListener("keydown", onKeyDown);
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

      <Show when={dslScenes().length > 0}>
        <SceneBar />
      </Show>

      <div class="synth-content">
        {/* VISUAL mode — graph canvas */}
        <Show when={viewMode() === "visual"}>
          <Canvas />
        </Show>

        {/* CODE mode — editor + mixer sidebar */}
        <Show when={viewMode() === "code"}>
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

        {/* SPLIT mode — tracks + code + master + detail */}
        <Show when={viewMode() === "split"}>
          <div class="split-layout">
            {/* Track cards */}
            <div class="split-tracks">
              <For each={trackInfos()}>
                {(track, i) => (
                  <TrackCard
                    trackName={track.name}
                    trackIdx={i()}
                    selected={selectedTrack() === track.name}
                    onSelect={() => setSelectedTrack(
                      selectedTrack() === track.name ? null : track.name
                    )}
                  />
                )}
              </For>
            </div>

            {/* Center: code + detail drawer */}
            <div class="split-center">
              <div class="split-code">
                <CodeView
                  onSource={handleSource}
                  source={dslSource()}
                  errors={dslErrors()}
                  accentColor="#ff6b35"
                />
              </div>
              <DetailDrawer trackName={selectedTrack()} />
            </div>

            {/* Master panel */}
            <MasterPanel />
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
