import { createMemo } from "solid-js";
import Knob from "./Knob";
import ActivityLed from "./ActivityLed";
import PatternVis from "./PatternVis";
import {
  sends, setModuleSend, muted, soloed, toggleMute, toggleSolo,
  getModulePagesForTrack, getModuleNameForTrack, setModuleParamForTrack,
  getPatternForTrack,
} from "../stores/tatum";

export default function KeysBlock(props: { trackName: string }) {
  const pages = createMemo(() => getModulePagesForTrack(props.trackName));
  const moduleName = createMemo(() => getModuleNameForTrack(props.trackName));
  const pattern = createMemo(() => getPatternForTrack(props.trackName));
  const color = "#00c9b1";

  const knob = (page: number, idx: number) => pages()[page]?.values[idx] ?? 50;
  const setK = (page: number, idx: number, v: number) => setModuleParamForTrack(props.trackName, page, idx, v);

  return (
    <div class="rw-block" data-m="keys">
      <div class="rw-header">
        <ActivityLed pattern={pattern()} color={color} />
        <span class="rw-label" style={{ color }}>KEYS{moduleName() ? ` · ${moduleName()}` : ""}</span>
        <div class="rw-btns">
          <button classList={{ "rw-m": true, active: muted().keys }} onClick={() => toggleMute("keys")}>M</button>
          <button classList={{ "rw-s": true, active: soloed().keys }} onClick={() => toggleSolo("keys")}>S</button>
        </div>
      </div>

      <div class="rw-knobs-primary">
        <Knob value={knob(0, 0)} label="Cut" color={color} size={48} onChange={(v) => setK(0, 0, v)} />
        <Knob value={knob(0, 2)} label="Cho" color={color} size={48} onChange={(v) => setK(0, 2, v)} />
        <Knob value={knob(2, 2)} label="Voi" color={color} size={48} onChange={(v) => setK(2, 2, v)} />
      </div>

      <div class="rw-knobs-secondary">
        <Knob value={knob(0, 1)} label="Det" color={color} size={32} onChange={(v) => setK(0, 1, v)} />
        <Knob value={knob(3, 0)} label="Res" color={color} size={32} onChange={(v) => setK(3, 0, v)} />
        <Knob value={knob(1, 0)} label="Atk" color={color} size={32} onChange={(v) => setK(1, 0, v)} />
        <Knob value={knob(1, 1)} label="Dcy" color={color} size={32} onChange={(v) => setK(1, 1, v)} />
        <Knob value={knob(2, 0)} label="LFO" color={color} size={32} onChange={(v) => setK(2, 0, v)} />
      </div>

      <PatternVis pattern={pattern()} color={color} />

      <div class="rw-sends">
        <Knob value={sends().keys.delay * 100} label="Dly" color="#5b8cff" size={26} onChange={(v) => setModuleSend("keys", "delay", v / 100)} />
        <Knob value={sends().keys.reverb * 100} label="Rev" color="#b07aff" size={26} onChange={(v) => setModuleSend("keys", "reverb", v / 100)} />
      </div>
    </div>
  );
}
