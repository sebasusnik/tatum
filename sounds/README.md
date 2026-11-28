# sounds

One sound per file -- one `instrument` graph or one `module` (`beats`, `bass`,
`fm`, `keys`) -- kept as `sounds/<type>/<name>.synth`, with a comment on top
that says what it is. A song plays one with `use`, resolved from the song's
own folder, by the instrument's or module's name:

```
use "../sounds/kicks/deep.synth"
use "../sounds/bass/roll.synth"

pattern four { C3 - - -  C3 - - -  C3 - - -  C3 - - - }
pattern off  { - - A1 -  - - A1 -  - - A1 -  - - A1 - }
track kick { play four using deep_kick }
track bass { play off using roll }
```

What is here:

| file | plays as | what it is |
|---|---|---|
| `kicks/deep.synth` | `deep_kick` | a deep held kick on 44.5 Hz, fitted to a minimal record |
| `bass/roll.synth` | `roll` | a dark rolling offbeat bass: two saws, a short filter pluck |
| `bass/abyss.synth` | `abyss` | a dark Oberheim-style bass graph: cross-mod, PWM, an `svf` swept through its `cutoff` port |
| `fm/pluck.synth` | `pluck` | a dry two-operator FM pluck, a detail layer |
| `keys/pad.synth` | `pad` | a dark poly pad whose filter breathes over four bars |
| `drums/machine.synth` | `machine` | a dry machine kit; lanes kick, snare, clap, hat, ... |
| `kicks/boom.synth` | `boom` | a long round boom in the spirit of an 808: a sine falling from 190 Hz, 21 semitones up, onto A1, 60 dB down at 920 ms |
| `hats/metal_closed.synth` | `metal_closed` | a closed hat in the spirit of a 606: six square waves, a band at 8 kHz, a 45 ms decay that is 60 dB down at 96 ms |
| `hats/metal_open.synth` | `metal_open` | the same metal, open: 60 dB down at 320 ms, however short the note |
| `keys/chorus_pad.synth` | `chorus_pad` | a warm poly pad in the spirit of a Juno-106: two saws, a breathing lowpass, a wide stereo chorus |
| `bass/rubber.synth` | `rubber` | a rubbery bass in the spirit of an SH-101: saw and sub through a resonant ladder that snaps shut |
| `hats/tick.synth` | `tick` | a closed hat in one `hat` node: six partials and a fifth of noise in a band at 9 kHz, 60 dB down at 45 ms |
| `snares/crack.synth` | `crack` | a dry snare in one `snare` node: a body on 200 Hz under wires at 5 kHz, 60 dB down at 170 ms |
| `claps/room.synth` | `room` | a clap in one `clap` node: four bursts 11 ms apart at 1.3 kHz, 60 dB down at 293 ms |
| `toms/low.synth` | `low_tom` | a low tom in one `tom` node: a sine dropping onto G2, 60 dB down at 345 ms |
| `liquid/bubbles.synth` | `bubbles` | one held note is a stream of swells off the grid, with no click: a `dust` moves a resonance over a saw's harmonics and, smoothed by slow filters, swells its level |
| `liquid/drop.synth` | `drop` | a long falling drop, one thin line: a sine and a faint twin fall four octaves onto the note, ever slower, under a 21 Hz vibrato; no two notes fall alike |
| `liquid/tide.synth` | `tide` | a liquid bass that slides into a note written `~note` in 140 ms, through a lowpass that wanders and never repeats |
| `liquid/shoal.synth` | `shoal` | a pad that is a cluster: five saws at 0, ±9 and ±20 cents, a 4.5 Hz vibrato, one slow `wander` on every voice's filter |
| `liquid/glass.synth` | `glass` | a soft FM bell, held: partials at 1.5, 2.5, 3.5... times the note, in pairs 10 Hz apart |

The ones "in the spirit of" a machine are original sounds made with that machine's
idea, not copies of it. `vintage_kit.synth`, here beside the folders, plays them
together: `tatum play sounds/vintage_kit.synth`. `liquid_kit.synth` does the same for the
five in `liquid/`, which are built on the random sources (`rand`, `wander`, `dust`; see
"Bubbles, drops and sweeps that never repeat" in docs/DSL.md): every note of them differs
from the last, and the piece is still the same samples on every render. The hats are quieter on an engine built
under 44.1 kHz (`TATUM_SAMPLE_RATE`): 2.5 dB at 32 kHz, 9.5 dB at 22.05 kHz.

`node_kit.synth` plays the kit made of one node a piece -- `kicks/deep`, `hats/tick`,
`snares/crack`, `claps/room` and `toms/low` -- together: `tatum play sounds/node_kit.synth`.
Each of those files is a single line that says what is heard, in Hz and ms, and the
comment over it says what was measured. Measured at the node on an engine built at 44.1,
32 and 22.05 kHz (`TATUM_SAMPLE_RATE`), one hit: the kick and the tom peak the same at
all three; `tick` has the same rms at all three and peaks at 0.47, 0.37 and 0.40 of full
scale; `crack` peaks at 0.96, 0.97 and 0.95, its rms 0.5 dB lower at 32 kHz and 0.9 dB
lower at 22.05; `room` peaks at 0.92, 0.83 and 0.99 (1.0 dB under and 0.6 dB over what it
does at 44.1 kHz, 1.6 dB between the two lower rates), its rms 0.2 and 0.6 dB lower.

Each file is made, and kept, with the designer:

```
tatum design sounds/kicks/deep.synth
```

It opens in the browser with a knob for every argument of every node, or
every parameter of a module, and plays the sound in a loop as they turn: a
single hit, four on the floor, a held note or a chord, on the note you pick (a
drum kit plays a lane, or a groove). It draws one hit's pitch and level over
time and lays a record's averaged hit next to it. A graph's nodes are added,
removed and wired with buttons. A wire can land on a node's port (`sweep >
filt.cutoff`): the list shows it so, the "to" box offers each node's ports
after the node, and Save writes it back as it was. The library is listed beside it: open any
sound here, or duplicate the one open to a new name (`keys/pad-dark` plays as
`pad_dark`) and make it your own. Save writes the file back as text anyone can
read and diff, and only when it compiles. It writes the comment on top and the
one definition, in its own spelling (`1.0` as `1`, `1.2s` as `1200ms`, `3khz`
as `3000hz`, the comment wrapped again), so the first Save of a file made by
hand can change how it is written, never what it plays; and a file with more
in it (a song, a comment inside the definition) is not opened at all. A file that does not
exist yet starts as a deep kick.

The name is what a song plays it by, so two sounds a song uses together need
two names.
