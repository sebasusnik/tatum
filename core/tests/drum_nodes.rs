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
