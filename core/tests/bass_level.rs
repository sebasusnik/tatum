//! A bass keeps its level over a song.

use tatum_core::song_engine::SongEngine;

/// Each oscillator drifts by a few cents on its own, and the phase between
/// two of them at the same pitch is the sum of that drift, a walk with
/// nothing pulling it back. Left free, a unison sub lost 4.6 dB over 128 bars
/// as its oscillators slid from adding up to cancelling. They now start
/// together on every note from silence.
#[test]
fn a_unison_bass_does_not_wander_over_a_song() {
    let src = "tempo 120\nmodule bass sub { osc1_wave square osc2_wave square cutoff 110hz cutoff_env 20hz \
               resonance 0% attack 5ms decay 1s sustain 100% release 120ms }\n\
               pattern p { - - C2:0.9 .. .. .. - C2:0.7 .. .. - - C2:0.8 .. .. - }\n\
               track sub { play p using sub level 0.5 gate 0.95 out > master }\n";
    let mut e = SongEngine::from_source(src).expect("compiles");
    e.start();
    let (l, _) = e.render(96);
    let bar = l.len() / 96;
    let level = |from: usize| {
        let s = &l[from * bar..(from + 8) * bar];
        10.0 * (s.iter().map(|v| v * v).sum::<f32>() / s.len() as f32).log10()
    };
    let levels: Vec<f32> = (0..96).step_by(16).map(level).collect();
    let (lo, hi) = levels.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    assert!(hi - lo < 0.5, "the sub's level wandered {:.1} dB over the song: {levels:?}", hi - lo);
}

/// `osc2_pitch 0.3st` is a detune. The offsets used to be whole semitones cut
/// with `as i8`: every fraction was dropped, so a detuned patch played as a
/// plain unison, and `11.92st` truncated to 11, a major seventh.
#[test]
fn a_fraction_of_a_semitone_detunes() {
    let render = |pitch: &str| {
        let src = format!(
            "tempo 120\nmodule bass b {{ osc2_pitch {pitch} cutoff 2khz sustain 100% }}\n\
             pattern p {{ C2:0.9 ..*15 }}\ntrack t {{ play p using b out > master }}\n"
        );
        let mut e = SongEngine::from_source(&src).expect("compiles");
        e.start();
        e.render(1).0
    };
    assert_ne!(render("0st"), render("0.3st"), "0.3st must not play as a unison");
    assert_eq!(render("0st"), render("0.0st"));
}
