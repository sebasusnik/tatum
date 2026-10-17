# compose — a loop to try voices in, a song as loops

```
tatum compose                      # this directory's session.synth
tatum compose --midi keylab        # the KeyLab's eight knobs on the chosen row
```

A groove plays. Each row holds one voice from the library, chosen with the
arrow keys and swapped on the next bar line; a role can have several rows, a
sub and a reese together. The chosen row's knobs turn from the keyboard or
the controller. A voice that sounds right is saved as it sounds, as a
`.synth` of its own, and any song gets it with one `use` line.

```
compose/
  session.synth          the scratch loop: one `use` line per groove and voice
  grooves/*.synth        tempo, scale, sends, master and a `drums` track
  voices/<role>/*.synth  its module, its patterns in scale degrees, a track named as the voice
  sets/<song>/NN-<part>.synth  a song as loops, in order: each one a loop like session.synth
```

A loop's file is written by the screen and watched like any other: edit it
by hand too, and add anything a song can say under the `use` lines (a
`track strings { level 0.22 }`, a `midi { }` block); those lines are kept. A
row playing another of its voice's patterns is a line of its own,
`track drums { play amen_chop }`.

## A song as loops

A set is a directory of loops, `sets/jungle/05-drop1.synth`, each a loop
like the scratch one whose `use` lines reach the library two directories up
(`use "../../grooves/jungle.synth"`). The browser lists them in order; a key
swaps to that loop on the next bar, and DUP copies the loop open as the next
one, to build a part from the one before. `sets/jungle` is
`examples/jungle_93.synth` taken apart: a loop per scene, a voice per part.

## A voice

```
# One line that says what it is, for the screen
module bass sub_dub { cutoff 165hz ... }
pattern sub_dub_line { 1.1:0.8 ..*7 -*8 }
pattern sub_dub_half { 1.1:0.8 ..*15 -*16 }
track sub_dub {
    play sub_dub_line
    using sub_dub
    level 0.9
    out > saturate(0.3) as drive > lowpass(700hz, 0.1) as lp > master
}
```

- The track is named as the voice, so two voices of a role play side by
  side and a loop never defines a track twice; the role is the directory.
- Every pattern the file writes is a variation the row chooses from, the
  one its track plays first.
- A voice can be layers: several modules and tracks, a bus between them
  (`voices/lead/hoover.synth`). The first track is the row's; the others
  mute, solo and change pattern with it.
- The pattern is in degrees (`1.1`, `5.2`, `[1.3 3.3 5.3]`), so the voice
  follows whatever `scale` the groove sets.
- Every node written `as <name>` gets a knob for its `wet`. Name the ones
  worth a hand: the drive, the filter, an effect that waits at `wet=0`.
- The module's name is the voice's, and so is the track's. Saving under a
  new name renames both, so a song can `use` two saved variants side by side.

## The knobs

The first five are the module parameters a hand reaches for first --
`cutoff resonance cutoff_env decay glide` on a bass, `mod_index feedback
decay release lfo_depth` on an FM voice, `cutoff resonance attack release
detune` on keys, the kick's decay, pitch and drive on a groove -- then the
level and the two sends, then the wet of every node the chain names. On a
controller they are knobs 1 to 8 (`--knobs 74,71,...` says which numbers;
the KeyLab Essential mk3's are the default).

A knob turned is held on the engine and not written anywhere until `s`
saves the voice: then every value goes into the file, in units, where the
voice writes it. `a` compares with the text's values; `x` puts the row's
knobs back.
