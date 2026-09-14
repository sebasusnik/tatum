import Knob from "./Knob";
import {
  masterLevel, setMasterLevelValue,
  fxParams, setFxParam,
} from "../stores/synth";

// Convert 0-100 knob value to dB display: 50 = 0dB, 0 = -12dB, 100 = +12dB
function toDB(v: number): string {
  const db = ((v - 50) / 50) * 12;
  return db >= 0 ? `+${db.toFixed(1)}` : db.toFixed(1);
}

export default function MasterPanel() {
  return (
    <div class="master-panel">
      <div class="master-panel-title">MASTER</div>

      {/* Master volume */}
      <div class="master-panel-vol">
        <div class="master-panel-db">{toDB(masterLevel())}</div>
        <input
          type="range"
          class="master-panel-slider"
          min="0" max="100"
          value={masterLevel()}
          onInput={(e) => setMasterLevelValue(parseInt(e.currentTarget.value))}
        />
      </div>

      <div class="master-panel-sep" />

      {/* EQ */}
      <div class="master-panel-section">
        <div class="master-panel-label">EQ</div>
        <div class="master-panel-eq-row">
          <span class="master-panel-eq-name">Low</span>
          <input type="range" class="master-panel-eq" min="0" max="100" value={fxParams.eqLow}
            onInput={(e) => setFxParam("eqLow", parseInt(e.currentTarget.value))} />
          <span class="master-panel-eq-val">{toDB(fxParams.eqLow)}</span>
        </div>
        <div class="master-panel-eq-row">
          <span class="master-panel-eq-name">Mid</span>
          <input type="range" class="master-panel-eq" min="0" max="100" value={fxParams.eqMid}
            onInput={(e) => setFxParam("eqMid", parseInt(e.currentTarget.value))} />
          <span class="master-panel-eq-val">{toDB(fxParams.eqMid)}</span>
        </div>
        <div class="master-panel-eq-row">
          <span class="master-panel-eq-name">High</span>
          <input type="range" class="master-panel-eq" min="0" max="100" value={fxParams.eqHigh}
            onInput={(e) => setFxParam("eqHigh", parseInt(e.currentTarget.value))} />
          <span class="master-panel-eq-val">{toDB(fxParams.eqHigh)}</span>
        </div>
      </div>

      <div class="master-panel-sep" />

      {/* Compressor */}
      <div class="master-panel-section">
        <div class="master-panel-label">COMP</div>
        <div class="master-panel-knobs">
          <Knob value={fxParams.compThresh} label="Thr" color="#ff6b35" size={28}
            onChange={(v) => setFxParam("compThresh", v)} />
          <Knob value={fxParams.compRatio} label="Rat" color="#ff6b35" size={28}
            onChange={(v) => setFxParam("compRatio", v)} />
          <Knob value={fxParams.compAttack} label="Atk" color="#ff6b35" size={28}
            onChange={(v) => setFxParam("compAttack", v)} />
          <Knob value={fxParams.compRelease} label="Rel" color="#ff6b35" size={28}
            onChange={(v) => setFxParam("compRelease", v)} />
        </div>
      </div>

      <div class="master-panel-sep" />

      {/* Output VU */}
      <div class="master-panel-section">
        <div class="master-panel-label">OUTPUT</div>
        <div class="master-panel-vu">
          <div class="master-panel-vu-bar"><div class="master-panel-vu-fill" /></div>
          <div class="master-panel-vu-bar"><div class="master-panel-vu-fill" /></div>
        </div>
        <div class="master-panel-vu-tags"><span>L</span><span>R</span></div>
      </div>
    </div>
  );
}
