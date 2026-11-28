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
