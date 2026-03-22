import Knob from "./Knob";
import {
  masterLevel, setMasterLevelValue,
  sidechainAmount, setSidechainValue,
  moduleLevels, setModuleLevel,
} from "../stores/synth";
import type { ModuleId } from "../stores/synth";

const MIX_CHANNELS: { id: ModuleId; label: string; color: string }[] = [
  { id: "bass", label: "Bass", color: "#ff6b35" },
  { id: "keys", label: "Keys", color: "#00c9b1" },
  { id: "fm", label: "FM", color: "#ffd23f" },
  { id: "beats", label: "Drum", color: "#ff5ea0" },
];

export default function MasterStrip() {
  return (
    <div class="rw-master">
      <div class="rw-master-knobs">
        <Knob value={masterLevel()} label="Level" color="#ffffff" size={36}
          onChange={(v) => setMasterLevelValue(v)} />
        <Knob value={sidechainAmount()} label="SC" color="#ffffff" size={32}
          onChange={(v) => setSidechainValue(v)} />
      </div>
      <div class="rw-master-divider" />
      <span class="rw-master-label">MIX</span>
      <div class="rw-master-knobs">
        {MIX_CHANNELS.map(({ id, label, color }) => (
          <Knob value={moduleLevels[id]} label={label} color={color} size={28}
            onChange={(v) => setModuleLevel(id, v)} />
        ))}
      </div>
    </div>
  );
}
