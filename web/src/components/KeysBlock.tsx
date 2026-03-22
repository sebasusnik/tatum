import Knob from "./Knob";
import InlineSeq from "./InlineSeq";
import {
  modules, patterns, currentStep, playing,
  setModuleParam, toggleStepFor, setStepNote,
  sends, setModuleSend, muted, soloed, toggleMute, toggleSolo,
} from "../stores/synth";

export default function KeysBlock() {
  const mod = () => modules.keys;
  const color = "#00c9b1";
  const pat = () => patterns.keys;

  const knob = (page: number, idx: number) => mod().pages[page].values[idx];
  const setK = (page: number, idx: number, v: number) => setModuleParam("keys", page, idx, v);

  return (
    <div class="rw-block" data-m="keys">
      <div class="rw-header">
        <span class="rw-label" style={{ color }}>KEYS</span>
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
        <Knob value={knob(0, 3)} label="Lvl" color={color} size={32} onChange={(v) => setK(0, 3, v)} />
        <Knob value={knob(3, 0)} label="Res" color={color} size={32} onChange={(v) => setK(3, 0, v)} />
        <Knob value={knob(2, 3)} label="Vib" color={color} size={32} onChange={(v) => setK(2, 3, v)} />
        <Knob value={knob(2, 0)} label="LFO" color={color} size={32} onChange={(v) => setK(2, 0, v)} />
      </div>

      <InlineSeq
        moduleId="keys"
        pattern={pat()}
        currentStep={currentStep()}
        playing={playing()}
        color={color}
        onToggle={(idx) => toggleStepFor("keys", idx)}
        onNoteChange={(idx, note) => setStepNote("keys", idx, note)}
      />

      <div class="rw-sends">
        <Knob value={sends().keys.delay * 100} label="Dly" color="#5b8cff" size={26} onChange={(v) => setModuleSend("keys", "delay", v / 100)} />
        <Knob value={sends().keys.reverb * 100} label="Rev" color="#b07aff" size={26} onChange={(v) => setModuleSend("keys", "reverb", v / 100)} />
      </div>
    </div>
  );
}
