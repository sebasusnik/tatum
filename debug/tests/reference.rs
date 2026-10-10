//! `tatum analyze` reads the engine's own songs back: a kick whose sweep is
//! written in the file lands where the file says, under a bass that starts
//! with it, and the tempo is the song's.

use tatum_core::dsl::isolate::{self, Isolation};
use tatum_debug::reference::{self, Profile};

fn measured(src: &str, bars: u32) -> Profile {
    let song = isolate::compile(src, &Isolation::default()).unwrap();
    let gain = isolate::output_gain(src).unwrap();
    let (l, r) = reference::render(song, gain, Some((1, bars))).unwrap();
    reference::measure(&l, &r)
}

fn semitones(a: f32, b: f32) -> f32 {
    12.0 * (a / b).log2()
}

/// A kick from `pitch_osc` + `perc`, alone for eight bars, then with a sub
/// bass on every beat, which is locked to it and so survives averaging.
fn song(bass_level: f32) -> String {
    format!(
        r#"
tempo 126
scale C minor
instrument boom {{
  pitch_osc sine(190, 46, 0.9985) as body
  body > perc(0.001, 0.16) as amp
  amp > out
}}
module bass sub {{ cutoff 300hz resonance 0% decay 300ms sustain 80% }}
pattern four {{ C3 .. .. -  C3 .. .. -  C3 .. .. -  C3 .. .. - }}
pattern roots {{ - - C2:0.9 ..  - - C2:0.9 ..  - - C2:0.9 ..  - - C2:0.9 .. }}
track kick {{ play four using boom level 0.9 }}
track bass {{ play roots using sub level {bass_level} }}
scene alone {{ track kick {{ play four using boom }} }}
scene full {{
  track kick {{ play four using boom }}
  track bass {{ play roots using sub }}
}}
arrange {{ alone x8 full x24 }}
"#
    )
}

#[test]
fn the_kick_lands_where_its_sweep_is_written() {
    let p = measured(&song(0.0), 16);
    let t = p.tempo.expect("a tempo");
    assert!((t - 126.0).abs() < 0.3, "tempo {t}");
    let k = p.kick.expect("a kick");
    assert!(semitones(k.tail_hz, 46.0).abs() < 0.5, "tail {} Hz", k.tail_hz);
    assert!(k.start_hz > 120.0, "hit {} Hz", k.start_hz);
    // The render, read sample by sample, is 12 dB down at about 45 ms.
    let d = k.decay12_ms.expect("a decay");
    assert!((35.0..60.0).contains(&d), "decay {d} ms");
    assert!(!k.masked());
}

#[test]
fn a_bass_under_it_does_not_move_it() {
    let p = measured(&song(0.9), 32);
    let k = p.kick.expect("a kick");
    assert!(semitones(k.tail_hz, 46.0).abs() < 0.5, "tail {} Hz", k.tail_hz);
    // Measured on the bars where it plays alone.
    assert!(k.measured < k.hits, "{} of {}", k.measured, k.hits);
}

#[test]
fn a_song_compared_with_itself_differs_in_nothing() {
    let p = measured(&song(0.9), 32);
    let report = reference::compare(&p, &p, "a", "b");
    assert!(report.contains("nothing differs"), "{report}");
}

/// Four bars, then four at another tempo, a kick on every beat.
fn two_tempos(first: u32, second: u32) -> String {
    format!(
        r#"
tempo {first}
instrument boom {{
  pitch_osc sine(190, 46, 0.9985) as body
  body > perc(0.001, 0.09) as amp
  amp > out
}}
pattern four {{ C3 - - -  C3 - - -  C3 - - -  C3 - - - }}
track kick {{ play four using boom }}
scene a {{ track kick {{ play four using boom }} }}
scene b {{ tempo {second} track kick {{ play four using boom }} }}
arrange {{ a x4 b x4 }}
"#
    )
}

fn rendered(src: &str, bars: Option<(u32, u32)>) -> Result<Vec<f32>, String> {
    let song = isolate::compile(src, &Isolation::default()).unwrap();
    let gain = isolate::output_gain(src).unwrap();
    reference::render(song, gain, bars).map(|(l, _)| l)
}

fn seconds(x: &[f32]) -> f32 {
    x.len() as f32 / tatum_core::SAMPLE_RATE
}

#[test]
fn bars_are_cut_where_they_fall_when_the_tempo_moves() {
    // Bars of 2 s, then bars of 1 s: bars 5-8 are the last four seconds.
    let src = two_tempos(120, 240);
    let l = rendered(&src, Some((5, 8))).unwrap();
    assert!((seconds(&l) - 4.0).abs() < 0.01, "{} s", seconds(&l));
    // It opens on bar 5's kick, not on the silent end of bar 4.
    let opening = l[..2205].iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(opening > 0.1, "opening peak {opening}");
    assert!((seconds(&rendered(&src, None).unwrap()) - 12.0).abs() < 0.01);
    // A slower scene later is rendered whole, not cut at the opening tempo.
    let l = rendered(&two_tempos(240, 120), None).unwrap();
    assert!((seconds(&l) - 12.0).abs() < 0.01, "{} s", seconds(&l));
}

#[test]
fn bars_past_the_end_are_an_error() {
    let e = rendered(&two_tempos(120, 240), Some((20, 24))).err().unwrap();
    assert!(e.contains("the song has 8 bars"), "{e}");
}
