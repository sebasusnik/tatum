# tatum-core as a portable synth with a live-coding surface

> Independent analysis, 2026-09-15. Everything it claims was measured in this
> repo with a temporary probe in release (later replaced by the permanent
> tests in `core/tests/bar_timing.rs` and `core/tests/live.rs`) and with the
> CLI. Where I did not measure, I say so.

## Summary on one screen

1. **The voices/performance split is right, but it is not a new language.**
   The `.synth` already has both halves: `module` + `bus` + `master` + global
   sends are the *rig* (tuned offline); `pattern` + `track` are *what is
   playing now*. `scene` + `arrange` are the composition layer, and the engine
   already runs without them: a 12-line file with no scenes compiles, renders
   whole bars and loops forever. The live-coding surface exists today as a
   subset. What is missing is a `use "rig.synth"` so the rig is not repeated,
   and a way to play without compiling the same thing twice.

2. **Effect chains live on both sides, and the criterion is how long their
   state lives.** A track insert (`out > saturate > master`) is part of the
   voice: it has no tail and is rebuilt with it. Sends, returns, buses and
   master are part of the session: they have seconds of tail (reverb 15 s,
   delay with 0.74 feedback) and that tail has to survive any edit. Today it
   survives no structural edit.

3. **The engine can take what live playing needs.** Compiling the largest song
   in the corpus takes less than 1 ms. The worst audio block uses 7 % of the
   budget. The audio path does not allocate. Nothing structural stands in the
   way of playing.

4. **The hot swap is not in shape.** I found four defects, three of them
   measurable in samples. The worst one is in the engine, not in the swap:
   `current_bar` changed one 16th before the real bar, and that already
   affected the 25 rendered songs without anyone noticing.

5. **The first step is `tatum play` and `tatum watch`, but not the way the
   roadmap asks for it.** The swap logic lives in `wasm/src/lib.rs`, not in the
   core. If the CLI copies it, there are two implementations of the most
   delicate part of the product, which is exactly what "What not to do"
   forbids. The real first step is a `LiveSession` in the core, shared by the
   WASM and the CLI, fixed, and only then the `play` on top of it.

## What I measured

### The engine has plenty of headroom

| song | parse | compile | build engine | render | worst block | budget per block |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `psytech_goa` (743 lines) | 722 µs | 87 µs | 43 µs | 40× real time | 206 µs | 2902 µs |
| `hitech_psy` (1075 lines) | 319 µs | 43 µs | 39 µs | 25× | 180 µs | 2902 µs |
| `dub_techno` | 159 µs | 21 µs | 30 µs | 48× | 109 µs | 2902 µs |

Compiling is negligible: it can recompile on every keystroke. The worst block
leaves 14× of headroom, so a Raspberry Pi 4 (some 8 to 10 times slower than
this machine in floating point) also fits, with less slack. I did not measure
this on the Pi; it is an extrapolation.

This changes the design: **a fast path is not needed to be fast**. The diff
exists so the engine is not rebuilt, but rebuilding it costs 40 µs. What does
cost is losing the state (tails, envelopes, LFO phase). The problem is
continuity, not speed.

### The bar counter ran one 16th early

`current_bar` was incremented at the end of `advance_step`, after firing the
last step of the bar. So for all of step 15 the engine already said "next
bar". At 120 BPM:

```
current_bar -> 1 at sample 82687   (bar 1 starts at 88200)
current_bar -> 2 at sample 170887  (expected 176400)
```

Three consequences, all silent:

- **Every scene change killed the note on step 15.** `apply_scene` runs when
  the counter crosses, that is, together with the firing of the last step: it
  fires the note and releases it on the same sample. Measured with a `keys`
  playing on every step with `gate 0.9`: step 13 audible for 100 % of the
  step, step 14 for 100 %, **step 15 for 0 %**. It happens at every transition
  of the 25 songs. After the fix, 100 % on all three.
- **Every render with an arrangement lost the last 16th.** `running = false`
  was set one step before the end. `hitech_psy`: 4 775 680 samples written
  against 4 779 871 expected. Tempo, freeze, `reverb_mix` and automations also
  started one step early.
- **The browser hot swap came in one 16th before the downbeat**, because it
  looks at `current_bar`. Every structural edit shifted the whole song 125 ms
  forward.

The fix is on the `live/session` branch (`advance_step` crosses the bar line
before firing, comparing `global_step / steps_per_bar` against `current_bar`
so that `start_from_bar` does not count twice). The full suite passes with the
fix. The reference WAVs in `core/test_output/probes/` change because they now
have the last 16th. No existing test caught the problem, because they all
check that something sounds and not when; the four in
`core/tests/bar_timing.rs` check when, and all four fail against the old
engine.

### The swap splits the block in the wrong place

With the counter fixed, I reproduced the logic of `wasm/src/lib.rs` natively
and looked at the swap bar with a kick on step 0:

```
kick onsets (samples from the bar line): reference [14]   swapped [14, 377]
```

Two kicks 8 ms apart. The WASM detects the bar change *after* rendering the
block, so the old engine has already fired the downbeat, and the new one fires
it again at the start of the next block. A flam on every edit. The fix is to
split the block at the exact sample: the engine knows how many samples are
left until the bar (`sample_counter` and `current_step_duration` say so), so
the old one renders up to there and the new one from there.

### The swap throws the tail away

Pad with a held chord, `reverb size=1.0`, `reverb_send 0.7`, `delay_send 0.5`.
Structural swap to an identical song (one unused pattern added). Reference
against swapped, RMS per 100 ms window after the bar line:

```
t+0ms    -19.5 dB   -20.8 dB   (-1.3)
t+100ms  -21.5 dB   -27.1 dB   (-5.6)
t+200ms  -21.9 dB   -25.6 dB   (-3.7)
```

The new engine is born with an empty reverb, an empty delay, an empty
`capture`, and the chord is retriggered from the attack. The 12 ms crossfade
covers the click, not the tail. For dub or ambient, where "the dry signal is
almost incidental" (roadmap item 11.9), every edit is a cut.

### The fast path skips the compiler

In `load_source`, if the diff is not structural, **the new AST is never
compiled**. Range validation lives in `compile_module_def`, so in the browser
`cutoff 7.0` is applied, silently clamped, while `tatum check` rejects it. It
is the same class of bug as the list of silent no-ops: the same text means two
things depending on the path it takes. Compiling costs 87 µs; there is no
reason not to do it every time.

### The swap allocates on the audio thread

`from_compiled` allocates (instruments, chains, capture buffers). In the WASM
that runs inside the worklet, on the same thread as `process`. It does not
show today because compiling is fast, but it is the rule 8.10 set and that the
native CLI cannot break: natively, the engine is built on the thread that
watches the file and handed to the audio thread already assembled; the retired
engine goes back over another channel so that its `drop` does not happen in
the callback.

## The answers

### Is the split right?

Yes, but the unit is not "config file vs performance file", it is **what
changes at what speed**:

| layer | contains | changes | state it carries |
| --- | --- | --- | --- |
| rig | `module`, `instrument`, `bus` + chain, `master`, global `delay`/`reverb`, returns | offline, with measurement | long tails (seconds), capture buffers |
| performance | `pattern`, `track` (play, using, level, pan, sends, inserts) | live, per bar | held notes, arp phase, insert filters |
| composition | `scene`, `arrange`, `auto` | never live | automations |

All three already exist in the DSL under those names. The split is done; what
is missing is for the hot swap to respect it: **what did not change in the
text keeps its state in the engine**. With that rule, editing a pattern does
not touch the reverb, changing a `level` touches nothing else, and adding a
track does not retrigger the pad that was already playing.

Two practical things are missing:

- `use "rig.synth"` in the performance file. The core is `no_std` and has no
  filesystem, so the CLI resolves it (it concatenates before parsing) and the
  core receives a single source. The parser does not need to change.
- With no scenes and no `arrange`, the engine loops the top-level tracks
  forever. That is the live mode and it already works. It has to be documented
  as such, and `single_scene` and `sidechain_without_kick` should not treat it
  as a half-written song.

### Where do the effect chains go?

On both sides, and they are already on the right side:

- **Track inserts** (`out > highpass > phaser > master`): part of the voice.
  Short state (a filter, a phaser). They are rebuilt with the track and it is
  not audible, as long as the swap is sample-exact.
- **Buses, global sends, returns, master**: part of the session. Long state.
  They have to survive everything that does not touch them. Today they survive
  nothing.

The roadmap's open question (11.18, "per-scene chains precompiled and
exchanged with a swap") is the same question: changing an insert chain per
scene is a partial swap of the voice. If the session swap keeps what did not
change, 11.18 comes for free as a special case.

### What breaks that you are not seeing?

1. The bar counter (above). It affects the whole corpus, not only live.
2. The swap flam from block granularity.
3. The lost tail.
4. The skipped validation on the fast path.
5. `find_track(...)` returning `None` in `apply_change` is one more silent
   no-op. It cannot happen today because the diff requires the same number of
   tracks, but it has exactly the shape the other eight had.
6. Two implementations of the swap if the CLI copies the WASM one.
7. A swap with `pending` already queued: if a parameter edit arrives while an
   engine is waiting for the bar, the diff is made against the newest AST but
   the change is applied to the old engine, with the new one's indices. It can
   move the `level` of the wrong track. In the browser it is hard to trigger
   (you have to edit twice within one bar); in `watch` with an editor that
   saves as you type, it is the normal case.

### Is there a shorter path?

Yes: **do not design a performance language**. What makes a live file short
is not new syntax, it is not having to write the rig or the arrangement. A
`use` and the no-scene mode give that. Everything else (what plays, which
pattern, which level, which filter) is already written in one line per track.

What is not shorter than it looks: `play` without fixing the swap. `tatum
play` can be had in an afternoon with cpal, but the real question ("does it
hold up for half an hour of playing?") is answered by the swap, not by `play`.
With today's swap the answer is no: every edit cuts the tail, flams the kick
and shifts the grid.

## Proposed design for the first step

### `core/src/live.rs`

Two objects, because natively they live on different threads and in WASM on
the same one:

- **`LivePlanner`** (control thread): receives the source, *always* parses and
  compiles (full validation), compares with the previous AST and produces a
  `Plan`:
  - `Unchanged`: identical text.
  - `Fast(Vec<FastOp>)`: parameter changes, already resolved to indices
    (`TrackLevel { track: 2, level }`, `ModuleParam { instrument: 0, id, value }`).
    No strings, no lookups on the audio thread. If something does not resolve,
    it is a swap, never a discard.
  - `Swap { engine, inherit, crossfade }`: a new engine already built, plus a
    map of what it inherits from the old one: sends if `delay`/`reverb`/returns
    are equal; master if its chain is equal; each bus by name if its chain is
    equal; each instrument by name if its definition is equal; each track by
    name if its definition, its pattern, its instrument and the scale are
    equal and the scenes did not change. `crossfade` only if something from
    the old one goes away.
- **`LivePlayer`** (audio thread): holds the engine, the `pending` and the
  crossfade. `process` splits the block at the exact sample of the bar. On a
  swap, the new engine takes the state it inherits with `mem::swap` (zero
  allocations), the tracks that do not continue release their notes in the old
  one so that an inherited instrument is not left with a note nobody will
  release, and the old engine is handed back to the caller to drop off the
  audio thread. A `Fast` that arrives with a `pending` queued is applied to the
  `pending`, which is what its indices belong to.

The WASM becomes a shell over the two. The `Tatum` API does not change.

### CLI

```
tatum play  song.synth [--device <name>]     # plays until the arrangement ends or 'q'
tatum watch song.synth [--device <name>]     # play + reload on save; an error prints and it keeps playing
```

cpal 0.18 (verified that it compiles and opens the device on this machine; the
default accepts 44.1 kHz, which is the only rate the engine can do). A thread
checks the `mtime` every 50 ms, plans, and sends the plan over a
`sync_channel`; the callback does `try_recv`, applies it and hands back what
was retired. On exit it prints the worst block against the budget and how
many swaps there were: that is the measure of "holds up".

### Tests to write

- The bar crosses at the exact sample; a scene change does not shorten step
  15; a render with an arrangement has all its samples.
- A structural swap to an identical song gives a render **bit for bit equal**
  to the render without a swap (with full inheritance there should be no
  difference; today there are 7 dB).
- A swap with a changed pattern does not touch the reverb tail: the
  `reverb_return` measured alone is the same before and after.
- No double kick on the swap bar.
- `cutoff 7.0` through the fast path is a compile error, same as in `check`.
- The `LivePlayer` swap under the counting allocator: zero allocations.

### Order

1. Bar fix with its tests. Commit on its own.
2. `live.rs` with inheritance and exact split. WASM on top of it. Commit.
3. `tatum play` / `tatum watch`. Commit.
4. Soak: `watch` with a script that edits the file every two seconds for
   several minutes, on headphones. Worst block and swaps, in the commit.
5. `use "rig.synth"`, lints that respect the live mode, `docs/DSL.md`.

What I leave out on purpose: MIDI input, resampling to 48 kHz, the Pi. None of
that changes the design; all of it depends on the swap being correct first.

## Result (`live/session` branch)

All five steps are done. Two things changed from the design above, both
because of measurements:

- **The swap does not split the block at the exact sample: it comes in at the
  start of the block the bar falls in.** The engine fires a step at the start
  of the block it falls in (the sequencer loop runs before the instruments
  render), so splitting the block gave a result *more* precise than the
  direct render, and therefore not identical. The new engine takes the whole
  block with its step clock phase-aligned to the old one, and starts
  positioned *before* the bar line (`start_before_bar`) so it crosses it
  itself: scene change, end of the arrangement and automation progress land
  on the same sample as in the direct render.
- **The old tail is summed with a fade-out; the new one does not fade in.** If
  everything is inherited, the old engine is left with nothing and sums
  silence, and the render is identical bit for bit. A traditional crossfade
  would have sunk 12 ms of what did not change.

Soak: `tatum watch` for five minutes on headphones with a script that saved
every two seconds (values, patterns, now and then an invalid save):

```
played 304.9 s in 26262 callbacks of ~512 frames (11.61 ms each)
worst callback 4.385 ms of 11.61 ms budget (38%), 0 late
39 swaps, 61 instant edits, 28 rejected saves
swap latency after save: mean 0.84 s, max 1.56 s (one bar at 120 BPM is 2 s)
```

Zero late callbacks, zero stale plans, zero engines freed on the audio
thread. The worst callback is the swap's (two engines in one block plus the
fade); in `play` with no edits the worst is 2.4 ms.
