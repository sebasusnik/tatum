import { createSignal, createMemo, For } from "solid-js";
import Knob from "./Knob";
import ActivityLed from "./ActivityLed";
import PatternVis from "./PatternVis";
import {
  getPatternForTrack, getModuleNameForTrack,
  dslSource, sendSource,
} from "../stores/synth";
import {
  DEGREE_LABELS, getChordDisplayName,
  generateChordPatternBlock,
} from "../dsl/chordBuilder";

const COLOR = "#b07aff"; // purple for chord module

export default function ChordBlock(props: { trackName: string }) {
  const moduleName = createMemo(() => getModuleNameForTrack(props.trackName));
  const pattern = createMemo(() => getPatternForTrack(props.trackName));

  // Chord builder state
  const [degree, setDegree] = createSignal(1);
  const [voicing, setVoicing] = createSignal(0.0);  // 0-1: triad→13th
  const [inversion, setInversion] = createSignal(0.0); // 0-1: root→3rd
  const [octave, setOctave] = createSignal(0.4);    // 0-1 → octave 1-5
  const [spread, setSpread] = createSignal(0.0);     // 0-1: close→open
  const [velocity, setVelocity] = createSignal(70);  // 0-100
  const [sustained, setSustained] = createSignal(true);

  const chordName = createMemo(() => getChordDisplayName(degree(), voicing()));

  function applyChord(deg?: number) {
    const d = deg ?? degree();
    if (deg !== undefined) setDegree(deg);

    // Generate the pattern block
    const patName = `chord_${props.trackName}`;
    const patternBlock = generateChordPatternBlock(
      patName, d, voicing(), inversion(), octave(), spread(),
      velocity() / 100, sustained(),
    );

    // Find and replace the pattern in the DSL, or append it
    const source = dslSource();
    const patRe = new RegExp(`pattern\\s+${patName}\\s*\\{[^}]*\\}`, "m");

    let patched: string;
    if (patRe.test(source)) {
      patched = source.replace(patRe, patternBlock);
    } else {
      // Append before the first track definition
      const trackIdx = source.indexOf("\ntrack ");
      if (trackIdx >= 0) {
        patched = source.slice(0, trackIdx) + "\n" + patternBlock + "\n" + source.slice(trackIdx);
      } else {
        patched = source + "\n\n" + patternBlock;
      }
    }

    // Also update the track to use this pattern
    const trackPlayRe = new RegExp(`(track\\s+${props.trackName}\\s*\\{[^}]*?)play\\s+\\w+`, "m");
    patched = patched.replace(trackPlayRe, `$1play ${patName}`);

    sendSource(patched);
  }

  return (
    <div class="rw-block" data-m="chord">
      <div class="rw-header">
        <ActivityLed pattern={pattern()} color={COLOR} />
        <span class="rw-label" style={{ color: COLOR }}>
          CHORD{moduleName() ? ` · ${moduleName()}` : ""}
        </span>
      </div>

      {/* Degree selector — 7 buttons */}
      <div class="chord-degrees">
        <For each={DEGREE_LABELS}>
          {(label, i) => (
            <button
              classList={{
                "chord-deg-btn": true,
                active: degree() === i() + 1,
              }}
              onClick={() => applyChord(i() + 1)}
            >
              {label}
            </button>
          )}
        </For>
      </div>

      {/* Chord name display */}
      <div class="chord-display">{chordName()}</div>

      {/* Voicing knobs */}
      <div class="rw-knobs-primary">
        <Knob value={voicing() * 100} label="Voice" color={COLOR} size={42}
          onChange={(v) => { setVoicing(v / 100); applyChord(); }} />
        <Knob value={inversion() * 100} label="Inv" color={COLOR} size={42}
          onChange={(v) => { setInversion(v / 100); applyChord(); }} />
        <Knob value={octave() * 100} label="Oct" color={COLOR} size={42}
          onChange={(v) => { setOctave(v / 100); applyChord(); }} />
      </div>

      <div class="rw-knobs-secondary">
        <Knob value={spread() * 100} label="Spread" color={COLOR} size={32}
          onChange={(v) => { setSpread(v / 100); applyChord(); }} />
        <Knob value={velocity()} label="Vel" color={COLOR} size={32}
          onChange={(v) => { setVelocity(v); applyChord(); }} />
        <button
          classList={{ "chord-sustain-btn": true, active: sustained() }}
          onClick={() => { setSustained(!sustained()); applyChord(); }}
        >
          {sustained() ? "SUST" : "STAB"}
        </button>
      </div>

      <PatternVis pattern={pattern()} color={COLOR} />
    </div>
  );
}
