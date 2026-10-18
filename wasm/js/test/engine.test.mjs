// The engine through wasm-bindgen's glue, outside a browser: what the
// worklet does, done by Node. It proves the module, the glue and the
// `command` export fit together, and that a song loaded over a files map
// plays sound. Run with `node --test` after `npm run build:wasm`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const repo = join(here, "..", "..", "..");
const { initSync, Tatum, wasm_memory } = await import("../pkg/tatum_wasm.js");

initSync({ module: readFileSync(join(here, "..", "pkg", "tatum_wasm_bg.wasm")) });

/** The repo's compose library as a files map. */
function library() {
  const files = {};
  for (const sub of ["grooves", "voices/bass", "voices/lead", "voices/pad", "voices/perc"]) {
    const dir = join(repo, "compose", sub);
    for (const name of readdirSync(dir).sort()) {
      if (name.endsWith(".synth")) files[`${sub}/${name}`] = readFileSync(join(dir, name), "utf8");
    }
  }
  return files;
}

function send(tatum, message) {
  return JSON.parse(tatum.command(JSON.stringify(message)));
}

function render(tatum, blocks) {
  const memory = wasm_memory();
  let energy = 0;
  for (let i = 0; i < blocks; i++) {
    const ptr = tatum.process(128) >>> 0;
    const mem = new Float32Array(memory.buffer, ptr, 256);
    for (const s of mem) energy += s * s;
  }
  return energy;
}

test("a message that is not one is refused", () => {
  const tatum = new Tatum();
  const answer = send(tatum, { op: "nope" });
  assert.equal(answer.ok, false);
  assert.match(answer.error, /nope/);
  tatum.free();
});

test("a song loaded over files plays, and the loop answers", () => {
  const tatum = new Tatum();
  assert.equal(send(tatum, { op: "files", files: library() }).ok, true);
  const opened = send(tatum, { op: "compose_open", root: "session.synth" });
  assert.equal(opened.ok, true, JSON.stringify(opened));
  assert.equal(opened.view.rows.length, 2); // the groove and the first bass
  assert.equal(opened.view.rows[1].name, "acid");
  assert.equal(send(tatum, { op: "start" }).ok, true);
  assert.ok(render(tatum, 200) > 1, "silence");
  const position = send(tatum, { op: "position" });
  assert.equal(position.running, true);
  assert.ok(position.bar >= 0);

  // A knob from a controller, by its bytes, then the loop's view says so.
  const turned = send(tatum, { op: "midi", bytes: [0xb0, 74, 127] });
  assert.match(turned.readings[0], /^acid cutoff/);
  const view = send(tatum, { op: "compose_view" }).view;
  assert.equal(view.macros[0].position, 127);
  assert.equal(view.macros[0].held, true);

  // Saved: a file to write, in units, and the session swaps to it.
  const saved = send(tatum, { op: "compose", action: "save", row: 1, name: "acid_open" });
  assert.equal(saved.ok, true, JSON.stringify(saved));
  assert.equal(saved.writes[0].path, "voices/bass/acid_open.synth");
  assert.match(saved.writes[0].text, /cutoff 20000hz/);
  assert.equal(saved.loaded.plan, "swap");
  assert.ok(render(tatum, 400) > 1, "silence after the swap");
  tatum.free();
});

test("the old entry point still loads a whole text", () => {
  const tatum = new Tatum();
  const text = readFileSync(join(repo, "examples", "dub_techno.synth"), "utf8");
  const loaded = send(tatum, { op: "text", source: text });
  assert.equal(loaded.ok, true, JSON.stringify(loaded));
  assert.equal(loaded.applied, "loaded");
  const bad = send(tatum, { op: "text", source: "tempo nope\n" });
  assert.equal(bad.ok, false);
  assert.equal(bad.errors[0].line, 1);
  tatum.free();
});
