# @tatum/engine

The tatum engine in the browser: the `.synth` language, its live session --
hot-swap on the bar line, knobs by name, keys and pads from a controller, the
loop of `tatum compose` -- in an AudioWorklet, with a TypeScript API over it.
The same `tatum-core` the terminal runs; nothing is reimplemented here.

```
npm run build        # wasm-pack (needs the Rust toolchain) + tsc
npm test             # the engine through the glue, in Node, no browser
```

```ts
import { Tatum } from "@tatum/engine";

const t = await Tatum.create({
  wasmUrl: new URL("@tatum/engine/wasm", import.meta.url),
  processorUrl: new URL("@tatum/engine/processor", import.meta.url),
});
await t.files({ "session.synth": session, "grooves/dub.synth": dub, "voices/bass/sub_dub.synth": sub });
await t.load("session.synth");       // `use` lines resolved over the files
await t.start();
await t.tryKnob("bass.cutoff", 96);  // → { readings: ["bass cutoff 1.2khz"] }
t.on("position", (p) => console.log(p.bar, p.tempo));
await t.connectMidi();               // WebMIDI, as `tatum watch --midi` reads it
```

How it fits together:

- `pkg/` is what `wasm-pack build --target web` writes: the module and
  wasm-bindgen's glue. Built, not committed.
- `src/tatum-processor.js` is the worklet. It imports the glue (after a
  `TextDecoder`/`TextEncoder` polyfill, since the worklet scope has none),
  takes the module's bytes from the page once, and from then on answers
  `{type: "command", id, message}` with `{type: "answer", id, answer}`, and
  posts the engine's position twenty times a second. Any render quantum is
  walked in blocks of 128.
- `src/tatum.ts` is the page side: `Tatum.create`, one method per message of
  `tatum_core::command` (`load`, `tryKnob`, `probeKnob`, `mute`, `scene`,
  `song`, `composeOpen`, `compose`, `composeView`, `patternGet`,
  `patternSet`...), `on("position")`,
  `connectMidi`. Every method returns the engine's answer as a Promise.

The messages themselves are documented in `core/src/command.rs`; this package
adds no meaning of its own, which is the point.
