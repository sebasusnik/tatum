// The song as loops: the set chosen, its loops as keys in order, the one
// playing lit. A key swaps to that loop on the next bar; DUP copies the
// loop open as the next one, to build a part from the one before.

import { For, Show } from "solid-js";
import { SESSION, dupLoop, loopName, openLoop, root, sets } from "../store";

export function Loops() {
  const setOf = (path: string) => /^sets\/([^/]+)\//.exec(path)?.[1] ?? "";
  const current = () => sets().find((s) => s.name === setOf(root()));
  const choose = (name: string) => {
    if (!name) return void openLoop(SESSION);
    const first = sets().find((s) => s.name === name)?.loops[0];
    if (first) void openLoop(first);
  };
  const dup = () => {
    const name = prompt("The new loop's name", `${loopName(root())}_b`)?.trim();
    if (name && /^[A-Za-z0-9_-]+$/.test(name)) void dupLoop(name);
  };
  return (
    <section class="loops">
      <span class="label">LOOPS</span>
      <select class="set" onChange={(e) => choose(e.currentTarget.value)} title="a song's loops, or the scratch loop">
        <option value="" selected={!current()}>
          SCRATCH
        </option>
        <For each={sets()}>
          {(s) => (
            <option value={s.name} selected={current()?.name === s.name}>
              {s.name.toUpperCase()}
            </option>
          )}
        </For>
      </select>
      <div class="loop-keys">
        <Show when={current()} fallback={<button class="key small loop lit">SESSION</button>}>
          <For each={current()!.loops}>
            {(path, i) => (
              <button
                class={`key small loop ${path === root() ? "lit" : ""}`}
                onClick={() => void openLoop(path)}
                title={`${path} · on the next bar`}
              >
                <span class="n">{i() + 1}</span>
                {loopName(path).replace(/_/g, " ").toUpperCase()}
              </button>
            )}
          </For>
        </Show>
      </div>
      <button class="key small dark" onClick={dup} title="the loop open copied as the next one of its set">
        DUP
      </button>
    </section>
  );
}
