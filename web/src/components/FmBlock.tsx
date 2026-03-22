import { For } from "solid-js";
import Knob from "./Knob";
import InlineSeq from "./InlineSeq";
import {
  modules, patterns, currentStep, playing,
  setModuleParam, toggleStepFor, setStepNote,
  sends, setModuleSend, muted, soloed, toggleMute, toggleSolo,
} from "../stores/synth";

const OP_LABELS = ["OP1", "OP2", "OP3", "OP4"];
const OP_PAGES = [3, 4, 5, 6]; // pages in FM module config

export default function FmBlock() {
  const mod = () => modules.fm;
  const color = "#ffd23f";
  const pat = () => patterns.fm;

  const knob = (page: number, idx: number) => mod().pages[page].values[idx];
  const setK = (page: number, idx: number, v: number) => setModuleParam("fm", page, idx, v);

  return (
    <div class="rw-block" data-m="fm">
      <div class="rw-header">
        <span class="rw-label" style={{ color }}>FM</span>
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

      {/* Per-operator controls */}
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

      <InlineSeq
        moduleId="fm"
        pattern={pat()}
        currentStep={currentStep()}
        playing={playing()}
        color={color}
        onToggle={(idx) => toggleStepFor("fm", idx)}
        onNoteChange={(idx, note) => setStepNote("fm", idx, note)}
      />

      <div class="rw-sends">
        <Knob value={sends().fm.delay * 100} label="Dly" color="#5b8cff" size={26} onChange={(v) => setModuleSend("fm", "delay", v / 100)} />
        <Knob value={sends().fm.reverb * 100} label="Rev" color="#b07aff" size={26} onChange={(v) => setModuleSend("fm", "reverb", v / 100)} />
      </div>
    </div>
  );
}
