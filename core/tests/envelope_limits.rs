//! How long an envelope may take. An `adsr` node's decay and release go up
//! to 30 s, for a control envelope that falls for seconds. Everything else
//! stops at 10 s as it always did -- and that includes what was never
//! checked against a range: an `fm` module's `opN_envelope` takes any
//! number, and a song that wrote 25 there has always played 10.

use tatum_core::song_engine::SongEngine;
use tatum_core::SAMPLE_RATE;

fn render(src: &str, bars: u32) -> Vec<f32> {
    SongEngine::from_source(src).unwrap_or_else(|e| panic!("{e}")).render(bars).0
}

fn fm(decay: f32, release: f32) -> String {
    format!(
        "tempo 60\n\
         module fm bell {{\n  algorithm two_op\n  mod_index 60%\n  op0_ratio 1x\n  op1_ratio 3.5x\n  \
         op0_envelope 0.001 {decay} 0.0 {release}\n  op1_envelope 0.001 {decay} 0.0 {release}\n}}\n\
         pattern p {{ C4 ..*7 - - - - - - - - }}\n\
         track t {{ play p using bell out > master }}\n"
    )
}

/// 8 seconds of an FM note whose operators decay over "25 s" and release
/// over "25 s" are, to the bit, those of one that says 10: the limit an
/// operator envelope has always had. When the `adsr` node's went to 30 this
/// one went with it, and such a song would have rung two and a half times
/// as long.
#[test]
fn an_fm_operator_envelope_still_stops_at_ten_seconds() {
    let (long, ten) = (render(&fm(25.0, 25.0), 2), render(&fm(10.0, 10.0), 2));
    assert!(ten.iter().any(|v| v.abs() > 0.01), "the bell sounds");
    if let Some(i) = long.iter().zip(&ten).position(|(a, b)| a.to_bits() != b.to_bits()) {
        panic!("op envelopes of 25 s and of 10 s differ at sample {i}: {} against {}", long[i], ten[i]);
    }
    // The limit is there: 5 s is not 10.
    let five = render(&fm(5.0, 5.0), 2);
    assert!(five.iter().zip(&ten).any(|(a, b)| (a - b).abs() > 1e-3), "5 s and 10 s sound alike");
}

fn graph(decay: f32) -> String {
    format!(
        "tempo 60\n\
         instrument i {{\n  osc sine(220) as src\n  src > adsr(0.001, {decay}, 0.0, 0.05) as amp\n  amp > out\n}}\n\
         pattern p {{ C4 ..*15 }}\n\
         track t {{ play p using i level 0.2 out > master }}\n"
    )
}

/// The `adsr` node does go to 30: a decay of 25 s is slower than one of 10,
/// by what the times say. A decay falls 60 dB in its time, so three
/// seconds in, one of 10 s is 18 dB down and one of 25 s is 7.2.
#[test]
fn an_adsr_node_decays_for_up_to_thirty_seconds() {
    let at = (3.0 * SAMPLE_RATE) as usize;
    let level = |buf: &[f32]| buf[at..at + (0.02 * SAMPLE_RATE) as usize].iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let top = |buf: &[f32]| buf[..(0.05 * SAMPLE_RATE) as usize].iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let db = |buf: &[f32]| 20.0 * (level(buf) / top(buf)).log10();
    let (ten, long, longest) = (render(&graph(10.0), 1), render(&graph(25.0), 1), render(&graph(30.0), 1));
    assert!((db(&ten) + 18.0).abs() < 1.0, "a decay of 10 s is {} dB down after 3 s", db(&ten));
    assert!((db(&long) + 7.2).abs() < 1.0, "a decay of 25 s is {} dB down after 3 s", db(&long));
    assert!((db(&longest) + 6.0).abs() < 1.0, "a decay of 30 s is {} dB down after 3 s", db(&longest));
}
