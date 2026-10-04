# tatum

[![CI](https://github.com/sebasusnik/tatum/actions/workflows/ci.yml/badge.svg)](https://github.com/sebasusnik/tatum/actions/workflows/ci.yml)

Music as text. A song is a `.synth` file: instruments, patterns, tracks, scenes and an
arrangement, written in a strict little language that the compiler checks the way a
programming language would, with line numbers, suggestions and warnings about the mix.
The engine that plays it is a Rust synthesizer (`no_std`, zero dependencies): four
Volca-style instruments (acid bass, 4-op FM, poly keys, drum machine) plus instruments
built from nodes, a 16-step sequencer with swing, ties, slides, arpeggiator and
parameter locks, insert and send effects, and every song leaves at the same loudness.
It runs as a CLI, live from your editor (`watch` swaps in each save on the next bar), in
the browser through WASM, and behind an MCP server so an AI can write songs with the
compiler in the loop.

```
tempo 126
scale A minor
sidechain 0.4

module beats kit { kick_level 1.0 }
module bass acid {
    cutoff 400hz
    resonance 80%
    glide 0.18
}

pattern beat {
    kick:  X - - -  X - - -  X - - -  X - - -
    hat:   - - x -  - - x -  - - x -  - - x -
}
pattern line {
    1.2  ~1.2  -  1.2   ~5.2  -  ~7.1  -
    1.2  -     -  ~3.2  ~1.2  -  5.1   ~1.2
}

track drums { play beat using kit out > master }
track acid  { play line using acid level 0.6 delay_send 0.2 out > saturate(0.4) > master }

scene intro { track drums { play beat using kit } }
scene drop {
    auto acid cutoff 0.2 > 0.6
    track drums { play beat using kit }
    track acid  { play line using acid }
}
arrange { intro x4  drop x8 }
```

## Getting started

You need Rust (stable). On Linux, audio output needs ALSA headers
(`sudo apt install libasound2-dev`); the WASM build needs the `wasm32-unknown-unknown`
target and, for a browser bundle, `wasm-pack`.

```
cargo run -p tatum-cli -- check examples/acid_arp.synth
cargo run -p tatum-cli -- render examples/acid_arp.synth -o acid_arp.wav
cargo run -p tatum-cli -- params bass
cargo run --release -p tatum-cli -- play examples/acid_arp.synth
cargo run --release -p tatum-cli -- watch live.synth     # re-evaluates on every save
cargo run --release -p tatum-cli -- watch live.synth --tui   # the same, full screen
cargo run --release -p tatum-cli -- debug examples/acid_arp.synth --solo acid
```

## What the CLI does

`play` and `watch` open the default output device through cpal (`--device <name>` picks
another, `--list-devices` shows them) and print the worst callback time against the
budget on exit. The engine renders at 44.1 kHz; a device that only offers another rate,
which is every Bluetooth headphone, gets the output resampled on the way out. `watch` is the live set: value edits apply at once, anything
else takes over on the next bar keeping every tail and voice the edit did not touch, and
a save that does not compile is reported while the last good version keeps playing.
Both read MIDI too: a `midi` block puts the song on a controller -- knobs and faders
on its parameters (`cc 74 > acid cutoff`), the keys on a track (`keys > solo`), the
pads on drums (`pad 36 > kick kick`).
See "Livecoding semantics" and "MIDI" in `docs/DSL.md` and the analysis in `docs/LIVE.md`.

`--tui` runs `play`, `watch` and `set play` full screen, for half a terminal with the
editor in the other half: the bar and the scene, the arrangement or the set's steps, the
mix as a scrolling spectrogram, a lane per track, the knobs as they turn and a save that
did not compile, in red, until one does. Its keys mute, solo and transform the tracks,
writing the words into the file (`rev`, `fast 2`, `every 4 rev`), and put knobs on them;
`?` lists them all. `tui-shot` takes a PNG of that screen at any second of a song or a
set, without a terminal or an audio device. See "The live screen" in `docs/DSL.md`.

`debug` is for finding where a noise comes from. It renders once with every track, bus
and send return kept apart, each as it sits in the mix, and writes into
`test_output/debug/<song>/` a WAV and a spectrogram per part, `sheet.png` with all of
them stacked on one time axis over the mix, a report of clicks, of tracks that do not go
quiet between notes and of energy below 25 Hz or above 16 kHz, with the bar each one
happens at, and a close-up of the worst moment of each part (the wave and its spectrum).
For the mix it gives each part's level per section against the loudest one, and which
parts sit level with each other on top of the same range, section by section; the sheet
has a "crowded" strip that shows where three or more parts pile up. `--bars 17-24` zooms in,
`--dry` adds each track before its insert chain. `render`, `play`, `watch` and `debug`
all take `--solo` and `--mute` with track names; a muted kick still drives the sidechain.

`audit` renders every tonal track on its own and dry and reports, note by note, how much
of its energy is not at a harmonic of the note asked for: a note going wrong, or an
effect dirtying a whole voice. `fmt --units` rewrites knob positions (`cutoff 0.1`) in
their units (`cutoff 800hz`).

## Where to go next

- `docs/DSL.md`: the language reference.
- `docs/PARAMS.md`: every module parameter (generated by `tatum params`).
- `docs/MCP.md`: the MCP server that lets Claude write songs with the compiler in the loop.
- `docs/LIVE.md`: what it takes to play the engine live, measured; the design of the live session.
- `examples/`: songs across genres; each one compiles and renders.
- `examples/rigs/`: rigs, not songs — a full instrument with every optional voice at
  `level 0` and every effect at `wet=0`, waiting to be faded in. A set is built on one.
- Editors: syntax highlighting for Zed in
  [zed-tatum](https://github.com/sebasusnik/zed-tatum), on the tree-sitter grammar in
  [tree-sitter-tatum](https://github.com/sebasusnik/tree-sitter-tatum).
- `sets/`: live sets. A directory of numbered `.synth` files, each the whole rig at a
  moment, walked the way it is played (hot swaps, blends, tempo ramps) by `tatum set render` and played live by
  `tatum set play`, a key or a pad moving to the next step on the next phrase.

Workspace: `core` (engine, DSL, registry), `cli`, `debug` (the analysis behind `tatum
debug`), `mcp`, `wasm` (browser bindings; `wasm/examples/worklet/` runs them in an
AudioWorklet).

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option. Unless you explicitly state otherwise, any
contribution you submit for inclusion in this project, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or conditions.
