import { For } from "solid-js";
import { dslScenes, activeSceneName, switchScene } from "../stores/synth";

export default function SceneBar() {
  return (
    <div class="scene-bar">
      <span class="scene-bar-label">SCENE</span>
      <For each={dslScenes()}>
        {(scene) => (
          <button
            classList={{ "scene-pill": true, active: activeSceneName() === scene.name }}
            onClick={() => switchScene(scene.name)}
          >
            {scene.name.replace(/_/g, " ")}
          </button>
        )}
      </For>
    </div>
  );
}
