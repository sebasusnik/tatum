//! Pads with `q=`: press and release land on the grid, like launch
//! quantization in a DAW. Within the first quarter of a division after a
//! line an event counts as that line; a release never lands before the line
//! after its press.

use tatum_core::live::{LivePlanner, LivePlayer, QUANTIZE_FORGIVENESS};
use tatum_core::song_engine::SongEngine;
use tatum_core::SAMPLE_RATE;

/// 120 BPM: a sixteenth is 5512.5 samples, a beat 22050, a bar 88200.
const STEP: f64 = SAMPLE_RATE as f64 * 60.0 / 120.0 / 4.0;
const BEAT: f64 = STEP * 4.0;
const BAR: f64 = STEP * 16.0;
/// How finely the tests watch: the player is run this many samples at a time.
const CHUNK: usize = 16;

const SONG: &str = r#"
tempo 120
module bass drone { cutoff 0.6 sustain 1.0 }
module beats kit { kick_level 1.0 }
pattern rest { -*16 }
pattern beat { kick: X - - - X - - - X - - - X - - - }
track drone { play rest using drone level 0.6 out > master }
track kick  { play beat using kit level 0.8 out > master }
midi {
    pad 40 > mute kick q=bar
    pad 41 > play drone q=1/16
    pad 42 > kick crash q=beat
    pad 43 > mute kick
}
"#;

struct Session {
    planner: LivePlanner,
    player: LivePlayer,
    at: usize,
    out: Vec<f32>,
}

impl Session {
    fn new(src: &str) -> Self {
        let mut planner = LivePlanner::new();
        planner.set_output_gain(1.0);
        let mut player = LivePlayer::new();
        player.apply(planner.plan(src, player.generation()).expect("compiles"));
        player.start();
        Self { planner, player, at: 0, out: Vec::new() }
    }

    /// Run to sample `to`, a chunk at a time, calling `watch` after each.
    fn run(&mut self, to: f64, mut watch: impl FnMut(usize, &SongEngine)) {
        let (mut l, mut r) = ([0.0f32; CHUNK], [0.0f32; CHUNK]);
        while (self.at as f64) < to {
            self.player.process(&mut l, &mut r);
            self.out.extend_from_slice(&l);
            self.at += CHUNK;
            watch(self.at, self.player.engine().unwrap());
            while self.player.take_retired().is_some() {}
        }
    }

    fn pad(&mut self, note: u8, velocity: u8) {
        let g = self.player.generation();
        for p in self.planner.pad(note, velocity, g).expect("mapped") {
            self.player.apply(p);
        }
    }

    /// The samples at which the kick went out and came back, from now to `to`.
    fn kick_changes(&mut self, to: f64) -> Vec<usize> {
        let mut seen = Vec::new();
        let mut last = self.player.engine().unwrap().track_muted(1);
        self.run(to, |at, e| {
            if e.track_muted(1) != last {
                last = !last;
                seen.push(at);
            }
        });
        seen
    }
}

/// Where a note on the downbeat of the song first makes a sample: every
/// voice takes this long to speak, pattern or pad, so a quantized note is on
/// its line when it speaks this long after it.
fn onset() -> f64 {
    let mut s = Session::new(SONG);
    s.run(0.25 * BAR, |_, _| {});
    s.out.iter().position(|v| v.abs() > 1e-6).expect("the kick on the one") as f64
}

/// `at` is within one chunk after `line`.
fn on(at: usize, line: f64) -> bool {
    let d = at as f64 - line;
    (0.0..=(CHUNK + 2) as f64).contains(&d)
}

#[test]
fn q_parses_and_is_refused_where_it_means_nothing() {
    let song = tatum_core::dsl::parse(SONG).unwrap();
    use tatum_core::dsl::ast::Quantize;
    let q: Vec<_> = song.midi.iter().map(|m| m.quantize).collect();
    assert_eq!(q, [Some(Quantize::Bar), Some(Quantize::Sixteenth), Some(Quantize::Beat), None]);
    assert!(SongEngine::from_source(SONG).is_ok());
    for (line, says) in [
        ("cc 74 > drone cutoff q=bar", "a knob or the keys act at once"),
        ("pad 44 > repeat 1/16 q=bar", "keeps its own time"),
        ("pad 44 > tapestop q=bar", "keeps its own time"),
        ("pad 44 > next q=beat", "keeps its own time"),
        ("pad 44 > mute kick q=1/3", "bar, beat, 1/8, 1/16 or off"),
    ] {
        let err = SongEngine::from_source(&format!("{SONG}midi {{\n  {line}\n}}\n")).err().expect(line);
        assert!(err.contains(says), "{line}: {err}");
    }
    let off = tatum_core::dsl::parse(&format!("{SONG}midi {{\n  pad 44 > mute kick q=off\n}}\n")).unwrap();
    assert_eq!(off.midi.last().unwrap().quantize, None);
}

/// `q=bar`: pressed mid-bar, out on the next bar line; let go mid-bar, back
/// on the line after. Unquantized, the same pad acts at once.
#[test]
fn a_bar_quantized_pad_acts_on_the_bar_lines() {
    let mut s = Session::new(SONG);
    s.run(0.5 * BAR, |_, _| {});
    s.pad(40, 100);
    s.run(0.6 * BAR, |_, _| {});
    s.pad(40, 0);
    let changes = s.kick_changes(2.5 * BAR);
    assert_eq!(changes.len(), 2, "{changes:?}");
    assert!(on(changes[0], BAR), "out at {} not on bar 1 ({BAR})", changes[0]);
    // The release came before the press had landed: it waits for the line
    // after the press, so even this tap lasts a bar.
    assert!(on(changes[1], 2.0 * BAR), "back at {} not on bar 2", changes[1]);

    let mut s = Session::new(SONG);
    s.run(0.5 * BAR, |_, _| {});
    s.pad(43, 100);
    assert!(s.player.engine().unwrap().track_muted(1), "the unquantized pad waited");
}

/// Within the first quarter of a bar the press counts as that bar's line
/// and acts at once; later, it waits. A tap released straight after still
/// lasts until the next line.
#[test]
fn a_late_press_inside_the_window_counts_as_the_line() {
    assert_eq!(QUANTIZE_FORGIVENESS, 0.25);
    let mut s = Session::new(SONG);
    s.run(BAR + 0.1 * BAR, |_, _| {});
    s.pad(40, 100);
    assert!(s.player.engine().unwrap().track_muted(1), "the forgiven press waited");
    s.run(BAR + 0.15 * BAR, |_, _| {});
    s.pad(40, 0);
    let changes = s.kick_changes(3.0 * BAR);
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert!(on(changes[0], 2.0 * BAR), "the tap let go at {} not bar 2", changes[0]);

    let mut s = Session::new(SONG);
    s.run(BAR + 0.3 * BAR, |_, _| {});
    s.pad(40, 100);
    let changes = s.kick_changes(2.5 * BAR);
    assert!(on(changes[0], 2.0 * BAR), "a press 30% in should wait for bar 2: {changes:?}");
}

/// `q=1/16` on a note: the note starts on the next sixteenth, to the sample,
/// not when the pad was hit.
#[test]
fn a_sixteenth_quantized_note_starts_on_the_line() {
    let only = SONG.replace("track kick  { play beat using kit level 0.8", "track kick  { play beat using kit level 0");
    let mut s = Session::new(&only);
    let hit = 3.0 * BEAT + 0.4 * STEP;
    s.run(hit, |_, _| {});
    s.pad(41, 100);
    s.run(hit + 2.0 * STEP, |_, _| {});
    let first = s.out.iter().position(|v| v.abs() > 1e-6).expect("the note sounded");
    let line = 3.0 * BEAT + STEP + onset();
    assert!((first as f64 - line).abs() < 3.0, "the note started at {first}, the line is {line}");
}

/// `q=beat` on a drum: struck on the beat; letting go does nothing.
#[test]
fn a_beat_quantized_drum_is_struck_on_the_beat() {
    let only =
        SONG.replace("track kick  { play beat using kit level 0.8", "track kick  { play rest using kit level 0.8");
    let mut s = Session::new(&only);
    s.run(1.5 * BEAT, |_, _| {});
    s.pad(42, 110);
    s.pad(42, 0);
    s.run(1.9 * BEAT, |_, _| {});
    assert!(s.out.iter().all(|v| v.abs() < 1e-6), "the crash came before the beat");
    assert_eq!(s.player.waiting_pads(), 1, "the press waits; the release of a drum is nothing");
    s.run(2.5 * BEAT, |_, _| {});
    let first = s.out.iter().position(|v| v.abs() > 1e-6).expect("struck");
    let beat = 2.0 * BEAT + onset();
    assert!((first as f64 - beat).abs() < 3.0, "struck at {first}, the beat speaks at {beat}");
}
