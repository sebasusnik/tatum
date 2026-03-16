import { onMount, onCleanup, createEffect } from "solid-js";
import Transport from "./components/Transport";
import ModuleTabs from "./components/ModuleTabs";
import Screen from "./components/Screen";
import Encoders from "./components/Encoders";
import FxBar from "./components/FxBar";
import Sequencer from "./components/Sequencer";
import {
  accentColor, switchModule, nextPage, prevPage,
  togglePlayback, setRecording,
} from "./stores/synth";

export default function App() {
  // Set CSS accent color reactively
  createEffect(() => {
    document.documentElement.style.setProperty("--accent", accentColor());
  });

  // Keyboard shortcuts
  function onKeyDown(e: KeyboardEvent) {
    if (e.key === " ") { e.preventDefault(); togglePlayback(); }
    if (e.key === "1") switchModule("bass");
    if (e.key === "2") switchModule("keys");
    if (e.key === "3") switchModule("fm");
    if (e.key === "4") switchModule("beats");
    if (e.key === "ArrowRight") nextPage();
    if (e.key === "ArrowLeft") prevPage();
    if (e.key === "r" || e.key === "R") setRecording((r) => !r);
  }

  onMount(() => document.addEventListener("keydown", onKeyDown));
  onCleanup(() => document.removeEventListener("keydown", onKeyDown));

  return (
    <div class="synth">
      <Transport />
      <ModuleTabs />
      <Screen />
      <Encoders />
      <FxBar />
      <Sequencer />
      <div class="hints">
        <span class="hint"><kbd>Space</kbd> Play</span>
        <span class="hint"><kbd>1</kbd>-<kbd>4</kbd> Module</span>
        <span class="hint">
          <kbd>{"\u2190"}</kbd><kbd>{"\u2192"}</kbd> Page
        </span>
        <span class="hint"><kbd>R</kbd> Motion Rec</span>
      </div>
    </div>
  );
}
