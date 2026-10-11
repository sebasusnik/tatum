# Reading and auditing a mix

What `tatum render` and `tatum audit` report, and how to read it. The commands and their
flags are in [CLI.md](CLI.md).

## Reading a mix

`tatum render` prints what the render measures. Levels answer "is anything
buried"; the three columns and two blocks after them answer the questions that
decide whether a song sounds good, and all three used to be worked out by hand.

```
  track          peak      rms   rms dB  peak dB  crest  width  band
  drums         0.484   0.0540     +0.0     -1.5    9.0     0%  low
  chords        0.278   0.0320     -4.5     -6.3    8.7    13%  mid
  shine         0.243   0.0228     -7.5     -7.5   10.6    32%  mid

sharing a band:
  low    drums +0.0, bass -3.1, under -5.9   <-- within 6 dB of each other
  mid    chords -4.5, shine -7.5, chime -8.8, stabs -12.4   <-- within 6 dB

sections:
  intro          8 bars   -23.1 dB  ##
  peak          16 bars   -17.2 dB  ########################
  outro          8 bars   -23.7 dB
  arc: 6.5 dB between the quietest section and the loudest
```

**`width`** is how much of a track is *not* in the middle. Everything at zero
is why instruments cannot be told apart however carefully their levels are
set — `pan`, `autopan` and `chorus_mix` are what move it. It is measured
full-band, so on a drum track the mono kick holds it near zero whatever the
hats are doing.

**`sharing a band`** is the same problem in frequency. Two instruments in one
band mask each other, and a column reading `mid` five times does not make that
jump out. Within 6 dB of each other is where one stops sitting clearly behind
the other; further apart is a mix decision and is listed without a marker.

**`sections`** is the shape. A drop that measures the same as the breakdown
before it is flat however good the parts are, and a table that averages the
whole render into one row per track cannot show it. Under 3 dB of arc gets
said out loud.

None of this is what `tatum audit` measures, and the two do not substitute for
each other. The audit asks whether a *voice* is dirty. A song can be perfectly
clean by that measure and still sound wrong, which is the usual case.

## Auditing a mix

`tatum check` reads the text. `tatum audit` listens to the render: it plays
every tonal track on its own and dry, and measures each note for energy that
is **not** at a harmonic of the note the pattern asked for — which is what a
detuned copy of a voice sounds like, whether the detune came from a chorus, a
saturator or an oscillator.

```
track         notes     median      worst    at bar   note    peak
bass            597   -12.7 dB    -5.0 dB     37.75    C#3   0.322
bell             25   -49.9 dB   -47.7 dB     83.75    F#5   0.266

Switched off one at a time, on the same voice:
  the chorus on `bell` accounts for 16.7 dB of its inharmonic content
```

**The absolute number means nothing next to another track's.** A saturated saw
reads dirtier than a bell however clean both are, so `median` is a fingerprint
of a timbre rather than a score, and the rows come out alphabetical to make
ranking them awkward on purpose.

Two comparisons *are* meaningful, and they are the two the tool reports:

- **A note against the other notes of the same voice.** One note far above the
  rest of its own voice is worth going to listen to. The bar is printed for
  that reason.
- **The same voice with and without one effect.** Both renders are the same
  timbre, so whatever the difference is, the effect caused it. This needs no
  threshold that has to travel between sounds, and it is the one that works on
  an effect that dirties a whole voice evenly — which is the shape the chorus
  bug turned out to have. It did not make one note bad; it raised the whole
  bell by 17 dB, and the ear caught it on the first note because a clean bell
  has nothing to hide it behind. `chorus` and `drive` are the suspects, and a
  track only gets a second render if it has one.

`--bars N` audits only the first N bars, which is six times faster and enough
to tell whether a voice is clean. `--json` prints the same numbers for a
script. `--strict` exits 1 if anything is reported.

### The net under the engine

Three songs have their numbers checked in at `cli/tests/audit_baseline.txt`,
and a release-only test fails if any of them moves more than 1.5 dB. Nothing
else in the suite would notice a filter that began ringing or a saturator that
got hotter: the bit-identity tests compare a render against the same code, the
callback-budget test measures time, and the lints read text.

```
UPDATE_AUDIT_BASELINE=1 cargo test --release -p tatum-cli --test audit_baseline
```

regenerates it — and then you read the diff. Three songs and not thirty on
purpose: thirty would take twenty-five minutes and would make regenerating the
file routine, which is how a baseline stops being read.

The other half of the answer is that **the audit is how you find a rule and a
lint is how you enforce it**. `chorus_beats` catches from the text, in
milliseconds and on every song, the same problem the audit took a minute to
find on one. Anywhere the audit turns up a class of fault twice, the move is
to try to make it a lint.

Four things it does on purpose, each of which cost a wrong diagnosis first:

- **Per note, not per song.** Averaged over three minutes, the track with the
  bug was the *cleanest* in the mix.
- **Soloed and dry.** In the mix everything masks everything — and with the
  sends open, a reverb tail puts the previous note inside this note's window,
  so the tool reported *more* dirty notes after the bug was fixed than before.
- **Absolute energy, not a percentage.** A window with a huge *share* of harsh
  energy is usually just a quiet window.
- **The per-note check steps back where a window cannot hold one clear
  pitch.** An arp runs below the step, so a window that fits inside a step
  still holds several of its notes. Vibrato deep enough to matter moves the
  upper harmonics out of their bands — half a semitone at C#6 moves the eighth
  harmonic 107 Hz. And a note below about 110 Hz has its harmonics closer
  together than the ±45 Hz bands they are measured in, so a sub can never be
  judged this way. Without those three rules the check reported 30 "dirty"
  notes on a sub, 24 on a low organ and 8 on an arpeggio — always the same
  handful of pitches over and over, which is the signature of a measurement
  being wrong about a pitch rather than a note being wrong. **The
  with-and-without comparison is unaffected by all of it**, because it is the
  same voice both times.

## Measuring a reference

A model writing a song cannot hear a record named as a reference, and it cannot hear
its own render either. `tatum analyze record.wav` (`tatum_analyze`) measures it, and
`tatum compare record.wav song.synth` (`tatum_compare`) measures the song the same way
and says what differs by enough to hear, each with what in the file moves it. Ask for a
WAV of the record rather than writing from its name.

How to read the two:

- **The kick's tail** is the note its pitch sweep lands on, in Hz and as a note. It is the
  number to tune first: a kick a semitone or two off the record's never sounds like it,
  whatever else matches. On an instrument kick it is `pitch_osc`'s end frequency; on
  `beats`, `kick_pitch` moves the whole sweep.
- **The sweep** is how long the pitch takes to land, and **the decay** how long the body
  takes to fall 12 and 24 dB. The comparison gives each as a factor on what the file
  already says: a sweep's time follows how far `pitch_osc`'s decay is from 1, a body's
  follows `perc`'s decay.
- **Under it** is how loud the kick's band is late in the beat, when the kick has died.
  Above about -10 dB something else fills it, usually a held bass, and the tail and the
  decay are partly that: measure a stretch where the kick plays alone (`--from`/`--to`).
- **Peak to loudness** is density. A loud club master sits well under 10 dB; the songs
  in `examples/` measure 12-17. The engine matches loudness and leaves density to the
  song: saturate or clip the kick and the drums, compress the drum bus.
- **The spectrum** is per octave, against the whole, so it compares a record at -8 LUFS
  with a song at -18. Three dB off in a band is audible; `tatum debug` says which tracks
  sit there.
