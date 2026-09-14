/**
 * Chord Builder — generates DSL pattern text from scale degrees + voicing params.
 *
 * Works like Nopia: pick a degree (I-VII), choose voicing complexity,
 * inversion, spread, and it outputs the correct chord notation
 * using the degree.octave format the DSL engine understands.
 *
 * No Rust changes needed — this is pure frontend pattern generation.
 */

/** Voicing types — how many notes to stack */
export type VoicingType = "triad" | "seventh" | "ninth" | "eleventh" | "thirteenth";

const VOICING_NOTE_COUNTS: Record<VoicingType, number> = {
  triad: 3,
  seventh: 4,
  ninth: 5,
  eleventh: 6,
  thirteenth: 7,
};

/** Convert a 0-1 knob value to a voicing type */
export function knobToVoicing(value: number): VoicingType {
  if (value < 0.2) return "triad";
  if (value < 0.4) return "seventh";
  if (value < 0.6) return "ninth";
  if (value < 0.8) return "eleventh";
  return "thirteenth";
}

/** Convert a 0-1 knob value to an inversion (0-3) */
export function knobToInversion(value: number): number {
  return Math.floor(value * 3.99);
}

/** Convert a 0-1 knob value to a base octave (1-5) */
export function knobToOctave(value: number): number {
  return 1 + Math.floor(value * 4.99);
}

/**
 * Build a chord from a scale degree.
 *
 * @param degree - 1-7 (scale degree, 1-indexed)
 * @param voicing - how complex the chord is
 * @param inversion - 0 = root position, 1 = 1st inversion, etc.
 * @param octave - base octave (1-6)
 * @param spread - 0 = close voicing, 1 = wide open voicing
 * @returns Array of { degree, octave } pairs representing the chord tones
 */
export function buildChord(
  degree: number,
  voicing: VoicingType,
  inversion: number,
  octave: number,
  spread: number,
): { degree: number; octave: number }[] {
  const noteCount = VOICING_NOTE_COUNTS[voicing];

  // Build chord by stacking thirds from the given degree
  // In a 7-note scale, stacking thirds means: degree, degree+2, degree+4, degree+6...
  const notes: { degree: number; octave: number }[] = [];
  for (let i = 0; i < noteCount; i++) {
    // Each "third" is 2 scale steps up from the previous
    const rawDegree = degree + i * 2; // 1-indexed degree in the scale
    const actualDegree = ((rawDegree - 1) % 7) + 1; // wrap to 1-7
    const octaveOffset = Math.floor((rawDegree - 1) / 7); // how many octaves up

    // Apply spread — wider spacing pushes upper notes to higher octaves
    const spreadOffset = spread > 0.5 ? Math.floor(i * (spread - 0.5) * 2) : 0;

    notes.push({
      degree: actualDegree,
      octave: octave + octaveOffset + spreadOffset,
    });
  }

  // Apply inversion — rotate bottom notes up an octave
  const inv = Math.min(inversion, notes.length - 1);
  for (let i = 0; i < inv; i++) {
    notes[i].octave += 1;
  }
  // Re-sort by pitch (degree.octave)
  notes.sort((a, b) => (a.octave * 7 + a.degree) - (b.octave * 7 + b.degree));

  return notes;
}

/**
 * Generate DSL pattern text for a chord.
 *
 * @param degree - 1-7
 * @param voicing - knob value 0-1
 * @param inversion - knob value 0-1
 * @param octave - knob value 0-1
 * @param spread - knob value 0-1
 * @param velocity - 0-1
 * @param sustained - if true, generates a 4-bar sustained chord pattern
 * @returns DSL pattern text like "[1.3 3.3 5.3]:0.70 .. .. ..\n.. .. .. .."
 */
export function generateChordPatternText(
  degree: number,
  voicing: number,
  inversion: number,
  octave: number,
  spread: number,
  velocity: number = 0.7,
  sustained: boolean = true,
): string {
  const voicingType = knobToVoicing(voicing);
  const inversionVal = knobToInversion(inversion);
  const octaveVal = knobToOctave(octave);

  const chord = buildChord(degree, voicingType, inversionVal, octaveVal, spread);

  // Format as DSL: [1.3 3.3 5.3]:0.70
  const noteStr = chord.map(n => `${n.degree}.${n.octave}`).join(" ");
  const vel = velocity.toFixed(2);
  const chordToken = `[${noteStr}]:${vel}`;

  if (sustained) {
    // 4x4 sustained chord (16 steps, all ties after first)
    const lines = [
      `    ${chordToken} .. .. ..`,
      `    ..${" ".repeat(chordToken.length - 2)} .. .. ..`,
      `    ..${" ".repeat(chordToken.length - 2)} .. .. ..`,
      `    ..${" ".repeat(chordToken.length - 2)} .. .. ..`,
    ];
    return lines.join("\n");
  } else {
    // Just the chord on beat 1, rest for the rest
    const lines = [
      `    ${chordToken} - - -`,
      `    - - - -`,
      `    - - - -`,
      `    - - - -`,
    ];
    return lines.join("\n");
  }
}

/**
 * Generate a full DSL pattern block.
 */
export function generateChordPatternBlock(
  patternName: string,
  degree: number,
  voicing: number,
  inversion: number,
  octave: number,
  spread: number,
  velocity: number = 0.7,
  sustained: boolean = true,
): string {
  const body = generateChordPatternText(degree, voicing, inversion, octave, spread, velocity, sustained);
  return `pattern ${patternName} {\n${body}\n}`;
}

/** Degree labels (Roman numerals) */
export const DEGREE_LABELS = ["I", "II", "III", "IV", "V", "VI", "VII"];

/** Voicing labels for display */
export const VOICING_LABELS: Record<VoicingType, string> = {
  triad: "Triad",
  seventh: "7th",
  ninth: "9th",
  eleventh: "11th",
  thirteenth: "13th",
};

/** Get human-readable chord name for display */
export function getChordDisplayName(degree: number, voicing: number): string {
  const roman = DEGREE_LABELS[degree - 1] ?? "?";
  const vType = knobToVoicing(voicing);
  const suffix = vType === "triad" ? "" : VOICING_LABELS[vType];
  return `${roman}${suffix}`;
}
