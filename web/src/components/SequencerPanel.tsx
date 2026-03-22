import { For, Show, createSignal } from "solid-js";
import {
  selectedModule, patterns, drumLanes,
  toggleStep, toggleDrumStep, setStepNote,
  currentStep, playing,
} from "../stores/synth";
import type { MelodicId } from "../stores/synth";
import { midiToNoteName, noteNameToMidi } from "../audio/param-map";

const DRUM_SHORT: Record<string, string> = {
  kick: "KK", snare: "SN", hihat: "HH", clap: "CP", tom: "TM", crash: "CR",
};

const NOTE_RANGE: Record<MelodicId, [number, number]> = {
  bass: [16, 60],   // E0–C4
  keys: [36, 84],   // C2–C6
  fm:   [36, 96],   // C2–C7
};

const PX_PER_SEMITONE = 10;
const CLICK_THRESHOLD = 4;

export default function SequencerPanel() {
  const isMelodic = () => selectedModule() !== "beats";

  return (
    <div class="seq-panel">
      <div class="seq-panel-header">
        <span class="seq-label">
          {isMelodic() ? "Sequencer" : "Drum Machine"}
        </span>
      </div>

      <Show when={isMelodic()} fallback={<DrumGrid />}>
        <MelodicGrid />
      </Show>
    </div>
  );
}

function MelodicGrid() {
  const modId = () => selectedModule() as MelodicId;
  const pat = () => patterns[modId()];

  return (
    <>
      <div class="steps">
        <For each={pat().on}>
          {(on, i) => (
            <NoteStep
              on={on}
              note={pat().notes[i()]}
              now={playing() && currentStep() === i()}
              modId={modId()}
              index={i()}
            />
          )}
        </For>
      </div>
      <div class="beats-row-melodic">
        <For each={Array(16)}>
          {(_, i) => (
            <span class="bm" classList={{ db: i() % 4 === 0 }}>
              {i() % 4 === 0 ? `${i() / 4 + 1}` : "\u00b7"}
            </span>
          )}
        </For>
      </div>
    </>
  );
}

function NoteStep(props: {
  on: boolean;
  note: string;
  now: boolean;
  modId: MelodicId;
  index: number;
}) {
  const [rx, setRx] = createSignal(0);
  const [dragging, setDragging] = createSignal(false);
  let startY = 0;
  let startMidi = 0;
  let currentMidi = 0;
  let didDrag = false;

  function onPointerDown(e: PointerEvent) {
    if (!props.on) {
      toggleStep(props.index);
      return;
    }

    const el = e.currentTarget as HTMLElement;
    el.setPointerCapture(e.pointerId);
    startY = e.clientY;
    startMidi = noteNameToMidi(props.note, props.modId);
    currentMidi = startMidi;
    didDrag = false;
    setDragging(true);
    setRx(0);
  }

  function onPointerMove(e: PointerEvent) {
    if (!dragging()) return;

    const dy = startY - e.clientY; // up = positive = higher pitch
    const totalSemitones = dy / PX_PER_SEMITONE;
    const [lo, hi] = NOTE_RANGE[props.modId];
    const newMidi = Math.round(Math.min(hi, Math.max(lo, startMidi + totalSemitones)));

    if (Math.abs(dy) > CLICK_THRESHOLD) didDrag = true;

    // Fractional rotation within current semitone for smooth 3D feel
    const frac = (totalSemitones - Math.round(totalSemitones)) * 90;
    setRx(frac);

    if (newMidi !== currentMidi) {
      currentMidi = newMidi;
      setStepNote(props.modId, props.index, midiToNoteName(newMidi));
    }
  }

  function onPointerUp() {
    if (!dragging()) return;
    setDragging(false);
    setRx(0);

    if (!didDrag) {
      toggleStep(props.index);
    }
  }

  const prevNote = () => {
    const midi = noteNameToMidi(props.note, props.modId);
    const [lo] = NOTE_RANGE[props.modId];
    return midi > lo ? midiToNoteName(midi - 1) : "";
  };

  const nextNote = () => {
    const midi = noteNameToMidi(props.note, props.modId);
    const [, hi] = NOTE_RANGE[props.modId];
    return midi < hi ? midiToNoteName(midi + 1) : "";
  };

  return (
    <div
      class="stp"
      classList={{
        on: props.on,
        now: props.now,
        dragging: dragging(),
      }}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
    >
      <Show
        when={props.on}
        fallback={<span class="stp-inner" />}
      >
        <div class="note-roller">
          <div
            class="roller-drum"
            style={{ transform: `rotateX(${rx()}deg)` }}
          >
            <span class="roller-face top">{nextNote()}</span>
            <span class="roller-face front">{props.note}</span>
            <span class="roller-face bottom">{prevNote()}</span>
          </div>
        </div>
      </Show>
      <div class="stp-fill" />
    </div>
  );
}

function DrumGrid() {
  return (
    <>
      <div class="drum-grid">
        <For each={drumLanes}>
          {(lane, laneIdx) => (
            <div class="drum-row">
              <span class="drum-lane-label">{DRUM_SHORT[lane.name] ?? lane.name}</span>
              <For each={lane.steps}>
                {(on, stepIdx) => (
                  <div
                    class="drum-cell"
                    classList={{
                      on: on,
                      now: playing() && currentStep() === stepIdx(),
                    }}
                    onClick={() => toggleDrumStep(laneIdx(), stepIdx())}
                  />
                )}
              </For>
            </div>
          )}
        </For>
      </div>
      <div class="beats-row">
        <span />
        <For each={Array(16)}>
          {(_, i) => (
            <span class="bm" classList={{ db: i() % 4 === 0 }}>
              {i() % 4 === 0 ? `${i() / 4 + 1}` : "\u00b7"}
            </span>
          )}
        </For>
      </div>
    </>
  );
}
