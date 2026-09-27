# Running tatum in an AudioWorklet

What drove the engine from the first web UI, kept as the one working example
of the WASM crate in a browser. The UI itself is gone.

- `tatum-processor.js`: the AudioWorklet. It instantiates the module, keeps one
  `Tatum` handle, forwards messages from the page (`load-source`, `transport`,
  `set-tempo`, `set-module-param`…) to the exports and copies each rendered
  block into the output.
- `TatumAudio.ts`: the page side. It creates the `AudioContext` at 44.1 kHz,
  loads the worklet, fetches the `.wasm` and sends its bytes over.

Build the module with `wasm-pack build wasm --target web` and serve
`tatum_wasm_bg.wasm` next to the page. The release profile optimizes for
speed; the `wasm` profile (`--profile wasm`) makes the module about 40%
smaller if the download matters more than CPU headroom in the worklet.

Known shortcomings, to fix when the UI is rewritten:

- The import object names wasm-bindgen's hashed glue functions
  (`__wbg___wbindgen_memory_edb3f01e…`). Any wasm-bindgen upgrade renames them
  and breaks it; use the generated glue, or `--target no-modules`.
- `alloc` has no matching `free`: every edit and every `set-module-param`
  leaks its buffer.
- Results are decoded one byte per character, which breaks on non-ASCII error
  messages. `TextDecoder` is not in the worklet scope; decode on the page.
- `process()` assumes a 128-frame render quantum; the engine renders at most
  `BLOCK_SIZE` frames per call.
