import { createMemo, For, Show } from "solid-js";
import type { ParsedPattern } from "../stores/tatum";
import { currentStep, playing } from "../stores/tatum";

interface Props {
  pattern: ParsedPattern | null;
  color: string;
}

export default function PatternVis(props: Props) {
  return (
    <div class="pat-screen">
      <Show when={props.pattern?.type === "drums"} fallback={
        <MelodicVis pattern={props.pattern} color={props.color} />
      }>
        <DrumVis pattern={props.pattern!} color={props.color} />
      </Show>
    </div>
  );
}

function MelodicVis(props: { pattern: ParsedPattern | null; color: string }) {
  const steps = createMemo(() => {
    if (!props.pattern || props.pattern.type !== "melodic") return [];
    return props.pattern.steps;
  });

  return (
    <div class="pat-vis-melodic">
      <For each={steps()}>
        {(step, i) => {
          const isNow = () => playing() && currentStep() === i();
          return (
            <Show when={step.noteCount > 1} fallback={
              <div
                classList={{ "pat-step": true, now: isNow() }}
                style={{
                  height: step.velocity > 0 ? `${Math.max(3, step.velocity * 20)}px` : step.isTie ? "2px" : "1px",
                  background: step.velocity > 0 ? props.color : step.isTie ? `${props.color}40` : "#222",
                  opacity: step.velocity > 0 ? 0.3 + step.velocity * 0.7 : step.isTie ? 0.3 : 0.5,
                  "--c": props.color,
                }}
              />
            }>
              <div classList={{ "pat-poly": true, now: isNow() }} style={{ "--c": props.color }}>
                <For each={Array.from({ length: step.noteCount })}>
                  {() => (
                    <div
                      class="pat-note"
                      style={{
                        height: `${Math.max(2, (step.velocity * 18) / step.noteCount)}px`,
                        background: props.color,
                        opacity: 0.35 + step.velocity * 0.65,
                      }}
                    />
                  )}
                </For>
              </div>
            </Show>
          );
        }}
      </For>
    </div>
  );
}

function DrumVis(props: { pattern: ParsedPattern; color: string }) {
  return (
    <div class="pat-vis-drums">
      <For each={props.pattern.drumLanes}>
        {(lane) => (
          <div class="pat-drum-row">
            <For each={lane.steps}>
              {(vel, i) => {
                const isNow = () => playing() && currentStep() === i();
                return (
                  <div
                    classList={{ "pat-drum-cell": true, now: isNow() }}
                    style={{
                      background: vel > 0 ? props.color : "#1a1a22",
                      opacity: vel > 0 ? 0.25 + vel * 0.75 : 1,
                    }}
                  />
                );
              }}
            </For>
          </div>
        )}
      </For>
    </div>
  );
}
