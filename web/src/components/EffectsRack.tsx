import Knob from "./Knob";
import { fxParams, setFxParam, modules, setModuleParam } from "../stores/synth";

// ── Props for per-instance usage ────────────────────

interface InstanceProps {
  color?: string;
  params?: Record<string, number>; // 0-100 values keyed by param name
  onParamChange?: (key: string, paramIdx: number, value: number) => void;
}

// ── Delay ───────────────────────────────────────────

export function DelayBlock(props?: InstanceProps) {
  const color = props?.color ?? "#5b8cff";
  const v = (key: string, globalKey: keyof typeof fxParams) =>
    props?.params?.[key] ?? fxParams[globalKey];
  const set = (key: string, idx: number, globalKey: keyof typeof fxParams, val: number) => {
    if (props?.onParamChange) props.onParamChange(key, idx, val);
    else setFxParam(globalKey, val);
  };

  return (
    <div class="rw-fx-block" data-fx="delay">
      <span class="rw-fx-label" style={{ color }}>DELAY</span>
      <div class="rw-fx-knobs">
        <Knob value={v("feedback", "delayFeedback")} label="Fdbk" color={color} size={32}
          onChange={(val) => set("feedback", 0, "delayFeedback", val)} />
        <Knob value={v("mix", "delayMix")} label="Mix" color={color} size={32}
          onChange={(val) => set("mix", 1, "delayMix", val)} />
        <Knob value={v("filter", "delayFilter")} label="Filt" color={color} size={32}
          onChange={(val) => set("filter", 2, "delayFilter", val)} />
      </div>
      <div class="rw-fx-knobs">
        <Knob value={v("lfoRate", "delayLfoRate")} label="LRate" color={color} size={26}
          onChange={(val) => set("lfoRate", 3, "delayLfoRate", val)} />
        <Knob value={v("lfoDepth", "delayLfoDepth")} label="LDpt" color={color} size={26}
          onChange={(val) => set("lfoDepth", 4, "delayLfoDepth", val)} />
      </div>
    </div>
  );
}

// ── Reverb ──────────────────────────────────────────

export function ReverbBlock(props?: InstanceProps) {
  const color = props?.color ?? "#b07aff";
  const v = (key: string, globalKey: keyof typeof fxParams) =>
    props?.params?.[key] ?? fxParams[globalKey];
  const set = (key: string, idx: number, globalKey: keyof typeof fxParams, val: number) => {
    if (props?.onParamChange) props.onParamChange(key, idx, val);
    else setFxParam(globalKey, val);
  };

  return (
    <div class="rw-fx-block" data-fx="reverb">
      <span class="rw-fx-label" style={{ color }}>REVERB</span>
      <div class="rw-fx-knobs">
        <Knob value={v("size", "reverbSize")} label="Size" color={color} size={32}
          onChange={(val) => set("size", 0, "reverbSize", val)} />
        <Knob value={v("damping", "reverbDamp")} label="Damp" color={color} size={32}
          onChange={(val) => set("damping", 1, "reverbDamp", val)} />
        <Knob value={v("mix", "reverbMix")} label="Mix" color={color} size={32}
          onChange={(val) => set("mix", 2, "reverbMix", val)} />
      </div>
      <div class="rw-fx-knobs">
        <Knob value={v("preDelay", "reverbPreDelay")} label="Pre" color={color} size={26}
          onChange={(val) => set("preDelay", 3, "reverbPreDelay", val)} />
      </div>
    </div>
  );
}

// ── 3-Band EQ ───────────────────────────────────────

export function EqBlock(props?: InstanceProps) {
  const color = props?.color ?? "#3dd68c";
  const v = (key: string, globalKey: keyof typeof fxParams) =>
    props?.params?.[key] ?? fxParams[globalKey];
  const set = (key: string, idx: number, globalKey: keyof typeof fxParams, val: number) => {
    if (props?.onParamChange) props.onParamChange(key, idx, val);
    else setFxParam(globalKey, val);
  };

  return (
    <div class="rw-fx-block" data-fx="eq">
      <span class="rw-fx-label" style={{ color }}>EQ</span>
      <div class="rw-fx-knobs">
        <Knob value={v("low", "eqLow")} label="Low" color={color} size={32}
          onChange={(val) => set("low", 0, "eqLow", val)} />
        <Knob value={v("mid", "eqMid")} label="Mid" color={color} size={32}
          onChange={(val) => set("mid", 1, "eqMid", val)} />
        <Knob value={v("high", "eqHigh")} label="High" color={color} size={32}
          onChange={(val) => set("high", 2, "eqHigh", val)} />
      </div>
    </div>
  );
}

// ── Compressor ──────────────────────────────────────

export function CompBlock(props?: InstanceProps) {
  const color = props?.color ?? "#ff8c42";
  const v = (key: string, globalKey: keyof typeof fxParams) =>
    props?.params?.[key] ?? fxParams[globalKey];
  const set = (key: string, idx: number, globalKey: keyof typeof fxParams, val: number) => {
    if (props?.onParamChange) props.onParamChange(key, idx, val);
    else setFxParam(globalKey, val);
  };

  return (
    <div class="rw-fx-block" data-fx="comp">
      <span class="rw-fx-label" style={{ color }}>COMP</span>
      <div class="rw-fx-knobs">
        <Knob value={v("threshold", "compThresh")} label="Thr" color={color} size={32}
          onChange={(val) => set("threshold", 0, "compThresh", val)} />
        <Knob value={v("ratio", "compRatio")} label="Rat" color={color} size={32}
          onChange={(val) => set("ratio", 1, "compRatio", val)} />
        <Knob value={v("attack", "compAttack")} label="Atk" color={color} size={32}
          onChange={(val) => set("attack", 2, "compAttack", val)} />
        <Knob value={v("release", "compRelease")} label="Rel" color={color} size={32}
          onChange={(val) => set("release", 3, "compRelease", val)} />
      </div>
    </div>
  );
}

// ── Filter (insert FX only) ─────────────────────────

export function FilterBlock(props?: InstanceProps) {
  const color = props?.color ?? "#e06cff";
  const v = (key: string) => props?.params?.[key] ?? 50;
  const set = (key: string, idx: number, val: number) =>
    props?.onParamChange?.(key, idx, val);

  return (
    <div class="rw-fx-block" data-fx="filter">
      <span class="rw-fx-label" style={{ color }}>FILTER</span>
      <div class="rw-fx-knobs">
        <Knob value={v("cutoff")} label="Cut" color={color} size={32}
          onChange={(val) => set("cutoff", 0, val)} />
        <Knob value={v("resonance")} label="Res" color={color} size={32}
          onChange={(val) => set("resonance", 1, val)} />
      </div>
    </div>
  );
}

// ── Saturator (insert FX only) ──────────────────────

export function SaturatorBlock(props?: InstanceProps) {
  const color = props?.color ?? "#ff4444";
  const v = (key: string) => props?.params?.[key] ?? 50;
  const set = (key: string, idx: number, val: number) =>
    props?.onParamChange?.(key, idx, val);

  return (
    <div class="rw-fx-block" data-fx="saturator">
      <span class="rw-fx-label" style={{ color }}>SATURATOR</span>
      <div class="rw-fx-knobs">
        <Knob value={v("drive")} label="Drv" color={color} size={32}
          onChange={(val) => set("drive", 0, val)} />
        <Knob value={v("mix")} label="Mix" color={color} size={32}
          onChange={(val) => set("mix", 1, val)} />
      </div>
    </div>
  );
}

// ── Chorus (insert FX only) ─────────────────────────

export function ChorusBlock(props?: InstanceProps) {
  const color = props?.color ?? "#00bcd4";
  const v = (key: string) => props?.params?.[key] ?? 50;
  const set = (key: string, idx: number, val: number) =>
    props?.onParamChange?.(key, idx, val);

  return (
    <div class="rw-fx-block" data-fx="chorus">
      <span class="rw-fx-label" style={{ color }}>CHORUS</span>
      <div class="rw-fx-knobs">
        <Knob value={v("rate")} label="Rate" color={color} size={32}
          onChange={(val) => set("rate", 0, val)} />
        <Knob value={v("depth")} label="Dpth" color={color} size={32}
          onChange={(val) => set("depth", 1, val)} />
        <Knob value={v("mix")} label="Mix" color={color} size={32}
          onChange={(val) => set("mix", 2, val)} />
      </div>
    </div>
  );
}

// ── Tilt EQ (insert FX only) ────────────────────────

export function TiltEqBlock(props?: InstanceProps) {
  const color = props?.color ?? "#3dd68c";
  const v = (key: string, globalKey?: keyof typeof fxParams) =>
    props?.params?.[key] ?? (globalKey ? fxParams[globalKey] : 50);
  const set = (key: string, idx: number, globalKey: keyof typeof fxParams | null, val: number) => {
    if (props?.onParamChange) props.onParamChange(key, idx, val);
    else if (globalKey) setFxParam(globalKey, val);
  };

  return (
    <div class="rw-fx-block" data-fx="tilteq">
      <span class="rw-fx-label" style={{ color }}>TILT EQ</span>
      <div class="rw-fx-knobs">
        <Knob value={v("tilt", "eqTilt")} label="Tilt" color={color} size={32}
          onChange={(val) => set("tilt", 0, "eqTilt", val)} />
      </div>
    </div>
  );
}

// ── Limiter (insert FX only) ────────────────────────

export function LimiterBlock(props?: InstanceProps) {
  const color = props?.color ?? "#ffaa00";
  const v = (key: string) => props?.params?.[key] ?? 50;
  const set = (key: string, idx: number, val: number) =>
    props?.onParamChange?.(key, idx, val);

  return (
    <div class="rw-fx-block" data-fx="limiter">
      <span class="rw-fx-label" style={{ color }}>LIMITER</span>
      <div class="rw-fx-knobs">
        <Knob value={v("threshold")} label="Thr" color={color} size={32}
          onChange={(val) => set("threshold", 0, val)} />
        <Knob value={v("release")} label="Rel" color={color} size={32}
          onChange={(val) => set("release", 1, val)} />
      </div>
    </div>
  );
}

// ── Bitcrusher (insert FX only) ─────────────────────

export function BitcrusherBlock(props?: InstanceProps) {
  const color = props?.color ?? "#ff3366";
  const v = (key: string) => props?.params?.[key] ?? 80;
  const set = (key: string, idx: number, val: number) =>
    props?.onParamChange?.(key, idx, val);

  return (
    <div class="rw-fx-block" data-fx="bitcrusher">
      <span class="rw-fx-label" style={{ color }}>BITCRUSH</span>
      <div class="rw-fx-knobs">
        <Knob value={v("bits")} label="Bits" color={color} size={32}
          onChange={(val) => set("bits", 0, val)} />
        <Knob value={v("rate")} label="Rate" color={color} size={32}
          onChange={(val) => set("rate", 1, val)} />
      </div>
    </div>
  );
}

// ── Tape Stop (insert FX only) ──────────────────────

export function TapeStopBlock(props?: InstanceProps) {
  const color = props?.color ?? "#9966ff";
  const v = (key: string) => props?.params?.[key] ?? 50;
  const set = (key: string, idx: number, val: number) =>
    props?.onParamChange?.(key, idx, val);

  return (
    <div class="rw-fx-block" data-fx="tapestop">
      <span class="rw-fx-label" style={{ color }}>TAPE STOP</span>
      <div class="rw-fx-knobs">
        <Knob value={v("speed")} label="Spd" color={color} size={32}
          onChange={(val) => set("speed", 0, val)} />
      </div>
    </div>
  );
}

// ── Envelope Filter (legacy, bass-specific) ─────────

export function EnvFltBlock() {
  const color = "#e06cff";
  return (
    <div class="rw-fx-block" data-fx="envflt">
      <span class="rw-fx-label" style={{ color }}>ENV FLT</span>
      <div class="rw-fx-knobs">
        <Knob value={modules.bass.pages[0].values[0]} label="Cut" color={color} size={32}
          onChange={(v) => setModuleParam("bass", 0, 0, v)} />
        <Knob value={modules.bass.pages[0].values[2]} label="Env" color={color} size={32}
          onChange={(v) => setModuleParam("bass", 0, 2, v)} />
        <Knob value={modules.bass.pages[0].values[1]} label="Res" color={color} size={32}
          onChange={(v) => setModuleParam("bass", 0, 1, v)} />
      </div>
    </div>
  );
}

// Grouped rack (kept for non-canvas layouts)
export default function EffectsRack() {
  return (
    <div class="rw-fx-rack">
      <DelayBlock />
      <ReverbBlock />
      <EqBlock />
      <CompBlock />
      <EnvFltBlock />
    </div>
  );
}
