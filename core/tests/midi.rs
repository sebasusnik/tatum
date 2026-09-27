//! MIDI controls: `midi { }` read by a live session.
//!
//! What has to hold for a knob: it moves what the block says it does, scaled
//! from the registry's own range; it reaches every copy of a module; the value
//! survives a save that rebuilds the engine; editing the knob's target in the
//! text takes it back; and a render never hears the block at all. For the
//! keys: they sound the track's instrument, and a mono one plays the newest
//! key held. For a pad: it hits the drum it names.

use tatum_core::dsl;
use tatum_core::dsl::ast::MidiSource;
use tatum_core::live::{FastOp, LivePlanner, LivePlayer, Plan};
use tatum_core::song_engine::SongEngine;
use tatum_core::BLOCK_SIZE;

const SONG: &str = r#"
tempo 124
scale A minor
module bass acid { cutoff 800hz resonance 60% }
module keys pad { voice_mode poly }
pattern line { 1.2 - 1.2 - 5.1 - 1.2 - }
pattern chords { [1.3 3.3 5.3] ..*15 }
track bass  { play line using acid level 0.7 out > lowpass(2khz, 0.1) as lp > master }
track stabs { play chords using pad level 0.4 out > master }
track wash  { play chords using pad level 0.3 out > master }
"#;

const KNOBS: &str = r#"
midi {
    cc 74 > acid cutoff
    cc 7  > pad level
    cc 10 > pad pan
    cc 20 > bass lp wet
    cc 21 > pad voice_mode
    cc 22 > reverb_mix
    cc 30 > bass level
}
"#;

fn song() -> String {
    format!("{}{}", SONG, KNOBS)
}

fn cc(source: MidiSource) -> u8 {
    match source {
        MidiSource::Cc(n) => n,
        other => panic!("not a knob: {:?}", other),
    }
}

fn ops(plans: &[Plan]) -> Vec<FastOp> {
    plans
        .iter()
        .filter_map(|p| match p {
            Plan::Control { op, .. } => Some(*op),
            _ => None,
        })
        .collect()
}

fn track(engine: &SongEngine, name: &str) -> usize {
    (0..engine.track_count()).find(|&i| engine.track_name(i) == name).expect("track")
}

/// A planner and a player with `src` loaded.
fn session(src: &str) -> (LivePlanner, LivePlayer) {
    let mut planner = LivePlanner::new();
    let mut player = LivePlayer::new();
    let plan = planner.plan(src, player.generation()).expect("compiles");
    player.apply(plan);
    (planner, player)
}

#[test]
fn a_midi_block_parses_into_dotted_targets() {
    let song = dsl::parse(&song()).expect("parses");
    let targets: Vec<(u8, &str)> = song.midi.iter().map(|m| (cc(m.source), m.target.as_str())).collect();
    assert_eq!(targets[0], (74, "acid.cutoff"));
    assert_eq!(targets[3], (20, "bass.lp.wet"));
    assert_eq!(targets[5], (22, "reverb_mix"));
}

#[test]
fn a_later_block_replaces_the_same_controller_and_keeps_the_rest() {
    let src = format!("{}{}\nmidi {{\n  cc 74 > pad level\n  cc 74 > acid resonance\n}}\n", SONG, KNOBS);
    let song = dsl::parse(&src).expect("parses");
    let on_74: Vec<&str> =
        song.midi.iter().filter(|m| m.source == MidiSource::Cc(74)).map(|m| m.target.as_str()).collect();
    // Both lines of the later block, and nothing of the earlier one.
    assert_eq!(on_74, ["pad.level", "acid.resonance"]);
    assert!(song.midi.iter().any(|m| m.source == MidiSource::Cc(7)), "cc 7 is untouched by the later block");
}

#[test]
fn a_knob_on_nothing_is_a_compile_error_on_its_line() {
    let src = format!("{}\nmidi {{\n  cc 74 > acid cutof\n  cc 20 > bass flt wet\n}}\n", SONG);
    let err = SongEngine::from_source(&src).err().expect("does not compile");
    let text = format!("{:?}", err);
    assert!(text.contains("Did you mean 'cutoff'"), "{}", text);
    assert!(text.contains("has no node named 'flt' (it has lp)"), "{}", text);
}

#[test]
fn a_render_never_hears_the_block() {
    let render = |src: &str| {
        let mut e = SongEngine::from_source(src).unwrap();
        e.start();
        e.render_steps(64)
    };
    assert_eq!(render(SONG), render(&song()), "the midi block changed the audio");
}

#[test]
fn a_knob_spans_what_the_registry_says() {
    let (mut planner, player) = session(&song());
    let g = player.generation();

    // cutoff is 0..1 on the knob: bottom and top of the travel.
    let low = ops(&planner.knob(74, 0, g).plans);
    let high = ops(&planner.knob(74, 127, g).plans);
    assert!(matches!(low[0], FastOp::ModuleParam { value, .. } if value == 0.0));
    assert!(matches!(high[0], FastOp::ModuleParam { value, .. } if value == 1.0));

    // Pan runs from hard left to hard right.
    let left = ops(&planner.knob(10, 0, g).plans);
    let right = ops(&planner.knob(10, 127, g).plans);
    assert!(matches!(left[0], FastOp::TrackPan { pan, .. } if pan == -1.0));
    assert!(matches!(right[0], FastOp::TrackPan { pan, .. } if pan == 1.0));

    // A fader at the top is `level 1.0`, not the ceiling a level can reach.
    assert!(matches!(ops(&planner.knob(30, 127, g).plans)[0], FastOp::TrackLevel { level, .. } if level == 1.0));
}

#[test]
fn a_knob_on_a_choice_steps_through_every_option() {
    let (mut planner, player) = session(&song());
    let g = player.generation();
    let reading = |planner: &mut LivePlanner, v| planner.knob(21, v, g).readings.join("");
    assert_eq!(reading(&mut planner, 0), "pad voice_mode poly");
    let top = reading(&mut planner, 127);
    assert!(!top.ends_with("poly"), "the top of the travel is the last option, got {}", top);
}

#[test]
fn a_knob_reads_in_the_targets_own_units() {
    let (mut planner, player) = session(&song());
    let g = player.generation();
    let read = |planner: &mut LivePlanner, cc, v| planner.knob(cc, v, g).readings.join("");
    assert!(read(&mut planner, 74, 64).ends_with("hz"), "cutoff reads in hertz");
    assert_eq!(read(&mut planner, 7, 127), "pad level +0.0 dB");
    assert_eq!(read(&mut planner, 7, 0), "pad level off");
    assert_eq!(read(&mut planner, 10, 0), "pad pan 100% L");
    assert_eq!(read(&mut planner, 20, 127), "bass lp wet 100%");
}

#[test]
fn a_module_knob_reaches_every_copy_and_a_level_every_track_it_plays_on() {
    let (mut planner, player) = session(&song());
    let g = player.generation();
    // `pad` plays on two tracks, and the engine gives each its own copy.
    assert_eq!(ops(&planner.knob(21, 127, g).plans).len(), 2, "voice_mode on both copies of pad");
    // `pad level` names no track, so it is the level of every track playing pad.
    assert_eq!(ops(&planner.knob(7, 127, g).plans).len(), 2, "level on both tracks playing pad");
    // `bass level` names a track: that one only.
    assert_eq!(ops(&planner.knob(30, 127, g).plans).len(), 1);
}

#[test]
fn a_controller_nothing_is_mapped_to_does_nothing() {
    let (mut planner, player) = session(&song());
    let turn = planner.knob(99, 64, player.generation());
    assert!(turn.plans.is_empty() && turn.readings.is_empty());
}

#[test]
fn a_knob_moves_the_engine_that_plays() {
    let (mut planner, mut player) = session(&song());
    for plan in planner.knob(30, 0, player.generation()).plans {
        player.apply(plan);
    }
    let engine = player.engine().unwrap();
    assert_eq!(engine.track_level(track(engine, "bass")), 0.0);
}

/// The level a `Plan::Swap` would bring in for `name`, read off the engine
/// before it plays.
fn swapped_level(plan: &Plan, name: &str) -> f32 {
    match plan {
        Plan::Swap { engine, .. } => engine.track_level(track(engine, name)),
        _ => panic!("expected a swap, got {}", plan.describe()),
    }
}

#[test]
fn a_save_that_rebuilds_the_engine_keeps_the_knob() {
    let src = song();
    let (mut planner, player) = session(&src);
    planner.knob(30, 0, player.generation());
    // An unused pattern is structural: a new engine is built.
    let edited = format!("{}\npattern unused {{ 1.1 - - - }}\n", src);
    let plan = planner.plan(&edited, player.generation()).unwrap();
    assert_eq!(swapped_level(&plan, "bass"), 0.0, "the save undid the knob");
}

#[test]
fn editing_the_knobs_target_in_the_text_takes_it_back() {
    let src = song();
    let (mut planner, mut player) = session(&src);
    planner.knob(30, 0, player.generation());
    // The text now says something else about exactly what the knob moves.
    let edited = src.replace("using acid level 0.7", "using acid level 0.9");
    let fast = planner.plan(&edited, player.generation()).unwrap();
    player.apply(fast);
    // And a later rebuild has no knob value left to put back.
    let rebuilt = format!("{}\npattern unused {{ 1.1 - - - }}\n", edited);
    let plan = planner.plan(&rebuilt, player.generation()).unwrap();
    assert!((swapped_level(&plan, "bass") - 0.9).abs() < 1e-6, "the text edit did not win");
}

#[test]
fn remapping_a_knob_takes_effect_without_a_rebuild() {
    let src = song();
    let (mut planner, player) = session(&src);
    let remapped = src.replace("cc 74 > acid cutoff", "cc 74 > bass level");
    let plan = planner.plan(&remapped, player.generation()).unwrap();
    assert_ne!(plan.describe(), "swap", "a new midi block should not rebuild the engine");
    let turn = planner.knob(74, 127, player.generation());
    assert!(matches!(ops(&turn.plans)[0], FastOp::TrackLevel { .. }), "cc 74 still moves the cutoff");
}

// ── Keys and pads ───────────────────────────────────────────────────────────

const PLAYED: &str = r#"
tempo 120
module bass lead { cutoff 0.5 glide 0.05 }
module keys chords { voice_mode poly release 50ms }
module beats kit { kick_level 1.0 }
pattern rest { -*16 }
track solo  { play rest using lead level 0.8 out > master }
track chord { play rest using chords level 0.8 out > master }
track drums { play rest using kit level 0.8 out > master }
midi {
    keys > solo
    keys > chord
    pad 36 > drums kick
    pad 38 > drums clap
}
"#;

/// The notes a list of plans plays on one track, as `(note, on)`.
fn notes_on(plans: &[Plan], engine: &SongEngine, name: &str) -> Vec<(u8, bool)> {
    let t = track(engine, name);
    plans
        .iter()
        .filter_map(|p| match p {
            Plan::Play { op: FastOp::NoteOn { track, note, .. }, .. } if *track == t => Some((*note, true)),
            Plan::Play { op: FastOp::NoteOff { track, note }, .. } if *track == t => Some((*note, false)),
            _ => None,
        })
        .collect()
}

/// How loud `blocks` blocks of the player come out, RMS over both sides.
fn loudness(player: &mut LivePlayer, blocks: usize) -> f32 {
    let (mut l, mut r) = ([0.0f32; BLOCK_SIZE], [0.0f32; BLOCK_SIZE]);
    let mut sum = 0.0f64;
    for _ in 0..blocks {
        player.process(&mut l, &mut r);
        sum += l.iter().chain(r.iter()).map(|x| (*x as f64) * (*x as f64)).sum::<f64>();
    }
    (sum / (blocks * BLOCK_SIZE * 2) as f64).sqrt() as f32
}

fn apply(player: &mut LivePlayer, plans: Option<Vec<Plan>>) {
    for plan in plans.expect("mapped") {
        player.apply(plan);
    }
}

#[test]
fn keys_and_pads_parse_as_their_own_sources() {
    let song = dsl::parse(PLAYED).expect("parses");
    let sources: Vec<MidiSource> = song.midi.iter().map(|m| m.source).collect();
    assert_eq!(sources, [MidiSource::Keys, MidiSource::Keys, MidiSource::Pad(36), MidiSource::Pad(38)]);
    assert_eq!(song.midi[2].target, "drums.kick");
}

#[test]
fn keys_and_pads_on_the_wrong_thing_are_compile_errors() {
    let bad = PLAYED
        .replace("keys > solo", "keys > drums")
        .replace("pad 36 > drums kick", "pad 36 > solo kick")
        .replace("pad 38 > drums clap", "pad 38 > drums cowbell");
    let text = format!("{:?}", SongEngine::from_source(&bad).err().expect("does not compile"));
    assert!(text.contains("'drums' plays drums, which the pads play"), "{}", text);
    assert!(text.contains("which is not a drum kit"), "{}", text);
    assert!(text.contains("'cowbell' is not a drum"), "{}", text);
}

#[test]
fn a_poly_instrument_gets_every_key_as_it_is() {
    let (mut planner, player) = session(PLAYED);
    let g = player.generation();
    let engine = player.engine().unwrap();
    let down = planner.key(60, 100, g).unwrap();
    assert_eq!(notes_on(&down, engine, "chord"), [(60, true)]);
    let up = planner.key(60, 0, g).unwrap();
    assert_eq!(notes_on(&up, engine, "chord"), [(60, false)]);
}

#[test]
fn a_mono_instrument_plays_the_newest_key_held() {
    let (mut planner, player) = session(PLAYED);
    let g = player.generation();
    let engine = player.engine().unwrap();
    let mut solo = |note, velocity| notes_on(&planner.key(note, velocity, g).unwrap(), engine, "solo");
    assert_eq!(solo(60, 100), [(60, true)]);
    assert_eq!(solo(64, 100), [(64, true)], "a second key glides to it");
    assert_eq!(solo(60, 0), [], "letting go of a key that is not sounding changes nothing");
    assert_eq!(solo(64, 0), [(64, false)], "the last key up releases");
    // Back to the key still held when the newest one lets go.
    solo(60, 100);
    solo(64, 100);
    assert_eq!(solo(64, 0), [(60, true)], "letting go of the top falls back to the key under it");
}

#[test]
fn a_key_sounds_and_its_release_lets_it_go() {
    let (mut planner, mut player) = session(PLAYED);
    player.start();
    assert!(loudness(&mut player, 8) < 1e-6, "the song itself is silent");
    let g = player.generation();
    apply(&mut player, planner.key(57, 110, g));
    assert!(loudness(&mut player, 8) > 1e-3, "a held key sounds");
    apply(&mut player, planner.key(57, 0, g));
    loudness(&mut player, 200);
    assert!(loudness(&mut player, 8) < 1e-4, "a released key dies away");
}

#[test]
fn a_key_on_a_muted_track_is_silent() {
    let muted = PLAYED
        .replace("using lead level 0.8", "using lead level 0")
        .replace("using chords level 0.8", "using chords level 0");
    let (mut planner, mut player) = session(&muted);
    player.start();
    let g = player.generation();
    apply(&mut player, planner.key(57, 110, g));
    assert!(loudness(&mut player, 8) < 1e-6, "the fader is down");
}

#[test]
fn a_pad_hits_the_drum_it_names() {
    let (mut planner, mut player) = session(PLAYED);
    let g = player.generation();
    let engine = player.engine().unwrap();
    let drums = track(engine, "drums");
    let hit = planner.pad(38, 100, g).unwrap();
    // `clap` is the note the kit plays its clap on, not the pad's own note.
    assert!(matches!(ops_of_play(&hit)[..], [FastOp::NoteOn { track, note: 39, .. }] if track == drums));
    assert!(planner.pad(38, 0, g).unwrap().is_empty(), "a pad coming up is not a second hit");
    assert!(planner.pad(40, 100, g).is_none(), "an unmapped pad says so");

    player.start();
    apply(&mut player, planner.pad(36, 127, g));
    assert!(loudness(&mut player, 8) > 1e-3, "the kick sounds");
}

fn ops_of_play(plans: &[Plan]) -> Vec<FastOp> {
    plans
        .iter()
        .filter_map(|p| match p {
            Plan::Play { op, .. } => Some(*op),
            _ => None,
        })
        .collect()
}

#[test]
fn a_note_reaches_whichever_engine_is_playing_and_never_the_queued_one() {
    let (mut planner, mut player) = session(PLAYED);
    player.start();
    // Queue a swap: the planner now knows two engines.
    let queued = planner.plan(&format!("{}\npattern unused {{ 1.1 - - - }}\n", PLAYED), player.generation()).unwrap();
    player.apply(queued);
    let plans = planner.key(57, 110, player.generation()).unwrap();
    let bases: Vec<u64> = plans
        .iter()
        .filter_map(|p| match p {
            Plan::Play { base, .. } => Some(*base),
            _ => None,
        })
        .collect();
    assert!(bases.contains(&player.generation()), "the playing engine gets the note");
    assert!(bases.iter().any(|b| *b != player.generation()), "and so does the queued one, resolved against it");
    // The one playing sounds it.
    for plan in plans {
        player.apply(plan);
    }
    assert!(loudness(&mut player, 8) > 1e-3);
}

#[test]
fn the_pitch_strip_bends_the_keys_two_semitones_either_way() {
    let (mut planner, player) = session(PLAYED);
    let g = player.generation();
    let ratio = |plans: Vec<Plan>| {
        plans
            .iter()
            .find_map(|p| match p {
                Plan::Control { op: FastOp::PitchBend { ratio, .. }, .. } => Some(*ratio),
                _ => None,
            })
            .unwrap()
    };
    assert!((ratio(planner.bend(16383, g)) - 2f32.powf(2.0 / 12.0)).abs() < 1e-3);
    assert!((ratio(planner.bend(0, g)) - 2f32.powf(-2.0 / 12.0)).abs() < 1e-3);
    assert!((ratio(planner.bend(8192, g)) - 1.0).abs() < 1e-6, "at rest is no bend");
}

/// Render until the player has swapped in the engine it was handed, then a
/// little more.
fn play_through_swap(player: &mut LivePlayer) {
    let (mut l, mut r) = ([0.0f32; BLOCK_SIZE], [0.0f32; BLOCK_SIZE]);
    for _ in 0..4000 {
        if player.process(&mut l, &mut r).is_some() {
            break;
        }
    }
    for _ in 0..8 {
        player.process(&mut l, &mut r);
    }
}

fn play_blocks(player: &mut LivePlayer, n: usize) {
    let (mut l, mut r) = ([0.0f32; BLOCK_SIZE], [0.0f32; BLOCK_SIZE]);
    for _ in 0..n {
        player.process(&mut l, &mut r);
    }
}

fn apply_all(player: &mut LivePlayer, plans: Vec<Plan>) {
    for p in plans {
        player.apply(p);
    }
}

#[test]
fn a_rebuild_while_playing_keeps_the_reverb_knob() {
    // The new engine used to start from the bar with both returns reset to
    // full, whatever the knob said.
    let src = song();
    let (mut planner, mut player) = session(&src);
    player.start();
    play_blocks(&mut player, 16);
    let g = player.generation();
    apply_all(&mut player, planner.knob(22, 20, g).plans);
    let edited = format!("{}\npattern unused {{ 1.1 - - - }}\n", src);
    player.apply(planner.plan(&edited, player.generation()).unwrap());
    play_through_swap(&mut player);
    assert_eq!(player.swaps(), 1);
    let mix = player.engine().unwrap().reverb_mix();
    assert!((mix - 20.0 / 127.0).abs() < 1e-3, "reverb_mix is {mix}");
}

/// Two one-bar scenes that both write the bass level, the second with an
/// `auto` lane on it.
const SCENES: &str = r#"
scene a { track bass { play line using acid level 0.7 } track stabs { play chords using pad } }
scene b {
    auto bass level 0.2 > 0.9
    track bass { play line using acid level 0.8 }
    track stabs { play chords using pad }
}
arrange { a x1 b x2 }
"#;

#[test]
fn a_scene_that_starts_does_not_move_a_held_fader() {
    let src = format!("{}{}", song(), SCENES);
    let (mut planner, mut player) = session(&src);
    player.start();
    play_blocks(&mut player, 4);
    let g = player.generation();
    apply_all(&mut player, planner.knob(30, 0, g).plans);
    // Into scene b, which writes 0.8 and sweeps 0.2 -> 0.9.
    let bar = (44100.0 * 60.0 / 124.0 * 4.0) as usize / BLOCK_SIZE;
    play_blocks(&mut player, bar + bar / 2);
    let engine = player.engine().unwrap();
    assert_eq!(engine.current_bar(), 1, "should be halfway through scene b");
    assert_eq!(engine.track_level(track(engine, "bass")), 0.0, "the scene or its auto lane moved the fader");
}

#[test]
fn a_value_written_in_the_text_lets_go_of_the_fader() {
    let src = format!("{}{}", song(), SCENES).replace("arrange { a x1 b x2 }", "arrange { a x4 }");
    let (mut planner, mut player) = session(&src);
    player.start();
    play_blocks(&mut player, 4);
    let g = player.generation();
    apply_all(&mut player, planner.knob(30, 0, g).plans);
    // A fast edit of exactly what the fader holds.
    let edited =
        src.replace("track bass  { play line using acid level 0.7", "track bass  { play line using acid level 0.5");
    assert_ne!(edited, src);
    assert_eq!(player.apply(planner.plan(&edited, player.generation()).unwrap()), tatum_core::live::Applied::Fast);
    // The scene restarts every bar; the fader would put 0 back if still held.
    let bar = (44100.0 * 60.0 / 124.0 * 4.0) as usize / BLOCK_SIZE;
    play_blocks(&mut player, bar * 2);
    let engine = player.engine().unwrap();
    assert!(engine.held().is_empty(), "{:?}", engine.held());
    assert!(engine.track_level(track(engine, "bass")) > 0.0);
}

fn module_value(op: &FastOp) -> f32 {
    match op {
        FastOp::ModuleParam { value, .. } => *value,
        other => panic!("not a module parameter: {:?}", other),
    }
}

#[test]
fn a_knob_with_a_range_sweeps_between_its_ends_in_the_targets_units() {
    use tatum_core::params::{self, ModuleKind};
    // One knob, three things, each over its own stretch: a macro. The level
    // is written high to low, so that fader works backwards.
    let src = format!(
        "{}\nmidi {{\n  cc 74 > acid cutoff 200hz..4khz\n  cc 74 > acid resonance 10%..60%\n  cc 74 > bass level 0.8..0\n}}\n",
        SONG
    );
    let (mut planner, player) = session(&src);
    let g = player.generation();
    let cutoff = params::lookup(ModuleKind::Bass, "cutoff").unwrap();
    let at = |ops: &[FastOp], i: usize| ops[i];

    let low = ops(&planner.knob(74, 0, g).plans);
    let high = ops(&planner.knob(74, 127, g).plans);
    assert_eq!(low.len(), 3, "{low:?}");
    let want_low = cutoff.value_from_quantity(200.0, "hz").unwrap();
    let want_high = cutoff.value_from_quantity(4.0, "khz").unwrap();
    assert!((module_value(&at(&low, 0)) - want_low).abs() < 1e-5);
    assert!((module_value(&at(&high, 0)) - want_high).abs() < 1e-5);
    assert!((module_value(&at(&low, 1)) - 0.10).abs() < 1e-5);
    assert!((module_value(&at(&high, 1)) - 0.60).abs() < 1e-5);
    assert!(matches!(at(&low, 2), FastOp::TrackLevel { level, .. } if (level - 0.8).abs() < 1e-6));
    assert!(matches!(at(&high, 2), FastOp::TrackLevel { level, .. } if level.abs() < 1e-6));

    // Halfway is halfway between the ends, on the parameter's own curve.
    let mid = ops(&planner.knob(74, 64, g).plans);
    let v = module_value(&at(&mid, 0));
    assert!(v > want_low && v < want_high);
    let reading = planner.knob(74, 127, g).readings;
    assert!(reading[0].contains("4") && reading[0].contains("khz"), "{reading:?}");
}

#[test]
fn a_range_the_target_cannot_take_is_a_compile_error_on_its_line() {
    for (line, says) in [
        ("cc 74 > acid cutoff 20ms..40ms", "'cutoff'"),
        ("cc 21 > pad voice_mode 0..1", "is a choice"),
        ("cc 22 > reverb_freeze 0..1", "takes no range"),
        ("cc 30 > bass level 0..9", "a level runs from 0 to"),
        ("cc 10 > pad pan -2..1", "outside -1..1"),
        ("cc 74 > acid cutoff 200hz 4khz", "two values with `..`"),
    ] {
        let src = format!("{}\nmidi {{\n  {}\n}}\n", SONG, line);
        let err = SongEngine::from_source(&src).err().unwrap_or_else(|| panic!("{line} compiled"));
        let text = format!("{:?}", err);
        assert!(text.contains(says), "{line}: {text}");
    }
}

#[test]
fn a_level_range_takes_decibels() {
    let src = format!("{}\nmidi {{\n  cc 30 > bass level -12db..0db\n}}\n", SONG);
    let (mut planner, player) = session(&src);
    let low = ops(&planner.knob(30, 0, player.generation()).plans);
    assert!(matches!(low[0], FastOp::TrackLevel { level, .. } if (level - 0.2512).abs() < 1e-3), "{low:?}");
}
