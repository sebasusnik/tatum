# compose — a loop to try voices in

```
tatum compose                      # this directory's session.synth
tatum compose --midi keylab        # the KeyLab's eight knobs on the chosen row
```

A groove plays. Each role holds one voice from the library, chosen with the
arrow keys and swapped on the next bar line. The chosen row's knobs turn from
the keyboard or the controller. A voice that sounds right is saved as it
sounds, as a `.synth` of its own, and any song gets it with one `use` line.

```
compose/
  session.synth        the loop: one `use` line per groove and voice
  grooves/*.synth      tempo, scale, sends, master and a `drums` track
  voices/<role>/*.synth  one module, its pattern in scale degrees, a track named by its role
```

The session file is written by the screen and watched like any other: edit
it by hand too, and add anything a song can say under the `use` lines (a
`track bass { play other }`, a `midi { }` block); those lines are kept.

## A voice

```
# One line that says what it is, for the screen
module bass sub_dub { cutoff 165hz ... }
pattern sub_dub_line { 1.1:0.8 ..*7 -*8 }
track bass {
    play sub_dub_line
    using sub_dub
    level 0.9
    out > saturate(0.3) as drive > lowpass(700hz, 0.1) as lp > master
}
```

- The track is named by its role (`bass`, `lead`, `pad`, `perc`), so one
  voice per role is in the loop at a time and a session never defines a
  track twice.
- The pattern is in degrees (`1.1`, `5.2`, `[1.3 3.3 5.3]`), so the voice
  follows whatever `scale` the groove sets.
- Every node written `as <name>` gets a knob for its `wet`. Name the ones
  worth a hand: the drive, the filter, an effect that waits at `wet=0`.
- The module's name is the voice's. Saving under a new name renames it, so
  a song can `use` two saved variants side by side.

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
