# hiperespacio — the KeyLab layout

One layout for the whole hour: a fader, knob or pad does the same job in every
track, the way a DJ's rack does in Ableton. What it reaches changes with the
track (the bass of T04 is a 303, the bass of T08 a three-oscillator gallop), what
it *means* does not. `_keylab.synth` maps what every track shares; each
`_tNN.synth` maps the rest onto its own voices with a `midi` block after its
`use`: faders F2 and F5, the `tema` page of knobs, bank B and keys A# and B.
The short version is `KEYLAB.md`.

## Faders (left to right)

| # | cc | role | notes |
|---|---|---|---|
| F1 | 73 | **kick** | the kit's `kick_level`, through the track name `drums` (_keylab) |
| F2 | 75 | **bass** | the track's bass, as its rig writes it (T04: the roll and the gallop together) |
| F3 | 79 | **hats** | the kit's `hihat_level` (_keylab) |
| F4 | 72 | **lead** | the keyboard's voices, all four (_keylab) |
| F5 | 80 | **FX** | the track's own voices together: main voice, second voice, signature (its rig) |
| F6 | 81 | **send: echo** | the keyboard's voices into the delay (_keylab) |
| F7 | 82 | **send: room** | the keyboard's voices into the reverb (_keylab) |
| F8 | 83 | **noise riser** | a noise held while the fader is up, louder and brighter as it rises (_keylab) |
| F9 | 85 | **DJ filter** | centre open, down darker (low-pass), up thinner (high-pass) (_keylab) |

`takeover pickup`: a fader or knob that is not where its target is moves
nothing until it gets there (the screen says which way). The DJ filter
starts in the centre; found anywhere else it waits for the centre. The
master volume is K1 on the `master` page, and starts at the top.

## Knobs: one page per voice

Tab on the computer (or an encoder once its number is known: `cc N > voice
step` in `_keylab.synth`) steps through the pages; the screen's header shows
the page (`knobs: bass`), and changing it says what each knob now moves.

| page | K1 74 | K2 71 | K3 76 | K4 77 | K5 93 | K6 18 | K7 19 | K8 16 | K9 17 |
|---|---|---|---|---|---|---|---|---|---|
| **tema** (start) | bass filter | main voice open | signature RPM | second voice open | tension | throw amount | kick decay | hats pitch | dirt |
| **bass** | cutoff | resonance | decay | env amount | glide | echo send | osc 2 pitch | vibrato | level |
| **lead** | brightness | resonance (pad) | decay | FM index | FM feedback | laser vowel | zap crusher | zap fall | vibrato |
| **drums** | kick decay | kick pitch | kick click | kick drive | snare level | snare decay | hats pitch | hats pan | stutter |
| **master** | master volume | thin the floor | room | echo | EQ low | EQ mid | EQ high | — | — |

The `tema` page is each track's own nine macros (below). The others are
written once in `_keylab.synth` by track name, so `bass cutoff` reaches every
track's bass; a knob whose target a track lacks (the FM bass of T05 has no
cutoff) reads `(not in this track)` and does nothing.

### The `tema` page

| # | cc | role | typical targets |
|---|---|---|---|
| K1 | 74 | **bass filter** | bass `cutoff` + `cutoff_env` (one macro), a little resonance |
| K2 | 71 | **main voice: open** | acid cutoff+resonance+env; FM `mod_index` (+ a touch of feedback) |
| K3 | 76 | **signature: RPM / character** | grinder: every op ratio together (+ index) = the motor's RPM; zaps: ratio/pitch; drone: index |
| K4 | 77 | **second voice: open** | its filter or index |
| K5 | 93 | **tension** | master `hp` up to ~400 Hz + the main voice's `delay_send` up: thins and washes, for a manual build |
| K6 | 18 | **throw amount** | main voice `delay_send` 0..80% |
| K7 | 19 | **kick** | kit `kick_decay` (short → boomy) |
| K8 | 16 | **hats** | kit `hihat_pitch`: darker left, the written tuning in the middle (`hihat_decay` is overwritten on every hit by the engine today) |
| K9 | 17 | **dirt** | the track's crush/saturation `wet` or drive (drone crush, bass drive) |

## Pads

Bank A (36–43): effects on the whole mix while held, as strong as the pad is
struck and then as hard as it is pressed (aftertouch, never below the strike):

| pad | note | action |
|---|---|---|
| A1 | 36 | `repeat 1/8` — DJ roll |
| A2 | 37 | `repeat 1/16` — tighter |
| A3 | 38 | `repeat 1/32` — the stutter before a drop |
| A4 | 39 | `tapestop` — slows to a stop; struck harder, sooner |
| A5 | 40 | `gate 1/16` — trance gate; harder, deeper |
| A6 | 41 | `crush` — bitcrusher; harder, fewer bits |
| A7 | 42 | `scream` — driven into a vowel filter; pressure moves it a → i |
| A8 | 43 | `freeze` the reverb — the free one: put anything here |

Bank B (44–51), the track's sounds (each rig):

| pad | note | action |
|---|---|---|
| B1 | 44 | `play` signature — hold it and the grinder/turbine/drone sounds; K3 (page `tema`) moves its RPM |
| B2 | 45 | `hold` riser — a sweep/riser figure while held |
| B3 | 46 | crash (drum hit) |
| B4 | 47 | `hold` second signature figure (zaps, shard, bell phrase, oud cell…) |
| B5 | 48 | `repeat 1/32` — the stutter before a drop |
| B6 | 49 | `throw` signature into the echo |
| B7 | 50 | free per track |
| B8 | 51 | free per track |

What bank A used to do moved to the lowest octave of the keyboard: kick out
is key D, the throw key B, the perc toggle key A#.

## Rules for the per-track maps

- Every track maps every row above that makes sense for it; a track without a
  second signature leaves B4 to something else of its own, and says so in a
  comment.
- A macro that sounds bad at an extreme is wrong: choose ranges where the whole
  knob travel is usable live (no screaming resonance, no silence, no clicks).
- The rig keeps `level 0` voices silent until the set (or a fader) brings them in;
  pad-played voices follow the engine's rule for `play` (see docs/DSL.md).

## Quantized pads

Every rig writes `q=` on the pads that need a steady hand: A1 kick out `q=bar`,
B3 crash `q=beat`, A3/B1/B6 `q=1/16`, B2/B4 `q=beat`. Press or let go roughly in
time and it lands on the line (docs/DSL.md, "Keys and pads").

## The keyboard (`_keylab.synth`)

The keys are split by MIDI note into three zones. The numbers are Arturia's
defaults with the octave buttons centred; `tatum midi monitor sets/hiperespacio`
shows what yours send.

| zone | notes | what it does |
|---|---|---|
| triggers | 36–47 (the lowest C to B) | each key does one thing, below |
| bass | 48–59 | held, the track's own `bass` rolls that note on the three sixteenths after each beat, in place of its line; let go and the line comes back. The keys sound in the line's own register (octave 1 in every track of the set): E3 on the keyboard plays E1 |
| lead | 60–84 | the scene's voice, every note kept to the track's scale (`lock snap`: a key outside it plays the nearest note in it, the lower one on a tie) |

Triggers:

| key | note | action |
|---|---|---|
| C | 36 | `play kruido` — noise that climbs for as long as it is held |
| C# | 37 | `play kimpact` — a crash of noise, on the beat |
| D | 38 | `mute drums` — out while held, back on the one |
| D# | 39 | `mute bass` — out while held |
| E | 40 | `cut` — the whole mix under a low-pass, harder the harder struck |
| F | 41 | `sweep` — a high-pass climbing for as long as it is held |
| F# | 42 | `hold kfill` — a snare roll in straight sixteenths while held |
| G | 43 | `throw` the lead (whichever the scene plays) into the echo |
| G# | 44 | `freeze` the reverb |
| A | 45 | `tapestop` — the whole mix slows to a stop |
| A# | 46 | the track's toggle (perc, shaker, clap, ride… its rig) |
| B | 47 | the track's throw: its main voice into the echo (its rig) |

Scenes, from the computer's keys only, each landing on the next bar:

The set starts in the first scene, intro, so the strip and the wheel answer
from the first bar. The strip and the wheel follow the hands. **Playing** means a key of the
lead zone is the newest held: they move the sound you play. **Hands off**
(no key held, `idle`): they move the song.

| key | scene | bass zone | lead zone | strip, playing | strip, hands off | wheel, playing | wheel, hands off |
|---|---|---|---|---|---|---|---|
| F1 | intro | `bass` roll | `kpluck`, a dark FM pluck | to the note 2 degrees away, in the scale | the song's bass line bends, up to an octave | **filter**: the pluck opens, 2.5 → 14 kHz | the song goes under (DJ low-pass to 1.2 kHz) |
| F2 | build | `bass` roll | `kzap`, the falling zap | ±24 semitones: dives | the song's bass line falls an octave | **FX**: a crusher on the zap, 0 → 70% | the floor thins (high-pass to 1.5 kHz) |
| F3 | drop | `bass` roll | `klaser`, the laser | 2 degrees | the song's bass line bends | **vowel**: the laser talks, a → u | a DJ filter sweep (low-pass to 600 Hz) |
| F4 | break | `bass` roll **with its own kick** | `kvox`, a vowel pad | 1 degree | whatever bass is left dives an octave | **vowel**: the pad walks a-e-i-o-u | under water (low-pass to 400 Hz) |

A gesture belongs to what it started on: if you let go of the key mid-sweep,
the sweep stays on your sound until the wheel (or the strip) comes home, and
only the next one moves the song.

A bend in degrees goes from the key held to the note so many degrees of the
scale away: half way it slides, at the end it lands in the scale. Every
wheel range starts at the sound as written, so the wheel at rest changes
nothing; changing scene puts the old scene's wheel targets back there and
moves the new ones to where the wheel is.

The roll plays through the track's fader: in a step where `bass` is at level 0
(a breakdown, or T04's drop, where the gallop is the bass) a held bass key is
silent. Stage 3's roles will let a step say which track is its bass.

The set moves from the computer: space or → next, ← back, 1–9 a step on
screen, g the list. No pad or key of the controller moves it.
