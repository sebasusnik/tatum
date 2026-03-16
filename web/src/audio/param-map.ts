import type { ModuleId } from "../stores/synth";

/**
 * Maps the store's (moduleId, pageIndex, knobIndex) to the WASM engine's
 * flat param_id (0–255). Values are normalized from store 0–100 to engine 0.0–1.0.
 *
 * `null` entries mean the knob isn't wired to a param_id yet (e.g. ADSR).
 * `"swing"` / `"humanize"` / `"probability"` are handled via sequencer methods.
 */

type SpecialParam = "swing" | "humanize" | "probability";
type ParamEntry = number | SpecialParam | null;

// PARAM_MAP[moduleId][pageIndex][knobIndex]
const PARAM_MAP: Record<ModuleId, ParamEntry[][]> = {
  bass: [
    // Page 0: Cutoff, Reso, Env Mod, Decay
    [0, 2, 1, 4],
    // Page 1: Glide, Osc2 Pitch, Osc3 Pitch, Osc1 Wave (drive)
    [3, 10, 11, 12],
    // Page 2: LFO Speed, LFO Depth, LFO Waveform, LFO Target
    [5, 6, 7, 8],
  ],
  keys: [
    // Page 0: Cutoff, Detune, Chorus Mix, Level
    [32, 33, 34, 35],
    // Page 1: Attack, Decay, Sustain, Release (no param IDs yet)
    [null, null, null, null],
    // Page 2: LFO Speed, LFO Depth, Voice Mode, Vibrato Depth
    [36, 37, 41, 43],
  ],
  fm: [
    // Page 0: Algorithm, Mod Index, Feedback, Level (no FM level param yet)
    [64, 65, 71, null],
    // Page 1: Attack, Decay, Sustain, Release (no param IDs yet)
    [null, null, null, null],
    // Page 2: LFO Speed, LFO Depth, Waveform, Chorus Mix
    [66, 67, 72, 73],
  ],
  beats: [
    // Page 0: Kick Decay, Snare Decay, Hihat Level, Clap Level
    [97, 98, 106, 107],
    // Page 1: Kick Pitch, Snare Pitch, Hihat Pitch, Kick Click
    [108, 109, 110, 103],
    // Page 2: Swing (special), Probability (special), Stutter Rate, Humanize (special)
    ["swing", "probability", 111, "humanize"],
  ],
};

export interface ParamResolution {
  paramId: number;
}

export interface SpecialResolution {
  special: SpecialParam;
}

export function resolveParam(
  moduleId: ModuleId,
  pageIdx: number,
  knobIdx: number
): ParamResolution | SpecialResolution | null {
  const entry = PARAM_MAP[moduleId]?.[pageIdx]?.[knobIdx];
  if (entry === null || entry === undefined) return null;
  if (typeof entry === "string") return { special: entry };
  return { paramId: entry };
}

export function uiToNormalized(value: number): number {
  return value / 100;
}

// Note name → MIDI number conversion
const NOTE_OFFSETS: Record<string, number> = {
  C: 0, D: 2, E: 4, F: 5, G: 7, A: 9, B: 11,
};

const BEAT_MIDI: Record<string, number> = {
  KK: 36, SN: 38, HH: 42, CP: 39,
};

export function noteNameToMidi(name: string, moduleId: ModuleId): number {
  if (moduleId === "beats") {
    return BEAT_MIDI[name] ?? 36;
  }
  if (name === "..") return 0;

  const letter = name[0];
  const octave = parseInt(name.slice(1), 10);
  const offset = NOTE_OFFSETS[letter];
  if (offset === undefined || isNaN(octave)) return 60; // fallback C4
  return (octave + 1) * 12 + offset;
}

// Module name → WASM module index
const MODULE_INDEX: Record<ModuleId, number> = {
  bass: 0,
  keys: 1,
  fm: 2,
  beats: 3,
};

export function moduleToIndex(moduleId: ModuleId): number {
  return MODULE_INDEX[moduleId];
}
