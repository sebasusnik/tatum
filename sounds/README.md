# sounds

One instrument per file, with a comment on top that says what it is. A song
plays one with `use`, resolved from the song's own folder:

```
use "../sounds/kicks/deep.synth"

pattern four { C3 - - -  C3 - - -  C3 - - -  C3 - - - }
track kick { play four using deep_kick }
```

Each file is made, and kept, with the designer:

```
tatum design sounds/kicks/deep.synth
```

It opens in the browser with a knob for every argument of every node, plays
the sound in a loop as they turn, draws its pitch and level over time, and
lays a record's averaged hit next to it. Save writes the file back here, in
the same text anyone can read and diff. A file that does not exist yet starts
as a deep kick.

The instrument's name is what a song plays it by, so two sounds a song uses
together need two names.
