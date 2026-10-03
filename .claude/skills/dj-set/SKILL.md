---
name: dj-set
description: Build a long DJ set as a relay of agents, each writing one step of a live-coded performance without seeing what came before. Use when the user wants a set, a long-form arrangement, or an exquisite-corpse performance built from a .synth rig.
---

# DJ set — an exquisite corpse for `.synth`

A set is not a song. It is **one rig, played over time**: a directory of numbered
`.synth` files, each the whole rig at a moment, walked by the engine with real
hot swaps. Each step is written by a fresh agent that sees the current state and
nothing else. What holds it together is not memory, it is the plan and the gate.

Read `docs/DSL.md` and `examples/rigs/live_rig.synth` before starting. The rig is the
instrument; the set is the performance.

## Why this works at all

Two edits apply **inside the bar**, with no hot swap and no voice restart:

| edit | what it does |
|---|---|
| `level 0` → `level 0.4` | bring a voice in. A muted track fires no notes and is skipped whole, so a rig of waiting voices is nearly free |
| `wet=0` → `wet=1` | switch an effect in. At 0 the node is not processed at all |

Values jump. A step that has to **build** — a filter opening, a riser coming up, an
FM lead getting harsher, the reverb freezing near the end — writes a lane with
its length in bars. It starts on the step's first bar, runs once and holds:

```
# set: bars=8 phase=build energy=6
use "_rig.synth"
track riser { level 0.3 }
auto master cutoff 1500 > 20000 over 8
auto riser level 0 > 0.4 over 8
auto reverb_freeze 0 > 0 > 0 > 1 over 8    # crosses 0.5, and freezes, in bar 7
```

A lane is a swap, like any `auto` edit. The next step does not have to undo it:
what it does not automate plays at its own text's value. Targets are the ones a
scene's `auto` takes: a module parameter, a track's `level`, `master cutoff`/`tilt`/
`drive`..., `reverb_mix`, `reverb_freeze`, `<track>.<node> wet`.

Everything else — adding a node, changing which pattern a track plays, renaming
anything — is a hot swap, quantized to the next bar line. A swap keeps every
voice, tail and send that did not change; only what actually changed restarts.

**Do not treat swaps as something to avoid.** The first set built with this
skill used nothing but `level` and `wet` for all eight steps, and it came out
as eight volumes of one loop. Switching which pattern a track plays is a swap,
lands on the bar, and is the most useful move there is — it is what a DJ does
when they change a loop. Use the fast levers for texture and the pattern switch
for structure. A step that only moves faders changed the mix, not the music.

A muted track keeps counting its pattern, so bringing a fader up drops that part
onto the grid rather than where it stopped.

## The shape of a set

```
sets/<name>/
  000.synth     the rig, almost everything off
  001.synth
  ...
```

A step is `use` plus what it overrides, not a copy of the rig:

```
# set: bars=16 phase=impact energy=9
# set-note: the whole floor lands at once

use "../../examples/rigs/my_rig.synth"

track drums { play beat_hard using kit level 0.25 ... }
track seq   { play seq_a using seq_osc level 0.24 ... }
```

**Write only the fields that change**, not the whole track line:

```
track drums { play beat_hard level 0.25 }
track seq   { level 0.24 }
```

A redefinition inherits everything it does not mention, so those two lines are
the rig with a pattern and two levels moved. Restating the whole line works and
means the same thing, but then the step owns the chain — in a progressive house
set every step had copied the pad's chain, and a fix to that chain in the rig
reached none of the twenty. Say less and the rig stays in charge of the rest. Written out in full, a twenty-step
set was 4100 lines of which 11 changed per step, and editing the rig orphaned
every step; this way it is 416 and the rig stays one file. Tell the step agents
to write it this way: the rig is what they read, the overrides are what they
write.

No manifest. Each file carries its own header, on comments the DSL ignores:

```
# set: bars=32 phase=build energy=5
# set-note: reese in under the pad, its autowah half open
```

`bars` is how long the step holds. `phase` and `energy` are the plan's labels.
`set-note` is one line for the next agents — it is the only thing they inherit.

## Running a set

### 1. Get the brief

Ask the user for the style and shape in their own words if they have not said
it, plus the length. "Start chill like a warm-up and build each iteration" is
enough.

**Density: a move every 10–20 seconds, not every 40.** The second set built
with this skill used eight steps of 24 bars, and the listener's note was that
every part felt static and the same length — which is what it was. A step is
one decision by the performer, and a performer makes one every few bars, not
once a minute. At 148 BPM, 8 bars is 13 seconds. Budget roughly **one step per
8–10 bars**, which for five minutes is about twenty steps.

State the cost before starting: one agent per step plus a gate run each.
Twenty steps is a real job; sixty is a large one. Offer a short version first.

### 2. Build the rig

Either start from `examples/rigs/live_rig.synth` or write one for the style. A rig
needs more voices than any one moment uses — that is the point. Aim for 8–12
tracks: drums, sub, a bass, two or three pads or keys voiced differently, a
lead, a texture. Every effect gets `wet=0` and a name (`as wah`). Every optional
voice starts at `level 0`.

**Each voice needs two or three patterns to choose between**, not one. That is
what a step switches to make something happen. Write, per voice: a sparse
version, the main one, and a busy or broken one; for the drums also a fill and
a half-time. Twenty-odd patterns across nine tracks is a normal rig. Four
patterns is a loop with faders on it, and the relay will have nothing to say.

**The genre is in the harmony before it is in the sound.** A rig written for
dark psychedelic goa came out as commercial trance, and the listener's word for
it was "careta". Nothing was wrong with the timbres: the fault was that the
chords ran Am / F / C / G — a functional progression that resolves, with three
major triads in it. That is the cheese, and no amount of distortion downstream
removes it. Dark psy sits on a drone and moves melodically in a mode, without
resolving; its signature is the flat second against the root, which is a clash,
not a cadence. Before writing a note, decide what the harmony is allowed to do,
and if the answer is "stay dark" then put no major thirds in the file at all —
use open fifths, root/flat-second clusters and tritones. Check it afterwards:
`grep -oE '\[[A-G][b#]?[0-9]( [A-G][b#]?[0-9])*\]' rig.synth | sort -u` lists
every chord in the rig on one screen.

**And a rig needs as many parts as voices, not just as many timbres.** The
first rig this skill ran on had eight tracks sharing four patterns: two basses
on one line, two pads on one chord part, all three FM voices on the same stab.
Every step passed and the peak still came out the sixth loudest section of
eight, because a third voice playing a rhythm two others already play adds
thickness and no power. Give each voice something of its own: a counter-line,
an answer to the main part, a different subdivision.

Check it: `tatum check <rig>.synth`, and confirm it plays with headroom.

### 3. Plan the arc — one agent, once

Spawn **one** agent with the brief. It returns the whole step plan: for each
step, a phase label, **how many bars it holds**, an energy target 0–10, and one
sentence saying what that step should do. This is what gives the set its shape, and every later agent
works blind against it. Do not let the step agents invent the arc.

A plan should breathe. Energy that only rises is as dull as energy that never
moves: peaks need a drop after them, and a long set wants two or three of each.

**Energy must not sawtooth.** The third set built with this skill ran
-17.9, -18.1, -21.8, -21.8, -21.1, -19.0, -19.0, -18.9, -15.4, -23.1, -16.5
across its second half, and the listener's note was that it "goes up and down
at random" and lost the relentlessness the genre lives on. Once the floor
lands it stays. **Variation at the same energy is the normal move**: switch
which pattern a voice plays and leave the level alone. Dropping the energy is
a device, not a way to keep things interesting, and a plan that reaches for it
every third step has nothing left when it matters.

**A build is one pass, and then it explodes.** A fill or a snare roll is a
promise; the only thing that can follow it is the thing it promised. Two builds
in a row means neither one lands — the second steals the first one's arrival
and then has nothing to arrive into. The same goes for climbing out of a
breakdown in stages: a floor that returns muffled, then a bit more, then fully
reads as three small breakdowns, not one big one. Out of the hole comes the
build, and out of the build comes everything, at once. The gate enforces this:
after a step whose drums play a fill, the next step must measure at least
1.5 dB louder, and two builds in a row are rejected outright.

**Leave the master headroom, or the limiter arranges the set for you.** This
one cost two sets before it was found. A rig whose tracks are hot enough that
the mix arrives at the master over full scale gets pinned: every step comes
out at the same loudness however the faders move, so agents reach for
saturation to buy the difference, which crushes the crest, which makes the mix
arrive hotter still. Measured on one set: the final step handed the master
1.36, the crest fell from 4.7 to 3.4, and the second half sat inside 0.2 dB —
three builds and not one of them landed. The gate now reports `master in` and
rejects anything over 1.0. Design the rig so everything-on peaks around 0.8
there, and the faders will do what they are supposed to do.

**Do not stack voices until everything is on.** "Never lower a level" stops
energy sawtoothing, and it also quietly produces a set where by the climax all
ten voices are playing and the kick is one of ten things instead of the thing
everything else hangs off. Bring a voice in by taking one out. Measured on the
same gate: `examples/hitech_psy.synth` at its fullest runs **5 voices** with
the floor **+13.3 dB** over the midrange; a set that piled up ran 10 voices at
+7.2 dB and the listener's note was that it did not sound like the genre at
all — not because the kick was quiet, but because it was buried. Pass
`--max-voices N` to the gate and it will reject a step that goes over.

**A breakdown is one gesture and it happens once.** Tension, then impact. Using
it twice halves it both times. Write it as three consecutive steps, not one:
the hole (8 bars, everything but the pad gone), the tension (4-8 bars, a riser
climbing, the hats getting busier, nothing resolving), then the explosion — a
long step at the highest energy in the set. Do not put a second hole anywhere
near it.

Look at how the genre actually does it. `examples/psytech_goa.synth` arranges
open 4 / intro 8 / gap 2 / build 8 / **drop 12** / bridge 6 / **breakdown 8** /
build 6 / **drop 14** / outro 6 — one breakdown, and the drops are long. 
`examples/hitech_psy.synth` runs drop1, drop2, drop3, drop4 back to back at the
same energy, varying the pattern each time and never letting the floor go. Read
the `arrange` block of a song in the target genre before planning; it is the
shape you are writing, without the labels.

**Steps must not all be the same length.** `bars=` is per step and it exists to
be used: 4 bars for a fill that stabs, 8 for a breakdown that has to hurt, 16
for a groove to settle into, 32 for a peak to ride. Genre decides — a psytrance
breakdown is short and violent, a liquid one can hang. If every step is within
a few bars of every other, the set reads as a slideshow however good each slide
is. Make the planner vary it hard and say so in the plan.

**Most steps must name a pattern to switch to**, not only a fader to move. A
plan written entirely in levels produces a set written entirely in levels. Say
"switch the bass to its broken variant", "drums to the fill for this step then
the busy one", "put the lead on its sparse line". Read the rig first so the
patterns you name exist.

### 4. Run the relay

For each step, spawn a **fresh** agent. Give it exactly:

- the current `.synth` in full
- `step 14 of 40 · 21:00 of 60:00 · phase: build · energy 4 → 5`
- the one sentence from the plan for this step
- the **last three `set-note` lines**, nothing older
- the rules below

It returns a complete `.synth` file and one `set-note` line.

Run the agents **in sequence** — each one needs the state the last one produced.
Do not fan out.

#### The rules to give each step agent

1. Change **one or two things**. A step is a move, not a remix. The set gets its
   variety from forty small moves, not from four big ones.
2. **At least one step in three has to change what is played, not just how loud
   it is** — switch a track to a different pattern with `play`. That is a swap,
   it lands on the bar line, and it is correct: it is how a set moves. Use
   `level` and `wet` for the texture around it.
3. **Never rename a module or a track** unless you mean that voice to restart.
   The hot swap inherits state by name; a rename is a cut.
4. Keep the header current: update `bars`, `phase` and `energy` to match, and
   write a `set-note` that says what you did in one line, for the agents after.
5. Return the whole file, not a diff.

### 5. The gate — every step, no exceptions

```
tatum set next sets/<name> <candidate>.synth --json
```

It answers with a verdict and why. It rejects a candidate that does not compile,
is silent, clips, changes nothing the engine can hear, or is inaudibly different
from the step before. It also reports whether the transition is `fast` or a
`swap`. Both are fine. A set that is all `fast` is a set that never changed
what it plays, which is a warning about the set, not about the engine.

On rejection, re-run the same step with the gate's message appended to the
prompt. Two retries. If it still fails, keep the previous state for that step,
write a `set-note` saying the set held, and move on — an hour-long set can
survive a bar of nothing, and stopping the relay loses everything.

Only after the gate accepts, save it as `sets/<name>/NNN.synth`.

### 6. Finish

```
tatum set check  sets/<name>              # every step and every transition
tatum set render sets/<name> -o set.wav   # the whole thing, walked as `set play` walks it
```

Read the check table before declaring victory. The `rms dB` column is the arc:
if it does not move the way the plan said, the set does not have a shape,
whatever the notes claim. `muted` shows how much of the rig is in play.

**The gate does not check the arc.** It asks whether a step is valid and
different, not whether it hit the energy the plan asked for — a step can be
accepted and still leave the set flat. That is what the check table is for, and
it is worth reading after every few steps rather than only at the end, while
there is still set left to fix. Measure the sections of the render too: a
breakdown shows up as the low end vanishing long before it shows up in RMS.

## Reporting back

Give the user the arc as a table — step, phase, energy, rms — and say plainly
where the plan and the render disagree. Name the steps the gate rejected and
what happened. Do not describe how the music sounds; they are about to hear it.
