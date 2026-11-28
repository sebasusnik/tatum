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

/// `noise as n` is a node, so `hat as h` is the natural way to get it wrong:
/// it said "expected node or identifier in chain, got As". It ends in how to
/// write it, at its line, for the four and for the kick, and says it once.
#[test]
fn a_drum_node_named_without_its_parentheses_says_how() {
    for piece in ["tom", "hat", "snare", "clap", "kick"] {
        let bare = format!(
            "tempo 120\ninstrument piece {{\n  {piece} as p\n  p > out\n}}\npattern beat {{ C3 - - - }}\ntrack t {{ play beat using piece }}\n"
        );
        let e = errors(&bare);
        assert!(e.ends_with(&format!("written with its parentheses, `{piece}() as ...`")), "{e}");
        assert!(e.starts_with("line 3:") && e.lines().count() == 1, "{e}");
        // In a chain too.
        let chained = bare.replace(&format!("{piece} as p\n  p > out"), &format!("{piece} as p > out"));
        let e = errors(&chained);
        assert!(e.ends_with(&format!("`{piece}() as ...`")) && e.lines().count() == 1, "{e}");
        // With them it is the node.
        assert_eq!(errors(&bare.replace(" as p", "() as p")), "");
    }
    // A name that is one of the words still takes what a name takes.
    let named = "tempo 120\ninstrument piece {\n  noise() as hat\n  hat > perc(0.001, 0.05) as clap\n  clap > out\n}\npattern beat { C3 - - - }\ntrack t { play beat using piece }\n";
    assert_eq!(errors(named), "");
}

/// Where the band `x` sounds in is centred: the middle of its spectrum (the
/// level in each Hz) on a log axis, where a band of one shape is as far
/// down an octave over its centre as an octave under it.
fn log_centre(x: &[f32]) -> f32 {
    let s = spectrum(x, 250.0);
    let total: f32 = s.iter().map(|(f, e)| e / f).sum();
    (s.iter().map(|(f, e)| f.ln() * e / f).sum::<f32>() / total).exp()
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

/// The rms of a window `win_ms` long starting `at_ms` after the onset.
fn rms_in(x: &[f32], at_ms: f32, win_ms: f32) -> f32 {
    let i = onset(x) + ms(at_ms);
    rms(&x[i..(i + ms(win_ms)).min(x.len())])
}

/// The first hit of `x`, from its start to its last sample.
fn first_hit(x: &[f32]) -> &[f32] {
    let t0 = onset(x);
    &x[t0..t0 + ms(sounding_ms(x))]
}

/// Each hit's length in samples, to its last sample that is not silence.
fn lengths(x: &[f32], within_ms: f32) -> Vec<usize> {
    hits(x, ms(within_ms)).iter().map(|h| h.iter().rposition(|v| *v != 0.0).unwrap_or(0)).collect()
}

/// A velocity of a half plays the same hit at half the level.
fn velocity_scales_it(node: &str) {
    let full = render(&song(node, "C3:1.0 - - -  - - - -  - - - -  - - - -"), 1);
    let half = render(&song(node, "C3:0.5 - - -  - - - -  - - - -  - - - -"), 1);
    let (pf, ph) = (peak_in(&full, 0.0, 100.0), peak_in(&half, 0.0, 100.0));
    assert!((ph / pf - 0.5).abs() < 0.01, "{node}: half the velocity is {} of the level", ph / pf);
}

// ── hat ──

/// The rms of the 100 ms `hat()` is over in, as `render` gives it: one hit at
/// full velocity on a track at 0.8, at 44.1 kHz.
const HAT_RMS: f32 = 0.010_16;

#[test]
fn a_hat_is_sixty_db_down_when_its_decay_says() {
    for decay in [45.0, 320.0, 1200.0] {
        let x = render(&song(&format!("hat(decay={decay}ms)"), ONE), 1);
        let len = sounding_ms(&x);
        assert!((len - decay).abs() < 0.02 * decay + 1.0, "decay={decay}ms sounds {len:.1} ms");
    }
    // Evenly in dB on the way: half way there it is 30 dB down.
    let x = render(&song("hat(decay=400ms, noise=100%)", ONE), 1);
    let half = db(rms_in(&x, 200.0, 20.0) / rms_in(&x, 0.0, 20.0));
    assert!((half + 30.0).abs() < 3.0, "-30 dB half way, got {half:.1}");
    // A bare hat() is a closed one: 60 ms.
    let bare = sounding_ms(&render(&song("hat()", ONE), 1));
    assert!((bare - 60.0).abs() < 2.0, "hat() sounds {bare:.1} ms");
}

#[test]
fn an_open_hat_rings_whole_on_a_note_of_any_length() {
    the_note_length_does_not_matter("hat(decay=320ms)");
    the_note_length_does_not_matter("hat(decay=30ms)");
    // A sixteenth at 120 BPM is 125 ms; the hat is still there at 250.
    let x = render(&song("hat(decay=320ms)", ONE), 1);
    assert!(rms_in(&x, 250.0, 20.0) > 0.0, "it rings past the step");
}

#[test]
fn a_hat_sounds_in_the_band_it_is_given() {
    // Noise alone, the floor out of the way: the band is all there is.
    let centre = |band: f32| {
        let x = render(&song(&format!("hat(decay=400ms, noise=100%, band={band}hz, floor=200hz)"), ONE), 1);
        log_centre(first_hit(&x))
    };
    for band in [2500.0, 5000.0] {
        let c = centre(band);
        assert!((c / band).ln().abs() < 0.2, "band={band}hz is centred on {c:.0} Hz");
    }
    // The floor cuts under it, whatever the source.
    let x = render(&song("hat(decay=400ms, band=5khz, floor=4khz)", ONE), 1);
    let under = 1.0 - share_over(first_hit(&x), 2000.0);
    assert!(under < 0.02, "{:.1}% of it is under 2 kHz with the floor at 4", under * 100.0);
    let open = render(&song("hat(decay=400ms, band=5khz, floor=200hz, noise=0)", ONE), 1);
    let under_open = 1.0 - share_over(first_hit(&open), 2000.0);
    assert!(under_open > 4.0 * under, "a low floor lets the clank through: {under_open} against {under}");
}

#[test]
fn a_hat_is_six_partials_that_follow_its_pitch() {
    // The metal alone, a low band so the partials themselves are heard. The
    // second partial is 1.4827 times the first; nothing sits at 1.63 times.
    for pitch in [205.0, 300.0] {
        let x = render(&song(&format!("hat(decay=1500ms, noise=0, pitch={pitch}hz, band=1khz, floor=200hz)"), ONE), 1);
        let t0 = onset(&x);
        let w = &x[t0..t0 + ms(500.0)];
        let (partial, gap) = (bin(w, pitch * 1.4827), bin(w, pitch * 1.63));
        assert!(partial > 8.0 * gap, "pitch={pitch}hz: {partial} at the partial, {gap} beside it");
        let top = bin(w, pitch * 3.8967);
        assert!(top > 8.0 * gap, "and the sixth: {top}");
    }
    // Noise is the other source: with it alone there are no partials.
    let x = render(&song("hat(decay=1500ms, noise=100%, band=1khz, floor=200hz)", ONE), 1);
    let t0 = onset(&x);
    let w = &x[t0..t0 + ms(500.0)];
    let (a, b) = (bin(w, 205.0 * 1.4827), bin(w, 205.0 * 1.63));
    assert!(a < 4.0 * b && b < 4.0 * a, "noise is even: {a} and {b}");
}

#[test]
fn a_hat_is_the_same_on_every_hit_and_every_run() {
    same_on_every_run("hat()");
    same_on_every_run("hat(decay=200ms, vary=20%)");
    let x = render(&song("hat()", FOUR), 2);
    let all = hits(&x, ms(100.0));
    assert_eq!(all.len(), 8);
    assert!(all.iter().all(|h| *h == all[0]), "every hit is the same samples");
    velocity_scales_it("hat()");
}

#[test]
fn a_hat_with_vary_breathes() {
    let src = song("hat(decay=100ms, vary=20%)", "C3 - C3 -  C3 - C3 -  C3 - C3 -  C3 - C3 -");
    let x = render(&src, 4);
    let lens = lengths(&x, 124.0);
    assert!(lens.len() >= 30, "{} hits", lens.len());
    let (lo, hi) = (*lens.iter().min().unwrap() as f32, *lens.iter().max().unwrap() as f32);
    assert!(hi / lo > 1.1 && hi / lo < 1.6, "the decay sways by about ±20%: {lo}..{hi} samples");
    let mean = lens.iter().sum::<usize>() as f32 / lens.len() as f32;
    assert!((mean / ms(100.0) as f32 - 1.0).abs() < 0.1, "around what it says: {mean} samples");
    // Slowly: one hit to the next moves much less than the whole sway.
    let step = lens.windows(2).map(|w| (w[1] as f32 - w[0] as f32).abs()).sum::<f32>() / (lens.len() - 1) as f32;
    assert!(step < 0.4 * (hi - lo), "hit to hit {step:.1} samples against a sway of {:.1}", hi - lo);
    // And the metal does not start twice alike.
    let h = hits(&x, ms(20.0));
    assert!(h[0] != h[1] && h[1] != h[2], "no two hits are the same samples");
}

/// The metal used to go through at one gain whatever the band, the noise
/// at one loudness: `hat(noise=0, band=2khz, floor=200hz, pitch=1khz)` came
/// out 14 dB over `hat()` and over full scale by itself, and a floor over the
/// band 26 dB under. The level is now found when the line is compiled, so
/// the colour of a hat is not also its volume.
#[test]
fn a_hat_holds_its_level_whatever_its_colour() {
    // One hit and the 100 ms it is over in, as rendered: after the track's
    // level and the engine's headroom, which are the same for every line.
    let hit = |node: &str| {
        let x = render(&song(node, ONE), 1);
        let w = &x[..ms(100.0)];
        (rms(w), w.iter().fold(0.0f32, |m, v| m.max(v.abs())))
    };
    let (level, peak) = hit("hat()");
    // The same number at every rate the engine is built for: the hat used to
    // lose 3 dB at 32 kHz and 4.6 dB at 22.05 kHz.
    assert!(db(level / HAT_RMS).abs() < 0.5, "hat() is at {level} at {SAMPLE_RATE} Hz");
    // A tom's hit is the node's full scale; a hat's peak is under it.
    let full = hit("tom()").1;
    assert!(peak < 0.6 * full, "hat() peaks at {} of full scale", peak / full);
    let (mut low, mut high) = (0.0f32, 0.0f32);
    for band in ["1khz", "2khz", "4khz", "8khz", "16khz"] {
        for floor in ["200hz", "1khz", "6khz", "12khz"] {
            for noise in ["0", "30%", "100%"] {
                for pitch in ["100hz", "205hz", "1khz"] {
                    let node = format!("hat(band={band}, floor={floor}, noise={noise}, pitch={pitch})");
                    let (l, p) = hit(&node);
                    let off = db(l / level);
                    assert!(off.abs() < 2.0, "{node}: {off:+.1} dB from hat()");
                    assert!(p < full, "{node}: peaks at {} of full scale", p / full);
                    (low, high) = (low.min(off), high.max(off));
                }
            }
        }
    }
    assert!(low > -0.5 && high < 0.5, "it holds much closer than that: {low:+.2}..{high:+.2} dB");
    // And with `vary`, where every hit is another, none goes over.
    for node in [
        "hat(vary=30%)",
        "hat(decay=320ms, band=9khz, floor=7khz, noise=20%, vary=30%)",
        "hat(band=16khz, floor=200hz, noise=0, pitch=100hz, vary=30%)",
    ] {
        let x = render(&song(node, "C3 C3 C3 C3  C3 C3 C3 C3  C3 C3 C3 C3  C3 C3 C3 C3"), 8);
        let top = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(top < full, "{node}: a hit peaks at {} of full scale", top / full);
    }
}

#[test]
fn a_hat_out_of_range_is_an_error_at_its_line() {
    let e = errors(&song("hat(decay=2ms)", FOUR));
    assert!(e.contains("decay = 2 is out of range (5..5000)") && e.contains(NODE_LINE), "{e}");
    let e = errors(&song("hat(band=20khz)", FOUR));
    assert!(e.contains("band = 20000 is out of range (1000..16000)") && e.contains(NODE_LINE), "{e}");
    let e = errors(&song("hat(noise=150%)", FOUR));
    assert!(e.contains("noise = 1.5 is out of range (0..1)") && e.contains(NODE_LINE), "{e}");
    let e = errors(&song("hat(60)", FOUR));
    assert!(e.contains("too many positional") && e.contains(NODE_LINE), "{e}");
    let e = errors(&song("hat(decay=60hz)", FOUR));
    assert!(e.contains("'decay' takes ms or s") && e.contains(NODE_LINE), "{e}");
    let e = errors(&song("hat(length=60)", FOUR));
    assert!(e.contains("unknown option 'length'"), "{e}");
}

#[test]
fn a_hat_asked_higher_than_the_rate_goes_is_held_under_it() {
    // The highest of everything: at 22050 Hz the band and the floor would sit
    // over half the rate. It plays, finite and in range, and what it plays is
    // high.
    let x = render(&song("hat(decay=300ms, band=16khz, floor=12khz, pitch=1khz)", ONE), 1);
    assert!(x.iter().all(|v| v.is_finite()));
    let peak = peak_in(&x, 0.0, 300.0);
    assert!(peak > 0.005 && peak < 1.0, "it plays, at {peak}");
    let c = log_centre(first_hit(&x));
    assert!(c > 0.25 * SAMPLE_RATE && c < 0.5 * SAMPLE_RATE, "centred on {c:.0} Hz at {SAMPLE_RATE} Hz");
}

// ── snare ──

#[test]
fn a_snare_body_lands_on_the_note_it_is_given() {
    // The wires off, so the body is all there is.
    let x = render(&song("snare(F#3, balance=0, body=600ms)", ONE), 1);
    let t0 = onset(&x);
    let w = &x[t0 + ms(60.0)..t0 + ms(360.0)];
    let top = loudest(w, 100.0, 260.0, 0.5);
    assert!((top - 185.0).abs() < 2.0, "F#3 is 185 Hz, the loudest bin is {top}");
    // The second mode is there, 1.593 times over the first, and quieter.
    let (first, second, gap) = (bin(w, 185.0), bin(w, 185.0 * 1.593), bin(w, 185.0 * 1.3));
    assert!(second > 10.0 * gap, "a second mode at {:.0} Hz: {second} against {gap} beside it", 185.0 * 1.593);
    assert!(second < first, "under the first: {second} against {first}");
    // A bare snare() is 185 Hz, F#3 to a hundredth of a cent.
    let bare = render(&song("snare(balance=0, body=600ms)", ONE), 1);
    let hz = render(&song("snare(185hz, balance=0, body=600ms)", ONE), 1);
    assert!(hz == bare, "snare() is snare(185hz)");
    // And the note moves it.
    let high = render(&song("snare(250hz, balance=0, body=600ms)", ONE), 1);
    let t0 = onset(&high);
    let top = loudest(&high[t0 + ms(60.0)..t0 + ms(360.0)], 100.0, 350.0, 0.5);
    assert!((top - 250.0).abs() < 2.5, "250 Hz, the loudest bin is {top}");
}

#[test]
fn a_snare_snaps_onto_its_note_in_the_time_it_says() {
    // One mode would be easier to time; the second is 1.593 times up and
    // quieter, so the crossings still follow the first. From 1.7 times the
    // note to within 10% of it in `snap`.
    let x = render(&song("snare(100hz, snap=40ms, balance=0, body=1500ms)", ONE), 1);
    let t0 = onset(&x);
    let at = |from: f32, to: f32| loudest(&x[t0 + ms(from)..t0 + ms(to)], 80.0, 200.0, 1.0);
    let (early, late) = (at(0.0, 25.0), at(60.0, 160.0));
    assert!(early > 120.0, "inside the snap it is well over the note: {early} Hz");
    assert!(late < 106.0 && late > 98.0, "after it, on 100 Hz: {late} Hz");
    // A short snap is over at once.
    let fast = render(&song("snare(100hz, snap=1ms, balance=0, body=1500ms)", ONE), 1);
    let t0 = onset(&fast);
    let early = loudest(&fast[t0 + ms(5.0)..t0 + ms(45.0)], 80.0, 200.0, 1.0);
    assert!(early < 106.0, "snap=1ms is on the note from the start: {early} Hz");
}

#[test]
fn a_snare_body_and_its_wires_each_fall_in_their_own_time() {
    let body = sounding_ms(&render(&song("snare(balance=0, body=150ms, wires=900ms)", ONE), 1));
    assert!((body - 150.0).abs() < 4.0, "the body alone is over in {body:.1} ms");
    let wires = sounding_ms(&render(&song("snare(balance=100%, body=900ms, wires=250ms)", ONE), 1));
    assert!((wires - 250.0).abs() < 6.0, "the wires alone are over in {wires:.1} ms");
    // Together it ends when the longer of the two does.
    let both = sounding_ms(&render(&song("snare(body=100ms, wires=300ms)", ONE), 1));
    assert!((both - 300.0).abs() < 7.0, "together, {both:.1} ms");
    // Evenly in dB: half way through, the wires are 30 dB down.
    let x = render(&song("snare(balance=100%, wires=400ms)", ONE), 1);
    let half = db(rms_in(&x, 200.0, 20.0) / rms_in(&x, 0.0, 20.0));
    assert!((half + 30.0).abs() < 3.0, "-30 dB half way, got {half:.1}");
}

#[test]
fn the_wires_sound_in_their_band_and_balance_sets_them_against_the_body() {
    for band in [2000.0, 5000.0] {
        let x = render(&song(&format!("snare(balance=100%, wires=400ms, band={band}hz)"), ONE), 1);
        let c = log_centre(first_hit(&x));
        assert!((c / band).ln().abs() < 0.25, "band={band}hz is centred on {c:.0} Hz");
    }
    // The share of the hit over 1 kHz is the wires'; it grows with balance.
    let high = |balance: u32| {
        let x = render(&song(&format!("snare(balance={balance}%, body=200ms, wires=200ms)"), ONE), 1);
        share_over(first_hit(&x), 1000.0)
    };
    let (none, some, most, all) = (high(0), high(30), high(70), high(100));
    assert!(none < 0.01, "balance=0 is the drum with its wires off: {none}");
    assert!(some > none && most > some && all > most, "more wires: {none} {some} {most} {all}");
    assert!(all > 0.9, "balance=100% is the wires alone: {all}");
}

#[test]
fn a_snare_is_the_same_on_every_hit_and_every_run() {
    same_on_every_run("snare()");
    same_on_every_run("snare(drive=0.5, vary=20%)");
    let x = render(&song("snare(drive=0.3)", FOUR), 2);
    let all = hits(&x, ms(250.0));
    assert_eq!(all.len(), 8);
    assert!(all.iter().all(|h| *h == all[0]), "every hit is the same samples");
    the_note_length_does_not_matter("snare(body=300ms, wires=400ms)");
    velocity_scales_it("snare()");
}

#[test]
fn a_snare_takes_a_drive_and_a_sway() {
    let clean = render(&song("snare(balance=20%)", ONE), 1);
    let driven = render(&song("snare(balance=20%, drive=1)", ONE), 1);
    let (pc, pd) = (peak_in(&clean, 0.0, 50.0), peak_in(&driven, 0.0, 50.0));
    assert!(pd <= 1.0 && pd > 0.9 * pc, "drive keeps it under full scale and no quieter: {pc} against {pd}");
    assert!(rms_in(&driven, 0.0, 60.0) > 1.15 * rms_in(&clean, 0.0, 60.0), "and fills it");

    let src = song("snare(body=80ms, wires=150ms, vary=20%)", FOUR);
    let x = render(&src, 8);
    let lens = lengths(&x, 400.0);
    assert!(lens.len() >= 30, "{} hits", lens.len());
    let (lo, hi) = (*lens.iter().min().unwrap() as f32, *lens.iter().max().unwrap() as f32);
    assert!(hi / lo > 1.1 && hi / lo < 1.6, "the wires sway by about ±20%: {lo}..{hi} samples");
    let step = lens.windows(2).map(|w| (w[1] as f32 - w[0] as f32).abs()).sum::<f32>() / (lens.len() - 1) as f32;
    assert!(step < 0.4 * (hi - lo), "slowly: hit to hit {step:.1} samples against a sway of {:.1}", hi - lo);
}

#[test]
fn a_snare_out_of_range_is_an_error_at_its_line() {
    let e = errors(&song("snare(60hz)", FOUR));
    assert!(e.contains("note = 60 is out of range (80..500)") && e.contains(NODE_LINE), "{e}");
    let e = errors(&song("snare(F#3, wires=5s)", FOUR));
    assert!(e.contains("wires = 5000 is out of range (10..3000)") && e.contains(NODE_LINE), "{e}");
    let e = errors(&song("snare(F#3, balance=-6db)", FOUR));
    assert!(e.contains("'balance' takes % or a plain 0..1") && e.contains(NODE_LINE), "{e}");
    let e = errors(&song("snare(F#3, band=200hz)", FOUR));
    assert!(e.contains("band = 200 is out of range (500..12000)") && e.contains(NODE_LINE), "{e}");
    let e = errors(&song("snare(F#3, decay=100)", FOUR));
    assert!(e.contains("unknown option 'decay'"), "{e}");
}

#[test]
fn a_snare_asked_higher_than_the_rate_goes_is_held_under_it() {
    let x = render(&song("snare(500hz, band=12khz, balance=100%, wires=300ms)", ONE), 1);
    assert!(x.iter().all(|v| v.is_finite()));
    let peak = peak_in(&x, 0.0, 300.0);
    assert!(peak > 0.05 && peak < 1.5, "it plays, at {peak}");
}

// ── clap ──

/// Where the bursts of a clap start, in ms from its onset, read off its level
/// itself. A clap of one burst and a long tail in the same band is the same
/// noise through the same filter under a level that only falls, so `hit`
/// over it, sample by sample, is the clap's level over time, and a burst is
/// where that jumps up (each starts 20 dB over where the last has fallen to).
fn bursts(hit: &[f32], band: &str) -> Vec<f32> {
    let plain = render(&song(&format!("clap(bursts=1, tail=3s, band={band})"), ONE), 1);
    let plain = &plain[onset(&plain)..];
    let mut found = vec![0.0f32];
    let mut before = 0.0f32;
    for (i, (v, p)) in hit.iter().zip(plain).enumerate() {
        // Where the noise passes near zero the two say nothing.
        if p.abs() < 1e-4 {
            continue;
        }
        let level = v / p;
        if before > 0.0 && level > 3.0 * before {
            found.push(i as f32 * 1000.0 / SAMPLE_RATE);
        }
        before = level;
    }
    found
}

#[test]
fn a_clap_is_as_many_bursts_as_it_says_as_far_apart_as_it_says() {
    for (count, spacing) in [(4, 11.0), (3, 20.0), (6, 8.0), (1, 11.0)] {
        let x = render(&song(&format!("clap(bursts={count}, spacing={spacing}ms, tail=300ms, band=4khz)"), ONE), 1);
        let found = bursts(first_hit(&x), "4khz");
        assert_eq!(found.len(), count, "bursts={count}: found them at {found:?} ms");
        for (n, at) in found.iter().enumerate() {
            let want = n as f32 * spacing;
            assert!((at - want).abs() < 0.2, "burst {} of {count} at {at:.1} ms, not {want:.1}", n + 1);
        }
    }
    // A bare clap() is four, 11 ms apart.
    let found = bursts(first_hit(&render(&song("clap()", ONE), 1)), "1200hz");
    assert_eq!(found.len(), 4, "{found:?}");
    assert!((found[3] - 33.0).abs() < 0.2, "{found:?}");
}

#[test]
fn a_clap_tail_is_sixty_db_down_when_it_says() {
    // From the last burst: three gaps of 10 ms, then the tail.
    for tail in [120.0, 500.0] {
        let x = render(&song(&format!("clap(bursts=4, spacing=10ms, tail={tail}ms)"), ONE), 1);
        let len = sounding_ms(&x);
        assert!((len - (30.0 + tail)).abs() < 0.02 * tail + 2.0, "tail={tail}ms sounds {len:.1} ms");
    }
    let x = render(&song("clap(bursts=1, tail=400ms)", ONE), 1);
    let half = db(rms_in(&x, 200.0, 20.0) / rms_in(&x, 0.0, 20.0));
    assert!((half + 30.0).abs() < 3.0, "-30 dB half way, got {half:.1}");
    // Each burst is 20 dB down when the next one comes.
    let x = render(&song("clap(bursts=2, spacing=40ms, tail=400ms)", ONE), 1);
    let fall = db(rms_in(&x, 36.0, 4.0) / rms_in(&x, 0.5, 4.0));
    assert!((fall + 18.0).abs() < 4.0, "a burst falls about 20 dB before the next: {fall:.1}");
}

#[test]
fn a_clap_sounds_in_the_band_it_is_given() {
    for band in [800.0, 1200.0, 3000.0] {
        let x = render(&song(&format!("clap(bursts=1, tail=600ms, band={band}hz)"), ONE), 1);
        let c = log_centre(first_hit(&x));
        assert!((c / band).ln().abs() < 0.25, "band={band}hz is centred on {c:.0} Hz");
    }
}

#[test]
fn a_clap_is_the_same_on_every_hit_and_every_run() {
    same_on_every_run("clap()");
    same_on_every_run("clap(vary=20%)");
    let x = render(&song("clap()", FOUR), 2);
    let all = hits(&x, ms(300.0));
    assert_eq!(all.len(), 8);
    assert!(all.iter().all(|h| *h == all[0]), "every hit is the same samples");
    the_note_length_does_not_matter("clap(tail=600ms)");
    velocity_scales_it("clap()");
}

#[test]
fn a_clap_with_vary_does_not_land_twice_alike() {
    let src = song("clap(bursts=3, spacing=20ms, tail=150ms, band=4khz, vary=20%)", FOUR);
    let x = render(&src, 8);
    let on = onsets(&x);
    let gaps: Vec<f32> = on
        .iter()
        .filter(|&&o| o + ms(400.0) < x.len())
        .map(|&o| {
            let b = bursts(&x[o..o + ms(400.0)], "4khz");
            assert_eq!(b.len(), 3, "{b:?}");
            b[1]
        })
        .collect();
    assert!(gaps.len() >= 30, "{} hits", gaps.len());
    let (lo, hi) = gaps.iter().fold((f32::MAX, 0.0f32), |m, &g| (m.0.min(g), m.1.max(g)));
    assert!(lo > 15.0 && hi < 25.0, "within 20% of 20 ms: {lo}..{hi}");
    assert!(hi - lo > 2.0, "the spacing sways: {lo}..{hi} ms");
    assert!(x == render(&src, 8), "the same on every play");
}

#[test]
fn a_clap_out_of_range_is_an_error_at_its_line() {
    let e = errors(&song("clap(bursts=12)", FOUR));
    assert!(e.contains("bursts = 12 is out of range (1..8)") && e.contains(NODE_LINE), "{e}");
    let e = errors(&song("clap(bursts=2.5)", FOUR));
    assert!(e.contains("bursts=2.5 is not a whole number") && e.contains(NODE_LINE), "{e}");
    let e = errors(&song("clap(spacing=1ms)", FOUR));
    assert!(e.contains("spacing = 1 is out of range (2..50)") && e.contains(NODE_LINE), "{e}");
    let e = errors(&song("clap(tail=220hz)", FOUR));
    assert!(e.contains("'tail' takes ms or s") && e.contains(NODE_LINE), "{e}");
    let e = errors(&song("clap(band=100hz)", FOUR));
    assert!(e.contains("band = 100 is out of range (300..8000)") && e.contains(NODE_LINE), "{e}");
    let e = errors(&song("clap(C3)", FOUR));
    assert!(e.contains("is a note") && e.contains(NODE_LINE), "{e}");
}

// ── Shares written as percentages ────────────────────────────────────────

/// `vary` is a share on every one of them, as on the kick: `vary=3%` is
/// `vary=0.03`, sample for sample, it is not `vary=0`, a share over the top
/// of its range is an error whichever way it is written, and a unit that is
/// not a share's says what a share takes.
#[test]
fn vary_takes_a_percentage_on_every_drum_node() {
    for (node, with, top) in [
        ("tom", "tom(A2, hold=40ms, release=150ms, ", "0.2"),
        ("hat", "hat(decay=100ms, ", "0.3"),
        ("snare", "snare(body=100ms, wires=120ms, ", "0.3"),
        ("clap", "clap(tail=120ms, ", "0.3"),
    ] {
        let play = |vary: &str| render(&song(&format!("{with}{vary})"), FOUR), 4);
        let (percent, plain, still) = (play("vary=3%"), play("vary=0.03"), play("vary=0"));
        assert!(percent == plain, "{node}: vary=3% is vary=0.03, sample for sample");
        assert!(percent != still, "{node}: and it is not vary=0");
        assert!(play("vary=20%") == play("vary=0.2"), "{node}: vary=20% is vary=0.2");

        let e = errors(&song(&format!("{with}vary=50%)"), FOUR));
        assert!(e.contains(&format!("vary = 0.5 is out of range (0..{top})")) && e.contains(NODE_LINE), "{node}: {e}");
        let e = errors(&song(&format!("{with}vary=3db)"), FOUR));
        assert!(e.contains(&format!("'vary' takes % or a plain 0..{top} ")) && e.contains(NODE_LINE), "{node}: {e}");
    }
}

// ── Where the kit meets the modulation ports ──

/// The five drum nodes, each as it is written bare.
const KIT: [&str; 5] = ["kick", "tom", "hat", "snare", "clap"];

/// An instrument with `body` for its lines, one note in its song. The body
/// starts on line 4.
fn patch(body: &str) -> String {
    format!("tempo 120\ninstrument piece {{\n  gain 1\n{body}\n}}\npattern beat {{ C3 - - - }}\ntrack t {{ play beat using piece out > master }}\n")
}

fn template(body: &str) -> tatum_core::graph::GraphTemplate {
    let src = patch(body);
    let ast = tatum_core::dsl::parse(&src).unwrap_or_else(|e| panic!("{e:?}\n{src}"));
    let song = tatum_core::dsl::compiler::compile(&ast).unwrap_or_else(|e| panic!("{e:?}\n{src}"));
    song.instruments[0].as_graph().expect("an instrument graph").clone()
}

/// One note on `inst`, let go after `held` blocks, played until the voice
/// ends: what it played, a whole number of blocks.
fn note(inst: &mut tatum_core::graph::voice::Instrument, held: usize) -> Vec<f32> {
    use tatum_core::BLOCK_SIZE;
    inst.note_on(48, 1.0);
    let mut out = Vec::new();
    let mut block = [0.0f32; BLOCK_SIZE];
    for n in 0..20 * SAMPLE_RATE as usize / BLOCK_SIZE {
        if n == held {
            inst.note_off(48);
        }
        inst.process_block(&mut block);
        out.extend_from_slice(&block);
        if inst.is_idle() {
            return out;
        }
    }
    panic!("the voice never ended");
}

fn seconds(x: &[f32]) -> f32 {
    x.len() as f32 / SAMPLE_RATE
}

/// A drum node has no ports: a wire into one, with a dot or a space, is an
/// error at its line that says so, by the rule every node without ports goes
/// by. A node named like a piece of the kit has the ports its kind has.
#[test]
fn a_drum_node_has_no_ports_and_a_name_like_one_keeps_its_own() {
    for piece in KIT {
        for wire in ["wob > p.pitch", "wob > p pitch", "lfo(4, 0.5) > p.cutoff", "osc sine(50) > gain(0.2) > p.fm"] {
            let e = errors(&patch(&format!("  {piece}() as p\n  lfo(4, 0.5) as wob\n  {wire}\n  p > out")));
            assert!(e.starts_with("line 6:") && e.lines().count() == 1, "{piece}, {wire}: {e}");
            assert!(e.contains(&format!("'p' is a {piece}, which has no ports to modulate")), "{piece}, {wire}: {e}");
        }
        // On the node as it opens, it is the port rule's own error.
        let e = errors(&patch(&format!("  lfo(4, 0.5) > {piece}() as p.pitch\n  p > out")));
        assert!(
            e.starts_with("line 4:") && e.contains("`p.pitch`: a port goes on a node that already has its name"),
            "{e}"
        );
        // A port on the bare word, with no node of that name, is still the
        // node written without its parentheses.
        let e = errors(&patch(&format!("  osc saw(55) as a\n  lfo(4, 0.5) > {piece}.pitch\n  a > out")));
        assert!(e.starts_with("line 5:") && e.contains(&format!("`{piece}()`")), "{piece}: {e}");
    }

    // Nodes named for the kit, with wires into their ports, against the same
    // graph under other names: the same samples.
    let graph = |osc: &str, filt: &str, amp: &str, src: &str| {
        format!(
            "  osc pulse(110) as {osc}\n  noise as {src}\n  {osc} > mix\n  {src} > gain(0.1) > mix\n  mix > svf(800, 0.3) as {filt}\n  lfo(2, 0.5) > {filt}.cutoff\n  lfo(3, 0.2) > {osc} width\n  {src} > gain(0.2) > {osc}.fm\n  {filt} > adsr(0.01, 0.1, 0.5, 0.1) as {amp}\n  {amp} > out"
        )
    };
    let named = patch(&graph("hat", "clap", "tom", "snare"));
    assert_eq!(errors(&named), "");
    let other = render(&patch(&graph("a", "f", "amp", "n")), 1);
    assert!(other.iter().any(|v| v.abs() > 0.01));
    assert_eq!(render(&named, 1), other, "a node named for a drum is the node it was made");
    let unwired = render(&patch(&graph("a", "f", "amp", "n").replace("a width", "a").replace("f.cutoff", "f")), 1);
    assert_ne!(other, unwired, "the ports are wired");
    // And one named `kick`, which opens no node without its parentheses.
    assert_eq!(errors(&patch("  osc saw(55) as kick\n  lfo(5, 0.1) > kick.pitch\n  kick > out")), "");
}

/// After a chain on the same line a word is a port of the last node unless
/// it opens a node: a drum node with its parentheses does, as `kick(` did,
/// and without them it gets the hint, not a port's error.
#[test]
fn a_drum_node_opens_after_a_chain_on_its_line() {
    for piece in KIT {
        let apart = patch(&format!("  noise as n\n  n > gain(0.01) > mix\n  {piece}() as p\n  p > mix\n  mix > out"));
        let joined = patch(&format!("  noise as n\n  n > gain(0.01) > mix  {piece}() as p  p > mix  mix > out"));
        assert_eq!(errors(&joined), "", "{piece}");
        let heard = render(&apart, 1);
        assert!(
            heard.iter().any(|v| v.abs() > 0.05),
            "{piece} sounds: {}",
            heard.iter().fold(0.0f32, |m, v| m.max(v.abs()))
        );
        assert_eq!(render(&joined, 1), heard, "{piece}: one line or three");

        let e = errors(&patch(&format!("  noise as n\n  n > gain(0.01) > mix  {piece} as p  p > mix  mix > out")));
        assert!(e.starts_with("line 5:") && e.lines().count() == 1, "{piece}: {e}");
        assert!(e.ends_with(&format!("written with its parentheses, `{piece}() as ...`")), "{piece}: {e}");
    }
    // A name that is one of the words is still a name there: two chains.
    let apart = patch("  noise as hat\n  gain(0.5) as b\n  hat > b\n  b > out");
    let joined = patch("  noise as hat\n  gain(0.5) as b\n  b > out  hat > b");
    assert_eq!(errors(&joined), "");
    assert_eq!(render(&joined, 1), render(&apart, 1));
}

/// A drum node is sound, and its own envelope: alone in its graph it plays
/// its whole tail however short the note, and then the voice ends. Beside an
/// envelope that only moves a port it is still what the voice waits for, and
/// the only thing.
#[test]
fn a_drum_node_alone_plays_its_tail_and_ends_the_voice() {
    use tatum_core::graph::voice::Instrument;
    for (node, least_ms, most_ms) in [
        ("kick(A1, hold=80ms, release=200ms)", 200.0, 500.0),
        ("tom(A2, hold=40ms, release=300ms)", 250.0, 600.0),
        ("hat(decay=400ms)", 350.0, 700.0),
        ("snare(body=120ms, wires=300ms)", 250.0, 700.0),
        ("clap(tail=400ms)", 350.0, 800.0),
    ] {
        let alone = template(&format!("  {node} as s\n  s > out"));
        assert_eq!(alone.env_count, 1, "{node}: the node is what the voice waits for");
        assert_eq!(alone.hold_mask, 1 << alone.env_nodes[0], "{node}");
        let short = note(&mut Instrument::new(alone.clone()), 1);
        let held = note(&mut Instrument::new(alone), 100_000);
        assert_eq!(short, held, "{node}: the length of the note is not heard");
        let len = seconds(&short) * 1000.0;
        assert!(len > least_ms && len < most_ms, "{node}: the voice ran {len} ms");
        // Loud at the start and gone by the end: the tail was played out.
        let last = &short[short.len() - ms(5.0)..];
        assert!(peak_in(&short, 0.0, 60.0) > 0.05, "{node} sounds");
        assert!(last.iter().all(|v| v.abs() < 2e-3), "{node}: cut at {}", peak_in(last, 0.0, 5.0));

        // Through a filter that an envelope with five seconds of release
        // sweeps: the sweep does not hold the voice, the drum still does.
        let filtered = format!("  {node} as s\n  s > svf(3000, 0.2) as f\n  f > out");
        let plain = note(&mut Instrument::new(template(&filtered)), 1);
        let swept_graph =
            template(&format!("{filtered}\n  adsr(0.001, 0.2, 0.5, 5.0) as fenv\n  fenv > gain(1.5) > f.cutoff"));
        assert_eq!(swept_graph.hold_mask.count_ones(), 1, "{node}: only the drum holds");
        let swept = note(&mut Instrument::new(swept_graph), 1);
        assert_eq!(swept.len(), short.len(), "{node}: the sweep's release held the voice, or cut the drum");
        assert_eq!(plain.len(), short.len());
        assert!(swept != plain && swept.iter().all(|v| v.is_finite()), "{node}: the sweep is heard");
    }
}

/// A drum node is sound, not a control signal: wired into a port and into
/// the mix as well it is not refused, as an LFO or a bare envelope would be,
/// through a `gain` or not.
#[test]
fn a_drum_node_into_a_port_and_the_sound_is_not_a_control_signal() {
    for piece in KIT {
        let voice = "  osc saw(55) as a\n  a > svf(800, 0.3) as f\n  f > mix\n  mix > out";
        for wires in ["  d > f.cutoff\n  d > mix", "  d > gain(2) as ctl\n  ctl > f.cutoff\n  ctl > mix"] {
            let src = patch(&format!("  {piece}() as d\n{voice}\n{wires}"));
            assert_eq!(errors(&src), "", "{piece}");
        }
        // The same wires from an LFO are the error.
        let e = errors(&patch(&format!("  lfo(4, 0.5) as d\n{voice}\n  d > f.cutoff\n  d > mix")));
        assert!(e.contains("'d' carries a control signal"), "{e}");
    }
}

/// What comes out of a drum node can go into any port, at audio rate, like
/// any other sound: a hat on a filter's cutoff, a snare on an oscillator's
/// FM, a tom on its pitch, a clap on a pulse's width, a kick on its sync.
/// The output is another sound, and stays finite and bounded. The drum is
/// then one of the things the voice waits for, heard or not.
#[test]
fn a_drum_node_drives_a_port_at_audio_rate() {
    use tatum_core::graph::voice::Instrument;
    const VOICE: &str = "  osc pulse(110, width=0.5) as a\n  a > svf(900, 0.4) as filt\n  filt > adsr(0.001, 0.1, 1.0, 0.05) as amp\n  amp > out";
    let plain = note(&mut Instrument::new(template(VOICE)), 40);
    assert!(plain.iter().any(|v| v.abs() > 0.1));
    for wire in [
        "hat(decay=300ms) > filt.cutoff",
        "hat(decay=300ms) > gain(4) > filt.cutoff",
        "snare() > a.fm",
        "snare() > gain(8) > a.fm",
        "tom(A2) > gain(0.5) > a.pitch",
        "clap() > gain(0.4) > a.width",
        "kick(A1) > a.sync",
        "clap() > filt.morph",
    ] {
        let t = template(&format!("{VOICE}\n  {wire}"));
        assert_eq!(t.env_count, 2, "{wire}");
        assert_eq!(t.hold_mask.count_ones(), 2, "{wire}: the drum and the envelope both hold");
        let bent = note(&mut Instrument::new(t.clone()), 40);
        assert!(
            bent.iter().all(|v| v.is_finite() && v.abs() < 4.0),
            "{wire}: {:?}",
            bent.iter().fold(0.0f32, |m, v| m.max(v.abs()))
        );
        let n = plain.len().min(bent.len());
        assert!(bent[..n] != plain[..n], "{wire}: nothing moved");
        // The same on every note with no `vary`: the drum starts with it.
        assert_eq!(note(&mut Instrument::new(t), 40), bent, "{wire}");
    }
    // A swaying drum as the modulator sways the port by the hit's number.
    let mut inst = Instrument::new(template(&format!("{VOICE}\n  snare(vary=30%) > gain(4) > a.fm")));
    let hits: Vec<Vec<f32>> = (0..3).map(|_| note(&mut inst, 40)).collect();
    assert!(hits[0] != hits[1] && hits[1] != hits[2], "no hit differs");
    let mut again = Instrument::new(template(&format!("{VOICE}\n  snare(vary=30%) > gain(4) > a.fm")));
    assert_eq!(note(&mut again, 40), hits[0]);
}

/// A voice dropped for leaving the numbers has its nodes made new from
/// their specs, a drum node among them: with the hat's level as it was found
/// when the line was compiled, and for a node that sways, on the next note,
/// the hit it would have been -- not hit 0 again, not the dropped one's.
#[test]
fn a_dropped_voice_leaves_every_drum_node_as_it_was_compiled() {
    use tatum_core::graph::node::NodeSpec;
    use tatum_core::graph::voice::Instrument;
    use tatum_core::BLOCK_SIZE;
    for node in [
        "tom(A2, hold=20ms, release=120ms, click=-20db, drive=0.3, vary=10%)",
        "hat(decay=90ms, band=5khz, floor=2khz, noise=60%, vary=20%)",
        "hat(decay=90ms, band=3khz, floor=500hz, noise=0, pitch=400hz)",
        "snare(F#3, wires=120ms, drive=0.4, vary=10%)",
        "clap(bursts=5, tail=150ms, vary=15%)",
    ] {
        let sways = node.contains("vary");
        let template = template(&format!("  {node} as p\n  p > gain(1) as trim\n  trim > out"));
        let gain = (0..template.node_count as usize)
            .find(|&i| matches!(template.specs[i], NodeSpec::Gain { .. }))
            .expect("the gain");
        let mut clean = Instrument::new(template.clone());
        let hits: Vec<Vec<f32>> = (0..4).map(|_| note(&mut clean, 1)).collect();
        assert!(hits[3].iter().any(|v| v.abs() > 0.02), "{node} sounds");
        assert_eq!(sways, hits[0] != hits[3] && hits[2] != hits[3], "{node}");

        for poison in [f32::NAN, f32::INFINITY] {
            let mut inst = Instrument::new(template.clone());
            assert_eq!(note(&mut inst, 1), hits[0], "{node}");
            assert_eq!(note(&mut inst, 1), hits[1], "{node}");
            // Hit 2 goes wrong and is dropped whole, on its first block.
            inst.template.specs[gain] = NodeSpec::Gain { amount: poison };
            let dropped = note(&mut inst, 1);
            assert!(dropped.iter().all(|&x| x == 0.0), "{node}: {poison} got out");
            assert_eq!(dropped.len(), BLOCK_SIZE, "{node}: dropped on its first block");
            inst.template.specs[gain] = NodeSpec::Gain { amount: 1.0 };
            let after = note(&mut inst, 1);
            assert_eq!(after.len(), hits[3].len(), "{node}");
            assert!(after.iter().zip(hits[3].iter()).all(|(a, b)| a.to_bits() == b.to_bits()), "{node}: not hit 3");
        }
    }
}
