// The song as loops: the set chosen, its loops as keys in order, the one
// playing lit. A key swaps to that loop on the next bar; a right click on it
// deletes it. DUP copies the loop open as the next one, to build a part
// from the one before; from the scratch loop, + SET starts a song of its own.

import { For, Show } from "solid-js";
import { SESSION, cleanName, deleteLoop, dupLoop, loopName, nextName, openLoop, root, sets } from "../store";

export function Loops() {
  const setOf = (path: string) => /^sets\/([^/]+)\//.exec(path)?.[1] ?? "";
  const current = () => sets().find((s) => s.name === setOf(root()));
  const choose = (name: string) => {
    if (!name) return void openLoop(SESSION);
    const first = sets().find((s) => s.name === name)?.loops[0];
    if (first) void openLoop(first);
  };
  const dup = () => {
    const name = prompt("The new loop's name: a part of the song (intro, drop, break)", nextName(loopName(root())));
    if (name !== null) void dupLoop(name);
  };
  const newSet = () => {
    const set = prompt("The new song's name: its loops are a set of their own");
    if (set === null) return;
    const taken = sets().some((s) => s.name === cleanName(set));
    if (taken && !confirm(`There is a ${cleanName(set)} already: add this loop to it?`)) return;
    const part = prompt("Its first loop's name", "intro");
    if (part !== null) void dupLoop(part, set);
  };
  const remove = (e: MouseEvent, path: string) => {
    e.preventDefault();
    if (confirm(`Delete the loop ${loopName(path)}? Its voices stay in the library.`)) void deleteLoop(path);
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
              {s.name.replace(/_/g, " ").toUpperCase()}
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
                onContextMenu={(e) => remove(e, path)}
                title={`${loopName(path)} · on the next bar · right click: delete`}
              >
                <span class="n">{i() + 1}</span>
                {loopName(path).replace(/_/g, " ").toUpperCase()}
              </button>
            )}
          </For>
        </Show>
      </div>
      <Show
        when={current()}
        fallback={
          <button class="key small dark" onClick={newSet} title="a new song: this loop, the first of a set of its own">
            + SET
          </button>
        }
      >
        <button class="key small dark" onClick={dup} title="the loop open copied as the next one of its set">
          DUP
        </button>
      </Show>
    </section>
  );
}
