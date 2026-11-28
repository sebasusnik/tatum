//! A note starts on its own sample, not on the first sample of the block it
//! falls in. The engine renders in blocks of 128; rendering each block after
//! the step clock had run through it started every note of the block at its
//! top, up to 2.9 ms early, so a four-to-the-floor kick at 120 BPM came
//! 22016 and 22144 samples apart instead of 22050.

use tatum_core::song_engine::SongEngine;

fn render(src: &str, bars: u32) -> Vec<f32> {
    let mut e = SongEngine::from_source(src).unwrap_or_else(|err| panic!("{}", err));
    let (l, _) = e.render(bars);
    l
}

/// Where each hit starts: the first sound after a silence.
fn onsets(x: &[f32]) -> Vec<usize> {
    let mut on = Vec::new();
    let mut quiet = usize::MAX;
    for (i, v) in x.iter().enumerate() {
        if *v != 0.0 {
            if quiet > 1000 {
                on.push(i);
            }
            quiet = 0;
        } else {
            quiet = quiet.saturating_add(1);
        }
    }
    on
}

/// A short kick that starts at its peak (phase 90), so its first sample is
/// where it starts, alone four to the bar.
fn graph_kick(tempo: f32) -> String {
    format!(
        r#"
tempo {tempo}
instrument boom {{
  kick(50, phase=90, hold=20ms, release=60ms) as k
  k > out
}}
pattern four {{ C3 - - -  C3 - - -  C3 - - -  C3 - - - }}
track kick {{ play four using boom }}
"#
    )
}

/// The drum machine's kick, four to the bar.
fn machine_kick(tempo: f32) -> String {
    format!(
        r#"
tempo {tempo}
module beats kit {{ kick_decay 5% }}
pattern four {{ kick: x - - -  x - - -  x - - -  x - - - }}
track kick {{ play four using kit }}
"#
    )
}

/// Every gap between hits is the beat, to the sample it can be: a beat that
/// is not a whole number of samples lands on one side or the other of it.
fn assert_on_the_beat(src: &str, tempo: f32) {
    let x = render(src, 4);
    let on = onsets(&x);
    assert!(on.len() >= 15, "{} hits", on.len());
    let beat = tatum_core::SAMPLE_RATE * 60.0 / tempo;
    for (n, &o) in on.iter().enumerate() {
        let due = on[0] as f32 + n as f32 * beat;
        assert!((o as f32 - due).abs() <= 1.0, "hit {} at {o}, due at {due:.1} (beat {beat:.2})", n + 1);
    }
}

#[test]
fn a_graph_instrument_starts_on_its_sample() {
    for tempo in [120.0, 123.0, 128.0, 137.0] {
        assert_on_the_beat(&graph_kick(tempo), tempo);
    }
}

#[test]
fn the_drum_machine_starts_on_its_sample() {
    for tempo in [120.0, 123.0, 128.0, 137.0] {
        assert_on_the_beat(&machine_kick(tempo), tempo);
    }
}

#[test]
fn two_tracks_on_the_same_step_start_together() {
    // A kick and a second voice on the same step start on the same sample,
    // whatever track they are on.
    let one = |which: &str| {
        render(
            &format!(
                r#"
tempo 123
instrument boom {{
  kick(50, phase=90, hold=20ms, release=60ms) as k
  k > out
}}
pattern four {{ C3 - - -  C3 - - -  C3 - - -  C3 - - - }}
track a {{ play four using boom level {} }}
track b {{ play four using boom level {} }}
"#,
                if which == "a" { 0.8 } else { 0.0 },
                if which == "b" { 0.8 } else { 0.0 }
            ),
            2,
        )
    };
    assert_eq!(onsets(&one("a")), onsets(&one("b")));
}
