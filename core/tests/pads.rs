//! The performance pads: `play` sounds a note on a track while held, `hold`
//! lets a track in only while held, and `repeat` rolls one division of the
//! output. And a knob still wins over a lane written in the text.

use tatum_core::dsl;
use tatum_core::live::{LivePlanner, LivePlayer, Plan};
use tatum_core::midi::play_note;
use tatum_core::song_engine::SongEngine;
use tatum_core::{BLOCK_SIZE, SAMPLE_RATE};

const STEP: usize = (SAMPLE_RATE * 60.0 / 120.0 / 4.0) as usize;
const BAR: usize = STEP * 16;

const SONG: &str = r#"
tempo 120
scale A minor
module bass drone { cutoff 0.6 sustain 1.0 }
module bass low { cutoff 0.5 sustain 0.8 }
module beats kit { kick_level 1.0 }
pattern rest { -*16 }
pattern beat { kick: X - - - X - - - X - - - X - - - }
pattern line { 1.1 1.3 - 1.5 1.1 - 1.7 1.3 1.1 - 1.5 1.3 - 1.1 1.7 1.5 }
track drone { play rest using drone level 0.6 out > master }
track kick  { play beat using kit level 0.8 out > master }
track bass  { play line using low level 0.5 out > master }
"#;

const PADS: &str = r#"
midi {
    pad 40 > play drone
    pad 39 > play drone C2
    pad 41 > hold kick
    pad 38 > repeat 1/16
    pad 37 > repeat 1/32
}
"#;

fn session(src: &str) -> (LivePlanner, LivePlayer) {
    let mut planner = LivePlanner::new();
    planner.set_output_gain(1.0);
    let mut player = LivePlayer::new();
    let plan = planner.plan(src, player.generation()).expect("compiles");
    player.apply(plan);
    player.start();
    (planner, player)
}

fn render(player: &mut LivePlayer, samples: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(samples);
    let (mut l, mut r) = ([0.0f32; BLOCK_SIZE], [0.0f32; BLOCK_SIZE]);
    while out.len() < samples {
        let n = BLOCK_SIZE.min(samples - out.len());
        player.process(&mut l[..n], &mut r[..n]);
        out.extend_from_slice(&l[..n]);
        while player.take_retired().is_some() {}
    }
    out
}

fn pad(planner: &mut LivePlanner, player: &mut LivePlayer, note: u8, velocity: u8) {
    for p in planner.pad(note, velocity, player.generation()).expect("mapped") {
        player.apply(p);
    }
}

/// The first sample where two renders part, if they do.
fn parts(a: &[f32], b: &[f32]) -> Option<usize> {
    a.iter().zip(b).position(|(x, y)| x.to_bits() != y.to_bits())
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

#[test]
fn the_new_pad_lines_parse() {
    let song = dsl::parse(&format!("{SONG}{PADS}")).expect("parses");
    let targets: Vec<&str> = song.midi.iter().map(|m| m.target.as_str()).collect();
    assert_eq!(targets, ["play.drone", "play.drone.C2", "hold.kick", "repeat.1/16", "repeat.1/32"]);
    assert!(SongEngine::from_source(&format!("{SONG}{PADS}")).is_ok());
}

#[test]
fn a_bad_pad_line_says_what_it_takes() {
    for (line, says) in [
        ("pad 40 > repeat 1/3", "1/4, 1/8, 1/16 or 1/32"),
        ("pad 40 > play drone H2", "is not a note"),
        ("pad 40 > play kick bell", "is not a drum"),
        ("pad 40 > play nobody", "no track named 'nobody'"),
        ("pad 40 > hold nobody", "no track named 'nobody'"),
    ] {
        let err = SongEngine::from_source(&format!("{SONG}\nmidi {{\n  {line}\n}}\n")).err().expect(line);
        assert!(err.contains(says), "{line}: {err}");
    }
}

/// The root of the scale, in octave 2 on a bass and 3 on anything else; the
/// kick on a kit; or what is written.
#[test]
fn a_play_pad_sounds_the_root_unless_told_otherwise() {
    let song =
        dsl::parse(&format!("{SONG}module keys pad {{ voice_mode poly }}\ntrack chords {{ play rest using pad }}\n"))
            .unwrap();
    assert_eq!(play_note(&song, "drone", None), Some(45), "A2");
    assert_eq!(play_note(&song, "chords", None), Some(57), "A3");
    assert_eq!(play_note(&song, "kick", None), Some(36), "the kick");
    assert_eq!(play_note(&song, "kick", Some("snare")), Some(38));
    assert_eq!(play_note(&song, "drone", Some("C2")), Some(36));
    assert_eq!(play_note(&song, "drone", Some("H2")), None);
}

/// A track whose pattern is all rests sounds while the pad is held, and
/// lets go when it comes up. At `level 0` it stays silent, as its pattern
/// and the keys do.
#[test]
fn a_play_pad_sounds_a_silent_track_while_held() {
    let only_drone = format!("{}{PADS}", SONG.replace("level 0.8", "level 0").replace("level 0.5", "level 0"));
    let (mut planner, mut player) = session(&only_drone);
    let before = rms(&render(&mut player, BAR));
    pad(&mut planner, &mut player, 40, 100);
    let held = rms(&render(&mut player, BAR));
    pad(&mut planner, &mut player, 40, 0);
    render(&mut player, BAR);
    let after = rms(&render(&mut player, BAR / 2));
    assert!(before < 1e-5, "the rests sounded: {before}");
    assert!(held > 0.01, "the pad did not sound: {held}");
    assert!(after < held * 0.01, "the note did not let go: {after} after {held}");

    let muted = only_drone
        .replace("track drone { play rest using drone level 0.6", "track drone { play rest using drone level 0");
    let (mut planner, mut player) = session(&muted);
    pad(&mut planner, &mut player, 40, 100);
    assert!(rms(&render(&mut player, BAR)) < 1e-5, "a track at level 0 sounded a pad");
}

/// `hold`: out until the pad goes down, in on the grid while it is held,
/// out again when it comes up.
#[test]
fn a_hold_pad_lets_its_track_in_only_while_held() {
    let only_kick = format!("{}{PADS}", SONG.replace("level 0.5", "level 0"));
    let (mut planner, mut player) = session(&only_kick);
    let before = rms(&render(&mut player, BAR));
    pad(&mut planner, &mut player, 41, 100);
    let held = rms(&render(&mut player, BAR));
    pad(&mut planner, &mut player, 41, 0);
    render(&mut player, BAR / 4);
    let after = rms(&render(&mut player, BAR));
    assert!(before < 1e-4, "the kick played before its pad: {before}");
    assert!(held > 0.01, "the kick did not come in: {held}");
    assert!(after < 1e-4, "the kick stayed in: {after}");
}

/// The hold survives a save that rebuilds the engine: the kick is still out.
#[test]
fn a_hold_track_stays_out_across_a_rebuild() {
    let only_kick = format!("{}{PADS}", SONG.replace("level 0.5", "level 0"));
    let (mut planner, mut player) = session(&only_kick);
    render(&mut player, BAR / 2);
    let edit = only_kick.replace("X - - - X - - - X - - - X - - -", "X - - - X - - - X - X - X - - -");
    let plan = planner.plan(&edit, player.generation()).unwrap();
    assert_eq!(plan.describe(), "swap");
    player.apply(plan);
    render(&mut player, BAR);
    assert!(rms(&render(&mut player, BAR)) < 1e-4, "the rebuilt engine let the kick in");
}

/// `repeat 1/16`: from the next sixteenth line one sixteenth is caught and
/// looped; the song goes on underneath, so after letting go the output is
/// the song exactly as it would have been.
#[test]
fn a_repeat_rolls_one_division_and_lets_the_song_back_in() {
    let src = format!("{SONG}{PADS}");
    let (_, mut straight) = session(&src);
    let reference = render(&mut straight, 4 * BAR);

    let (mut planner, mut player) = session(&src);
    // About a third into a sixteenth, on a block: the reference is rendered
    // in whole blocks, and the press lands between two.
    let press = (BAR + 3 * STEP + STEP / 3) / BLOCK_SIZE * BLOCK_SIZE;
    let mut out = render(&mut player, press);
    // The engine is mid-block; the press applies at the next block.
    pad(&mut planner, &mut player, 38, 100);
    out.extend(render(&mut player, 2 * BAR - press));
    pad(&mut planner, &mut player, 38, 0);
    out.extend(render(&mut player, 2 * BAR));

    let grid = BAR + 4 * STEP;
    let len = STEP;
    assert_eq!(
        parts(&out[..grid + len], &reference[..grid + len]),
        None,
        "before the roll, and while it is caught, the song plays"
    );
    // Every pass after that is the caught slice, away from its edges.
    for pass in 1..8 {
        for k in 64..len - 64 {
            let (got, want) = (out[grid + pass * len + k], reference[grid + k]);
            assert!((got - want).abs() < 1e-6, "pass {pass}, sample {k}: {got} vs {want}");
        }
    }
    assert!(rms(&out[grid + 2 * len..grid + 3 * len]) > 0.01);
    let back = 2 * BAR + BLOCK_SIZE + 256;
    assert_eq!(parts(&out[back..], &reference[back..]), None, "after letting go, the song as it was");
}

/// A shorter division pressed while a roll plays cuts the slice already
/// caught; a release before the roll starts never rolls at all.
#[test]
fn a_shorter_repeat_cuts_the_slice_and_an_early_release_never_rolls() {
    let src = format!("{SONG}{PADS}");
    let (_, mut straight) = session(&src);
    let reference = render(&mut straight, 3 * BAR);

    let (mut planner, mut player) = session(&src);
    let mut out = render(&mut player, BAR);
    pad(&mut planner, &mut player, 38, 100);
    out.extend(render(&mut player, 4 * STEP));
    pad(&mut planner, &mut player, 37, 100);
    out.extend(render(&mut player, 4 * STEP));
    let half = STEP / 2;
    // The last stretch loops the first half of the slice caught at bar 1.
    let end = out.len();
    for k in 64..half - 64 {
        let a = out[end - 2 * half + k];
        assert!((a - reference[BAR + k]).abs() < 1e-6, "sample {k}");
    }

    let (mut planner, mut player) = session(&src);
    let mut out = render(&mut player, BAR + BLOCK_SIZE);
    pad(&mut planner, &mut player, 38, 100);
    pad(&mut planner, &mut player, 38, 0);
    out.extend(render(&mut player, 2 * BAR - out.len()));
    assert_eq!(parts(&out, &reference[..2 * BAR]), None, "a repeat let go before its line still rolled");
}

/// A knob held on a level that a top-level `auto` lane also moves: the
/// knob wins, and keeps winning while the lane would hold.
#[test]
fn a_knob_wins_over_a_top_level_lane() {
    let src = format!("{SONG}auto bass level 0 > 1 over 4\nmidi {{\n  cc 30 > bass level\n}}\n");
    let (mut planner, mut player) = session(&src);
    render(&mut player, BAR);
    let turn = planner.knob(30, 20, player.generation());
    for p in turn.plans {
        player.apply(p);
    }
    let engine = |p: &LivePlayer| {
        let e = p.engine().unwrap();
        let t = (0..e.track_count()).find(|&i| e.track_name(i) == "bass").unwrap();
        e.track_level(t)
    };
    let knob = engine(&player);
    render(&mut player, 5 * BAR);
    assert!((engine(&player) - knob).abs() < 1e-6, "the lane moved the knob's level: {} vs {knob}", engine(&player));
    assert!(knob < 0.5, "{knob}");
    let _ = Plan::Unchanged;
}

/// A track a `hold` pad keeps out is out of the measurement a live session
/// levels the song by: turning its level does not move the gain.
#[test]
fn a_held_out_track_is_not_measured() {
    let measure = |src: &str, rest: bool| {
        let mut ast = dsl::parse(src).unwrap();
        assert_eq!(tatum_core::midi::at_rest(&mut ast), rest);
        SongEngine::loudness(&dsl::compiler::compile(&ast).unwrap()).unwrap()
    };
    let quiet = format!("{SONG}{PADS}");
    let loud =
        quiet.replace("track kick  { play beat using kit level 0.8", "track kick  { play beat using kit level 1.6");
    assert!((measure(&quiet, true) - measure(&loud, true)).abs() < 1e-3, "the held-out kick was measured");
    // Without the pad it is, which is what makes the test mean anything.
    let (q, l) = (quiet.replace("pad 41 > hold kick", ""), loud.replace("pad 41 > hold kick", ""));
    assert!((measure(&q, false) - measure(&l, false)).abs() > 0.5);
}
