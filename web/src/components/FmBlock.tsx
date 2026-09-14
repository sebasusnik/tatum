import { createMemo, For } from "solid-js";
import Knob from "./Knob";
import ActivityLed from "./ActivityLed";
import PatternVis from "./PatternVis";
import {
  sends, setModuleSend, muted, soloed, toggleMute, toggleSolo,
  getModulePagesForTrack, getModuleNameForTrack, setModuleParamForTrack,
  getPatternForTrack,
} from "../stores/synth";

const OP_LABELS = ["OP1", "OP2", "OP3", "OP4"];
const OP_PAGES = [3, 4, 5, 6];

export default function FmBlock(props: { trackName: string }) {
  const pages = createMemo(() => getModulePagesForTrack(props.trackName));
  const moduleName = createMemo(() => getModuleNameForTrack(props.trackName));
  const pattern = createMemo(() => getPatternForTrack(props.trackName));
  const color = "#ffd23f";

  const knob = (page: number, idx: number) => pages()[page]?.values[idx] ?? 50;
  const setK = (page: number, idx: number, v: number) => setModuleParamForTrack(props.trackName, page, idx, v);

  return (
    <div class="rw-block" data-m="fm">
      <div class="rw-header">
        <ActivityLed pattern={pattern()} color={color} />
        <span class="rw-label" style={{ color }}>FM{moduleName() ? ` · ${moduleName()}` : ""}</span>
        <div class="rw-btns">
          <button classList={{ "rw-m": true, active: muted().fm }} onClick={() => toggleMute("fm")}>M</button>
          <button classList={{ "rw-s": true, active: soloed().fm }} onClick={() => toggleSolo("fm")}>S</button>
        </div>
      </div>

      <div class="rw-knobs-primary">
        <Knob value={knob(0, 0)} label="Algo" color={color} size={48} onChange={(v) => setK(0, 0, v)} />
        <Knob value={knob(0, 1)} label="Mod" color={color} size={48} onChange={(v) => setK(0, 1, v)} />
        <Knob value={knob(0, 2)} label="Fbk" color={color} size={48} onChange={(v) => setK(0, 2, v)} />
      </div>

      <div class="rw-knobs-secondary">
        <Knob value={knob(2, 2)} label="Wav" color={color} size={32} onChange={(v) => setK(2, 2, v)} />
        <Knob value={knob(2, 3)} label="Cho" color={color} size={32} onChange={(v) => setK(2, 3, v)} />
        <Knob value={knob(1, 0)} label="Atk" color={color} size={32} onChange={(v) => setK(1, 0, v)} />
        <Knob value={knob(1, 1)} label="Dcy" color={color} size={32} onChange={(v) => setK(1, 1, v)} />
        <Knob value={knob(1, 2)} label="Sus" color={color} size={32} onChange={(v) => setK(1, 2, v)} />
      </div>

      <div class="fm-operators">
        <For each={OP_LABELS}>
          {(label, i) => {
            const pg = () => OP_PAGES[i()];
            return (
              <div class="fm-op-row">
                <span class="fm-op-label">{label}</span>
                <Knob value={knob(pg(), 0)} label="Rat" color={color} size={26} onChange={(v) => setK(pg(), 0, v)} />
                <Knob value={knob(pg(), 1)} label="Fbk" color={color} size={26} onChange={(v) => setK(pg(), 1, v)} />
                <Knob value={knob(pg(), 2)} label="Atk" color={color} size={26} onChange={(v) => setK(pg(), 2, v)} />
                <Knob value={knob(pg(), 3)} label="Dcy" color={color} size={26} onChange={(v) => setK(pg(), 3, v)} />
              </div>
            );
          }}
        </For>
      </div>

      <PatternVis pattern={pattern()} color={color} />

      <div class="rw-sends">
        <Knob value={sends().fm.delay * 100} label="Dly" color="#5b8cff" size={26} onChange={(v) => setModuleSend("fm", "delay", v / 100)} />
        <Knob value={sends().fm.reverb * 100} label="Rev" color="#b07aff" size={26} onChange={(v) => setModuleSend("fm", "reverb", v / 100)} />
      </div>
    </div>
  );
}
