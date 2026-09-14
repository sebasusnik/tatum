import { createMemo, createSignal, createEffect } from "solid-js";
import type { ParsedPattern } from "../stores/synth";
import { currentStep, playing } from "../stores/synth";

interface Props {
  pattern: ParsedPattern | null;
  color: string;
}

export default function ActivityLed(props: Props) {
  const [lit, setLit] = createSignal(false);

  createEffect(() => {
    const step = currentStep();
    if (!playing() || step < 0 || !props.pattern) return;

    let hasNote = false;
    if (props.pattern.type === "drums") {
      hasNote = props.pattern.drumLanes.some(
        lane => lane.steps[step] != null && lane.steps[step] > 0
      );
    } else {
      const s = props.pattern.steps[step];
      hasNote = s != null && s.velocity > 0;
    }

    if (hasNote) {
      setLit(true);
      setTimeout(() => setLit(false), 100);
    }
  });

  return (
    <div
      classList={{ "activity-led": true, on: lit() }}
      style={{ "--led-color": props.color }}
    />
  );
}
