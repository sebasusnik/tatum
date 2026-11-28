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
| `fm/pluck.synth` | `pluck` | a dry two-operator FM pluck, a detail layer |
| `keys/pad.synth` | `pad` | a dark poly pad whose filter breathes over four bars |
| `drums/machine.synth` | `machine` | a dry machine kit; lanes kick, snare, clap, hat, ... |
| `kicks/boom.synth` | `boom` | a long round boom in the spirit of an 808: a sine falling from 190 Hz, 21 semitones up, onto A1, 60 dB down at 920 ms |
| `hats/metal_closed.synth` | `metal_closed` | a closed hat in the spirit of a 606: six square waves, a band at 8 kHz, a 45 ms decay that is 60 dB down at 96 ms |
| `hats/metal_open.synth` | `metal_open` | the same metal, open: 60 dB down at 320 ms, however short the note |
| `keys/chorus_pad.synth` | `chorus_pad` | a warm poly pad in the spirit of a Juno-106: two saws, a breathing lowpass, a wide stereo chorus |
| `bass/rubber.synth` | `rubber` | a rubbery bass in the spirit of an SH-101: saw and sub through a resonant ladder that snaps shut |

The ones "in the spirit of" a machine are original sounds made with that machine's
idea, not copies of it. `vintage_kit.synth`, here beside the folders, plays them
together: `tatum play sounds/vintage_kit.synth`. The hats are quieter on an engine built
under 44.1 kHz (`TATUM_SAMPLE_RATE`): 2.5 dB at 32 kHz, 9.5 dB at 22.05 kHz.

Each file is made, and kept, with the designer:

```
tatum design sounds/kicks/deep.synth
```

It opens in the browser with a knob for every argument of every node, or
every parameter of a module, and plays the sound in a loop as they turn: a
single hit, four on the floor, a held note or a chord, on the note you pick (a
drum kit plays a lane, or a groove). It draws one hit's pitch and level over
time and lays a record's averaged hit next to it. A graph's nodes are added,
removed and wired with buttons. The library is listed beside it: open any
sound here, or duplicate the one open to a new name (`keys/pad-dark` plays as
`pad_dark`) and make it your own. Save writes the file back in the same text
anyone can read and diff, and only when it compiles. A file that does not
exist yet starts as a deep kick.

The name is what a song plays it by, so two sounds a song uses together need
two names.
