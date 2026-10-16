# tatum · compose, in the browser

The loop of `tatum compose` with a mouse: a groove, one voice per role from
`compose/voices`, the chosen row's pattern as a grid to click into, the
voice's knobs to drag (or a controller's, over WebMIDI), A/B, and a save that
writes the voice as it sounds -- into this browser, and down to the disk as a
`.synth` any song can `use`.

The grid is the pattern the row plays, a lane per drum or one lane of notes,
sixteen cells a bar. A click cycles a cell (rest, hit, accent, ghost on a
drum; rest, note, tie on a voice), a drag up or down moves a note by scale
degrees, the wheel moves its velocity, a double click opens the token itself
for anything the language has (chords, slides, runs), and a right click rests
the step. Each change is one token to the engine, which rewrites the pattern
in the file that defines it and takes it on the next bar.

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
