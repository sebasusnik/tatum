# tatum · compose, in the browser

The loop of `tatum compose` with a mouse: a groove, one voice per role from
`compose/voices`, the chosen row's pattern as a grid to click into, the
voice's knobs to drag (or a controller's, over WebMIDI), A/B, and a save that
writes the voice as it sounds -- into this browser, and down to the disk as a
`.synth` any song can `use`.

A voice's pattern is a piano roll: a row per degree of the song's scale, a
column per step, each note a bar as long as it is held, and a velocity lane
under it. A click on an empty cell draws a note (drag right while holding to
make it longer), a drag on a note moves it, a drag on its right edge sets its
length, a right click deletes it, shift-click slides into it, alt-click adds
the row to the step as a chord, and a double click opens the token itself
for what the roll does not draw (runs, notes off the scale). A drum
pattern is a lane per drum, sixteen cells a bar: a click cycles a cell
(rest, hit, accent, ghost), the wheel moves its velocity, a right click
rests it. Each gesture goes to the engine as the steps it changed, which
rewrites the pattern in the file that defines it and takes it on the next
bar.

```
cd wasm/js && npm ci && npm run build     # the engine: wasm-pack + tsc
cd ../../web && npm ci && npm run dev     # http://localhost:5173
```

Three modes under the rows:

- **EDIT**: the pattern (piano roll or drum lanes) and the voice's knobs.
- **FX**: the row's effects as pedals, from IN to where the chain ends. A
  `+` between two puts an effect from the language's registry there; on a
  pedal, ◀ ▶ move it, × takes it out, its knobs turn its arguments (MORE
  shows the named ones), WET blends it, and its name is what a knob or an
  `auto` reaches. Each change rewrites the track's `out > ...` line.
- **MIX**: a strip per row, the level on a fader, pan and the two sends on
  knobs, mute and solo. Moves are held like knobs; the row's save writes
  them into the voice's track.

The display's KEY steps the song's root a semitone at a time and picks the
scale; the session then carries a `scale` line over the groove's, and
GROOVE takes it out. Patterns in degrees follow it.

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
