import { createSignal } from "solid-js";
import { createStore } from "solid-js/store";
import { SynthAudio } from "../audio/SynthAudio";
import type { DslError, DslResult } from "../audio/SynthAudio";

// ── FX types (mirrors core InsertFxType) ─────────────

export const FX_TYPE = {
  NONE: 0,
  FILTER: 1,
  SATURATOR: 2,
  CHORUS: 3,
  TILT_EQ: 4,
  COMPRESSOR: 5,
  DELAY: 6,
  REVERB: 7,
  LIMITER: 8,
  THREE_BAND_EQ: 9,
  BITCRUSHER: 10,
  TAPE_STOP: 11,
} as const;

export type FxTypeId = typeof FX_TYPE[keyof typeof FX_TYPE];

export const FX_LABELS: Record<FxTypeId, string> = {
  [FX_TYPE.NONE]: "None",
  [FX_TYPE.FILTER]: "Filter",
  [FX_TYPE.SATURATOR]: "Saturator",
  [FX_TYPE.CHORUS]: "Chorus",
  [FX_TYPE.TILT_EQ]: "Tilt EQ",
  [FX_TYPE.COMPRESSOR]: "Compressor",
  [FX_TYPE.DELAY]: "Delay",
  [FX_TYPE.REVERB]: "Reverb",
  [FX_TYPE.LIMITER]: "Limiter",
  [FX_TYPE.THREE_BAND_EQ]: "3-Band EQ",
  [FX_TYPE.BITCRUSHER]: "Bitcrusher",
  [FX_TYPE.TAPE_STOP]: "Tape Stop",
};

/** Pool sizes per FX type — matches Rust engine pool allocations */
const FX_POOL_SIZES: Record<FxTypeId, number> = {
  [FX_TYPE.NONE]: 0,
  [FX_TYPE.FILTER]: 12,
  [FX_TYPE.SATURATOR]: 8,
  [FX_TYPE.CHORUS]: 6,
  [FX_TYPE.TILT_EQ]: 8,
  [FX_TYPE.COMPRESSOR]: 8,
  [FX_TYPE.DELAY]: 4,
  [FX_TYPE.REVERB]: 4,
  [FX_TYPE.LIMITER]: 4,
  [FX_TYPE.THREE_BAND_EQ]: 4,
  [FX_TYPE.BITCRUSHER]: 6,
  [FX_TYPE.TAPE_STOP]: 4,
};

export const MAX_INSERT_FX = 4;
export const MAX_MASTER_FX = 6;

export interface InsertFxState {
  type: FxTypeId;
  instanceIdx: number;
  enabled: boolean;
  params: Record<string, number>; // named params per FX type, 0-1 normalized
}

export interface TrackFxState {
  insertFx: InsertFxState[];
}

export interface MasterFxState {
  level: number; // 0-100 UI
  insertFx: InsertFxState[];
}

function emptyFxSlot(): InsertFxState {
  return { type: FX_TYPE.NONE, instanceIdx: 0, enabled: false, params: {} };
}

// ── Types ────────────────────────────────────────────

export interface Page {
  keys: string[];
  values: number[];
}

export interface ModuleConfig {
  color: string;
  label: string;
  sub: string;
  pages: Page[];
}

export interface ParamLockEntry {
  paramId: number;
  value: number; // 0.0-1.0 normalized
}

export interface MelodicPattern {
  notes: string[];
  on: boolean[];
  velocities: number[];    // 0.0-1.0 per step
  gates: number[];         // 0.0-1.0 per step
  locks: ParamLockEntry[][]; // per step: array of param locks
  slides: boolean[];
  lockSlides: boolean[];
}

export interface DrumLane {
  name: string;
  steps: boolean[];
  velocities: number[];    // 0.0-1.0 per step
}

export type ModuleId = "bass" | "keys" | "fm" | "beats" | "arp";
export type MelodicId = "bass" | "keys" | "fm";
type PageModuleId = "bass" | "keys" | "fm" | "beats";

/** "arp" has no knob pages; everything that indexes `modules` narrows to this. */
function pageModule(id: ModuleId): PageModuleId | null {
  return id === "arp" ? null : id;
}

// ── Module configs (knob UI) ─────────────────────────

const MODULES: Record<PageModuleId, ModuleConfig> = {
  bass: {
    color: "#ff6b35",
    label: "BASS",
    sub: "3 OSC \u00b7 LADDER 4P",
    pages: [
      // Cutoff=8 (low resting ~80Hz), Reso=75 (quacky wah), EnvMod=70 (big wah sweep), Glide=15
      { keys: ["Cutoff", "Reso", "Env Mod", "Glide"], values: [8, 75, 70, 15] },
      // Attack=0 (snappy), Decay=10 (default 0.2s), Sustain=80 (default!), Release=8 (0.15s)
      { keys: ["Attack", "Decay", "Sustain", "Release"], values: [0, 10, 80, 8] },
      // Osc2=25 (sub octave), Osc3=50 (center), Wave=0 (saw), Keytrack=50
      { keys: ["Osc2", "Osc3", "Wave", "Keytrk"], values: [25, 50, 0, 50] },
      { keys: ["LFO Spd", "LFO Dpt", "LFO Wav", "LFO Tgt"], values: [40, 0, 0, 0] },
    ],
  },
  keys: {
    color: "#00c9b1",
    label: "KEYS",
    sub: "POLY \u00b7 2 SAW \u00b7 DETUNE",
    pages: [
      // Cutoff=15 (dark pad), Detune=10, Chorus=20, Level=70
      { keys: ["Cutoff", "Detune", "Chorus", "Level"], values: [15, 10, 20, 70] },
      // A=15(0.3s), D=25(0.5s), S=70, R=25(1.0s) — log scaled
      { keys: ["Attack", "Decay", "Sustain", "Release"], values: [15, 25, 70, 25] },
      // Voice=25 (unison), Vibrato=10
      { keys: ["LFO Spd", "LFO Dpt", "Voice", "Vibrato"], values: [30, 20, 25, 10] },
      // Resonance=20 (matches hardcoded 0.2)
      { keys: ["Reso"], values: [20] },
    ],
  },
  fm: {
    color: "#ffd23f",
    label: "FM",
    sub: "ALG 2 \u00b7 4 OP \u00b7 CLAVINET",
    pages: [
      // Algo=29 (algo 2), ModIdx=35 (~1.4), Feedback=20 (~0.14), Level=65
      { keys: ["Algo", "Mod Idx", "Feedback", "Level"], values: [29, 35, 20, 65] },
      // Ultra-snappy percussive envelope
      { keys: ["Attack", "Decay", "Sustain", "Release"], values: [1, 6, 5, 4] },
      { keys: ["LFO Spd", "LFO Dpt", "Wave", "Chorus"], values: [35, 0, 0, 15] },
      // Per-operator pages: Ratio, Feedback, Attack, Decay
      { keys: ["Ratio", "Fdbk", "Attack", "Decay"], values: [50, 0, 0, 30] },
      { keys: ["Ratio", "Fdbk", "Attack", "Decay"], values: [50, 0, 0, 30] },
      { keys: ["Ratio", "Fdbk", "Attack", "Decay"], values: [50, 0, 0, 30] },
      { keys: ["Ratio", "Fdbk", "Attack", "Decay"], values: [50, 0, 0, 30] },
    ],
  },
  beats: {
    color: "#ff5ea0",
    label: "BEATS",
    sub: "KICK \u00b7 SNARE \u00b7 HAT \u00b7 CLAP",
    pages: [
      {
        keys: ["Kick Dcy", "Snare", "Hat Dcy", "Clap"],
        values: [55, 40, 30, 50],
      },
      {
        keys: ["Kick Pit", "Snr Pit", "Hat Pit", "Click"],
        values: [50, 50, 50, 20],
      },
      { keys: ["Swing", "Prob", "Stutter", "Human"], values: [50, 100, 0, 10] },
    ],
  },
};

// ── Default melodic patterns ─────────────────────────

const DEFAULT_PATTERNS: Record<MelodicId, MelodicPattern> = {
  // Syncopated funk bass — A0(21), G0(19), C1(24). Ghost notes create the pocket.
  // Matching test_funk_envelope_filter velocities, gates, and param locks.
  bass: {
    notes:      ["A0","A0","A0","A0","A0","G0","G0","A0","A0","A0","A0","A0","C1","C1","A0","A0"],
    on:         [true,false,false,true,false,true,false,false,true,false,true,false,true,false,true,false],
    velocities: [0.90, 0.0, 0.0, 0.55, 0.0, 0.70, 0.0, 0.0, 0.80, 0.0, 0.40, 0.0, 0.50, 0.0, 0.65, 0.0],
    gates:      [0.40, 0.0, 0.0, 0.25, 0.0, 0.30, 0.0, 0.0, 0.35, 0.0, 0.20, 0.0, 0.25, 0.0, 0.30, 0.0],
    // Per-step CutoffEnv (param 1, exp scaled) and Resonance (param 2) for wah variation
    locks: [
      [{paramId:1,value:0.75},{paramId:2,value:0.80}], // 0: deep wah
      [], // 1: rest
      [], // 2: rest
      [{paramId:1,value:0.50},{paramId:2,value:0.65}], // 3: lighter ghost
      [], // 4: rest (the pocket)
      [{paramId:1,value:0.65},{paramId:2,value:0.75}], // 5: chromatic approach
      [], [], // 6-7: rest
      [{paramId:1,value:0.70},{paramId:2,value:0.78}], // 8: beat 3
      [], // 9: rest
      [{paramId:1,value:0.40},{paramId:2,value:0.55}], // 10: subtle ghost
      [], // 11: rest
      [{paramId:1,value:0.55},{paramId:2,value:0.70}], // 12: minor third
      [], // 13: rest
      [{paramId:1,value:0.60},{paramId:2,value:0.72}], // 14: pickup
      [], // 15: rest
    ],
    slides: Array(16).fill(false),
    lockSlides: Array(16).fill(false),
  },
  // Keys: off by default (pad enters later in a performance)
  keys: {
    notes:      ["A3","A3","C4","C4","E4","E4","G4","G4","A3","A3","C4","C4","E4","E4","G4","G4"],
    on:         Array(16).fill(false),
    velocities: Array(16).fill(0.5),
    gates:      Array(16).fill(0.75),
    locks:      Array.from({length:16}, () => []),
    slides:     Array(16).fill(false),
    lockSlides: Array(16).fill(false),
  },
  // FM clavinet stabs — Am7 chord tones, varied velocity for expression
  fm: {
    notes:      ["A3","A3","C4","C4","E4","E4","A3","A3","G4","G4","E4","E4","C4","C4","A3","A3"],
    on:         [false,false,true,false,false,true,true,false,false,true,false,true,false,true,false,true],
    velocities: [0.0, 0.0, 0.50, 0.0, 0.0, 0.55, 0.35, 0.0, 0.0, 0.50, 0.0, 0.40, 0.0, 0.55, 0.0, 0.30],
    gates:      [0.0, 0.0, 0.30, 0.0, 0.0, 0.30, 0.20, 0.0, 0.0, 0.30, 0.0, 0.25, 0.0, 0.30, 0.0, 0.20],
    locks:      Array.from({length:16}, () => []),
    slides:     Array(16).fill(false),
    lockSlides: Array(16).fill(false),
  },
};

// ── Default drum lanes ───────────────────────────────

// Funk drum pattern matching test_funk_envelope_filter
// Per-step velocity for dynamics (ghost notes, accents)
const DEFAULT_DRUM_LANES: DrumLane[] = [
  // Kick: beat 1, and-of-3 (step 7), beat 3 (step 8)
  { name: "kick",  steps: [true,false,false,false,false,false,false,true,true,false,false,false,false,false,false,false],
    velocities: [0.95,0,0,0,0,0,0,0.70,0.85,0,0,0,0,0,0,0] },
  // Snare: beat 2 (4), beat 4 (12) — no ghost in variation 0
  { name: "snare", steps: [false,false,false,false,true,false,false,false,false,false,false,false,true,false,false,false],
    velocities: [0,0,0,0,0.85,0,0,0,0,0,0,0,0.80,0,0,0] },
  // Closed hihat: only on steps without kick/snare/open-hat
  { name: "hihat",
    steps:      [false,true,true,true,false,true,false,false,false,true,true,true,false,true,false,true],
    velocities: [0,0.35,0.25,0.35,0,0.35,0,0,0,0.35,0.35,0.30,0,0.25,0,0.35] },
  // Clap: none in variation 0
  { name: "clap",  steps: Array(16).fill(false), velocities: Array(16).fill(0) },
  // Open hat (MIDI 46): steps 6, 14
  { name: "o-hat",
    steps:      [false,false,false,false,false,false,true,false,false,false,false,false,false,false,true,false],
    velocities: [0,0,0,0,0,0,0.45,0,0,0,0,0,0,0,0.40,0] },
  { name: "crash", steps: Array(16).fill(false), velocities: Array(16).fill(0.7) },
];

// ── Signals ──────────────────────────────────────────

export const [selectedModule, setSelectedModule] =
  createSignal<ModuleId>("bass");
export const [currentPage, setCurrentPage] = createSignal(0);
export const [playing, setPlaying] = createSignal(false);
export const [currentStep, setCurrentStep] = createSignal(-1);
export const [bpm, setBpm] = createSignal(112); // funk tempo
export const [recording, setRecording] = createSignal(false);
export const [activePattern, setActivePattern] = createSignal(0);

// Harmony
export const [harmonyRoot, setHarmonyRoot] = createSignal(9); // A (0=C..9=A..11=B)
export const [harmonyScale, setHarmonyScale] = createSignal(1); // 0=major,1=minor,...
export const [harmonyDegree, setHarmonyDegree] = createSignal(0); // 0-6

// Per-module send levels (0.0–1.0)
export const [sends, setSends] = createSignal<Record<ModuleId, { delay: number; reverb: number }>>({
  bass: { delay: 0, reverb: 0 },
  keys: { delay: 0, reverb: 0 },
  fm: { delay: 0, reverb: 0 },
  beats: { delay: 0, reverb: 0 },
  arp: { delay: 0, reverb: 0 },
});

// Mute / Solo per module (indexed by ModuleId)
export const [muted, setMuted] = createSignal<Record<ModuleId, boolean>>({
  bass: false,
  keys: false,
  fm: false,
  beats: false,
  arp: false,
});
export const [soloed, setSoloed] = createSignal<Record<ModuleId, boolean>>({
  bass: false,
  keys: false,
  fm: false,
  beats: false,
  arp: false,
});

// ── Arp params (0-100 UI range) ──────────────────────

export const [arpParams, setArpParamsStore] = createStore({
  rate: 50,
  gate: 60,
  pattern: 0,
  level: 30,
  waveform: 0,     // 0=Saw (default)
  attack: 0,       // → 0.001s (log: 0.001 * 2000^0)
  decay: 50,       // → ~0.045s
  sustain: 30,     // direct
  release: 40,     // → ~0.02s
  filterCutoff: 50, // → ~632 Hz (exp: 20 * 1000^0.5)
  filterResonance: 30,
  octaveRange: 33,  // → 2 oct (1 + 0.33*3 ≈ 2)
});

// ── Effects params (0-100 UI range) ──────────────────

export interface FxState {
  delayFeedback: number;
  delayMix: number;
  delayFilter: number;
  delayLfoRate: number;
  delayLfoDepth: number;
  reverbSize: number;
  reverbDamp: number;
  reverbMix: number;
  reverbPreDelay: number;
  eqLow: number;
  eqMid: number;
  eqHigh: number;
  eqTilt: number;
  compThresh: number;
  compRatio: number;
  compAttack: number;
  compRelease: number;
}

export const [fxParams, setFxParamsStore] = createStore<FxState>({
  delayFeedback: 25,
  delayMix: 10,
  delayFilter: 60,
  delayLfoRate: 0,
  delayLfoDepth: 0,
  reverbSize: 30,
  reverbDamp: 60,
  reverbMix: 10,
  reverbPreDelay: 15,   // ~15ms pre-delay
  eqLow: 67,            // → +4.0 dB
  eqMid: 58,            // → +2.0 dB
  eqHigh: 54,           // → +1.0 dB
  eqTilt: 50,           // centered (no tilt)
  compThresh: 23,        // → -9.2 dB
  compRatio: 21,         // → 5.0:1
  compAttack: 8,         // → 8.1 ms
  compRelease: 14,       // → 78.6 ms
});

// ── Module levels (0-100 UI) ─────────────────────────

export const [moduleLevels, setModuleLevelsStore] = createStore<Record<ModuleId, number>>({
  bass: 80,
  keys: 70,
  fm: 65,
  beats: 80,
  arp: 30,
});

// ── Master / Limiter / Sidechain (0-100 UI) ──────────

export const [masterLevel, setMasterLevelStore] = createSignal(80);
export const [limiterThreshold, setLimiterThresholdStore] = createSignal(50);
export const [sidechainAmount, setSidechainStore] = createSignal(40);

// ── Track + Master FX state ──────────────────────────

// Default master chain matches Rust: TiltEQ(0) → ThreeBandEQ(0) → Compressor(0) → Limiter(0)
export const [tracksFx, setTracksFx] = createStore<TrackFxState[]>([
  { insertFx: Array.from({ length: MAX_INSERT_FX }, emptyFxSlot) }, // track 0: bass
  { insertFx: Array.from({ length: MAX_INSERT_FX }, emptyFxSlot) }, // track 1: keys
  { insertFx: Array.from({ length: MAX_INSERT_FX }, emptyFxSlot) }, // track 2: fm
  { insertFx: Array.from({ length: MAX_INSERT_FX }, emptyFxSlot) }, // track 3: beats
  { insertFx: Array.from({ length: MAX_INSERT_FX }, emptyFxSlot) }, // track 4: spare
  { insertFx: Array.from({ length: MAX_INSERT_FX }, emptyFxSlot) }, // track 5: spare
]);

export const [masterFx, setMasterFx] = createStore<MasterFxState>({
  level: 80,
  insertFx: [
    { type: FX_TYPE.TILT_EQ, instanceIdx: 0, enabled: true, params: { tilt: 0.5 } },
    { type: FX_TYPE.THREE_BAND_EQ, instanceIdx: 0, enabled: true, params: { low: 0.5, mid: 0.5, high: 0.5 } },
    { type: FX_TYPE.COMPRESSOR, instanceIdx: 0, enabled: true, params: { threshold: 0.23, ratio: 0.21, attack: 0.08, release: 0.14 } },
    { type: FX_TYPE.LIMITER, instanceIdx: 0, enabled: true, params: { threshold: 0.5, release: 0.1, makeup: 0.5 } },
    emptyFxSlot(),
    emptyFxSlot(),
  ],
});

// ── Stores ───────────────────────────────────────────

export const [modules, setModules] = createStore(MODULES);
export const [patterns, setPatterns] = createStore(DEFAULT_PATTERNS);
export const [drumLanes, setDrumLanes] = createStore(DEFAULT_DRUM_LANES);

// ── Derived (kept for backward compat with knob UI) ──

// Alias: currentModule still works for components that import it
export const currentModule = selectedModule;
export const setCurrentModule = setSelectedModule;

export const mod = () => modules[pageModule(selectedModule()) ?? "bass"];
export const page = () => mod().pages[currentPage()];
export const accentColor = () => mod().color;
export const pageCount = () => mod().pages.length;

// Pattern data for the currently selected module (melodic or drum)
// Provides backward-compatible .on / .notes / .slides / .locks / .lockSlides for Sequencer
export const modPattern = () => {
  const m = selectedModule();
  if (m === "beats") {
    // For beats, flatten drum lanes into a step-on array (lane 0 = kick as primary view)
    return {
      notes: drumLanes.map((dl) => dl.name.slice(0, 2).toUpperCase()),
      on: drumLanes[0].steps as boolean[],
      velocities: Array(16).fill(0.8),
      gates: Array(16).fill(0.5),
      locks: Array.from({length:16}, () => []) as ParamLockEntry[][],
      slides: Array(16).fill(false) as boolean[],
      lockSlides: Array(16).fill(false) as boolean[],
    };
  }
  return patterns[m as MelodicId];
};

// ── DSL source (single source of truth) ─────────────

export const [dslSource, setDslSource] = createSignal("");
export const [dslErrors, setDslErrors] = createSignal<DslError[]>([]);

/** Track info from the loaded song */
export interface TrackInfo {
  name: string;
  kind: string;
  level: number;
  pan: number;
}
export const [trackInfos, setTrackInfos] = createSignal<TrackInfo[]>([]);

// ── DSL Module Instances (parsed from DSL text) ─────────────

export interface DslModuleInstance {
  kind: string;
  name: string;
  params: Record<string, number>;
}

// Key: "kind:name" e.g. "bass:warmth"
export const [dslModules, setDslModules] = createStore<Record<string, DslModuleInstance>>({});

// Track name → module name mapping (e.g. "bass" → "warmth")
export const [trackModuleMap, setTrackModuleMap] = createSignal<Record<string, string>>({});

// ── Parsed patterns from DSL ────────────────────────

/** A single step in a pattern — velocity 0 = rest, >0 = note on */
export interface ParsedStep {
  velocity: number;      // 0 = rest/tie placeholder, >0 = note on
  noteCount: number;     // 1 = mono, >1 = chord (polyphonic)
  isTie: boolean;        // ".." continuation
}

export interface ParsedDrumLane {
  name: string;          // "kick", "snare", "hat", etc.
  steps: number[];       // velocity per step (0 = rest)
}

export interface ParsedPattern {
  type: "melodic" | "drums";
  steps: ParsedStep[];       // for melodic patterns (16 steps)
  drumLanes: ParsedDrumLane[]; // for drum patterns
}

// Pattern name → parsed data
export const [dslPatterns, setDslPatterns] = createSignal<Record<string, ParsedPattern>>({});

// ── Parsed scenes from DSL ──────────────────────────

export interface ParsedSceneTrack {
  name: string;
  pattern: string;
}

export interface ParsedScene {
  name: string;
  tracks: ParsedSceneTrack[];
}

export const [dslScenes, setDslScenes] = createSignal<ParsedScene[]>([]);

// Track name → current pattern name (from active scene)
export const [trackPatternMap, setTrackPatternMap] = createSignal<Record<string, string>>({});

// Currently active scene name
export const [activeSceneName, setActiveSceneName] = createSignal("");

/** Get the parsed pattern for a given track (based on current scene) */
export function getPatternForTrack(trackName: string): ParsedPattern | null {
  const patName = trackPatternMap()[trackName];
  if (!patName) return null;
  return dslPatterns()[patName] ?? null;
}

// ── DSL param name → [pageIdx, knobIdx] mappings ────────────

const BASS_PARAM_MAP: Record<string, [number, number]> = {
  cutoff: [0, 0], resonance: [0, 1], cutoff_env: [0, 2], glide: [0, 3],
  attack: [1, 0], decay: [1, 1], sustain: [1, 2], release: [1, 3],
  osc2_pitch: [2, 0], osc1_wave: [2, 2], keytrack: [2, 3],
  lfo_rate: [3, 0], lfo_depth: [3, 1], lfo_waveform: [3, 2], lfo_target: [3, 3],
};

const KEYS_PARAM_MAP: Record<string, [number, number]> = {
  cutoff: [0, 0], detune: [0, 1], chorus_mix: [0, 2],
  attack: [1, 0], decay: [1, 1], sustain: [1, 2], release: [1, 3],
  lfo_rate: [2, 0], lfo_depth: [2, 1], voice_mode: [2, 2],
  resonance: [3, 0],
};

const FM_PARAM_MAP: Record<string, [number, number]> = {
  algorithm: [0, 0], mod_index: [0, 1], feedback: [0, 2],
  attack: [1, 0], decay: [1, 1], sustain: [1, 2], release: [1, 3],
  lfo_rate: [2, 0], lfo_depth: [2, 1], waveform: [2, 2], chorus_mix: [2, 3],
  op0_ratio: [3, 0], op0_feedback: [3, 1],
  op1_ratio: [4, 0], op1_feedback: [4, 1],
  op2_ratio: [5, 0], op2_feedback: [5, 1],
  op3_ratio: [6, 0], op3_feedback: [6, 1],
};

const BEATS_PARAM_MAP: Record<string, [number, number]> = {
  kick_decay: [0, 0], snare_decay: [0, 1], hihat_decay: [0, 2],
  kick_pitch: [1, 0], snare_pitch: [1, 1], hihat_pitch: [1, 2], kick_click: [1, 3],
  kick_level: [0, 0], snare_level: [0, 1], hihat_level: [0, 2], clap_level: [0, 3],
  kick_drive: [1, 0], snare_drive: [1, 1], snare_snap: [1, 2],
};

const PARAM_MAPS: Record<string, Record<string, [number, number]>> = {
  bass: BASS_PARAM_MAP, keys: KEYS_PARAM_MAP, fm: FM_PARAM_MAP, beats: BEATS_PARAM_MAP,
};

/** Get module pages for a specific track, with DSL values overlaid on defaults */
export function getModulePagesForTrack(trackName: string): Page[] {
  const moduleName = trackModuleMap()[trackName];
  const tracks = trackInfos();
  const track = tracks.find(t => t.name === trackName);
  if (!track) return [];

  const kind = track.kind as PageModuleId;
  const defaults = MODULES[kind];
  if (!defaults) return [];

  // Deep clone default pages
  const pages: Page[] = defaults.pages.map(p => ({
    keys: [...p.keys],
    values: [...p.values],
  }));

  if (!moduleName) return pages;

  const dslMod = dslModules[`${kind}:${moduleName}`];
  if (!dslMod) return pages;

  const paramMap = PARAM_MAPS[kind];
  if (!paramMap) return pages;

  // Overlay DSL values (0.0-1.0 → 0-100 UI)
  for (const [paramName, value] of Object.entries(dslMod.params)) {
    const mapping = paramMap[paramName];
    if (mapping) {
      const [pageIdx, knobIdx] = mapping;
      if (pages[pageIdx] && knobIdx < pages[pageIdx].values.length) {
        pages[pageIdx].values[knobIdx] = Math.round(value * 100);
      }
    }
  }

  return pages;
}

/** Get the module name used by a track (e.g. "warmth" for track "bass") */
export function getModuleNameForTrack(trackName: string): string {
  return trackModuleMap()[trackName] ?? "";
}

// ── Audio Bridge ─────────────────────────────────────

let audio: SynthAudio | null = null;
let initPromise: Promise<void> | null = null;

async function initAudio(): Promise<void> {
  if (audio?.ready) return;
  if (initPromise) return initPromise;
  initPromise = (async () => {
    audio = new SynthAudio();
    await audio.init(
      (step) => setCurrentStep(step),
      (result: DslResult) => {
        if (result.ok) {
          setDslErrors([]);
          // Sync store signals from the loaded DSL
          syncStoreFromDsl(dslSource());
          // Request track info from the engine
          audio!.requestTrackInfo((tracks) => {
            setTrackInfos(tracks.map((t) => ({
              name: t.name,
              kind: t.kind,
              level: t.level,
              pan: t.pan,
            })));
          });
        } else {
          setDslErrors(result.errors ?? []);
        }
      },
    );
  })();
  return initPromise;
}

/** Send .synth DSL source text for parsing/compilation/loading.
 *  Stores the source and sends to WASM for recompilation.
 *  The engine handles hot-swap internally (no audible glitch). */
export function sendSource(source: string): void {
  setDslSource(source);
  if (audio?.ready) {
    console.log("[synth] sendSource → loadSource (audio ready)");
    audio.loadSource(source);
  } else {
    console.log("[synth] sendSource → skipped (audio not ready)");
  }
}

/** Update the DSL text without recompiling.
 *  Used by mixer/transport persist — the real-time message already
 *  changed the engine's live state, we just sync the text. */
export function updateDslTextOnly(source: string): void {
  setDslSource(source);
}

// ── DSL name mappings ─────────────────────────────────

const ROOT_NAMES_DSL = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
const SCALE_DSL_NAMES = ["major", "minor", "dorian", "mixolydian", "pentatonic_minor"];

/** Persist a top-level DSL global (tempo, swing, etc.) into the source text.
 *  Only updates the text — does NOT recompile. */
export function persistGlobal(key: string, value: string): void {
  const source = dslSource();
  const re = new RegExp(`^(${key})\\s+.*$`, "m");
  const patched = source.replace(re, `$1 ${value}`);
  if (patched !== source) setDslSource(patched);
}

/** Persist a DSL global AND recompile (for structural changes like scale). */
function persistGlobalAndRecompile(key: string, value: string): void {
  const source = dslSource();
  const re = new RegExp(`^(${key})\\s+.*$`, "m");
  const patched = source.replace(re, `$1 ${value}`);
  if (patched !== source) {
    console.log(`[synth] persistGlobalAndRecompile: ${key} → ${value}, audio ready=${audio?.ready}`);
    sendSource(patched);
  } else {
    console.warn(`[synth] persistGlobalAndRecompile: regex did not match for key="${key}"`);
  }
}

/** Parse key state from DSL source text and sync store signals. */
export function syncStoreFromDsl(source: string): void {
  // Parse tempo
  const tempoMatch = source.match(/^tempo\s+(\d+)/m);
  if (tempoMatch) setBpm(parseInt(tempoMatch[1]));

  // Parse scale → root + mode
  const scaleMatch = source.match(/^scale\s+(\S+)\s+(\S+)/m);
  if (scaleMatch) {
    const rootIdx = ROOT_NAMES_DSL.indexOf(scaleMatch[1]);
    if (rootIdx >= 0) setHarmonyRoot(rootIdx);
    const scaleIdx = SCALE_DSL_NAMES.indexOf(scaleMatch[2]);
    if (scaleIdx >= 0) setHarmonyScale(scaleIdx);
  }

  // Parse module definitions: module <kind> <name> { ... }
  const parsedModules: Record<string, DslModuleInstance> = {};
  const moduleRe = /^module\s+(bass|keys|fm|beats|chord)\s+(\w+)\s*\{([^}]*)\}/gm;
  let mm;
  while ((mm = moduleRe.exec(source)) !== null) {
    const [, kind, name, body] = mm;
    const params: Record<string, number> = {};
    // Parse single-value params: "  paramName value"
    const paramRe = /^\s+(\w+)\s+([-\d.]+)$/gm;
    let pp;
    while ((pp = paramRe.exec(body)) !== null) {
      params[pp[1]] = parseFloat(pp[2]);
    }
    // Parse multi-value envelope params: "  op0_envelope 0.1 0.2 0.3 0.4"
    const envRe = /^\s+(op\d+_envelope)\s+([-\d.]+)\s+([-\d.]+)\s+([-\d.]+)\s+([-\d.]+)/gm;
    let ee;
    while ((ee = envRe.exec(body)) !== null) {
      const prefix = ee[1].replace("_envelope", "");
      params[`${prefix}_attack`] = parseFloat(ee[2]);
      params[`${prefix}_decay`] = parseFloat(ee[3]);
      params[`${prefix}_sustain`] = parseFloat(ee[4]);
      params[`${prefix}_release`] = parseFloat(ee[5]);
    }
    parsedModules[`${kind}:${name}`] = { kind, name, params };
  }
  setDslModules(parsedModules);

  // Parse track → module mappings: track <name> { ... using <moduleName> ... }
  const mapping: Record<string, string> = {};
  const trackRe = /^track\s+(\w+)\s*\{[^}]*?using\s+(\w+)/gm;
  let tt;
  while ((tt = trackRe.exec(source)) !== null) {
    mapping[tt[1]] = tt[2];
  }
  setTrackModuleMap(mapping);

  // Parse patterns
  const parsedPatterns: Record<string, ParsedPattern> = {};
  // Match pattern blocks — need to handle multiline bodies with nested braces
  const patternBlockRe = /^pattern\s+(\w+)\s*\{([^}]*)\}/gm;
  let pb;
  while ((pb = patternBlockRe.exec(source)) !== null) {
    const [, patName, body] = pb;
    const lines = body.split("\n").map(l => l.trim()).filter(l => l.length > 0);

    // Detect drum pattern: lines start with "laneName:"
    const isDrum = lines.some(l => /^\w+:/.test(l));

    if (isDrum) {
      const drumLanes: ParsedDrumLane[] = [];
      for (const line of lines) {
        const laneMatch = line.match(/^(\w+):\s+(.+)$/);
        if (!laneMatch) continue;
        const [, laneName, stepsStr] = laneMatch;
        const tokens = stepsStr.trim().split(/\s+/);
        const steps = tokens.map(tok => {
          if (tok === "-") return 0;
          if (tok === "X" || tok === "x") return 1.0;
          const vm = tok.match(/^[xX]:?([\d.]+)$/);
          if (vm) return parseFloat(vm[1]);
          // Ghost notes: g?0.30
          const gm = tok.match(/^g\?([\d.]+)$/);
          if (gm) return parseFloat(gm[1]);
          // Accent: o
          if (tok === "o") return 0.6;
          return 0;
        });
        drumLanes.push({ name: laneName, steps });
      }
      parsedPatterns[patName] = { type: "drums", steps: [], drumLanes };
    } else {
      // Melodic pattern — parse all tokens across all lines
      const allTokens: string[] = [];
      for (const line of lines) {
        allTokens.push(...line.split(/\s+/));
      }
      const steps: ParsedStep[] = allTokens.map(tok => {
        if (tok === "-") return { velocity: 0, noteCount: 1, isTie: false };
        if (tok === "..") return { velocity: 0, noteCount: 0, isTie: true };
        // Chord: [notes]:vel
        const chordMatch = tok.match(/^\[([^\]]+)\](?::([\d.]+))?$/);
        if (chordMatch) {
          const notes = chordMatch[1].trim().split(/\s+/);
          const vel = chordMatch[2] ? parseFloat(chordMatch[2]) : 0.8;
          return { velocity: vel, noteCount: notes.length, isTie: false };
        }
        // Single note: degree.octave:vel or NoteName:vel
        const noteMatch = tok.match(/^[^:]+(?::([\d.]+))?$/);
        if (noteMatch && tok !== "-") {
          const vel = noteMatch[1] ? parseFloat(noteMatch[1]) : 0.8;
          return { velocity: vel, noteCount: 1, isTie: false };
        }
        return { velocity: 0, noteCount: 1, isTie: false };
      });
      parsedPatterns[patName] = { type: "melodic", steps, drumLanes: [] };
    }
  }
  setDslPatterns(parsedPatterns);

  // Parse scenes
  const scenes: ParsedScene[] = [];
  const sceneRe = /^scene\s+(\w+)\s*\{([^}]*)\}/gm;
  let sc;
  while ((sc = sceneRe.exec(source)) !== null) {
    const [, sceneName, body] = sc;
    const tracks: ParsedSceneTrack[] = [];
    const stRe = /track\s+(\w+)\s*\{\s*play\s+(\w+)/g;
    let st;
    while ((st = stRe.exec(body)) !== null) {
      tracks.push({ name: st[1], pattern: st[2] });
    }
    scenes.push({ name: sceneName, tracks });
  }
  setDslScenes(scenes);

  // Set initial active scene to the first one, populate trackPatternMap
  if (scenes.length > 0) {
    setActiveSceneName(scenes[0].name);
    const patMap: Record<string, string> = {};
    for (const st of scenes[0].tracks) {
      patMap[st.name] = st.pattern;
    }
    setTrackPatternMap(patMap);
  }
}

/** Update a module param for a specific track: RT audio + DSL text patch (no recompile) */
export function setModuleParamForTrack(
  trackName: string,
  pageIdx: number,
  knobIdx: number,
  value: number,
): void {
  const clamped = Math.round(Math.min(100, Math.max(0, value)));
  const tracks = trackInfos();
  const trackIdx = tracks.findIndex(t => t.name === trackName);
  if (trackIdx < 0) return;

  const track = tracks[trackIdx];
  const paramMap = PARAM_MAPS[track.kind];
  if (!paramMap) return;

  // Reverse lookup: find DSL param name from page+knob indices
  const entry = Object.entries(paramMap).find(
    ([, [p, k]]) => p === pageIdx && k === knobIdx,
  );
  if (!entry) return;
  const paramName = entry[0];

  // RT message to engine (instant audio change)
  setModuleParamRT(trackIdx, paramName, clamped / 100);

  // Patch the DSL text (no recompile)
  const moduleName = trackModuleMap()[trackName];
  if (moduleName) {
    patchModuleParam(track.kind, moduleName, paramName, clamped / 100);
  }
}

/** Patch a single param value inside a module block in the DSL text */
function patchModuleParam(kind: string, moduleName: string, param: string, value: number): void {
  const source = dslSource();
  // Match the param line within the specific module block
  const modBlockRe = new RegExp(
    `(module\\s+${kind}\\s+${moduleName}\\s*\\{[^}]*?)\\b(${param})\\s+[-\\d.]+`,
    "m",
  );
  const formatted = value % 1 === 0 ? value.toFixed(1) : value.toFixed(4).replace(/0+$/, "").replace(/\.$/, ".0");
  const patched = source.replace(modBlockRe, `$1$2 ${formatted}`);
  if (patched !== source) {
    updateDslTextOnly(patched);
  }
}

/** Set a real-time track level (0.0-1.0) without recompiling. */
export function setTrackLevelRT(trackIdx: number, level: number): void {
  if (!audio) return;
  audio.setTrackLevel(trackIdx, level);
  // Update local track info
  setTrackInfos((prev) => prev.map((t, i) => i === trackIdx ? { ...t, level } : t));
}

/** Switch to a scene by name — updates trackPatternMap for the UI */
export function switchScene(sceneName: string): void {
  const scenes = dslScenes();
  const scene = scenes.find(s => s.name === sceneName);
  if (!scene) return;
  setActiveSceneName(sceneName);
  const patMap: Record<string, string> = {};
  for (const st of scene.tracks) {
    patMap[st.name] = st.pattern;
  }
  setTrackPatternMap(patMap);
}

/** Set tempo in real-time without recompiling. */
export function setTempoRT(bpm: number): void {
  if (!audio) return;
  audio.setTempo(bpm);
  setBpm(Math.round(bpm));
}

/** Set a track's pattern in real-time without recompiling. */
export function setTrackPatternRT(trackIdx: number, patternIdx: number): void {
  if (!audio) return;
  audio.setTrackPattern(trackIdx, patternIdx);
}

/** Set a module parameter in real-time without recompiling. */
export function setModuleParamRT(instIdx: number, paramName: string, value: number): void {
  if (!audio) return;
  audio.setModuleParam(instIdx, paramName, value);
}

/** Set a real-time track pan (-1.0 to 1.0) without recompiling. */
export function setTrackPanRT(trackIdx: number, pan: number): void {
  if (!audio) return;
  audio.setTrackPan(trackIdx, pan);
  setTrackInfos((prev) => prev.map((t, i) => i === trackIdx ? { ...t, pan } : t));
}

/**
 * Persist a track property into the DSL source text (called on fader release).
 * Only updates the text — does NOT recompile. The real-time message already
 * changed the engine's live state.
 */
export function persistTrackProp(trackName: string, prop: string, value: number): void {
  const source = dslSource();
  const patched = patchTrackProp(source, trackName, prop, value);
  if (patched !== source) {
    setDslSource(patched); // update text only, no recompile
  }
}

/** Patch a track property value in DSL source text. */
function patchTrackProp(source: string, trackName: string, prop: string, value: number): string {
  // Find the top-level track block
  const trackRe = new RegExp(`(^track\\s+${trackName}\\s*\\{[^}]*?)\\b${prop}\\s+[\\-\\d.]+`, "m");
  const formatted = prop === "pan" ? value.toFixed(2) : value.toFixed(2);
  const match = source.match(trackRe);
  if (match) {
    // Replace existing prop value
    return source.replace(trackRe, `$1${prop} ${formatted}`);
  }
  // Prop doesn't exist yet — insert it after the track opening
  const insertRe = new RegExp(`(^track\\s+${trackName}\\s*\\{\\s*\\n)`, "m");
  const insertMatch = source.match(insertRe);
  if (insertMatch) {
    return source.replace(insertRe, `$1    ${prop} ${formatted}\n`);
  }
  return source;
}

// syncEffectsToWasm, handleSpecialParam — removed (DSL-driven)

// ── Actions: Module selection ────────────────────────

export function switchModule(id: ModuleId): void {
  setSelectedModule(id);
  setCurrentPage(0);
}

export function nextPage(): void {
  setCurrentPage((p) => (p + 1) % pageCount());
}

export function prevPage(): void {
  setCurrentPage((p) => (p - 1 + pageCount()) % pageCount());
}

// ── Actions: Params ──────────────────────────────────

export function setParamValue(knobIdx: number, value: number): void {
  const m = pageModule(selectedModule());
  if (!m) return;
  const p = currentPage();
  const clamped = Math.round(Math.min(100, Math.max(0, value)));
  setModules(m, "pages", p, "values", knobIdx, clamped);
}

export function setRawParam(_paramId: number, _normalized: number): void {
  // No-op: params are now controlled through DSL source
}

// ── Actions: Arp params ──────────────────────────────

const ARP_PARAM_IDS: Record<string, number> = {
  rate: 128, gate: 129, pattern: 130, level: 131,
  waveform: 140, attack: 141, decay: 142, sustain: 143,
  release: 144, filterCutoff: 145, filterResonance: 146, octaveRange: 147,
};

export type ArpParamKey = keyof typeof ARP_PARAM_IDS;

export function setArpParam(key: ArpParamKey, value: number): void {
  const clamped = Math.round(Math.min(100, Math.max(0, value)));
  setArpParamsStore(key as keyof typeof arpParams, clamped);
}

// ── Actions: FX params ───────────────────────────────

export function setFxParam<K extends keyof FxState>(key: K, value: number): void {
  const clamped = Math.round(Math.min(100, Math.max(0, value)));
  setFxParamsStore(key, clamped);
}

// ── Actions: Module level / pan ──────────────────────

export function setModuleLevel(modId: ModuleId, value: number): void {
  const clamped = Math.round(Math.min(100, Math.max(0, value)));
  setModuleLevelsStore(modId, clamped);
}

export function setModulePan(_modId: ModuleId, _value: number): void {
  // No-op: pan is now controlled through DSL source
}

// ── Actions: Master / Limiter / Sidechain ────────────

export function setMasterLevelValue(value: number): void {
  const clamped = Math.round(Math.min(100, Math.max(0, value)));
  setMasterLevelStore(clamped);
}

export function setLimiterThresholdValue(value: number): void {
  const clamped = Math.round(Math.min(100, Math.max(0, value)));
  setLimiterThresholdStore(clamped);
}

export function setSidechainValue(value: number): void {
  const clamped = Math.round(Math.min(100, Math.max(0, value)));
  setSidechainStore(clamped);
}

/** Set a param on any module directly (for railway layout where all modules are visible). */
export function setModuleParam(modId: ModuleId, pageIdx: number, knobIdx: number, value: number): void {
  const m = pageModule(modId);
  if (!m) return;
  const clamped = Math.round(Math.min(100, Math.max(0, value)));
  setModules(m, "pages", pageIdx, "values", knobIdx, clamped);
}

// ── Actions: Melodic step toggle ─────────────────────

/** Toggle a step on a specific module (used by per-module InlineSeq blocks). */
export function toggleStepFor(modId: MelodicId, idx: number): void {
  setPatterns(modId, "on", idx, (v: boolean) => !v);
}

export function toggleStep(idx: number): void {
  const m = selectedModule();

  // If beats module is selected, delegate to drum lane toggle (lane 0 by default)
  if (m === "beats") return;

  toggleStepFor(m as MelodicId, idx);
}

export function setStepNote(
  modId: MelodicId,
  idx: number,
  noteName: string,
): void {
  setPatterns(modId, "notes", idx, noteName);
}

export function setStepSlide(
  modId: MelodicId,
  idx: number,
  slide: boolean,
): void {
  setPatterns(modId, "slides", idx, slide);
}

// ── Actions: Drum lane toggle ────────────────────────

export function toggleDrumStep(lane: number, step: number): void {
  setDrumLanes(lane, "steps", step, (v: boolean) => !v);
}

export function setDrumStepVelocity(lane: number, step: number, velocity: number): void {
  setDrumLanes(lane, "velocities", step, velocity);
}

export function clearDrumLane(lane: number): void {
  setDrumLanes(lane, "steps", Array(16).fill(false));
}

// ── Actions: Harmony ─────────────────────────────────

export function setHarmony(root: number, scale: number, degree: number): void {
  setHarmonyRoot(root);
  setHarmonyScale(scale);
  setHarmonyDegree(degree);
  // Patch DSL and recompile
  const rootName = ROOT_NAMES_DSL[root] ?? "C";
  const scaleName = SCALE_DSL_NAMES[scale] ?? "major";
  persistGlobalAndRecompile("scale", `${rootName} ${scaleName}`);
}

export function setRoot(root: number): void {
  setHarmonyRoot(root);
  // Patch DSL scale line and recompile (structural change — different notes)
  const scaleName = SCALE_DSL_NAMES[harmonyScale()] ?? "major";
  const rootName = ROOT_NAMES_DSL[root] ?? "C";
  persistGlobalAndRecompile("scale", `${rootName} ${scaleName}`);
}

export function setScale(scale: number): void {
  setHarmonyScale(scale);
  const rootName = ROOT_NAMES_DSL[harmonyRoot()] ?? "C";
  const scaleName = SCALE_DSL_NAMES[scale] ?? "major";
  persistGlobalAndRecompile("scale", `${rootName} ${scaleName}`);
}

export function setDegree(degree: number): void {
  setHarmonyDegree(degree);
  // Visual indicator only for now — degree-based patterns already use the scale
}

// ── Actions: Mute / Solo ─────────────────────────────

export function toggleMute(id: ModuleId): void {
  setMuted((prev) => ({ ...prev, [id]: !prev[id] }));
}

export function toggleSolo(id: ModuleId): void {
  setSoloed((prev) => ({ ...prev, [id]: !prev[id] }));
}

// ── Actions: Module Sends ────────────────────────────

export function setModuleSend(id: ModuleId, effect: "delay" | "reverb", amount: number): void {
  const clamped = Math.max(0, Math.min(1, amount));
  setSends((prev) => ({ ...prev, [id]: { ...prev[id], [effect]: clamped } }));
}

// ── Actions: Transport ───────────────────────────────

export async function startPlayback(): Promise<void> {
  await initAudio();
  // Send the current DSL source to WASM (first time or after edits while stopped)
  const source = dslSource();
  if (source && audio) {
    audio.loadSource(source);
  }
  audio!.start();
  setPlaying(true);
}

export function stopPlayback(): void {
  audio?.stop();
  setPlaying(false);
  setCurrentStep(-1);
}

export function togglePlayback(): void {
  if (playing()) stopPlayback();
  else startPlayback();
}

export function adjustBpm(delta: number): void {
  const newBpm = Math.round(Math.min(300, Math.max(40, bpm() + delta)));
  setBpm(newBpm);
  if (audio?.ready) {
    audio.setTempo(newBpm);
  }
}

/** Persist the current BPM value into the DSL text (called on drag release). */
export function persistBpm(): void {
  persistGlobal("tempo", String(bpm()));
}

// ── FX instance allocation ──────────────────────────

/** Find the next free pool instance for a given FX type across all tracks + master. */
function allocateFxInstance(fxType: FxTypeId): number | null {
  if (fxType === FX_TYPE.NONE) return null;
  const poolSize = FX_POOL_SIZES[fxType];
  const used = new Set<number>();

  // Collect used instances from all tracks
  for (const track of tracksFx) {
    for (const slot of track.insertFx) {
      if (slot.type === fxType) used.add(slot.instanceIdx);
    }
  }
  // Collect used instances from master
  for (const slot of masterFx.insertFx) {
    if (slot.type === fxType) used.add(slot.instanceIdx);
  }

  for (let i = 0; i < poolSize; i++) {
    if (!used.has(i)) return i;
  }
  return null; // pool exhausted
}

/** Get remaining instances available for a given FX type. */
export function fxPoolRemaining(fxType: FxTypeId): number {
  if (fxType === FX_TYPE.NONE) return 0;
  const poolSize = FX_POOL_SIZES[fxType];
  const used = new Set<number>();
  for (const track of tracksFx) {
    for (const slot of track.insertFx) {
      if (slot.type === fxType) used.add(slot.instanceIdx);
    }
  }
  for (const slot of masterFx.insertFx) {
    if (slot.type === fxType) used.add(slot.instanceIdx);
  }
  return poolSize - used.size;
}

// ── Actions: Track insert FX ────────────────────────

/** Add an FX to a track's insert chain. Returns false if pool is exhausted. */
export function addTrackInsertFx(track: number, slotIdx: number, fxType: FxTypeId): boolean {
  const instanceIdx = allocateFxInstance(fxType);
  if (instanceIdx === null) return false;
  setTracksFx(track, "insertFx", slotIdx, {
    type: fxType, instanceIdx, enabled: true, params: {},
  });
  return true;
}

export function clearTrackInsertFx(track: number, slotIdx: number): void {
  setTracksFx(track, "insertFx", slotIdx, emptyFxSlot());
}

export function toggleTrackInsertFx(track: number, slotIdx: number): void {
  const current = tracksFx[track].insertFx[slotIdx].enabled;
  setTracksFx(track, "insertFx", slotIdx, "enabled", !current);
}

export function setTrackInsertFxParam(_track: number, _slotIdx: number, _paramIdx: number, _value: number): void {
  // No-op: FX params controlled through DSL
}

// ── Actions: Master insert FX ───────────────────────

/** Add an FX to the master chain. Returns false if pool is exhausted. */
export function addMasterInsertFx(slotIdx: number, fxType: FxTypeId): boolean {
  const instanceIdx = allocateFxInstance(fxType);
  if (instanceIdx === null) return false;
  setMasterFx("insertFx", slotIdx, {
    type: fxType, instanceIdx, enabled: true, params: {},
  });
  return true;
}

export function clearMasterInsertFx(slotIdx: number): void {
  setMasterFx("insertFx", slotIdx, emptyFxSlot());
}

export function toggleMasterInsertFx(slotIdx: number): void {
  const current = masterFx.insertFx[slotIdx].enabled;
  setMasterFx("insertFx", slotIdx, "enabled", !current);
}

export function setMasterInsertFxParam(_slotIdx: number, _paramIdx: number, _value: number): void {
  // No-op: FX params controlled through DSL
}
