//! Per-step filter locks on `keys`.

use tatum_core::song_engine::SongEngine;

fn render(pattern: &str) -> Vec<f32> {
    let src = format!(
        "tempo 120\nmodule keys k {{ voice_mode poly cutoff 5khz resonance 20% chorus_mix 0% attack 1ms decay 300ms sustain 50% release 50ms }}\npattern p {{ {pattern} }}\ntrack t {{ play p using k gate 0.9 out > master }}\n"
    );
    let mut e = SongEngine::from_source(&src).expect("compiles");
    e.start();
    let (l, _) = e.render(1);
    l
}

/// Energy of a stretch of the render after a crude first difference, which
/// weighs the highs: a lower cutoff reads as less of it.
fn brightness(x: &[f32]) -> f32 {
    x.windows(2).map(|w| (w[1] - w[0]) * (w[1] - w[0])).sum()
}

/// A quarter note at 120 BPM in samples.
const Q: usize = 22_050;

#[test]
fn a_cutoff_lock_darkens_its_own_note_and_only_that_one() {
    let open = render("C4 - - - C4 - - - - - - - - - - -");
    let locked = render("C4(cutoff=0.1) - - - C4 - - - - - - - - - - -");
    let (a, b) = (brightness(&open[..Q]), brightness(&locked[..Q]));
    assert!(b < a * 0.5, "the locked note should be darker: {b} vs {a}");
    // The next note, without a lock, is back on the module's filter.
    let (c, d) = (brightness(&open[Q..2 * Q]), brightness(&locked[Q..2 * Q]));
    assert!((d - c).abs() < c * 0.05, "the lock must not stick: {d} vs {c}");
}
