import { For } from "solid-js";
import Knob from "./Knob";
import InlineSeq from "./InlineSeq";
import {
  modules, patterns, currentStep, playing,
  setModuleParam, toggleStepFor, setStepNote,
  sends, setModuleSend, muted, soloed, toggleMute, toggleSolo,
} from "../stores/synth";

export default function BassBlock() {
  const mod = () => modules.bass;
  const color = "#ff6b35";
  const pat = () => patterns.bass;

  // Page 0: Cutoff(0), Reso(1), EnvMod(2), Glide(3)
  // Page 1: Attack(0), Decay(1), Sustain(2), Release(3)
  // Page 2: Osc2(0), Osc3(1), Wave(2), Keytrack(3)
  const knob = (page: number, idx: number) => mod().pages[page].values[idx];
  const setK = (page: number, idx: number, v: number) => setModuleParam("bass", page, idx, v);

  return (
    <div class="rw-block" data-m="bass">
      <div class="rw-header">
        <span class="rw-label" style={{ color }}>BASS</span>
        <div class="rw-btns">
          <button classList={{ "rw-m": true, active: muted().bass }} onClick={() => toggleMute("bass")}>M</button>
          <button classList={{ "rw-s": true, active: soloed().bass }} onClick={() => toggleSolo("bass")}>S</button>
        </div>
      </div>

      <div class="rw-knobs-primary">
        <Knob value={knob(0, 0)} label="Cut" color={color} size={48} onChange={(v) => setK(0, 0, v)} />
        <Knob value={knob(0, 1)} label="Res" color={color} size={48} onChange={(v) => setK(0, 1, v)} />
        <Knob value={knob(0, 2)} label="Env" color={color} size={48} onChange={(v) => setK(0, 2, v)} />
      </div>

      <div class="rw-knobs-secondary">
        <Knob value={knob(0, 3)} label="Gld" color={color} size={32} onChange={(v) => setK(0, 3, v)} />
        <Knob value={knob(1, 0)} label="Atk" color={color} size={32} onChange={(v) => setK(1, 0, v)} />
        <Knob value={knob(1, 1)} label="Dcy" color={color} size={32} onChange={(v) => setK(1, 1, v)} />
        <Knob value={knob(2, 2)} label="Wav" color={color} size={32} onChange={(v) => setK(2, 2, v)} />
        <Knob value={knob(2, 0)} label="Os2" color={color} size={32} onChange={(v) => setK(2, 0, v)} />
      </div>

      <InlineSeq
        moduleId="bass"
        pattern={pat()}
        currentStep={currentStep()}
        playing={playing()}
        color={color}
        onToggle={(idx) => toggleStepFor("bass", idx)}
        onNoteChange={(idx, note) => setStepNote("bass", idx, note)}
      />

      <div class="rw-sends">
        <Knob value={sends().bass.delay * 100} label="Dly" color="#5b8cff" size={26} onChange={(v) => setModuleSend("bass", "delay", v / 100)} />
        <Knob value={sends().bass.reverb * 100} label="Rev" color="#b07aff" size={26} onChange={(v) => setModuleSend("bass", "reverb", v / 100)} />
      </div>
    </div>
  );
}
