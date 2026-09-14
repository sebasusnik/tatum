import { For } from "solid-js";
import {
  trackInfos,
  setTrackLevelRT,
  setTrackPanRT,
  persistTrackProp,
} from "../stores/synth";

export default function Mixer() {
  return (
    <div class="mixer">
      <div class="mixer-header">MIXER</div>
      <div class="mixer-channels">
        <For each={trackInfos()}>
          {(track, idx) => (
            <div class="mixer-channel">
              <div class="mixer-label">{track.name}</div>
              <input
                type="range"
                class="mixer-fader"
                ref={(el) => el.setAttribute("orient", "vertical")}
                min="0"
                max="100"
                value={Math.round(track.level * 100)}
                onInput={(e) => {
                  const val = parseInt(e.currentTarget.value) / 100;
                  setTrackLevelRT(idx(), val);
                }}
                onChange={(e) => {
                  const val = parseInt(e.currentTarget.value) / 100;
                  persistTrackProp(track.name, "level", val);
                }}
              />
              <div class="mixer-value">{Math.round(track.level * 100)}</div>
              <input
                type="range"
                class="mixer-pan"
                min="-100"
                max="100"
                value={Math.round(track.pan * 100)}
                onInput={(e) => {
                  const val = parseInt(e.currentTarget.value) / 100;
                  setTrackPanRT(idx(), val);
                }}
                onChange={(e) => {
                  const val = parseInt(e.currentTarget.value) / 100;
                  persistTrackProp(track.name, "pan", val);
                }}
              />
              <div class="mixer-pan-label">
                {track.pan === 0 ? "C" : track.pan < 0 ? `L${Math.abs(Math.round(track.pan * 100))}` : `R${Math.round(track.pan * 100)}`}
              </div>
            </div>
          )}
        </For>
      </div>
    </div>
  );
}
