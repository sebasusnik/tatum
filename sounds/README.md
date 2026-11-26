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
