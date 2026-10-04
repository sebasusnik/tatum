# The command line

`tatum <command>`; `tatum help` prints the short version of this page. With cargo,
`cargo run --release -p tatum-cli -- <command>`.

| command | what it does |
|---------|--------------|
| [`check`](#check) | compile a song and report errors and design warnings, without sound |
| [`render`](#render) | render a song to a WAV, with a report of the mix |
| [`play`](#play-and-watch) | play a song on the audio device |
| [`watch`](#play-and-watch) | play it, and play every save of the file on the next bar |
| [`debug`](#debug) | render every part apart, with spectrograms and a report of clicks and mud |
| [`audit`](#audit) | measure every note of every tonal track for what is not its harmonics |
| [`set`](#set) | render, check, gate and play a live set |
| [`tui-shot`](#tui-shot) | a picture of the live screen at any second |
| [`params`](#params) | every module parameter, its range and units |
| [`fmt`](#fmt) | rewrite knob positions in their units |

Every command that reads a song resolves its `use "..."` lines first, so a song
built on a rig compiles as one file.

## check

```
tatum check song.synth
```

Parses and compiles, and prints what is wrong with the line it is on and a suggestion.
Then the design warnings: things that compile and will not sound the way they read,
like a track ducked by a sidechain in a scene with no kick. They are listed in
[DSL.md](DSL.md#design-warnings). It exits 1 on an error, 0 with warnings.

## render

```
tatum render song.synth [-o out.wav] [--bars N] [--solo a,b] [--mute c]
```

Renders the whole arrangement to a WAV (`output.wav` by default; `--bars N` renders
only the first N bars).
Every song leaves at -18 LUFS with its true peak limited at -1 dB, after the master
chain. On the way it prints the mix: each track's peak, RMS, crest, width and band, the
tracks that share a band at a similar level, each section's loudness against the
loudest, and what the master chain did to the peaks. At the end it lists what it heard
that sounds wrong: clicks, tracks that do not go quiet between notes. How to read that
report is in [DSL.md](DSL.md#reading-a-mix).

## play and watch

```
tatum play  song.synth [--tui] [--device <name>] [--rate <hz>] [--midi <name>]
tatum watch song.synth [--tui] [--device <name>] [--rate <hz>] [--midi <name>]
```

`play` plays the song on the default output device until it ends or you type `q`.
`watch` does the same and plays every save of the file. A value edit applies at once.
Anything else takes over on the next bar, keeping every tail and voice the edit did not
touch, and a save that does not compile is reported while the last good version keeps
playing. A song without `scene` and `arrange` loops: that is a live set in one file.
See "Livecoding semantics" in [DSL.md](DSL.md#livecoding-semantics).

- `--device <name>` picks the output device; `--list-devices` lists them.
- The engine renders at 44.1 kHz. A device that only offers another rate, as every
  Bluetooth headphone does at 48 kHz, gets the output resampled; `--rate` forces one.
- A MIDI controller plays the song through its `midi { }` block: knobs and faders on
  parameters, the keys on a track, the pads on drums. Every MIDI input is read unless
  `--midi` names one; `--list-midi` lists them. See [DSL.md](DSL.md#midi-knobs-keys-and-pads).
- `--tui` runs full screen; `--glass` is the same for a terminal with translucent cells.
  See [TUI.md](TUI.md).

On exit both print the worst audio callback time against its budget.

## debug

```
tatum debug song.synth [--bars N | A-B] [--dry] [-o dir] [--solo a,b] [--mute c]
```

For finding where a noise comes from. It renders once with every track, bus and send
return kept apart, each as it sits in the mix, and writes into
`test_output/debug/<song>/` (or `-o`):

- a WAV and a spectrogram per part;
- `sheet.png`, all of them stacked on one time axis over the mix, with a "crowded"
  strip where three or more parts pile up on the same range;
- `report.txt`: clicks (and which fall on a section change), tracks that do not go
  quiet between notes, energy below 25 Hz and fizz above 16 kHz, each with its bar,
  and per section each part's level against the loudest and which parts sit level
  with each other on top of the same range;
- `zoom.*.png`, the worst moment of each part up close, its wave and its spectrum.

`--bars 17-24` zooms in on those bars; `--dry` adds each track before its insert chain.

## audit

```
tatum audit song.synth [--bars N] [--json] [--strict]
```

Renders every tonal track on its own and dry, and reports per note how much of its
energy is not at a harmonic of the note the pattern asked for. That finds two things:
a note going wrong against the other notes of the same voice, and an effect dirtying a
whole voice, by rendering it with and without each effect. The absolute number is a
fingerprint of a timbre and means nothing next to another track's. `--json` prints the
same numbers for a script; `--strict` exits 1 if anything is reported. More in
[DSL.md](DSL.md#auditing-a-mix).

## set

A set is a directory of numbered `.synth` files, each the whole rig at a moment,
walked in filename order. A step's header says how long it holds and what it is:
`# set: bars=32 phase=build energy=5`. Writing one is in
[DSL.md](DSL.md#playing-a-set-live).

```
tatum set check  <dir> [--bars N] [--json]
tatum set render <dir> [-o set.wav] [--bars N] [--phrase 1] [--ramp 4] [--blend 0] [--perform script.txt]
tatum set next   <dir> <candidate.synth> [--json] [--max-voices N]
tatum set play   <dir> [--tui] [--auto] [--phrase 8] [--ramp 4] [--blend 0] [--device <name>] [--midi <name>]
```

- **`check`** validates every step and reports the set's arc.
- **`render`** walks the set the way it would be played, with real hot swaps, tempo
  ramps and blends, into one continuous WAV, and prints where each step starts.
  `--perform` plays a script of knobs, pads, keys and step moves on top, for review
  audio without the hardware.
- **`next`** is the gate a proposed step has to pass against the last one: it says yes
  or no and why. It is meant for an agent writing a set step by step.
- **`play`** plays it live. The space bar, a key or a pad asks for the next step, which
  lands on the next phrase line (`--phrase`, 8 bars by default). `--auto` lets the set
  walk itself by its headers, `--ramp` is how many bars a tempo change takes, `--blend`
  mixes two steps the way a DJ does. `--tui` runs full screen, with the steps and their
  cues ([TUI.md](TUI.md)).

## tui-shot

```
tatum tui-shot <song.synth | set dir> [--step N] [--at <seconds>] [--size 160x48] [-o shot.png]
```

A PNG of the `--tui` screen at a moment of a song or a set's step, without a terminal
or an audio device. Its other flags are in [TUI.md](TUI.md#a-picture-of-the-screen-tui-shot).

## params

```
tatum params [bass|fm|keys|beats|track|fx] [--json]
```

Every module parameter with its range, units, default and what it does, as markdown, or
JSON with `--json`. [PARAMS.md](PARAMS.md) is its output, checked in CI.

## fmt

```
tatum fmt --units song.synth
```

Rewrites every module parameter written as a knob position (`cutoff 0.1`) in its units
(`cutoff 800hz`), with as few decimals as read back to the same knob, so the song sounds
the same.

## Solo and mute

`render`, `play`, `watch` and `debug` take `--solo` and `--mute` with track names,
comma-separated or with the flag repeated. A muted track is taken to level 0
everywhere, level automation included. A muted kick still drives the sidechain, so
what is left pumps the way it does in the mix.
