import Knob from "./Knob";
import {
  arpParams, setArpParam,
  muted, soloed, toggleMute, toggleSolo,
} from "../stores/tatum";

export default function ArpBlock() {
  const color = "#7dd3fc";

  return (
    <div class="rw-fx-block arp-block" data-fx="arp">
      <div class="arp-header">
        <span class="rw-fx-label" style={{ color }}>ARP</span>
        <div class="arp-header-btns">
          <button classList={{ "rw-m": true, active: muted().arp }} onClick={() => toggleMute("arp")}>M</button>
          <button classList={{ "rw-s": true, active: soloed().arp }} onClick={() => toggleSolo("arp")}>S</button>
        </div>
      </div>
      <div class="rw-fx-knobs">
        <Knob value={arpParams.rate} label="Rate" color={color} size={32}
          onChange={(v) => setArpParam("rate", v)} />
        <Knob value={arpParams.gate} label="Gate" color={color} size={32}
          onChange={(v) => setArpParam("gate", v)} />
        <Knob value={arpParams.pattern} label="Pat" color={color} size={32}
          onChange={(v) => setArpParam("pattern", v)} />
        <Knob value={arpParams.level} label="Lvl" color={color} size={32}
          onChange={(v) => setArpParam("level", v)} />
      </div>
      <div class="arp-section-label">WAVE</div>
      <div class="rw-fx-knobs">
        <Knob value={arpParams.waveform} label="Wave" color={color} size={28}
          onChange={(v) => setArpParam("waveform", v)} />
        <Knob value={arpParams.octaveRange} label="Oct" color={color} size={28}
          onChange={(v) => setArpParam("octaveRange", v)} />
      </div>
      <div class="arp-section-label">ENV</div>
      <div class="rw-fx-knobs">
        <Knob value={arpParams.attack} label="Atk" color={color} size={28}
          onChange={(v) => setArpParam("attack", v)} />
        <Knob value={arpParams.decay} label="Dcy" color={color} size={28}
          onChange={(v) => setArpParam("decay", v)} />
        <Knob value={arpParams.sustain} label="Sus" color={color} size={28}
          onChange={(v) => setArpParam("sustain", v)} />
        <Knob value={arpParams.release} label="Rel" color={color} size={28}
          onChange={(v) => setArpParam("release", v)} />
      </div>
      <div class="arp-section-label">FILTER</div>
      <div class="rw-fx-knobs">
        <Knob value={arpParams.filterCutoff} label="Cut" color={color} size={28}
          onChange={(v) => setArpParam("filterCutoff", v)} />
        <Knob value={arpParams.filterResonance} label="Res" color={color} size={28}
          onChange={(v) => setArpParam("filterResonance", v)} />
      </div>
    </div>
  );
}
