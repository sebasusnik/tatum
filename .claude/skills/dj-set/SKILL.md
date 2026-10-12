---
name: dj-set
description: Build a long DJ set to play live: a few tracks (temas), each with its own rig and personality, walked step by step and performed with a MIDI controller. Any genre. Use when the user wants a set, a long-form arrangement, or a performance built from `.synth` rigs.
---

# DJ set — temas to play, not a song to listen to

A set is **a few records mixed into each other and played by hand**. Each tema
has its own rig (its sounds, its patterns, its room) and a handful of steps,
the structural moments. Everything else — fills, throws, cuts, risers, filter
rides, drops of the bass — the performer does live with the controller. The
set is finished when it is good to *play*, not only to hear.

Read `docs/DSL.md` (steps, `auto ... over N`, the `midi`, `perform` and
`keyboard` blocks) and one finished set before starting:
`sets/hiperespacio/` (psytrance, 12 temas) — its `PLAN.md`, one rig
(`_t05.synth`), its steps (`05*.synth`), `_keylab.synth` and `KEYLAB.md`.

## The division of labour

| a step (a file) carries | the controller carries |
|---|---|
| a pattern switch, a part that enters or leaves | fills, stutters, rolls |
| a new harmony, a new voice, the tema's signature arriving | echo throws, reverb freeze |
| the breakdown and the drop: the material changes | taking the drums or the bass out for a bar or two |
| the mix-in and mix-out between temas | filter rides, sweeps, cuts, risers |
| the slow automation that takes 8+ bars (`auto ... over N`) | the signature's character (a knob) |

**If the controller can do it, it is not a step.** A 2-bar gap, an echo
throw, a step that only mutes something: those are hands, and written as steps
they make the set long and tedious to walk. The first hour built this way had
111 steps; with the controller about a third of them were redundant. Aim for
**5–7 steps a tema**. The performer can walk forward and back between steps,
so a step may loop for a long time; length is not a reason to split it.

## 1. The brief and the genre profile

Ask the user for the style in their own words, a reference song if they have
one (`examples/*.synth`), the length, and the controller they play (default:
the KeyLab Essential 49 mk3, mapped in `sets/hiperespacio/_keylab.synth`).
Then write down the **genre profile**, the decisions every later agent follows.
Two filled in, as examples of how different they are:

| | psytrance (hiperespacio) | deep house |
|---|---|---|
| tempo | 138 → 147, a step per tema | 118 → 124, barely moving |
| grid | straight, `swing 0.5`, no humanize | swung `0.54`+, humanize, ghost notes |
| harmony | phrygian, b2 rub, no major thirds, no chords | minor 7/9/11 chords, a borrowed bVI or bII, never resolving brightly |
| kick | dry, tight, rolling bass between kicks | deep long body, sub under it, room on everything else |
| low end | rolling 16ths that never touch the kick | sub + a growl above it, long notes, the groove in the gaps |
| space | short dry room, echo as a gesture | long dub echoes and reverb as part of the sound |
| a phrase | 8 bars with a turn | 16–32 bars, change is slow, the filter takes 16 bars to breathe |
| releases | tension → explosion, 4–5 in an hour, each with a different device | few: a breakdown with chords and pads, the drums come back; groove over explosion |
| steps | 8–32 bars | 16–64 bars |
| signature | a sound that transforms: a talking FM, a grinder, a 303 opening | a dub chord stab and its echo, a Rhodes line, a vocal chop, a sequence breathing |
| the gestures | rolls, tape stop, scream, crush, freeze, dives | dub throws, filter rides, EQ kills, freeze on a chord, the bass out for a bar |

Read the reference song's `arrange` block: it is the shape of one tema. Ask
the user for what they do not want; write it into the profile (the user's
past notes are in memory: dark, precise, no cheesy harmony, gestures not beds).

**The genre is in the harmony before it is in the sound.** A rig meant as
dark goa came out as commercial trance because its chords ran Am / F / C / G.
Decide what the harmony is allowed to do before writing a note, and check the
rigs afterwards: `grep -oE '\[[A-G][b#]?[0-9]( [A-G][b#]?[0-9])*\]' _t*.synth | sort -u`.

## 2. The plan — `PLAN.md`, written by you, once

The plan gives the set its shape; the agents work inside it. It has:

- **The arc**: the temas in order, the tempo of each, the key of each (keys
  that mix: same root, a fifth, relative), and where the releases are.
- **Each tema's personality**, one paragraph, all different on purpose:
  its kit tuning, its bass, its room, its harmony colour, its signature
  sound and the 2–3 moments it appears. Two temas must not share a signature
  or a bass sound. "Each part has its personality and nothing gets
  repetitive" is decided here, not later.
- **The step list of every tema**, from the vocabulary below, with bars.
- **The controls**: what the keyboard, pads, knobs and wheels do in this
  genre (section 4), and per tema what its signature knob and pad do.

### Step vocabulary

| step | bars | what it is |
|---|---|---|
| `entrada` | 16–32, `blend=` | the tema mixes in under the last one's `salida`: drums and low end only, nothing tonal yet |
| `piso` | 16–32 | the groove lands: the full kit, the bass's main part |
| `firma` | 16–32 | the signature voice arrives — the tema's reason to exist |
| `variacion` | 16–32 | something played differently: other patterns, a counter-line, a new chord voicing. Same energy |
| `hueco` | 8–16 | the breakdown: the floor goes, what stays is the tema's most beautiful thing |
| `subida` | 8 | one pass of tension, then it must land. Never two in a row |
| `estallido` | 32 | the drop: everything at once, plus something not heard before |
| `salida` | 32 | stripped for the next tema to mix over: kick, hats, the plain bass, nothing tonal |

A tema is usually `entrada · piso · firma · variacion · (hueco · subida ·
estallido) · salida`. Not every tema gets a release: in an hour, 3–5 of them,
each built from a different device (a drum gap, a grinder spin, a riser, a
freeze, a hard cut). In a house set fewer, and softer.

**Bridges move on by themselves.** A step that only makes sense as a passage
— a `hueco`, a `subida`, a short bridge between two variations — gets `auto`
in its header (`# set: bars=8 auto`; `auto=2` plays it twice first): in
`set play` it moves to the next step where its bars end, without a hand, and
the TUI counts down to it. The performer keeps the whole step with Shift+L,
or holds `l` (or a `loop` pad) over a few bars to loop just those, when the
room wants more. Everything else loops until the performer
moves — that is where the DJ chooses when — but with a cap: `max=2` on every
other step (`max=1` on a 64-bar one, none on the last), so a step left alone
moves on after two passes instead of looping for ever.

**Energy must not sawtooth.** Once the floor lands it stays; variation at
the same energy is the normal move. Dropping the energy is a device, used at
the releases and nowhere else.

**Most steps change what is played**, not only how loud: a pattern switch
lands on the bar and is the most useful move there is. A step that only moves
faders changed the mix, not the music — and the faders are the performer's.

## 3. The rigs — agents, two temas each

Layout:

```
sets/<name>/
  PLAN.md
  _base.synth          shared: grid, sidechain, sends, returns, master chain (named dj/hp filters, eq, gain as vol), `use "_<controller>.synth"`
  _<controller>.synth  the controller's map, shared by every tema (section 4)
  _voces.synth         the keyboard's own voices: riser, impact, fill...
  _t01.synth ...       one rig per tema: `use "_base.synth"` + its modules, patterns, tracks, its `midi` additions
  01a-entrada.synth    steps: `use "_t01.synth"` + only what changes
```

Steps play in filename order. A step is `use` plus the fields that change —
`track bass { play roll_b level 0.6 }`, not the whole track — so the rig
stays the one place a sound is defined. Header:

```
# set: bars=32 phase=03-dub energy=6 blend=32
# set-note: one line saying what this step does
# cue: 9 the chords open; ride K2 up over 8 bars
```

A bridge, break or build adds `auto` to its `# set:` line (section 2).

Spawn one agent per **two consecutive temas** (they own the mix between
them), in parallel, each with the genre profile, its temas' paragraphs and
step lists from the plan, `_base.synth`, and these rules:

1. The rig: its own kit tuning, its own bass, its own room if the tema wants
   one, its signature. Every optional voice at `level 0`, every effect named
   and at `wet=0` until a step or a control uses it.
2. **Every voice has 2–4 patterns**: sparse, main, busy or broken; the drums
   also a fill and a stripped mix-in/out version. Write patterns as whole
   phrases with a turn at the end (in house, 16 bars; in psy, 8). Nothing
   runs over ~3 minutes unchanged.
3. **Each voice has a part of its own** — a counter-line, an answer, another
   subdivision — not three voices doubling one rhythm.
4. **Do not stack voices until everything is on.** Bring a voice in by
   taking one out. The kick is the thing everything hangs from.
5. **Every sustained sound moves**: an LFO on the filter, an `auto`, autopan.
6. The tema's controls (section 4): its signature on a pad and a knob, a
   `ktema` track for the keyboard to play its lead voice, its `midi` lines,
   and a page for each of its own instruments (2–3) with their faders.
7. `tatum check` on every file; `tatum set check sets/<name>` when done.

### The signature

Every tema needs one sound that is its own and is used as **2–3 placed
gestures, not a bed**: it arrives, does its thing, leaves before the next
section lands. What makes one, from the sets so far:

- **A sound that transforms, not a tune.** The best-liked moment of
  hiperespacio is T05's robot (`_t05.synth`): a buzzy FM tone (feedback
  turns the sine into a near-saw, so there are harmonics for the formants)
  through `vowel(o, i, bars=1, mix=0.85)`, on a fixed two-note cell. The
  vowel moves, not the melody — it seems to speak.
- A parameter that travels: a grinder's RPM, a 303 opening bar by bar over
  p-locks, an index climbing over 32 bars, a sequence's filter breathing
  over 16.
- In house: a chord stab whose echo is most of the sound, a Rhodes line
  with room between the notes, a choir turning from "oh" to "uh".

Give its movement to a knob (`page tema`, K3 by convention) so the performer
can play it.

## 4. The controls — shaped to the genre

Write `_<controller>.synth` (the `midi`, `perform` and `keyboard` blocks;
`docs/DSL.md`, "Zones, scenes and the computer's keys" onward) once for the
set, and let each rig add its lines. Start from
`sets/hiperespacio/_keylab.synth`; keep its layout, change what it does:

- **Zones**: the lowest octave triggers, then the bass, then the lead.
  The lead zone plays each tema's own lead voice (`ktema`: a copy of the main
  voice's module under its own name, in a silent track, measured to sit about
  14 dB under the mix). A copy, not the same module: a knob names a module
  and reaches every track that plays it, so a shared one would turn the
  tema's voice too. One page (`teclas`) shapes it, knobs and faders. What
  the zones play suits the genre: in psy, the bass rolls the held note; in
  house, the bass zone plays the sub's notes and the lead zone a Rhodes or
  the chord voice.
- **Scenes** (`perform`, from the computer's F-keys): what the keys play, the
  scale, and what the wheels do, per phase of the tema.
- **Wheels**: playing, they shape your note; hands off (`idle`), they move
  the song — the DJ filter in any genre.
- **Faders**: on the first page a mixer, one per role (kick, bass, hats, the
  lead, the signature, the sends). F9 the DJ filter (`guard`), on no page.
- **Pages** (Tab or the encoder), knobs *and* faders: a page's line for any
  controller replaces the block's while it is chosen. `tema` (each tema's
  macros, the faders as the mixer), then the shared `bass`, `lead`, `drums`,
  `master`, then each rig's pages for its own instruments (`acorde`,
  `secuencia`, `coro`, `acid`...). On an instrument page the knobs shape its
  sound (K1 brightness, K2 colour, K3 movement, K4 envelope, K5 tension...)
  and the faders are its strip: F1 level (twice the written, so half travel
  is the text), F2 echo, F3 room, F4 pan, F5–F8 related voices. Tab passes
  over a page whose tracks play nothing in the step, so a page per instrument
  costs nothing when it rests. `takeover pickup`.
- **Pads and trigger keys**, the genre's gestures:

| psy, techno | house, dub, deep |
|---|---|
| roll 1/8 · 1/16 · 1/32 | dub throw of the chords / the Rhodes |
| tape stop | freeze on the chord's tail |
| crush · scream | the bass out for a bar |
| trance gate | low cut / high cut on the master (EQ kills: `master eq eq_low -12..0`) |
| riser · impact | a filter ride on the chords (`cut`, `sweep`) |
| freeze | the vocal chop, a percussion fill |

Then audit: `tatum set controls sets/<name>` lists every control that moves
nothing, step by step. Fix them all; a dead knob on stage is a lie.

Write the map for the performer (`KEYLAB.md` in hiperespacio: the keyboard,
wheels, faders, the pages, the pads, the computer keys, in the user's
language), and `# cue:` lines in the steps telling when to do what.

## 5. Measure the hour, send fixes back

```
tatum set render sets/<name> -o sets/<name>/<name>.wav    # walks it as set play does
```

`tatum set check`'s rms column measures only a step's first bars; measure the
render: loudness per step from the step boundaries (ffmpeg `ebur128`), the
low end per section. Check, against the plan:

- each tema's floor sits within ~1 dB of the others' (trim with a
  `gain(...) as trim` in the rig's master override);
- a build is never louder than its drop; a drop is the loudest of its tema;
- the master input stays under 1.0 everything-on (a pinned limiter flattens
  the arc and makes every step the same loudness);
- the mixes between temas do not pile two low ends.

Send each agent concrete fixes ("T03's build is 2 dB over its drop: take the
riser down, the drop's bass up"), re-render, repeat until it holds.

When the user dislikes a sound ("the bass of the intro"), don't swap it
blind: write 2–3 candidates as copies of the rig in `/tmp`, render each in
its context (the tema's steps, `tatum set render` on a copy of the set) to
`sets/<name>/<what>/a-*.mp3`, and let the user choose. Change only the sound
asked about; the rest of the tema stays. Renders go
in `sets/<name>/`, never committed.

Then render a performance (`tatum set render --perform script.txt`, the
script events in `docs/CLI.md`) with 2–3 gestures a tema, one at a time, so
the user hears what the controls are for.

## Reporting back

The arc as a table — tema, steps, tempo, key, signature, loudness — where
the plan and the render disagree, the controls map, the command to play it:

```
tatum set play sets/<name> --tui          # walk it by hand
tatum set play sets/<name> --tui --auto   # steps follow their headers
```

Do not describe how the music sounds; they are about to hear it.
