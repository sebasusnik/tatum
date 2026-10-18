// The rows of the loop as a strip of cards: its shape, the voice it plays
// with keys for the one before and the next, the pattern it plays among its
// voice's, its mute, and a key to take it out. A role can have several rows;
// the + keys after them add one. A click on a card chooses the row the
// pattern and the knobs are for.

import { Index, Show } from "solid-js";
import type { Row, View } from "@tatum/engine";
import { act, focus } from "../store";

/** A shape and a colour for each role, Bauhaus primaries; others cycle. */
const SHAPES = ["circle", "square", "triangle", "half", "bar"];
const ROLE_SHAPE: Record<string, string> = { drums: "circle", bass: "square", lead: "triangle", pad: "half", perc: "bar" };

export function shapeOf(role: string, i: number): string {
  return ROLE_SHAPE[role] ?? SHAPES[i % SHAPES.length];
}

/** What a voice from nothing can be: its role in the loop and its instrument. */
const NEW = [
  { key: "bass", label: "BAJO", role: "bass", kind: "bass" },
  { key: "lead-fm", label: "LEAD FM", role: "lead", kind: "fm" },
  { key: "lead-keys", label: "LEAD TECLADO", role: "lead", kind: "keys" },
  { key: "pad", label: "PAD", role: "pad", kind: "keys" },
  { key: "perc", label: "PERC (BATERÍA)", role: "perc", kind: "beats" },
];

/** A pattern without what its voice's all begin with: `amen_chop` as `chop`. */
function short(row: Row, name: string): string {
  const all = row.patterns;
  let common = all[0] ?? "";
  for (const p of all) while (!p.startsWith(common)) common = common.slice(0, -1);
  const cut = common.lastIndexOf("_") + 1;
  return (all.length > 1 && cut < name.length ? name.slice(cut) : name).replace(/_/g, " ");
}

export function Rows(props: { view: View }) {
  const roles = () => props.view.roles;
  return (
    <section class="rows">
      <Index each={props.view.rows}>
        {(row, i) => (
          <div
            class={`row ${i === props.view.focus ? "focused" : ""} ${row().name ? "" : "empty"} ${row().muted ? "muted" : ""}`}
            onClick={() => focus(i)}
            title={row().doc ?? ""}
          >
            <div class="row-head">
              <i class={`shape ${shapeOf(row().role, i)}`} />
              <span class="role">{row().role.toUpperCase()}</span>
              <span class={`held ${row().held ? "on" : ""}`} title="knobs turned, not saved" />
              <Show when={i > 0}>
                <button
                  class="row-out"
                  onClick={(e) => (e.stopPropagation(), act({ action: "remove", row: i }))}
                  title="the row out of the loop"
                >
                  ×
                </button>
              </Show>
            </div>
            <div class="row-voice">
              <button class="key small dark arrow" onClick={(e) => (e.stopPropagation(), act({ action: "choose", row: i, by: -1 }))} title="the voice before">
                ◀
              </button>
              <span class="name">{row().name ?? "—"}</span>
              <button class="key small dark arrow" onClick={(e) => (e.stopPropagation(), act({ action: "choose", row: i, by: 1 }))} title="the next voice">
                ▶
              </button>
            </div>
            <Show when={row().patterns.length > 1}>
              <div class="row-play">
                <button class="lcd-step" onClick={(e) => (e.stopPropagation(), act({ action: "vary", row: i, by: -1 }))} title="the pattern before">
                  ◀
                </button>
                <span class="play" title={row().play}>
                  {short(row(), row().play).toUpperCase()}
                  <i>
                    {row().patterns.indexOf(row().play) + 1}/{row().patterns.length}
                  </i>
                </span>
                <button class="lcd-step" onClick={(e) => (e.stopPropagation(), act({ action: "vary", row: i, by: 1 }))} title="the next pattern">
                  ▶
                </button>
              </div>
            </Show>
            <div class="row-foot">
              <span class="kind">{row().name ? row().kind : `${row().count} voices`}</span>
              <span class="which">{row().index ? `${row().index}/${row().count}` : ""}</span>
              <button
                class={`key small mute ${row().muted ? "lit" : ""}`}
                disabled={!row().name}
                onClick={(e) => (e.stopPropagation(), act({ action: "mute", row: i }))}
                title="mute the row's track, or bring it back"
              >
                M
              </button>
              <button
                class={`key small solo ${row().soloed ? "lit" : ""}`}
                disabled={!row().name}
                onClick={(e) => (e.stopPropagation(), act({ action: "solo", row: i }))}
                title="the row alone, or back with the rest"
              >
                S
              </button>
            </div>
          </div>
        )}
      </Index>
      <div class="row add">
        <span class="role">+ ROW</span>
        <Index each={roles()}>
          {(role) => (
            <button class="key small" onClick={() => act({ action: "add", role: role() })} title={`another ${role()} voice in the loop`}>
              + {role().toUpperCase()}
            </button>
          )}
        </Index>
        <select
          class="lane-pick"
          onChange={(e) => {
            const choice = NEW.find((n) => n.key === e.currentTarget.value);
            e.currentTarget.value = "";
            if (!choice) return;
            const name = prompt(`The new ${choice.label.toLowerCase()} voice's name (letters, digits, _)`, `${choice.role}_${choice.kind}`)?.trim();
            if (name) void act({ action: "new", role: choice.role, kind: choice.kind, name });
          }}
          title="a voice from nothing, its instrument at its defaults"
        >
          <option value="">+ NUEVA</option>
          {NEW.map((n) => (
            <option value={n.key}>{n.label}</option>
          ))}
        </select>
      </div>
    </section>
  );
}
