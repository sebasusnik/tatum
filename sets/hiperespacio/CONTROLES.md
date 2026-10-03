# hiperespacio — the KeyLab layout

One layout for the whole hour: a fader, knob or pad does the same job in every
track, the way a DJ's rack does in Ableton. What it reaches changes with the
track (the bass of T04 is a 303, the bass of T08 a three-oscillator gallop), what
it *means* does not. `_base.synth` maps what every track shares; each `_tNN.synth`
maps the rest onto its own voices with a `midi` block after its `use`.

## Faders (left to right)

| # | cc | role | notes |
|---|---|---|---|
| F1 | 73 | **drums** | the main kit track's level |
| F2 | 75 | **perc / hats** | the second drum layer (rims, toms, ticks, shaker) |
| F3 | 79 | **bass** | the rolling bass |
| F4 | 72 | **main voice** | the track's protagonist: acid, pluck, robot, laser, oud, screamer… |
| F5 | 80 | **second voice** | arp, pad, drone, haze — whatever plays under/against the main voice |
| F6 | 81 | **signature** | the grinder / turbine / zaps / shard — its level (its pads are below) |
| F7 | 82 | **reverb** | `reverb_mix` (base) |
| F8 | 83 | **echo** | `delay_mix` (base) |
| F9 | 85 | **DJ filter** | centre open, down dark, up thin (base) |

## Knobs (left to right)

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

Bank A (36–43), performance:

| pad | note | action |
|---|---|---|
| A1 | 36 | `mute` drums — kick out while held |
| A2 | 37 | `toggle` perc/hats layer |
| A3 | 38 | `throw` main voice into the echo |
| A4 | 39 | `freeze` the reverb (base) |
| A5 | 40 | `repeat 1/8` — DJ roll |
| A6 | 41 | `repeat 1/16` — tighter roll |
| A7 | 42 | `prev` step (base) |
| A8 | 43 | `next` step (base) |

Bank B (44–51), the track's sounds:

| pad | note | action |
|---|---|---|
| B1 | 44 | `play` signature — hold it and the grinder/turbine/drone sounds; K3 moves its RPM |
| B2 | 45 | `hold` riser — a sweep/riser figure while held |
| B3 | 46 | crash (drum hit) |
| B4 | 47 | `hold` second signature figure (zaps, shard, bell phrase, oud cell…) |
| B5 | 48 | `repeat 1/32` — the stutter before a drop |
| B6 | 49 | `throw` signature into the echo |
| B7 | 50 | free per track |
| B8 | 51 | free per track |

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
