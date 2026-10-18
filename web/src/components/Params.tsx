// Every parameter of the chosen row's module as small knobs, grouped the
// way a synth panel is -- oscillators, filter, envelope, LFO, vibrato -- or,
// on drums, by piece. A turn is held on the engine like any knob; the row's
// save writes it, in units.

import { Index, Show } from "solid-js";
import type { Macro } from "@tatum/engine";
import { act } from "../store";
import { Knob } from "./Knob";

/** The piece of a kit a parameter belongs to: `kick_decay` to `kick`. */
const PIECES: Record<string, string> = { kick: "kick", snare: "snare", hihat: "hat", clap: "clap", tom: "tom", crash: "crash" };

/** A drum lane's name as the kit's parameters call it: `hat` and `openhat` are `hihat`. */
export function pieceOf(lane: string): string {
  if (lane === "hat" || lane === "openhat") return "hat";
  if (lane.startsWith("tom")) return "tom";
  return lane;
}

function groupOf(name: string): string {
  const prefix = name.split("_")[0];
  if (PIECES[prefix]) return PIECES[prefix];
  if (/^(osc|detune|voice_mode|unison|sub|noise|waveform|algorithm|op\d|mod_index|feedback|ratio)/.test(name)) return "OSC";
  if (/^(cutoff|resonance|keytrack|filter|drive)/.test(name)) return "FILTER";
  if (/^(attack|decay|sustain|release|glide|amp)/.test(name)) return "ENV";
  if (/^lfo/.test(name)) return "LFO";
  if (/^vibrato/.test(name)) return "VIBRATO";
  if (/^chorus/.test(name)) return "CHORUS";
  return "MORE";
}

/** Small knobs for `params` (the view's), all of them or only one group's. */
export function Params(props: { row: number; params: Macro[]; only?: string }) {
  const groups = () => {
    const by = new Map<string, { m: Macro; i: number }[]>();
    props.params.forEach((m, i) => {
      const g = groupOf(m.label);
      if (props.only && g !== props.only) return;
      by.set(g, [...(by.get(g) ?? []), { m, i }]);
    });
    return [...by.entries()];
  };
  const short = (label: string) => {
    const prefix = label.split("_")[0];
    return (PIECES[prefix] ? label.slice(prefix.length + 1) : label).replace(/_/g, " ").toUpperCase();
  };
  return (
    // By index, not by object: every answer is a new view, and a knob
    // mid-drag has to stay the same element to keep the pointer.
    <div class="params">
      <Index each={groups()}>
        {(g) => (
          <div class="param-group">
            <Show when={!props.only}>
              <span class="group-label">{g()[0].toUpperCase()}</span>
            </Show>
            <div class="param-knobs">
              <Index each={g()[1]}>
                {(p) => (
                  <div class={`param ${p().m.held ? "held" : ""}`} title={p().m.label}>
                    <Knob
                      value={p().m.position}
                      tone="white"
                      onChange={(value) => act({ action: "set", row: props.row, param: p().m.label, value })}
                    />
                    <span class="label">{short(p().m.label)}</span>
                    <span class="reading">{p().m.reading}</span>
                  </div>
                )}
              </Index>
            </div>
          </div>
        )}
      </Index>
    </div>
  );
}
