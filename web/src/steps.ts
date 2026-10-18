// A step of a pattern as the grid edits it, and back to its token: the
// notes it plays (one, a chord, or a run inside the step), its velocity,
// the chance it sounds and what is locked on it. The grid changes these and
// writes the token again, so what it does not draw -- a lock, a chance --
// stays as it was.

import type { PatternNote, PatternScale, PatternStep } from "@tatum/engine";

const NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

export function nameOf(midi: number): string {
  return `${NAMES[midi % 12]}${Math.floor(midi / 12) - 1}`;
}

/** The degree of the scale a MIDI note is, if it is one. */
export function degreeOf(midi: number, scale: PatternScale): { degree: number; octave: number } | null {
  const above = (((midi - scale.root) % 12) + 12) % 12;
  const d = scale.intervals.indexOf(above);
  if (d < 0) return null;
  const octave = (midi - scale.root - scale.intervals[d]) / 12 - 1;
  return octave >= 0 && octave <= 9 ? { degree: d + 1, octave } : null;
}

/** How a MIDI note is written: its degree, or its name when off the scale. */
export function noteText(midi: number, scale: PatternScale): string {
  const d = degreeOf(midi, scale);
  return d ? `${d.degree}.${d.octave}` : nameOf(midi);
}

/** A number as the language writes an amount: three places at most, no trailing zeros. */
export function amount(v: number): string {
  return String(Math.round(v * 1000) / 1000);
}

/** What is locked on a step, the way a token ends: `?0.5(gate=0.3, cutoff=0.6)`. */
export interface Locks {
  prob?: number | null;
  gate?: number | null;
  cutoff?: number | null;
  res?: number | null;
  edepth?: number | null;
}

export function locksOf(s: PatternStep): Locks {
  return { prob: s.prob, gate: s.gate, cutoff: s.cutoff, res: s.res, edepth: s.edepth };
}

function lockText(l: Locks): string {
  const parts: string[] = [];
  if (l.cutoff != null) parts.push(`cutoff=${amount(l.cutoff)}`);
  if (l.edepth != null) parts.push(`edepth=${amount(l.edepth)}`);
  if (l.res != null) parts.push(`res=${amount(l.res)}`);
  if (l.gate != null) parts.push(`gate=${amount(l.gate)}`);
  return (l.prob != null && l.prob < 0.999 ? `?${amount(l.prob)}` : "") + (parts.length ? `(${parts.join(", ")})` : "");
}

export const rest = (): PatternStep => ({ token: "-", kind: "rest" });
export const tie = (): PatternStep => ({ token: "..", kind: "tie" });

/** The notes a step plays, as written; empty for a rest or a tie. */
export function notesOf(s: PatternStep): PatternNote[] {
  if (s.notes) return s.notes;
  return (s.midi ?? []).map((midi) => ({ midi, text: nameOf(midi), slide: !!s.slide, velocity: null }));
}

function note(n: PatternNote, withVelocity: boolean): string {
  return `${n.slide ? "~" : ""}${n.text}${withVelocity && n.velocity != null ? `:${amount(n.velocity)}` : ""}`;
}

/**
 * A step that plays `notes` together: a note, or a chord when there are
 * several. Velocity and locks are the step's. The notes' text is taken
 * again from the scale, so a note moved is written as its new degree.
 */
export function chordStep(midis: number[], from: PatternStep | null, scale: PatternScale, slide = false): PatternStep {
  const sorted = [...new Set(midis)].sort((a, b) => a - b);
  if (sorted.length === 0) return rest();
  const velocity = from?.velocity ?? null;
  const locks = from ? locksOf(from) : {};
  const notes: PatternNote[] = sorted.map((m) => ({ midi: m, text: noteText(m, scale), slide: sorted.length === 1 && slide, velocity: null }));
  const v = velocity != null ? `:${amount(velocity)}` : "";
  const token = notes.length === 1 ? `${note(notes[0], false)}${v}${lockText(locks)}` : `[${notes.map((n) => note(n, false)).join(" ")}]${v}${lockText(locks)}`;
  return {
    token,
    kind: notes.length === 1 ? "note" : "chord",
    note: notes.map((n) => n.text).join(" "),
    midi: sorted,
    notes,
    velocity,
    slide: notes.length === 1 && slide,
    ...locks,
  };
}

/** A run inside one step: `notes` evenly spaced, each its own pitch and slide. */
export function runStep(notes: PatternNote[], scale: PatternScale): PatternStep {
  const written = notes.map((n) => ({ ...n, text: noteText(n.midi, scale) }));
  return {
    token: `<${written.map((n) => note(n, true)).join(" ")}>`,
    kind: "sub",
    note: written.map((n) => n.text).join(" "),
    midi: written.map((n) => n.midi),
    notes: written,
  };
}

/** The same step with another velocity, or other locks: everything else kept. */
export function withValues(s: PatternStep, scale: PatternScale, values: { velocity?: number | null } & Locks): PatternStep {
  if (s.kind === "hit") return hitStep(values.velocity ?? s.velocity ?? 0.8, { ...locksOf(s), ...values }, s.roll ?? 1);
  if (s.kind !== "note" && s.kind !== "chord") return s;
  const merged = { ...s, ...values };
  return chordStep(s.midi ?? [], merged, scale, !!s.slide);
}

/** A drum hit: `X`, `x`, `o` for the three velocities the letters mean, `x:v` otherwise. */
export function hitStep(velocity: number, locks: Locks = {}, roll = 1): PatternStep {
  const v = Math.max(0.05, Math.min(1, Math.round(velocity * 100) / 100));
  const near = (a: number) => Math.abs(v - a) < 0.001;
  const letter = near(1) ? "X" : near(0.8) ? "x" : near(0.35) ? "o" : `x:${amount(v)}`;
  return {
    token: `${letter}${roll > 1 ? `*${roll}` : ""}${lockText(locks)}`,
    kind: "hit",
    velocity: v,
    roll,
    ...locks,
  };
}
