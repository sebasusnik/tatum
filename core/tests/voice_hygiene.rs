//! What every voice must never do.
//!
//! Every other test in this suite checks that something sounds. None of them
//! check that it sounds right, which is how the kick shipped for months with a
//! click that was white noise running at full level to 20 kHz, and the hi-hat
//! with 60% of its energy above 10 kHz and its peak at 15 kHz. Both were
//! audible in every song in the repo and invisible to the suite.
//!
//! These are the assertions that would have caught them, applied to every voice.


use synth_core::song_engine::SongEngine;
use synth_core::SAMPLE_RATE;

const DRUM_LANES: [&str; 7] = ["kick", "snare", "hihat", "openhat", "clap", "tom", "crash"];
const MODULES: [(&str, &str); 3] = [("bass", "1.2"), ("keys", "1.3"), ("fm", "1.3")];

/// One drum lane, hit once, rendered alone.
fn drum(lane: &str) -> Vec<f32> {
    let src = format!(
        "tempo 120\nscale C major\n\
         module beats kit {{ kick_level 1.0 snare_level 1.0 hihat_level 1.0 clap_level 1.0 }}\n\
         pattern p {{ {}: X -*31 }}\n\
         track d {{ play p using kit out > master }}\n\
         master {{ in > out }}\n\
         scene a {{ track d {{ play p using kit }} }}\narrange {{ a x1 }}\n",
        lane
    );
    render(&src)
}

/// One melodic module holding a note, rendered alone. `note` is a degree.
fn melodic(kind: &str, note: &str, steps: usize) -> Vec<f32> {
    let src = format!(
        "tempo 120\nscale C major\n\
         module {kind} v {{ }}\n\
         pattern p {{ {note}:0.9 ..*{hold} -*{rest} }}\n\
         track t {{ play p using v out > master }}\n\
         master {{ in > out }}\n\
         scene a {{ track t {{ play p using v }} }}\narrange {{ a x1 }}\n",
        kind = kind, note = note, hold = steps - 1, rest = 32 - steps
    );
    render(&src)
}

fn render(src: &str) -> Vec<f32> {
    let mut engine = SongEngine::from_source(src).unwrap_or_else(|e| panic!("{}\n{}", e, src));
    engine.start();
    engine.render(2).0
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

/// Share of energy above `hz`, via a steep highpass built from the same
/// cascaded one-poles the band meter uses.
fn energy_above(x: &[f32], hz: f32) -> f32 {
    const POLES: usize = 3;
    // Same cascade correction the band meter uses, which is calibrated for
    // three poles: 1 / sqrt(2^(1/3) - 1).
    let c = synth_core::math::exp(-2.0 * synth_core::math::PI * hz * 1.9615 / SAMPLE_RATE);
    let mut state = [0.0f32; POLES];
    let (mut high, mut total) = (0.0f64, 0.0f64);
    for v in x {
        let mut low = *v;
        for s in state.iter_mut() {
            *s = low * (1.0 - c) + *s * c;
            low = *s;
        }
        let h = v - low;
        high += (h * h) as f64;
        total += (v * v) as f64;
    }
    if total <= 0.0 { 0.0 } else { (high / total) as f32 }
}

// ── No voice may produce a broken signal ──

#[test]
fn no_voice_produces_non_finite_samples() {
    for lane in DRUM_LANES {
        assert!(drum(lane).iter().all(|v| v.is_finite()), "{} produced NaN or infinity", lane);
    }
    for (kind, note) in MODULES {
        assert!(melodic(kind, note, 16).iter().all(|v| v.is_finite()), "{} produced NaN or infinity", kind);
    }
}

#[test]
fn no_voice_leaves_a_dc_offset() {
    // A constant offset eats headroom and sums across tracks without ever
    // being heard, so it is invisible until the master clips early.
    for lane in DRUM_LANES {
        let x = drum(lane);
        let mean = x.iter().map(|v| *v as f64).sum::<f64>() / x.len() as f64;
        let p = peak(&x).max(1e-9);
        assert!(
            (mean.abs() as f32) < p * 0.01,
            "{}: DC offset {:.6} against a peak of {:.3}", lane, mean, p
        );
    }
    for (kind, note) in MODULES {
        let x = melodic(kind, note, 16);
        let mean = x.iter().map(|v| *v as f64).sum::<f64>() / x.len() as f64;
        let p = peak(&x).max(1e-9);
        assert!((mean.abs() as f32) < p * 0.01, "{}: DC offset {:.6} against a peak of {:.3}", kind, mean, p);
    }
}

#[test]
fn every_voice_falls_silent_after_its_note_ends() {
    // A voice that never releases holds a slot forever and builds up.
    for lane in DRUM_LANES {
        // A crash and an open hat are meant to ring; the rest are not.
        let limit = if lane == "crash" || lane == "openhat" { 0.12 } else { 0.02 };
        let x = drum(lane);
        let tail = &x[x.len() * 3 / 4..];
        assert!(
            peak(tail) < peak(&x) * limit,
            "{} is still sounding at 75% of the render: {:.4} against a peak of {:.4}",
            lane, peak(tail), peak(&x)
        );
    }
    for (kind, note) in MODULES {
        let x = melodic(kind, note, 8);
        let tail = &x[x.len() * 7 / 8..];
        assert!(
            peak(tail) < peak(&x) * 0.05,
            "{} is still sounding long after note-off: {:.4} against a peak of {:.4}",
            kind, peak(tail), peak(&x)
        );
    }
}

// ── No voice may live in the wrong part of the spectrum ──

#[test]
fn no_drum_voice_dumps_its_energy_into_the_top_octave() {
    // The hi-hat used to put 60% of its energy above 10 kHz with its peak at
    // 15 kHz, which reads as hiss rather than as a cymbal and is fatiguing
    // under a 16th pattern. Nothing in the engine should live up there.
    let limits: [(&str, f32); 7] = [
        ("kick", 0.02), ("snare", 0.25), ("hihat", 0.55), ("openhat", 0.45),
        ("clap", 0.10), ("tom", 0.02), ("crash", 0.55),
    ];
    for (lane, limit) in limits {
        let above = energy_above(&drum(lane), 10000.0);
        assert!(
            above < limit,
            "{}: {:.0}% of its energy is above 10 kHz (limit {:.0}%)",
            lane, above * 100.0, limit * 100.0
        );
    }
}

#[test]
fn the_kick_keeps_its_weight_in_the_low_end() {
    let x = drum("kick");
    let above = energy_above(&x, 250.0);
    assert!(above < 0.35, "the kick put {:.0}% of its energy above 250 Hz", above * 100.0);
}

#[test]
fn the_kick_click_is_a_beater_and_not_a_tick() {
    // Rendering the same kick with and without the click isolates it. It used
    // to be white noise with only a highpass, running to 20 kHz.
    let with_click = drum("kick");
    let src = "tempo 120\nscale C major\n\
        module beats kit { kick_level 1.0 kick_click 0.0 }\n\
        pattern p { kick: X -*31 }\n\
        track d { play p using kit out > master }\n\
        master { in > out }\n\
        scene a { track d { play p using kit } }\narrange { a x1 }\n";
    let without = render(src);
    let click: Vec<f32> = with_click.iter().zip(&without).map(|(a, b)| a - b).collect();
    assert!(peak(&click) > 0.001, "the click made no difference, so this test proves nothing");
    let above = energy_above(&click, 8000.0);
    assert!(
        above < 0.25,
        "the kick click puts {:.0}% of its energy above 8 kHz, which is a tick and not a beater",
        above * 100.0
    );
}

// ── No voice may alias ──

#[test]
fn raising_the_pitch_does_not_fold_energy_back_down() {
    // Aliasing shows up as partials moving the wrong way: play a note an
    // octave higher and the share of energy in the top band should rise, not
    // fall. When it falls, content above Nyquist is folding back down.
    for (kind, _) in MODULES {
        let low = melodic(kind, "1.2", 16);
        let high = melodic(kind, "1.5", 16);
        let (a, b) = (energy_above(&low, 8000.0), energy_above(&high, 8000.0));
        assert!(
            b >= a * 0.7,
            "{}: three octaves up the share above 8 kHz went from {:.1}% to {:.1}%, which means it is folding",
            kind, a * 100.0, b * 100.0
        );
    }
}

// ── No voice may click ──

#[test]
fn no_voice_starts_or_stops_with_a_discontinuity() {
    // A step between one sample and the next is a click no envelope asked for.
    for (kind, note) in MODULES {
        let x = melodic(kind, note, 8);
        let p = peak(&x).max(1e-9);
        let worst = x.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
        // A 20 kHz full-scale sine steps by about 2.8 of its peak per sample at
        // 44.1 kHz, so anything under that is a signal and not a click.
        assert!(
            worst < p * 1.5,
            "{}: a {:.3} jump between consecutive samples against a peak of {:.3}",
            kind, worst, p
        );
    }
}
