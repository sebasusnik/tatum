// The worklet and the module go into public/engine/, served as they are:
// an AudioWorklet module is loaded by URL and imports the glue beside it,
// which no bundler follows. `@tatum/engine` must be built first
// (`npm run build` in wasm/js).

import { copyFileSync, existsSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const engine = join(here, "..", "node_modules", "@tatum", "engine");
const out = join(here, "..", "public", "engine");

const files = [
  ["src/tatum-processor.js", "src/tatum-processor.js"],
  ["src/text-codec.js", "src/text-codec.js"],
  ["pkg/tatum_wasm.js", "pkg/tatum_wasm.js"],
  ["pkg/tatum_wasm_bg.wasm", "pkg/tatum_wasm_bg.wasm"],
];

for (const [from, to] of files) {
  const src = join(engine, from);
  if (!existsSync(src)) {
    console.error(`missing ${src}: build @tatum/engine first (cd wasm/js && npm run build)`);
    process.exit(1);
  }
  mkdirSync(dirname(join(out, to)), { recursive: true });
  copyFileSync(src, join(out, to));
}
console.log(`engine copied into public/engine (${files.length} files)`);
