import { createSignal } from "solid-js";
import { createStore } from "solid-js/store";
import { SynthAudio } from "../audio/SynthAudio";
import { resolveParam, uiToNormalized, noteNameToMidi, moduleToIndex } from "../audio/param-map";

export interface Page {
  keys: string[];
  values: number[];
}

export interface ModuleConfig {
  color: string;
  label: string;
  sub: string;
  pages: Page[];
  notes: string[];
  on: boolean[];
  locks: boolean[];
  slides: boolean[];
  lockSlides: boolean[];
}

export type ModuleId = "bass" | "keys" | "fm" | "beats";

const ALL_MODULES: ModuleId[] = ["bass", "keys", "fm", "beats"];

const MODULES: Record<ModuleId, ModuleConfig> = {
  bass: {
    color: "#ff6b35",
    label: "BASS",
    sub: "3 OSC \u00b7 LADDER 4P",
    pages: [
      { keys: ["Cutoff", "Reso", "Env Mod", "Decay"], values: [72, 35, 60, 45] },
      { keys: ["Glide", "Osc2", "Osc3", "Drive"], values: [20, 50, 50, 30] },
      { keys: ["LFO Spd", "LFO Dpt", "LFO Wav", "LFO Tgt"], values: [40, 25, 0, 0] },
    ],
    notes: ["C2", "D2", "E2", "F2", "G2", "A2", "B2", "C3", "D3", "E3", "F3", "G3", "A3", "B3", "C4", "D4"],
    on: [true, false, false, false, true, false, false, true, false, false, true, false, false, true, false, false],
    locks: [false, false, false, false, true, false, false, false, false, false, false, false, false, false, false, false],
    slides: [false, false, false, false, false, false, false, true, false, false, false, false, false, false, false, false],
    lockSlides: [false, false, false, false, true, false, false, false, false, false, false, false, false, false, false, false],
  },
  keys: {
    color: "#00c9b1",
    label: "KEYS",
    sub: "POLY \u00b7 2 SAW \u00b7 DETUNE",
    pages: [
      { keys: ["Cutoff", "Detune", "Chorus", "Level"], values: [80, 40, 55, 70] },
      { keys: ["Attack", "Decay", "Sustain", "Release"], values: [15, 40, 65, 50] },
      { keys: ["LFO Spd", "LFO Dpt", "Voice", "Vibrato"], values: [30, 20, 0, 10] },
    ],
    notes: ["C3", "D3", "E3", "F3", "G3", "A3", "B3", "C4", "D4", "E4", "F4", "G4", "A4", "B4", "C5", "D5"],
    on: [true, false, false, false, false, false, true, false, false, false, false, false, true, false, false, false],
    locks: Array(16).fill(false),
    slides: Array(16).fill(false),
    lockSlides: Array(16).fill(false),
  },
  fm: {
    color: "#ffd23f",
    label: "FM",
    sub: "ALG 1 \u00b7 4 OP \u00b7 FEEDBACK",
    pages: [
      { keys: ["Algo", "Mod Idx", "Feedback", "Level"], values: [0, 50, 20, 65] },
      { keys: ["Attack", "Decay", "Sustain", "Release"], values: [5, 60, 30, 40] },
      { keys: ["LFO Spd", "LFO Dpt", "Wave", "Chorus"], values: [35, 15, 0, 40] },
    ],
    notes: ["C4", "D4", "E4", "F4", "G4", "A4", "B4", "C5", "D5", "E5", "F5", "G5", "A5", "B5", "C6", "D6"],
    on: [true, false, true, false, false, true, false, false, true, false, false, true, false, false, true, false],
    locks: Array(16).fill(false),
    slides: Array(16).fill(false),
    lockSlides: Array(16).fill(false),
  },
  beats: {
    color: "#ff5ea0",
    label: "BEATS",
    sub: "KICK \u00b7 SNARE \u00b7 HAT \u00b7 CLAP",
    pages: [
      { keys: ["Kick Dcy", "Snare", "Hat Dcy", "Clap"], values: [55, 40, 30, 50] },
      { keys: ["Kick Pit", "Snr Pit", "Hat Pit", "Click"], values: [50, 50, 50, 20] },
      { keys: ["Swing", "Prob", "Stutter", "Human"], values: [50, 100, 0, 10] },
    ],
    notes: ["KK", "..", "..", "..", "SN", "..", "..", "..", "HH", "..", "HH", "..", "HH", "..", "HH", ".."],
    on: [true, false, false, false, true, false, false, false, true, false, true, false, true, false, true, false],
    locks: Array(16).fill(false),
    slides: Array(16).fill(false),
    lockSlides: Array(16).fill(false),
  },
};

// ── Signals ──────────────────────────────────────────

export const [currentModule, setCurrentModule] = createSignal<ModuleId>("bass");
export const [currentPage, setCurrentPage] = createSignal(0);
export const [playing, setPlaying] = createSignal(false);
export const [currentStep, setCurrentStep] = createSignal(-1);
export const [bpm, setBpm] = createSignal(120);
export const [recording, setRecording] = createSignal(false);
export const [activePattern, setActivePattern] = createSignal(0);

// ── Store ────────────────────────────────────────────

export const [modules, setModules] = createStore(MODULES);

// ── Derived ──────────────────────────────────────────

export const mod = () => modules[currentModule()];
export const page = () => mod().pages[currentPage()];
export const accentColor = () => mod().color;
export const pageCount = () => mod().pages.length;

// ── Audio Bridge ─────────────────────────────────────

let audio: SynthAudio | null = null;
let initPromise: Promise<void> | null = null;

async function initAudio(): Promise<void> {
  if (audio?.ready) return;
  if (initPromise) return initPromise;
  initPromise = (async () => {
    audio = new SynthAudio();
    await audio.init((step) => {
      setCurrentStep(step);
    });
    console.log("[store] audio fully initialized");
    audio.setBpm(bpm());
    syncAllStepsToWasm();
    syncAllParamsToWasm();
  })();
  return initPromise;
}

function syncAllStepsToWasm(): void {
  if (!audio) return;
  // Tell the engine which module the sequencer should target
  const m = currentModule();
  audio.setActiveModule(moduleToIndex(m));
  const modData = modules[m];
  for (let i = 0; i < 16; i++) {
    if (modData.on[i]) {
      const note = noteNameToMidi(modData.notes[i], m);
      audio.setStep(i, note, 0.8, 0.75);
      if (modData.slides[i]) audio.setStepSlide(i, true);
      if (modData.lockSlides[i]) audio.setStepLockSlide(i, true);
    } else {
      audio.clearStep(i);
    }
  }
}

function syncAllParamsToWasm(): void {
  if (!audio) return;
  for (const modId of ALL_MODULES) {
    const modData = modules[modId];
    for (let p = 0; p < modData.pages.length; p++) {
      for (let k = 0; k < modData.pages[p].values.length; k++) {
        const resolved = resolveParam(modId, p, k);
        if (resolved && "paramId" in resolved) {
          audio.setParam(resolved.paramId, uiToNormalized(modData.pages[p].values[k]));
        } else if (resolved && "special" in resolved) {
          handleSpecialParam(resolved.special, uiToNormalized(modData.pages[p].values[k]));
        }
      }
    }
  }
}

function handleSpecialParam(special: string, normalized: number): void {
  if (!audio) return;
  switch (special) {
    case "swing":
      audio.setSwing(0.5 + normalized * 0.17); // 0–100 → 0.5–0.67
      break;
    case "humanize":
      audio.setHumanize(normalized);
      break;
    case "probability":
      // Global probability doesn't map directly; applied per-step
      break;
  }
}

// ── Actions ──────────────────────────────────────────

export function switchModule(id: ModuleId): void {
  setCurrentModule(id);
  setCurrentPage(0);
  // Re-sync this module's steps to WASM sequencer
  syncAllStepsToWasm();
}

export function nextPage(): void {
  setCurrentPage((p) => (p + 1) % pageCount());
}

export function prevPage(): void {
  setCurrentPage((p) => (p - 1 + pageCount()) % pageCount());
}

export function setParamValue(knobIdx: number, value: number): void {
  const m = currentModule();
  const p = currentPage();
  const clamped = Math.round(Math.min(100, Math.max(0, value)));
  setModules(m, "pages", p, "values", knobIdx, clamped);

  // Push to WASM
  if (audio) {
    const resolved = resolveParam(m, p, knobIdx);
    if (resolved && "paramId" in resolved) {
      audio.setParam(resolved.paramId, uiToNormalized(clamped));
    } else if (resolved && "special" in resolved) {
      handleSpecialParam(resolved.special, uiToNormalized(clamped));
    }
  }
}

export function toggleStep(idx: number): void {
  const m = currentModule();
  setModules(m, "on", idx, (v: boolean) => !v);

  if (audio) {
    const isOn = modules[m].on[idx];
    if (isOn) {
      const note = noteNameToMidi(modules[m].notes[idx], m);
      audio.setStep(idx, note, 0.8, 0.75);
      if (modules[m].slides[idx]) audio.setStepSlide(idx, true);
      if (modules[m].lockSlides[idx]) audio.setStepLockSlide(idx, true);
    } else {
      audio.clearStep(idx);
    }
  }
}

export async function startPlayback(): Promise<void> {
  console.log("[store] startPlayback called, module =", currentModule());
  await initAudio();
  console.log("[store] audio ready, setting BPM and starting");
  audio!.setBpm(bpm());
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
  setBpm((b) => Math.round(Math.min(300, Math.max(40, b + delta))));
  audio?.setBpm(bpm());
}
