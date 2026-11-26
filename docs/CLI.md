# The command line

`tatum <command>`; `tatum help` prints the short version of this page. With cargo,
`cargo run --release -p tatum-cli -- <command>`.

| command | what it does |
|---------|--------------|
| [`check`](#check) | compile a song and report errors and design warnings, without sound |
| [`render`](#render) | render a song to a WAV, with a report of the mix |
| [`play`](#play-and-watch) | play a song on the audio device |
| [`watch`](#play-and-watch) | play it, and play every save of the file on the next bar |
| [`debug`](#debug) | render every part apart, with spectrograms and a report of clicks and mud |
| [`audit`](#audit) | measure every note of every tonal track for what is not its harmonics |
| [`analyze`](#analyze-and-compare) | measure a reference track: tempo, key, the kick, the spectrum, density |
| [`compare`](#analyze-and-compare) | a reference and your song side by side, and what in the file moves each difference |
| [`design`](#design) | design a sound in the browser, against a record's kick, and save it into `sounds/` |
| [`set`](#set) | render, check, gate and play a live set |
| [`tui-shot`](#tui-shot) | a picture of the live screen at any second |
| [`params`](#params) | every module parameter, its range and units |
| [`fmt`](#fmt) | rewrite knob positions in their units |

Every command that reads a song resolves its `use "..."` lines first, so a song
built on a rig compiles as one file.

## check

```
tatum check song.synth
```

Parses and compiles, and prints what is wrong with the line it is on and a suggestion.
Then the design warnings: things that compile and will not sound the way they read,
like a track ducked by a sidechain in a scene with no kick. They are listed in
[DSL.md](DSL.md#design-warnings). It exits 1 on an error, 0 with warnings.

## render

```
tatum render song.synth [-o out.wav] [--bars N] [--solo a,b] [--mute c]
```

Renders the whole arrangement to a WAV (`output.wav` by default; `--bars N` renders
only the first N bars).
Every song leaves at -18 LUFS with its true peak limited at -1 dB, after the master
chain. On the way it prints the mix: each track's peak, RMS, crest, width and band, the
tracks that share a band at a similar level, each section's loudness against the
loudest, and what the master chain did to the peaks. At the end it lists what it heard
that sounds wrong: clicks, tracks that do not go quiet between notes. How to read that
report is in [MIX.md](MIX.md#reading-a-mix).

## play and watch

```
tatum play  song.synth [--tui] [--device <name>] [--rate <hz>] [--midi <name>] [--buffer <frames>]
tatum watch song.synth [--tui] [--device <name>] [--rate <hz>] [--midi <name>] [--buffer <frames>]
tatum midi monitor [<song.synth> | <set dir> [--step N]] [--midi <name>]
tatum midi list
```

`play` plays the song on the default output device until it ends or you type `q`.
`watch` does the same and plays every save of the file. A value edit applies at once.
Anything else takes over on the next bar, keeping every tail and voice the edit did not
touch, and a save that does not compile is reported while the last good version keeps
playing. A song without `scene` and `arrange` loops: that is a live set in one file.
See "Livecoding semantics" in [DSL.md](DSL.md#livecoding-semantics).

- `--device <name>` picks the output device; `--list-devices` lists them.
- The engine renders at 44.1 kHz. A device that only offers another rate, as every
  Bluetooth headphone does at 48 kHz, gets the output resampled; `--rate` forces one.
- A MIDI controller plays the song through its `midi { }` block: knobs and faders on
  parameters, the keys on a track, the pads on drums. Every MIDI input is read unless
  `--midi` names one; `--list-midi` lists them. See [DSL.md](DSL.md#midi-knobs-keys-and-pads).
- `--tui` runs full screen; `--glass` is the same for a terminal with translucent cells.
  See [TUI.md](TUI.md).
- `--buffer 1024` asks the device for that many frames a callback, for headroom when
  its default leaves too little.
- `tatum midi monitor` prints every message a controller sends (notes, pads, knobs, the
  pitch strip, aftertouch, program changes) with its channel and bytes, and, given a
  song or a set, what its `midi` block, zones and trigger keys do with it: the way to
  find out what a controller really sends before mapping it. `i` on the live screen
  shows the same.

On exit both print the worst audio callback time against its budget, and how many
dropouts the device reported.

## debug

```
tatum debug song.synth [--bars N | A-B] [--dry] [-o dir] [--solo a,b] [--mute c]
```

For finding where a noise comes from. It renders once with every track, bus and send
return kept apart, each as it sits in the mix, and writes into
`test_output/debug/<song>/` (or `-o`):

- a WAV and a spectrogram per part;
- `sheet.png`, all of them stacked on one time axis over the mix, with a "crowded"
  strip where three or more parts pile up on the same range;
- `report.txt`: clicks (and which fall on a section change), tracks that do not go
  quiet between notes, energy below 25 Hz and fizz above 16 kHz, each with its bar,
  and per section each part's level against the loudest and which parts sit level
  with each other on top of the same range;
- `zoom.*.png`, the worst moment of each part up close, its wave and its spectrum.

`--bars 17-24` zooms in on those bars; `--dry` adds each track before its insert chain.

![sheet.png for dark_techno: each part's spectrogram on one time axis, sections on top, clicks marked, the crowded strip and the mix](img/debug-sheet.png)

## audit

```
tatum audit song.synth [--bars N] [--json] [--strict]
```

Renders every tonal track on its own and dry, and reports per note how much of its
energy is not at a harmonic of the note the pattern asked for. That finds two things:
a note going wrong against the other notes of the same voice, and an effect dirtying a
whole voice, by rendering it with and without each effect. The absolute number is a
fingerprint of a timbre and means nothing next to another track's. `--json` prints the
same numbers for a script; `--strict` exits 1 if anything is reported. More in
[MIX.md](MIX.md#auditing-a-mix).

## analyze and compare

```
tatum analyze reference.wav [--from 1:04] [--to 1:36]
tatum compare reference.wav song.synth [--from S] [--to S] [--bars N | A-B] [--solo a,b] [--mute c]
tatum compare reference.wav other.wav
```

A record named as a reference is a word to whoever writes the song, and more so to a
model, which cannot hear it or its own render. `analyze` turns it into numbers: the
tempo (and its span when it moves), the key, the notes under 250 Hz, the loudness and
how far the peaks sit over it, the width above 250 Hz and under 150 Hz, the spectrum per
octave against the whole, and the kick -- the frequency it hits at, the note its sweep
lands on, how long the sweep takes, how long the body holds and then falls (all timed
from the hit), how much click, and how much else fills its band. It reads a WAV from 8 to 384 kHz, PCM or float; an MP3 has to be converted
first (`ffmpeg -i song.mp3 song.wav`). `--from`/`--to` take a stretch, in seconds or
`m:ss`. At most 15 minutes are measured at once, enough for a long extended mix; a longer
file, such as a DJ set, is refused with a pointer to `--from`/`--to`, and only the stretch
picked is ever converted.

What it says about `dark_techno` rendered, the engine's own song:

```
dark_techno.wav: 300.7 s
  tempo        137.0 BPM (137..150)
  key          F minor / G# major, weakly: little in it is tonal
  low notes    D# 19%, E 14%, G 10%   (under 250 Hz, the kick's tail included)
  loudness     -18.0 LUFS, peak -1.2 dBFS, peak to loudness 16.8 dB
  width        3% above 250 Hz, 0.1% under 150 Hz
  kick         1016 hits, measured on the 50 with the least under them
    hit        89 Hz (the first half cycle)
    tail       39.0 Hz (D#1)
    sweep      33 ms to within 10% of the tail
    hold       none: it falls from the hit
    decay      -12 dB at 95 ms, -24 dB at 111 ms
    click      -32.1 dB (first 10 ms over 2 kHz against the body)
    under it   -45.6 dB (the band under 400 Hz late in the beat)
  spectrum     31.5    63   125   250   500    1k    2k    4k    8k   16k
  (dB)         -2.2  -7.0 -10.9 -12.6 -16.7 -19.5 -22.9 -21.1 -21.3 -22.6
```

The kick is measured on every hit averaged together, so the bass, the pads and the reverb
under its tail cancel and the kick is left; and only on the hits with the least else in
its band late in the beat, which in a track made for DJs are the intro and the outro,
where the kick plays alone. A bass that starts with every kick is locked to it and
survives the average; when one fills the band throughout, the report says so and the
tail and the decay are not to be trusted -- point `--from`/`--to` at a stretch where the
kick plays alone. The tail is measured on the stretch where the kick is within 12 dB of
its peak, and when the sweep is still falling there, from where it is heading: a pitch
sweep is an exponential towards its end note. A sweep that lands in a few milliseconds
is quicker than a cycle, so `hit` reads lower than where it starts. The key comes with
its relative, which has the same notes; the two cannot be told apart by the notes alone.

`compare` measures your song the same way (rendered as `render` renders it, or a second
WAV) and prints both side by side, then what differs by enough to hear, each with what in
the file moves it: the note to put at the end of `pitch_osc`'s sweep, the factor for the
sweep's decay and the body's `perc`, the octave bands to take down or bring up, the
density, the low end's width, the tempo. Loudness itself is not compared: every song
leaves the engine at -18 LUFS and a record is mastered far louder. How far the peaks sit
over the loudness is, because that is the density a master hears as punch, and it comes
from saturating, clipping and compressing the drums, not from the level. `--solo drums`
compares the kit alone against the record's kick; `--bars` picks the stretch of the song,
counted as `debug` counts it, each scene at its own tempo, and a first bar past the end is
an error. `--bars`, `--solo` and `--mute` pick parts of a song, so with a second WAV they
are an error rather than ignored; that WAV is measured whole.

## design

```
tatum design sounds/kicks/deep.synth [--port 7350] [--no-open]
```

A sound designer in the browser, served from this machine (127.0.0.1 only). A sound
is a file with one `instrument` in it and a comment on top that says what it is; a
file that does not exist yet starts as a deep kick, and Save creates it.

The page has a knob for every argument of every node of the instrument, from the same
table `params` prints and the compiler checks against, so whatever the sound is built
from gets its knobs; a knob's × takes the argument out of the file, back to its
default. Each turn is rendered at once -- one hit, the instrument alone, with no
master chain and no loudness -- and played in a loop at the tempo given. Beside the
knobs, the hit's pitch over time on a log scale and its level in dB, and the figures
`analyze` reports for a kick: the tail and its note, the sweep, the hold, the falls.

A record can be laid next to it. Load a WAV, give the stretch (an intro or an outro
where the kick plays alone or nearly) and how many milliseconds of each hit to keep
(as many as come before an offbeat bass); its hits are averaged on the beat grid, so
what comes with every hit stays and the rest thins out. The two are drawn on the same
axes, and the loop can play the record's hit, yours, or the two in turn.

Once a record is loaded, **Fit to reference** sets the kick node's knobs to what
`tatum kick fit` finds for it (see below), and says what error is left.

The file's text is on the page too: edit it to add a node or rewire, apply, and the
knobs follow. The text is the sound: a song plays it with
`use "../sounds/kicks/deep.synth"` and `play ... using deep_kick`.

It answers only its own page. Any site open in the same browser can send a request to
127.0.0.1, so a request whose `Host` or `Origin` is not this server, or a POST without
the page's own header, is refused (403), and Save writes only the file it was started
on. A WAV is taken up to 400 MB; a request that stops sending is let go after 30
seconds.

## kick fit

```
tatum kick fit <reference.wav> [--from S] [--to S] [--bpm N] [--ms 230] [--effort 1] [--hit hit.wav]
```

The `kick(...)` line whose hit matches a record's kick. The record's hits between
`--from` and `--to` (seconds or `m:ss`) are averaged on its beat grid, the first `--ms`
of each: pick a stretch where the kick plays alone or nearly, an intro or an outro, and
only as many milliseconds as come before an offbeat bass. The tempo is measured unless
`--bpm` gives it; a tempo measured a hundredth of a BPM off still averages cleanly,
since every hit is aligned again on its own attack.

The node's note, start, knee, snap, chirp, body, hold, sag, release, phase and click are
then fitted to that average: its pitch in semitones on an even grid from 0.6 to 215 ms
(the attack, to 10 ms, counted again on its own) and its level in dB. After the first
10 ms both are read through an 800 Hz low-pass, so hats on the beat are not taken for
the kick, and the pitch is only compared while the record's hit is within 30 dB of its
peak. The search starts from what the curves say (the tail, where the sweep starts, the
hold, the fall) and from four guesses around it; the click comes last, as the one whose
whole score is best, so the highs a master adds are reported, not chased.

The line is then rendered by the whole engine, measured against the record again, and
refined while that render is further off than the node alone was. It prints the line,
ready to paste, the error left, and the two hits side by side:

```
$ tatum kick fit outro.wav --from 5:40 --to 6:10
outro.wav: hits averaged at 124.00 BPM, the first 200 ms of each
5389 kicks tried in 2.9 s
  round 1: the engine's render is off by 0.08 st in pitch (0.31 st over the attack), 0.03 dB in level, click +1.3 dB
kick(F#1, start=1.66khz, knee=172.3hz, snap=2.12ms, chirp=3.24khz, chirp_time=0.61ms, body=79.4ms, hold=95ms, sag=-1.9db, release=209.9ms, phase=6, click=-32.5db)
# left: pitch 0.08 st (attack 0.31 st), level 0.03 dB, click +1.3 dB (the record's has more)
```

`--hit` writes the averaged hit to a WAV, to hear what was fitted to. `drive=` and
`attack=` are not fitted: a record's saturation comes back as phase and click. In
`tatum design`, **Fit to reference** does the same to the sound's kick node once a record
is loaded, and its knobs move to the result.

## set

A set is a directory of numbered `.synth` files, each the whole rig at a moment,
walked in filename order. A step's header says how long it holds and what it is:
`# set: bars=32 phase=build energy=5`; every step is a song in the language of
[DSL.md](DSL.md), usually a rig (`use "_rig.synth"`) and what changes.

```
tatum set check  <dir> [--bars N] [--json]
tatum set render <dir> [-o set.wav] [--bars N] [--phrase 1] [--ramp 4] [--blend 0] [--perform script.txt]
tatum set next   <dir> <candidate.synth> [--json] [--max-voices N]
tatum set play   <dir> [--tui] [--auto] [--phrase 8] [--ramp 4] [--blend 0] [--device <name>] [--midi <name>] [--buffer <frames>]
tatum set controls <dir> [--step N] [--bars 2]
```

- **`check`** validates every step and reports the set's arc.
- **`render`** walks the set the way it would be played, with real hot swaps, tempo
  ramps and blends, into one continuous WAV, and prints where each step starts.
  `--perform` plays a script of knobs, pads, keys and step moves on top, for review
  audio without the hardware.
- **`controls`** tries every control of every step the way a hand would -- each knob
  on each page turned end to end, each pad and trigger key held, the strip and the
  wheel thrown, with the key or pad a voice needs held when it needs one -- and prints
  per step the ones that changed nothing (`NOTHING`) and how much the others did.
- **`next`** is the gate a proposed step has to pass against the last one: it says yes
  or no and why. It is meant for an agent writing a set step by step.
- **`play`** plays it live. The space bar, a key or a pad asks for the next step, which
  lands on the next phrase line (`--phrase`, 8 bars by default). `--auto` lets the set
  walk itself by its headers, `--ramp` is how many bars a tempo change takes, `--blend`
  mixes two steps the way a DJ does. `--tui` runs full screen, with the steps and their
  cues ([TUI.md](TUI.md)).

### Playing a set live

A set is a directory of numbered `.synth` files, each the whole rig at one moment
(`sets/` has five). `tatum set play <dir>` plays it with a controller in hand:

```
tatum set play sets/mine --phrase 8 --ramp 4
```

It starts on the first step. The space bar (or `n`, or →) asks for the next one, ←
for the one before, a digit for a step by number, and a pad can do the same with
`pad 48 > next`, `prev` or `step 3`; a song's `keyboard { }` block binds the
computer's keys to its performance scenes and to these moves (see "Zones, scenes and
the computer's keys" in [DSL.md](DSL.md)). A step's header can bring a scene in as it
lands: `# set: perform=drop`. What was asked for does not come in at once: it
lands on the next phrase line, the next bar that is a multiple of `--phrase` (8 by
default), so the set keeps its phrasing whoever is at the controls. The status line
counts down the bars to it. Asking for the step that plays cancels the move.

A step that needs a new engine is built in the last bar of the phrase and handed over
on the line, the way a save is: what the two steps share keeps playing and what leaves
rings out. A step that only changes values is sent on the line itself. A step at another
tempo starts at the tempo that was playing and ramps to its own over `--ramp` bars (4
by default; 0 jumps). Knobs, toggled tracks and held pads carry from step to step.

`--blend 8` mixes instead of handing over, the way a DJ does: the step that was playing
keeps going under the new one for 8 bars while it fades out and the new one fades in,
the two on the same tempo through the ramp, and the low end changes hands at the
midpoint in 50 ms so two basses never play at once. A step can ask for its own with
`# set: blend=8` in its header. It is for going between steps that have little in
common; steps built on one rig hand over well on the line.

`--auto` lets the set walk itself, the way `set render` does: each step holds the bars
its header gives and asks for the next in its last bar, so the hands are free for the
music. The phrase becomes 1 bar unless `--phrase` says otherwise, because a step's bars
need not be a multiple of 8. A key or a pad still moves the set at any time.

A bridge, a break or a build should not have to be watched. `# set: bars=16 auto`
makes one step walk on its own without `--auto`: it plays its bars and the next step
lands where they end, even off the phrase line. `auto=2` plays them twice first (and
`set render` plays the step for its bars times two). The TUI counts down to it
(`AUTO → next in 4 bars`, flashing in the last two). Shift+L keeps the whole step
looping instead; pressed again, it lets it go, at the end of its bars or, if they
have passed, on the next phrase line. Under `--auto` the same key keeps any step.

`# set: max=2` caps a step instead: it loops for the performer like any other, but
left alone it moves on by itself after two passes of its bars, so a set never sits on
one step waiting for a hand. The TUI says so quietly (`max x2 → next in 40 bars`) and
louder in its last four bars; Shift+L keeps it. `set render` plays such a step once,
as written.

`l` loops bars, the way a looper or a DJ player's loop does. Held, the bar it went
down in is the loop's first and the bar it comes up in its last; from the next bar
line those bars play again and again (`⟲ LOOP bars 5-8`). A tap lets it go and the
step plays on from where the music is: the bars looped do not count against its own,
so a step that moves on by itself waits while it loops. It is the sequence that
loops, not a recording: the knobs and faders still move what plays, and the `auto`
lanes go back with it. Knowing when a key comes up needs a terminal with the kitty
keyboard protocol (Ghostty, kitty, WezTerm, iTerm2); elsewhere one tap is the in and
the next the out. A pad or trigger key with `> loop` works the same way, held. A step
that lands ends the loop.

A step can carry its own practice sheet. `# cue: <bar> <what>` lines, with the bar of
the step counted from 1 and a fraction for the beat (`8.75` is bar 8, beat 4), show
under the steps in `set play --tui`: the bar of the step and its beat, the cue whose
bar is playing, and the next one counting down.

```
# set: bars=24 phase=08-amoladora energy=8
# cue: 17 B1 held + K3 from 0 to the top · the grinder: idle, scream, cut
# cue: 20.75 let go of B1
```

The file of the step playing is watched like `tatum watch` watches one: save it and the
edit plays. `q` or Ctrl-C quits and leaves the terminal as it was.

A step builds tension with `auto ... over N` (see "Lanes in a song without scenes" in [DSL.md](DSL.md#lanes-in-a-song-without-scenes)):
its lanes start on the step's first bar, whenever the step lands, and hold once done.

`tatum set render <dir> -o set.wav` walks the set the same way with nobody at the
controls: each step is asked for a bar before its header's `bars` run out, so it lands
there, the tempo ramps (`--ramp 4`), `blend=` in a header blends (`--blend` sets it for
every step), and the render is the set as it would be played. `--phrase 8` lands steps
on phrase lines as `play` does, moving a step whose predecessor ends off the line to
the next one; the default, 1, keeps every step to its header. It prints where each
step starts, `m:ss  name  bars  phase  note`, one line per step.

`--perform script.txt` plays a performance on top, for review audio without the
hardware. The script is one event a line, `<bar> <what>`, the bar counted from 0 at the
set's first downbeat and fractional:

```
# hiperespacio, one take
12.0   cc 74 64        # a knob or fader to 64
18.0   pad 44 110      # a pad down, struck at 110
19.0   pad 44 0        # and up
20.5   key 48 100      # a key down (velocity 0 lets it go)
21.0   bend 12000      # the pitch strip, 0..16383, 8192 at rest
35.0   next            # ask for the next step; also `prev`, `step 3`
40.0   perform drop    # a scene, as its computer key would call it
41.0   press pad 40 90 # aftertouch on a held pad (or `press key 45 90`)
42.0   voice bass      # the knobs' page (also `voice next`, `voice prev`)
96.0   end             # stop here
```

The events go through the same planner the live session uses, against the `midi`
blocks of the step playing: knobs, toggles and held pads carry from step to step as in
`set play`, and a pad mapped to `next` moves the set. An event lands on the sample its
bar falls on, to within a block (3 ms). A script that moves through the set (`next`,
`prev`, `step`, or a pad mapped to one) is the only thing that does, and a move lands
on the next phrase line as it would live; one that does not leaves the steps to their
headers. Without `end`, a scripted walk stops when the last step reached has played its
header's bars and every event has happened. The render prints what the script did
under the step markers, in the script's bars, from 0: `bar 12.00: acid cutoff 1.2khz`,
`bar 35.00: asked for 4/12 04.synth, lands on bar 36`, `bar 36: now 4/12 04.synth`.
(`set play`'s status line counts bars from 1, as a performer does.)

## tui-shot

```
tatum tui-shot <song.synth | set dir> [--step N] [--at <seconds>] [--size 160x48] [--font f.ttf] [--frames N] [-o shot.png]
```

A PNG of the `--tui` screen at a moment of a song or a set's step, without a terminal
or an audio device. Every picture in [TUI.md](TUI.md) was taken with it; its other
flags are there too.

## params

```
tatum params [bass|fm|keys|beats|track|fx] [--json]
```

Every module parameter with its range, units, default and what it does, as markdown, or
JSON with `--json`. [PARAMS.md](PARAMS.md) is its output, checked in CI.

## fmt

```
tatum fmt --units song.synth
```

Rewrites every module parameter written as a knob position (`cutoff 0.1`) in its units
(`cutoff 800hz`), with as few decimals as read back to the same knob, so the song sounds
the same.

## Solo and mute

`render`, `play`, `watch` and `debug` take `--solo` and `--mute` with track names,
comma-separated or with the flag repeated. A muted track is taken to level 0
everywhere, level automation included. A muted kick still drives the sidechain, so
what is left pumps the way it does in the mix.

## Building for another sample rate

The engine renders at 44.1 kHz. A build can choose another rate, for a machine
that cannot afford that many samples a second or a sound card that runs at
48 kHz without resampling:

```
TATUM_SAMPLE_RATE=32000 cargo build --release -p tatum-cli
```

Without the variable the build is the one there has always been: every song
renders bit for bit as before. With it, what is tuned in time stays in time:
the reverb, the chorus and the limiter keep their lengths in milliseconds, a
drum's decay its time constant, a glide its speed, and the loudness every song
is brought to is measured with filters designed for that rate.

What a lower rate costs is treble. The highest frequency a rate can carry is
half of it:

| rate | treble up to | CPU against 44.1 kHz | sounds |
|---|---|---|---|
| 48 kHz | 24 kHz | about 8% more | the same |
| 44.1 kHz | 22 kHz | — | the reference |
| 32 kHz | 16 kHz | about 30% less | close: a little less air on hats and cymbals |
| 22.05 kHz | 11 kHz | about half | audibly duller |

The click finder in the render's report is tuned at 44.1 kHz. Below it, a fast
attack falls on fewer samples and can read as a click; the report says so when
the build is at another rate. It is a build setting rather than an option
because several buffers have a fixed size in samples; a microcontroller's
firmware is built for one rate anyway.
