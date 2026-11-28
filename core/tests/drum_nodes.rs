//! The rest of the kit as nodes: `tom`, `hat`, `snare` and `clap`, each a
//! whole piece written in what is heard, in Hz and ms. Each test renders one
//! in a song, through the compiler, and checks what is heard against what the
//! file says. Times are counted from the engine's rate, so the same tests
//! hold on a build at 22050 or 32000 Hz (`TATUM_SAMPLE_RATE`).

use tatum_core::song_engine::SongEngine;
use tatum_core::SAMPLE_RATE;

/// One node straight to `out`, on `pattern`, at 120 BPM. The node is on
/// line 5 of the file.
fn song(node: &str, pattern: &str) -> String {
    format!(
        r#"
tempo 120
scale A minor
instrument piece {{
  {node} as p
  p > out
}}
pattern beat {{ {pattern} }}
track t {{ play beat using piece level 0.8 out > master }}
"#
    )
}

/// The line the node is written on in `song`.
const NODE_LINE: &str = "line 5";

const FOUR: &str = "C3 - - -  C3 - - -  C3 - - -  C3 - - -";
/// One hit and a bar of room after it.
const ONE: &str = "C3 - - -  - - - -  - - - -  - - - -";
/// The same hit held for three beats.
const ONE_HELD: &str = "C3 .. .. ..  .. .. .. ..  .. .. .. ..  - - - -";

fn render(src: &str, bars: u32) -> Vec<f32> {
    let mut e = SongEngine::from_source(src).unwrap_or_else(|err| panic!("{}", err));
    let (l, _) = e.render(bars);
    l
}

/// Every error the file gives, each with its line.
fn errors(src: &str) -> String {
    let song = match tatum_core::dsl::parse(src) {
        Ok(song) => song,
        Err(errs) => {
            return errs.iter().map(|e| format!("line {}: {}", e.line, e.message)).collect::<Vec<_>>().join("\n")
        }
    };
    match tatum_core::dsl::compiler::compile(&song) {
        Ok(_) => String::new(),
        Err(errs) => errs.iter().map(|e| format!("line {}: {}", e.line, e.message)).collect::<Vec<_>>().join("\n"),
    }
}

fn ms(t: f32) -> usize {
    (t * 0.001 * SAMPLE_RATE) as usize
}

/// Where each hit starts: the first sound after a silence.
fn onsets(x: &[f32]) -> Vec<usize> {
    let mut on = Vec::new();
    let mut quiet = usize::MAX;
    for (i, v) in x.iter().enumerate() {
        if *v != 0.0 {
            if quiet > ms(20.0) {
                on.push(i);
            }
            quiet = 0;
        } else {
            quiet = quiet.saturating_add(1);
        }
    }
    assert!(!on.is_empty(), "it sounds");
    on
}

fn onset(x: &[f32]) -> usize {
    onsets(x)[0]
}

/// Each hit, from its start, `len` samples long.
fn hits(x: &[f32], len: usize) -> Vec<&[f32]> {
    onsets(x).into_iter().filter(|&o| o + len <= x.len()).map(|o| &x[o..o + len]).collect()
}

/// Frequency between two times after the onset, from the upward zero crossings.
fn pitch(x: &[f32], from_ms: f32, to_ms: f32) -> f32 {
    let t0 = onset(x);
    let (a, b) = (t0 + ms(from_ms), t0 + ms(to_ms));
    let ups: Vec<f32> = (a + 1..b)
        .filter(|&i| x[i - 1] < 0.0 && x[i] >= 0.0)
        .map(|i| (i - 1) as f32 + x[i - 1] / (x[i - 1] - x[i]))
        .collect();
    assert!(ups.len() >= 2, "no cycles between {from_ms} and {to_ms} ms");
    SAMPLE_RATE * (ups.len() - 1) as f32 / (ups[ups.len() - 1] - ups[0])
}

/// The peak in a window `win_ms` long starting `at_ms` after the onset.
fn peak_in(x: &[f32], at_ms: f32, win_ms: f32) -> f32 {
    let i = onset(x) + ms(at_ms);
    x[i..(i + ms(win_ms)).min(x.len())].iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

fn db(ratio: f32) -> f32 {
    20.0 * ratio.max(1e-9).log10()
}

/// How long the first hit sounds: from its start to the last sample that is
/// not silence, in ms. A drum node ends its voice at -60 dB, so this is the
/// time to -60 dB.
fn sounding_ms(x: &[f32]) -> f32 {
    let t0 = onset(x);
    let last = x.iter().rposition(|v| *v != 0.0).unwrap_or(t0);
    (last - t0) as f32 * 1000.0 / SAMPLE_RATE
}

/// The level of `x` around `hz`, by a DFT bin over the whole slice (Hann).
fn bin(x: &[f32], hz: f32) -> f32 {
    let n = x.len() as f32;
    let w = std::f32::consts::TAU * hz / SAMPLE_RATE;
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for (i, v) in x.iter().enumerate() {
        let hann = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n).cos();
        let a = (v * hann) as f64;
        re += a * (w * i as f32).cos() as f64;
        im += a * (w * i as f32).sin() as f64;
    }
    ((re * re + im * im).sqrt() / n as f64) as f32
}

/// The frequency between `lo` and `hi` where `x` is loudest, to `step` Hz.
fn loudest(x: &[f32], lo: f32, hi: f32, step: f32) -> f32 {
    let mut best = (lo, 0.0f32);
    let mut f = lo;
    while f <= hi {
        let b = bin(x, f);
        if b > best.1 {
            best = (f, b);
        }
        f += step;
    }
    best.0
}

/// The energy of `x` in each band `width` Hz wide from 0 up to half the
/// rate, as (centre, energy): a coarse spectrum, by DFT bins `width` apart
/// over slices of the hit.
fn spectrum(x: &[f32], width: f32) -> Vec<(f32, f32)> {
    let slice = (SAMPLE_RATE / width) as usize;
    let mut out = Vec::new();
    let mut f = width;
    while f < 0.5 * SAMPLE_RATE {
        let e: f32 = x.chunks_exact(slice).map(|c| bin(c, f).powi(2)).sum();
        out.push((f, e));
        f += width;
    }
    out
}

/// The spectral centre of `x`: the mean frequency weighted by energy.
#[allow(dead_code)]
fn centroid(x: &[f32]) -> f32 {
    let s = spectrum(x, 250.0);
    let total: f32 = s.iter().map(|(_, e)| e).sum();
    s.iter().map(|(f, e)| f * e).sum::<f32>() / total
}

/// The share of the energy of `x` over `hz`.
fn share_over(x: &[f32], hz: f32) -> f32 {
    let s = spectrum(x, 250.0);
    let total: f32 = s.iter().map(|(_, e)| e).sum();
    s.iter().filter(|(f, _)| *f > hz).map(|(_, e)| e).sum::<f32>() / total
}

/// Two engines built from the same file play the same samples.
fn same_on_every_run(node: &str) {
    let src = song(node, FOUR);
    let (a, b) = (render(&src, 2), render(&src, 2));
    assert!(a.iter().any(|v| *v != 0.0), "{node} sounds");
    assert!(a == b, "{node}: two renders differ");
}

/// A hit plays the same samples whether its note lasts a step or three beats.
fn the_note_length_does_not_matter(node: &str) {
    let short = render(&song(node, ONE), 1);
    let held = render(&song(node, ONE_HELD), 1);
    let (a, b) = (onset(&short), onset(&held));
    assert_eq!(a, b);
    assert!(short[a..] == held[b..], "{node}: a held note plays differently");
}

// ── tom ──

#[test]
fn a_tom_lands_on_the_note_it_is_given() {
    let x = render(&song("tom(A2, hold=300ms)", FOUR), 1);
    let tail = pitch(&x, 150.0, 290.0);
    assert!((tail - 110.0).abs() < 0.5, "A2 is 110 Hz, the tail is {tail:.2}");
    // And by a DFT bin: nothing else is louder between 60 and 400 Hz.
    let t0 = onset(&x);
    let top = loudest(&x[t0 + ms(100.0)..t0 + ms(300.0)], 60.0, 400.0, 1.0);
    assert!((top - 110.0).abs() <= 2.0, "the loudest bin is {top} Hz");
    // A bare tom() is A2 too.
    let bare = render(&song("tom(hold=300ms)", FOUR), 1);
    assert!(x == bare, "tom() is tom(A2)");
}

#[test]
fn a_tom_drops_onto_its_note_in_the_time_it_says() {
    // From 1.6 times the note to within 10% of it in `drop` ms.
    let x = render(&song("tom(200hz, drop=60ms, hold=300ms)", FOUR), 1);
    let early = pitch(&x, 0.0, 20.0);
    let before = pitch(&x, 25.0, 50.0);
    let after = pitch(&x, 65.0, 120.0);
    assert!(early > 250.0 && early < 320.0, "it starts near 320 Hz: {early:.1}");
    assert!(before > 222.0, "before the drop is over it is more than 10% up: {before:.1}");
    assert!(after < 220.0 && after > 199.0, "after it, within 10% of 200 Hz: {after:.1}");
    // start= is where it falls from.
    let high = render(&song("tom(200hz, start=600hz, drop=60ms, hold=300ms)", FOUR), 1);
    assert!(pitch(&high, 0.0, 10.0) > 400.0, "a higher start is heard");
}

#[test]
fn a_tom_holds_then_falls_sixty_db_over_the_release() {
    let x = render(&song("tom(150hz, hold=100ms, release=200ms)", ONE), 1);
    let top = peak_in(&x, 5.0, 15.0);
    assert!(db(peak_in(&x, 80.0, 15.0) / top).abs() < 0.5, "still at full level inside the hold");
    let half = db(peak_in(&x, 200.0, 10.0) / top);
    assert!((half + 30.0).abs() < 4.0, "-30 dB half way through the release, got {half:.1}");
    let len = sounding_ms(&x);
    assert!((len - 300.0).abs() < 8.0, "hold and release are 300 ms to -60 dB, it sounds {len:.1} ms");
}

#[test]
fn a_tom_is_the_same_on_every_hit_and_every_run() {
    same_on_every_run("tom(A2, click=-24db, drive=0.4)");
    let x = render(&song("tom(A2, click=-24db, drive=0.4)", FOUR), 2);
    let all = hits(&x, ms(300.0));
    assert_eq!(all.len(), 8);
    assert!(all.iter().all(|h| *h == all[0]), "every hit is the same samples");
    the_note_length_does_not_matter("tom(A2, click=-24db, hold=150ms, release=400ms)");
}

#[test]
fn a_tom_takes_a_click_a_drive_and_a_sway() {
    let plain = render(&song("tom(A2)", ONE), 1);
    let clicked = render(&song("tom(A2, click=-12db, tone=5khz)", ONE), 1);
    let t0 = onset(&plain);
    let head = t0..t0 + ms(4.0);
    assert!(share_over(&clicked[head.clone()], 3000.0) > 10.0 * share_over(&plain[head], 3000.0), "the click is high");
    let at = t0 + ms(20.0);
    let worst = plain[at..].iter().zip(&clicked[at..]).fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
    assert!(worst < 1e-3, "the click is over by 20 ms: {worst}");

    let driven = render(&song("tom(A2, drive=1)", ONE), 1);
    let (pc, pd) = (peak_in(&plain, 0.0, 30.0), peak_in(&driven, 0.0, 30.0));
    assert!(db(pd / pc).abs() < 0.5, "drive keeps the peak: {pc} against {pd}");
    let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
    let w = t0..t0 + ms(50.0);
    assert!(rms(&driven[w.clone()]) > 1.1 * rms(&plain[w]), "and fills the body");

    let src = song("tom(A2, hold=40ms, release=150ms, vary=10%)", FOUR);
    let swayed = render(&src, 4);
    let lens: Vec<usize> =
        hits(&swayed, ms(400.0)).iter().map(|h| h.iter().rposition(|v| *v != 0.0).unwrap_or(0)).collect();
    let (lo, hi) = (*lens.iter().min().unwrap() as f32, *lens.iter().max().unwrap() as f32);
    assert!(hi / lo > 1.03 && hi / lo < 1.25, "the length sways by about what it says: {lo}..{hi} samples");
    assert!(swayed == render(&src, 4), "and the same on every play");
}

#[test]
fn a_tom_out_of_range_is_an_error_at_its_line() {
    let e = errors(&song("tom(20hz)", FOUR));
    assert!(e.contains("note = 20 is out of range (40..1000)") && e.contains(NODE_LINE), "{e}");
    let e = errors(&song("tom(A2, release=9s)", FOUR));
    assert!(e.contains("release = 9000 is out of range (5..5000)") && e.contains(NODE_LINE), "{e}");
    let e = errors(&song("tom(A2, snap=3)", FOUR));
    assert!(e.contains("unknown option 'snap'"), "{e}");
    let e = errors(&song("tom(200hz, start=210hz)", FOUR));
    assert!(e.contains("start=210 is within 10% of the note 200") && e.contains(NODE_LINE), "{e}");
    let e = errors(&song("tom(A2, drop=40hz)", FOUR));
    assert!(e.contains("'drop' takes ms or s"), "{e}");
}

#[test]
fn a_node_named_tom_is_still_a_name_and_a_bare_one_says_how() {
    let named = r#"
tempo 120
instrument piece {
  pitch_osc sine(300, 110, 0.999) as tom
  tom > perc(0.001, 0.3) as hat
  hat > out
}
pattern beat { C3 - - -  C3 - - -  C3 - - -  C3 - - - }
track t { play beat using piece }
"#;
    assert_eq!(errors(named), "");
    for piece in ["tom", "hat", "snare", "clap"] {
        let bare = format!(
            "tempo 120\ninstrument piece {{\n  {piece} > out\n}}\npattern beat {{ C3 - - - }}\ntrack t {{ play beat using piece }}\n"
        );
        let e = errors(&bare);
        assert!(e.contains(&format!("`{piece}()`")), "{e}");
    }
}
