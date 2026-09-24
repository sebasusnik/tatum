import { createMemo, For } from "solid-js";
import { createStore } from "solid-js/store";
import Knob from "./Knob";
import ActivityLed from "./ActivityLed";
import PatternVis from "./PatternVis";
import {
  drumLanes, currentStep, playing,
  toggleDrumStep, setRawParam,
  sends, setModuleSend, muted, soloed, toggleMute, toggleSolo,
  getModulePagesForTrack, getModuleNameForTrack, setModuleParamForTrack,
  getPatternForTrack,
} from "../stores/tatum";

const DRUM_LEVEL_IDS = [104, 105, 106, 107];
const DRUM_PAN_IDS = [99, 100, 101, 102];
const DRUM_NAMES = ["K", "S", "H", "C"];

const [drumMix, setDrumMix] = createStore({
  levels: [80, 70, 60, 65],
  pans: [50, 50, 50, 50],
});

export default function BeatsBlock(props: { trackName: string }) {
  const pages = createMemo(() => getModulePagesForTrack(props.trackName));
  const moduleName = createMemo(() => getModuleNameForTrack(props.trackName));
  const pattern = createMemo(() => getPatternForTrack(props.trackName));
  const color = "#ff5ea0";

  const knob = (page: number, idx: number) => pages()[page]?.values[idx] ?? 50;
  const setK = (page: number, idx: number, v: number) => setModuleParamForTrack(props.trackName, page, idx, v);

  function setDrumLevel(drum: number, v: number) {
    const clamped = Math.round(Math.min(100, Math.max(0, v)));
    setDrumMix("levels", drum, clamped);
    setRawParam(DRUM_LEVEL_IDS[drum], clamped / 100);
  }

  function setDrumPan(drum: number, v: number) {
    const clamped = Math.round(Math.min(100, Math.max(0, v)));
    setDrumMix("pans", drum, clamped);
    setRawParam(DRUM_PAN_IDS[drum], clamped / 100);
  }

  return (
    <div class="rw-block rw-block-beats" data-m="beats">
      <div class="rw-header">
        <ActivityLed pattern={pattern()} color={color} />
        <span class="rw-label" style={{ color }}>BEATS{moduleName() ? ` · ${moduleName()}` : ""}</span>
        <div class="rw-btns">
          <button classList={{ "rw-m": true, active: muted().beats }} onClick={() => toggleMute("beats")}>M</button>
          <button classList={{ "rw-s": true, active: soloed().beats }} onClick={() => toggleSolo("beats")}>S</button>
        </div>
      </div>

      <div class="rw-drum-grid">
        <For each={drumLanes}>
          {(lane, laneIdx) => (
            <div class="rw-drum-row">
              <span class="rw-drum-label">{lane.name.slice(0, 2).toUpperCase()}</span>
              <For each={Array.from({ length: 16 })}>
                {(_, stepIdx) => (
                  <div
                    classList={{
                      "rw-drum-cell": true,
                      on: lane.steps[stepIdx()],
                      now: playing() && currentStep() === stepIdx(),
                    }}
                    style={{ "--cell-color": color }}
                    onClick={() => toggleDrumStep(laneIdx(), stepIdx())}
                  />
                )}
              </For>
            </div>
          )}
        </For>
      </div>

      <div class="rw-knobs-secondary">
        <Knob value={knob(0, 0)} label="K.Dcy" color={color} size={32} onChange={(v) => setK(0, 0, v)} />
        <Knob value={knob(0, 1)} label="S.Dcy" color={color} size={32} onChange={(v) => setK(0, 1, v)} />
        <Knob value={knob(1, 0)} label="K.Pit" color={color} size={32} onChange={(v) => setK(1, 0, v)} />
        <Knob value={knob(2, 0)} label="Swng" color={color} size={32} onChange={(v) => setK(2, 0, v)} />
      </div>

      <div class="drum-mixer">
        <For each={DRUM_NAMES}>
          {(name, i) => (
            <div class="drum-mixer-ch">
              <span class="drum-mixer-label">{name}</span>
              <Knob value={drumMix.levels[i()]} label="Lvl" color={color} size={22} onChange={(v) => setDrumLevel(i(), v)} />
              <Knob value={drumMix.pans[i()]} label="Pan" color={color} size={22} onChange={(v) => setDrumPan(i(), v)} />
            </div>
          )}
        </For>
      </div>

      <PatternVis pattern={pattern()} color={color} />

      <div class="rw-sends">
        <Knob value={sends().beats.delay * 100} label="Dly" color="#5b8cff" size={26} onChange={(v) => setModuleSend("beats", "delay", v / 100)} />
        <Knob value={sends().beats.reverb * 100} label="Rev" color="#b07aff" size={26} onChange={(v) => setModuleSend("beats", "reverb", v / 100)} />
      </div>
    </div>
  );
}
