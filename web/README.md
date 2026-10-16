# tatum · compose, in the browser

The loop of `tatum compose` with a mouse: a groove, one voice per role from
`compose/voices`, the chosen voice's knobs to drag (or a controller's, over
WebMIDI), A/B, and a save that writes the voice as it sounds -- into this
browser, and down to the disk as a `.synth` any song can `use`.

```
cd wasm/js && npm ci && npm run build     # the engine: wasm-pack + tsc
cd ../../web && npm ci && npm run dev     # http://localhost:5173
```

What it is made of:

- `@tatum/engine` (`../wasm/js`), the engine in an AudioWorklet and the API
  over it. This app knows nothing of `.synth`; every change goes to the
  engine as a message and comes back as the loop's view.
- The library is the repo's `compose/` directory, bundled as text at build
  time (`src/library.ts`). Voices saved from the screen, and the session as
  it was left, live in `localStorage` by path, over the shipped ones;
  `forget` goes back to the shipped library. A saved voice is also offered
  as a download.
- `public/engine/` is the worklet and the module, copied from the package by
  `scripts/copy-engine.mjs` before `dev` and `build`: an AudioWorklet module
  is loaded by URL and imports the glue beside it, which no bundler follows.

Solid + Vite, no other dependency.
