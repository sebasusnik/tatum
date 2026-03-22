import { For, createSignal } from "solid-js";
import type { MelodicPattern, MelodicId } from "../stores/synth";

const NOTE_NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

function midiToName(midi: number): string {
  return NOTE_NAMES[midi % 12] + (Math.floor(midi / 12) - 1);
}

function nameToMidi(name: string): number {
  const offsets: Record<string, number> = { C: 0, D: 2, E: 4, F: 5, G: 7, A: 9, B: 11 };
  const letter = name[0];
  const sharp = name[1] === "#" ? 1 : 0;
  const octave = parseInt(name.slice(sharp ? 2 : 1), 10);
  return (octave + 1) * 12 + (offsets[letter] ?? 0) + sharp;
}

interface Props {
  moduleId: MelodicId;
  pattern: MelodicPattern;
  currentStep: number;
  playing: boolean;
  color: string;
  onToggle: (idx: number) => void;
  onNoteChange: (idx: number, note: string) => void;
}

export default function InlineSeq(props: Props) {
  return (
    <div class="rw-seq">
      <For each={Array.from({ length: 16 })}>
        {(_, i) => {
          const [dragging, setDragging] = createSignal(false);
          let startY = 0;
          let startMidi = 0;

          const isOn = () => props.pattern.on[i()];
          const isNow = () => props.playing && props.currentStep === i();
          const note = () => props.pattern.notes[i()];

          function onPointerDown(e: PointerEvent) {
            if (!isOn()) {
              props.onToggle(i());
              return;
            }
            (e.target as HTMLElement).setPointerCapture(e.pointerId);
            startY = e.clientY;
            startMidi = nameToMidi(note());
            setDragging(true);
          }

          function onPointerMove(e: PointerEvent) {
            if (!dragging()) return;
            const dy = startY - e.clientY;
            const semitones = Math.round(dy / 8);
            const newMidi = Math.max(21, Math.min(108, startMidi + semitones));
            const newName = midiToName(newMidi);
            if (newName !== note()) {
              props.onNoteChange(i(), newName);
            }
          }

          function onPointerUp(e: PointerEvent) {
            if (dragging()) {
              (e.target as HTMLElement).releasePointerCapture(e.pointerId);
              setDragging(false);
              // If barely moved, toggle off
              const dy = Math.abs(startY - e.clientY);
              if (dy < 3) props.onToggle(i());
            }
          }

          return (
            <div
              classList={{
                "rw-step": true,
                on: isOn(),
                now: isNow(),
              }}
              style={{ "--step-color": props.color }}
              onPointerDown={onPointerDown}
              onPointerMove={onPointerMove}
              onPointerUp={onPointerUp}
            >
              {isOn() && <span class="rw-step-note">{note()}</span>}
            </div>
          );
        }}
      </For>
    </div>
  );
}
