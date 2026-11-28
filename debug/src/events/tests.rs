use super::inside::Fig;
use super::report::Summary;
use super::*;
use std::f32::consts::TAU;

/// The same noise every run, -1..1.
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.0 >> 8) as f32 / 8_388_608.0 - 1.0
    }
}

fn at(s: f32) -> usize {
    (s * SR) as usize
}

/// A sine struck at `when`: it rises in a straight line over `attack_ms`,
/// falls with time constant `tau_ms`, and bends `st_per_s` semitones a second.
fn ping(out: &mut [f32], when: f32, hz: f32, tau_ms: f32, amp: f32, attack_ms: f32, st_per_s: f32) {
    let start = at(when);
    let mut phase = 0.0f32;
    for i in 0..at(tau_ms * 0.012).min(out.len() - start) {
        let t = i as f32 / SR;
        let rise = if attack_ms > 0.0 { (t * 1000.0 / attack_ms).min(1.0) } else { 1.0 };
        out[start + i] += amp * rise * (-t * 1000.0 / tau_ms).exp() * (TAU * phase).sin();
        phase = (phase + hz * 2f32.powf(st_per_s * t / 12.0) / SR).fract();
    }
}

/// A pitch that falls from `f0` towards `f1` with time constant `tau_f_ms`,
/// struck at full level and falling with `tau_a_ms`: a kick's body, a zap.
fn sweep(out: &mut [f32], when: f32, f0: f32, f1: f32, tau_f_ms: f32, tau_a_ms: f32, amp: f32) {
    let start = at(when);
    let mut phase = 0.0f64;
    for i in 0..at(tau_a_ms * 0.008).min(out.len() - start) {
        let t = i as f32 * 1000.0 / SR;
        out[start + i] += amp * (-t / tau_a_ms).exp() * (std::f64::consts::TAU * phase).sin() as f32;
        phase += ((f1 + (f0 - f1) * (-t / tau_f_ms).exp()) / SR) as f64;
    }
}

fn bed(seconds: f32, noise: f32, pad: f32) -> Vec<f32> {
    let mut n = Noise(7);
    (0..at(seconds))
        .map(|i| {
            let t = i as f32 / SR;
            noise * n.next() + pad * ((TAU * 220.0 * t).sin() + (TAU * 331.0 * t).sin() + (TAU * 443.0 * t).sin())
        })
        .collect()
}

fn hiss(x: &mut [f32], amp: f32, seed: u32) {
    let mut n = Noise(seed);
    for v in x.iter_mut() {
        *v += amp * n.next();
    }
}

fn run(x: Vec<f32>) -> Report {
    measure(x.clone(), x, &Options::default(), true)
}

fn val(f: Fig, what: &str) -> f32 {
    match f {
        Fig::Value(v) => v,
        other => panic!("{what}: {other:?}"),
    }
}

fn median_of(r: &Report, key: &str) -> Summary {
    let me = report::MEASURES.iter().find(|m| m.key == key).unwrap();
    report::population(r, me)
}

fn advice_of(a: &Report, b: &Report) -> Vec<String> {
    let rows = report::rows(a, b);
    report::advice(a, b, &rows, "b.wav")
}

const TIMES: [f32; 12] = [0.31, 0.52, 0.61, 0.93, 1.28, 1.37, 1.74, 2.02, 2.33, 2.41, 2.86, 3.19];

fn pings(tau_ms: f32, noise: f32, pad: f32) -> Vec<f32> {
    let mut x = bed(3.8, noise, pad);
    for (k, t) in TIMES.iter().enumerate() {
        ping(&mut x, *t, 800.0 + 90.0 * ((k * 5) % 12) as f32, tau_ms, 0.25, 0.0, 0.0);
    }
    x
}

#[test]
fn events_at_irregular_times_over_a_bed_are_found_to_two_milliseconds() {
    let r = run(pings(18.0, 0.01, 0.03));
    assert_eq!(r.events.len(), TIMES.len(), "{:?}", r.events.iter().map(|e| e.start_s).collect::<Vec<_>>());
    for (e, t) in r.events.iter().zip(TIMES) {
        assert!((e.start_s - t).abs() < 0.002, "found at {} for {t}", e.start_s);
    }
    // The band is where the pings are, not where the pad is.
    assert!(r.band.0 > 450.0 && r.band.0 < 800.0 && r.band.1 > 1790.0, "{:?}", r.band);
    // With nothing but the bed, nothing is found.
    assert!(run(bed(3.8, 0.01, 0.03)).events.is_empty());
    // Nor in half a minute of hiss alone, whose every bin flutters.
    let hiss = run(bed(30.0, 0.05, 0.0));
    assert!(
        hiss.events.is_empty(),
        "{:?}",
        hiss.events.iter().map(|e| (e.start_s, e.over_floor_db)).collect::<Vec<_>>()
    );
}

#[test]
fn a_damped_sine_gives_back_its_attack_decay_and_width() {
    // 1 kHz, a 5 ms straight rise (4 ms from 10% to 90%), tau 25 ms:
    // 8.686 / 25 = 0.347 dB/ms, a -3 dB width of 1 / (pi tau) = 12.7 Hz, Q 78.5.
    let mut x = vec![0.0f32; at(2.0)];
    for t in [0.2, 0.6, 1.0, 1.4] {
        ping(&mut x, t, 1000.0, 25.0, 0.5, 5.0, 0.0);
    }
    let r = run(x);
    assert_eq!(r.events.len(), 4);
    for e in &r.events {
        let i = &e.inside;
        assert!((val(i.attack_ms, "attack") - 4.0).abs() < 0.5, "attack {:?}", i.attack_ms);
        let rate = val(i.decay_db_ms, "decay");
        assert!((rate / 0.347 - 1.0).abs() < 0.03, "decay {rate} dB/ms");
        assert!(val(i.decay_r2, "r2") > 0.99, "an exponential is straight in dB: {:?}", i.decay_r2);
        assert!((val(i.t20_ms, "t20") - 57.6).abs() < 4.0, "-20 dB at {:?}", i.t20_ms);
        assert!((i.start_hz - 1000.0).abs() < 2.0 && i.bend_st.abs() < 0.05, "{} Hz, bend {}", i.start_hz, i.bend_st);
        assert_eq!(i.shape, inside::Shape::Flat);
        assert!((val(i.bandwidth_hz, "width") / 12.73 - 1.0).abs() < 0.1, "width {:?}", i.bandwidth_hz);
        assert!((val(i.q, "q") / 78.5 - 1.0).abs() < 0.1, "Q {:?}", i.q);
        assert!((val(i.ring_q, "ring q") / 78.5 - 1.0).abs() < 0.03, "Q from the decay {:?}", i.ring_q);
    }
    // Struck with no rise at all, it reads under a millisecond, as a bound:
    // eight tenths of a period is as fast as the band rises.
    let mut x = vec![0.0f32; at(1.0)];
    ping(&mut x, 0.3, 1000.0, 25.0, 0.5, 0.0, 0.0);
    let r = run(x);
    assert!(matches!(r.events[0].inside.attack_ms, Fig::AtMost(v) if v < 1.0), "{:?}", r.events[0].inside.attack_ms);
    // And a quarter of the time constant is four times the decay rate.
    let mut x = vec![0.0f32; at(1.0)];
    ping(&mut x, 0.3, 1000.0, 6.25, 0.5, 0.0, 0.0);
    let rate = val(run(x).events[0].inside.decay_db_ms, "decay");
    assert!((rate / 1.39 - 1.0).abs() < 0.05, "decay {rate} dB/ms");
}

#[test]
fn on_damped_sines_every_value_is_near_the_truth_and_every_bound_holds_it() {
    // Pitches 50 Hz to 12 kHz, time constants 2 to 150 ms: inside a measure's
    // domain the value, outside it a bound on the right side or nothing.
    let holds = |f: Fig, truth: f32, tol: f32, what: &str| match f {
        Fig::Value(v) => assert!((v / truth - 1.0).abs() < tol, "{what}: {v} for {truth}"),
        Fig::AtMost(v) => assert!(truth <= v * 1.01, "{what}: at most {v} for {truth}"),
        Fig::AtLeast(v) => assert!(truth >= v * 0.99, "{what}: at least {v} for {truth}"),
        Fig::No(_) => {}
    };
    for hz in [50.0f32, 110.0, 400.0, 1000.0, 5000.0, 12000.0] {
        for tau in [2.0f32, 6.0, 25.0, 150.0] {
            let gap = (tau * 0.014).max(0.5);
            let mut x = vec![0.0f32; at(0.3 + 4.0 * gap + 0.9)];
            for k in 0..4 {
                ping(&mut x, 0.3 + gap * k as f32, hz, tau, 0.4, 0.0, 0.0);
            }
            let r = run(x);
            assert_eq!(r.events.len(), 4, "{hz} Hz, tau {tau}");
            let what = |m: &str| format!("{hz} Hz, tau {tau}: {m}");
            for e in &r.events {
                let i = &e.inside;
                holds(i.decay_db_ms, 8.686 / tau, 0.03, &what("decay"));
                holds(i.t20_ms, tau * 10f32.ln(), 0.05, &what("fall to -20"));
                holds(i.bandwidth_hz, 1000.0 / (std::f32::consts::PI * tau), 0.05, &what("width"));
                holds(i.q, std::f32::consts::PI * hz * tau / 1000.0, 0.05, &what("Q"));
                holds(i.ring_q, std::f32::consts::PI * hz * tau / 1000.0, 0.03, &what("ring Q"));
                holds(i.start, hz, 0.015, &what("start pitch"));
                // Struck with no rise: 0 ms is under every attack bound.
                assert!(!matches!(i.attack_ms, Fig::Value(v) if v > 0.3), "{}: {:?}", what("attack"), i.attack_ms);
                assert!(!matches!(i.tail_db, Fig::Value(_)), "{}: {:?}", what("tail"), i.tail_db);
            }
        }
    }
}

#[test]
fn low_notes_refuse_what_their_band_cannot_resolve() {
    // At 110 Hz the attack band rises no faster than 7 ms: an instant onset
    // and a 4 ms one both read as "at most", and nothing is advised.
    let make = |attack: f32| {
        let mut x = vec![0.0f32; at(4.0)];
        for k in 0..4 {
            ping(&mut x, 0.3 + 0.9 * k as f32, 110.0, 80.0, 0.4, attack, 0.0);
        }
        run(x)
    };
    let (instant, five) = (make(0.0), make(5.0));
    for r in [&instant, &five] {
        assert!(
            r.events.iter().all(|e| matches!(e.inside.attack_ms, Fig::AtMost(_))),
            "{:?}",
            r.events[0].inside.attack_ms
        );
    }
    assert!(advice_of(&instant, &five).iter().all(|l| !l.starts_with("attack")), "{:?}", advice_of(&instant, &five));
    // A slow one is read: 15 ms of rise is 12 ms from 10 to 90%.
    let slow = make(15.0);
    let a = val(slow.events[0].inside.attack_ms, "attack");
    assert!((a / 12.0 - 1.0).abs() < 0.15, "attack {a}");
    // A 2 ms decay at 110 Hz is faster than its band: a bound, and no
    // skirt-dependent figure.
    let mut x = vec![0.0f32; at(3.0)];
    for k in 0..4 {
        ping(&mut x, 0.3 + 0.6 * k as f32, 110.0, 2.0, 0.4, 0.0, 0.0);
    }
    let r = run(x);
    for e in &r.events {
        assert!(matches!(e.inside.decay_db_ms, Fig::AtLeast(_) | Fig::No(_)), "{:?}", e.inside.decay_db_ms);
        assert!(e.inside.tonal_db.value().is_none() && e.inside.h2_db.value().is_none(), "{:?}", e.inside.h2_db);
    }
    // At 50 Hz a ping of 25 ms has under four cycles above -20 dB: no pitch.
    let mut x = vec![0.0f32; at(3.0)];
    for k in 0..4 {
        ping(&mut x, 0.3 + 0.6 * k as f32, 50.0, 25.0, 0.4, 0.0, 0.0);
    }
    let r = run(x);
    for e in &r.events {
        match e.inside.start {
            Fig::Value(hz) => assert!((hz / 50.0 - 1.0).abs() < 0.01, "{hz}"),
            Fig::No(_) => {}
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn a_bend_is_followed_to_a_few_cents() {
    // Falling 30 semitones a second: three semitones over the 100 ms it sounds.
    let mut x = vec![0.0f32; at(1.5)];
    ping(&mut x, 0.4, 1400.0, 40.0, 0.5, 0.0, -30.0);
    let r = run(x);
    let e = &r.events[0];
    let truth = |ms: f32| 1400.0 * 2f32.powf(-30.0 * ms * 0.001 / 12.0);
    assert!(e.inside.pitch.len() > 30, "{} readings", e.inside.pitch.len());
    for &(ms, hz) in &e.inside.pitch {
        let cents = 1200.0 * (hz / truth(ms)).log2();
        assert!(cents.abs() < 6.0, "{cents} cents off at {ms} ms");
    }
    let (first, last) = (e.inside.pitch[0].0, e.inside.pitch[e.inside.pitch.len() - 1].0);
    let expect = -0.030 * (last - first);
    assert!(expect < -2.0, "the readings span {first}..{last} ms");
    assert!((e.inside.bend_st - expect).abs() < 0.2, "bend {} for {expect}", e.inside.bend_st);
    assert_eq!(e.inside.shape, inside::Shape::Straight);
    // The glide is taken out of the width: it is the decay's, 8 Hz, not the 250 Hz swept.
    assert!(val(e.inside.bandwidth_hz, "width") < 16.0, "width {:?}", e.inside.bandwidth_hz);
}

/// A start pitch is the truth within 8%, or a bound the truth is inside --
/// one that says at least 60% of it for a sweep the tool reads (under a
/// tenth of its frequency a cycle); a bend likewise within 1.5 st or a bound.
fn start_holds(f: Fig, truth: f32, handled: bool, what: &str) {
    let least = if handled { 0.6 } else { 0.0 };
    match f {
        Fig::Value(v) => assert!((v / truth - 1.0).abs() < 0.08, "{what}: start {v} for {truth}"),
        Fig::AtLeast(v) => assert!(v <= truth * 1.01 && v >= least * truth, "{what}: start at least {v} for {truth}"),
        other => panic!("{what}: start {other:?} for {truth}"),
    }
}

fn bend_holds(f: Fig, truth: f32, handled: bool, what: &str) {
    match f {
        Fig::Value(v) => assert!((v - truth).abs() < 1.5, "{what}: bend {v} for {truth}"),
        Fig::AtMost(v) => {
            assert!(truth <= v + 0.3 && (!handled || v < 0.5 * truth), "{what}: bend at most {v} for {truth}")
        }
        other => panic!("{what}: bend {other:?} for {truth}"),
    }
}

#[test]
fn a_kick_sweep_and_a_zap_are_followed_from_their_start() {
    // (from, to, pitch time constant, level time constant, what it falls to
    // -40 dB in): kicks whose spectrogram frames are longer than the sweep,
    // a zap, and a kick faster than the fastest sweep read.
    for (f0, f1, tau_f, tau_a, what) in [
        (200.0f32, 50.0f32, 20.0f32, 150.0f32, "kick 200-50"),
        (150.0, 45.0, 40.0, 200.0, "kick 150-45"),
        (400.0, 50.0, 10.0, 150.0, "kick 400-50"),
        (2000.0, 500.0, 20.0, 80.0, "zap 2000-500"),
        (400.0, 40.0, 3.0, 150.0, "kick 400-40 in 3 ms"),
    ] {
        let mut x = vec![0.0f32; at(4.0)];
        for k in 0..4 {
            sweep(&mut x, 0.3 + 0.9 * k as f32, f0, f1, tau_f, tau_a, 0.5);
        }
        let r = run(x);
        assert_eq!(r.events.len(), 4, "{what}");
        // The pitch over the stretch above -20 dB: from f0 to where the sweep
        // is when the level is 20 dB down.
        let t20 = tau_a * 10f32.ln();
        let at20 = f1 + (f0 - f1) * (-t20 / tau_f).exp();
        let bend = 12.0 * (at20 / f0).log2();
        for e in &r.events {
            let i = &e.inside;
            // The share of its frequency the sweep moves in its first cycle.
            let share = (f0 - f1) / tau_f * (1000.0 / f0) / f0;
            let handled = share <= inside::FASTEST_SWEEP;
            // A sweep over before one cycle of where it lands is no glide to
            // read: it is the landing note, and what came before is the burst.
            let landed = tau_f * 3.0 < 1000.0 / f1;
            if landed && i.start.value().is_some_and(|v| (v / f1 - 1.0).abs() < 0.08) {
                continue;
            }
            start_holds(i.start, f0, handled, what);
            bend_holds(i.bend, bend, handled, what);
            let decay = val(i.decay_db_ms, what);
            assert!((decay / (8.686 / tau_a) - 1.0).abs() < 0.05, "{what}: decay {decay}");
            let len = (e.end_s - e.start_s) * 1000.0;
            let to40 = tau_a * 100f32.ln();
            assert!((len / to40 - 1.0).abs() < 0.1, "{what}: length {len} for {to40}");
            assert!(!matches!(i.tail_db, Fig::Value(_)), "{what}: a dry sweep has no tail: {:?}", i.tail_db);
            assert!(!i.lost, "{what}");
        }
    }
}

#[test]
fn noise_struck_with_the_event_lowers_its_tonal_reading_by_what_was_added() {
    let make = |noise: f32| {
        let mut x = vec![0.0f32; at(2.0)];
        let mut n = Noise(3);
        for t in [0.3, 0.8, 1.3] {
            ping(&mut x, t, 1000.0, 25.0, 0.4, 2.0, 0.0);
            // Noise with the ping's own envelope: its power against the
            // sine's is noise^2 / 3 over 1 / 2.
            for i in 0..at(0.3) {
                let s = i as f32 / SR;
                x[at(t) + i] += 0.4 * noise * (s * 500.0).min(1.0) * (-s * 40.0).exp() * n.next();
            }
        }
        let r = run(x);
        assert_eq!(r.events.len(), 3);
        median_of(&r, "tonal_db")
    };
    // A damped sine alone is as pure as can be told: a bound, well up.
    let clean = make(0.0);
    assert!(matches!(clean, Summary::AtLeast(v, _) if v > 12.0), "a damped sine alone: {clean:?}");
    // Noise 3 dB under the sine: (0.867^2 / 3) / 0.5 = 0.5.
    let noisy = make(0.867).median().expect("a value");
    assert!((noisy - 3.0).abs() < 1.5, "{noisy} dB with noise 3 dB under");
    // And 10 dB under.
    let less = make(0.387).median().expect("a value");
    assert!((less - 10.0).abs() < 2.0, "{less} dB with noise 10 dB under");
}

#[test]
fn harmonics_are_read_against_the_fundamental() {
    let mut x = vec![0.0f32; at(1.5)];
    for t in [0.2, 0.6, 1.0] {
        ping(&mut x, t, 900.0, 30.0, 0.4, 1.0, 0.0);
        ping(&mut x, t, 1800.0, 30.0, 0.04, 1.0, 0.0);
        ping(&mut x, t, 2700.0, 30.0, 0.0126, 1.0, 0.0);
    }
    let r = run(x);
    let e = &r.events[0];
    assert!((e.inside.start_hz - 900.0).abs() < 3.0, "{}", e.inside.start_hz);
    assert!((val(e.inside.h2_db, "h2") + 20.0).abs() < 1.5, "2nd {:?}", e.inside.h2_db);
    assert!((val(e.inside.h3_db, "h3") + 30.0).abs() < 2.0, "3rd {:?}", e.inside.h3_db);
}

/// Six pings at 1 kHz with a planted 2nd harmonic 20 dB down and noise
/// struck with them 8 dB down, over a white bed `bed_db` under the ping.
fn coloured(bed_db: f32, seed: u32) -> Report {
    let mut x = vec![0.0f32; at(6.0)];
    let mut n = Noise(99);
    for k in 0..6 {
        let t = 0.4 + 0.83 * k as f32;
        ping(&mut x, t, 1000.0, 40.0, 0.3, 2.0, 0.0);
        ping(&mut x, t, 2000.0, 40.0, 0.03, 2.0, 0.0);
        for i in 0..at(0.4) {
            let s = i as f32 / SR;
            x[at(t) + i] += 0.3 * 0.55 * (s * 500.0).min(1.0) * (-s * 25.0).exp() * n.next();
        }
    }
    // The ping's peak is 0.212 rms; the bed's rms is amp / sqrt 3.
    let amp = 0.212 * 10f32.powf(bed_db / 20.0) * 3f32.sqrt();
    hiss(&mut x, amp, seed);
    run(x)
}

#[test]
fn purity_and_harmonics_hold_as_the_bed_rises_until_they_are_withheld() {
    // Each event's noise is its own, so each is held against its own reading
    // with no bed; the same six events come back as the bed rises 76 dB.
    let quiet = coloured(-70.0, 1);
    assert_eq!(quiet.events.len(), 6);
    let h2_0: Vec<f32> = quiet.events.iter().map(|e| val(e.inside.h2_db, "h2")).collect();
    let ton_0: Vec<f32> = quiet.events.iter().map(|e| val(e.inside.tonal_db, "tonal")).collect();
    let cen_0: Vec<f32> = quiet.events.iter().map(|e| val(e.inside.centroid_hz, "centroid")).collect();
    let mv_0: Vec<f32> = quiet.events.iter().map(|e| val(e.inside.centroid_move_st, "move")).collect();
    for v in &h2_0 {
        assert!((v + 20.0).abs() < 2.5, "h2 {v} for a planted -20");
    }
    let mut last_values = usize::MAX;
    for bed_db in [-60.0, -50.0, -40.0, -30.0, -20.0, -10.0, 0.0, 6.0] {
        let r = coloured(bed_db, 2);
        if bed_db <= -20.0 {
            assert_eq!(r.events.len(), 6, "bed {bed_db}");
        }
        let mut values = 0;
        for e in &r.events {
            let Some(k) = quiet.events.iter().position(|q| (q.start_s - e.start_s).abs() < 0.03) else { continue };
            let x = &e.inside;
            match x.h2_db {
                Fig::Value(v) => {
                    assert!((v - h2_0[k]).abs() < 1.0, "bed {bed_db}: h2 {v} for {}", h2_0[k]);
                    values += 1;
                }
                Fig::AtMost(v) => assert!(v >= h2_0[k] - 1.0, "bed {bed_db}: a bound under the truth {v}"),
                Fig::AtLeast(v) => panic!("bed {bed_db}: h2 at least {v}"),
                Fig::No(_) => {}
            }
            match x.tonal_db {
                Fig::Value(v) => assert!((v - ton_0[k]).abs() < 1.0, "bed {bed_db}: tonal {v} for {}", ton_0[k]),
                Fig::AtLeast(v) => assert!(v <= ton_0[k] + 1.0, "bed {bed_db}: a bound over the truth {v}"),
                Fig::AtMost(v) => panic!("bed {bed_db}: tonal at most {v}"),
                Fig::No(_) => {}
            }
            if let Fig::Value(c) = x.centroid_hz {
                assert!((12.0 * (c / cen_0[k]).log2()).abs() < 0.5, "bed {bed_db}: centroid {c} for {}", cen_0[k]);
            }
            if let Fig::Value(m) = x.centroid_move_st {
                assert!((m - mv_0[k]).abs() < 1.0, "bed {bed_db}: the centroid moves {m} st for {}", mv_0[k]);
            }
        }
        if bed_db <= -20.0 {
            assert_eq!(values, 6, "bed {bed_db}: the 2nd harmonic stands 30 dB over the bed and is not read");
        }
        last_values = values;
    }
    assert_eq!(last_values, 0, "a bed louder than the ping, and the 2nd harmonic is still read as a number");
    // A pure ping has no harmonic to read: never a number for one, at any bed.
    for bed_db in [-90.0, -50.0, -30.0] {
        let mut x = vec![0.0f32; at(6.0)];
        for k in 0..6 {
            ping(&mut x, 0.4 + 0.83 * k as f32, 1000.0, 25.0, 0.3, 2.0, 0.0);
        }
        hiss(&mut x, 0.212 * 10f32.powf(bed_db / 20.0) * 3f32.sqrt(), 5);
        let r = run(x);
        for e in &r.events {
            assert!(
                e.inside.h2_db.value().is_none() && e.inside.h3_db.value().is_none(),
                "bed {bed_db}: {:?}",
                e.inside.h2_db
            );
            assert!(e.inside.burst_db.value().is_none(), "bed {bed_db}: burst {:?}", e.inside.burst_db);
        }
    }
    // Only the bed differs: nothing to advise.
    let (a, b) = (coloured(-60.0, 3), coloured(-30.0, 4));
    assert!(advice_of(&a, &b).is_empty(), "{:?}", advice_of(&a, &b));
}

#[test]
fn the_level_over_the_bed_and_in_the_mix() {
    // A steady tone at 1050 Hz, 0.02, in the ping's own band: a ping of 0.2 is 20 dB over it.
    let mut x: Vec<f32> = (0..at(2.0)).map(|i| 0.02 * (TAU * 1050.0 * i as f32 / SR).sin()).collect();
    for t in [0.5, 1.0, 1.5] {
        ping(&mut x, t, 1000.0, 30.0, 0.2, 1.0, 0.0);
    }
    let r = run(x);
    assert_eq!(r.events.len(), 3);
    for e in &r.events {
        assert!((e.inside.over_bed_db - 20.0).abs() < 1.5, "over the bed {}", e.inside.over_bed_db);
    }
    // In noise of 0.2 rms the same ping (0.141 rms at its peak) is 3 dB under
    // the mix; read in a band a few semitones wide, which a ping that is
    // already falling never quite fills, it comes out within 2 dB of that.
    let mut n = Noise(11);
    let mut x: Vec<f32> = (0..at(3.0)).map(|_| 0.2 * 1.732 * n.next()).collect();
    for t in [0.7, 1.5, 2.3] {
        ping(&mut x, t, 1000.0, 30.0, 0.2, 1.0, 0.0);
    }
    let r = run(x);
    assert_eq!(r.events.len(), 3, "{:?}", r.events.iter().map(|e| e.start_s).collect::<Vec<_>>());
    for e in &r.events {
        let v = val(e.inside.in_mix_db, "in the mix");
        assert!(v < -2.0 && v > -5.0, "in the mix {v}");
    }
}

#[test]
fn alone_the_mix_is_not_read_and_a_decay_change_is_advised_as_a_decay() {
    // The same ping, same level, alone: ringing 25 against 100 ms.
    let make = |tau: f32| {
        let mut x = vec![0.0f32; at(6.5)];
        for k in 0..6 {
            ping(&mut x, 0.4 + 0.83 * k as f32, 1000.0, tau, 0.3, 2.0, 0.0);
        }
        run(x)
    };
    let (a, b) = (make(25.0), make(100.0));
    for e in a.events.iter().chain(&b.events) {
        assert!(matches!(e.inside.in_mix_db, Fig::No(_)), "{:?}", e.inside.in_mix_db);
    }
    let lines = advice_of(&a, &b);
    assert!(lines.iter().any(|l| l.starts_with("decay")), "{lines:?}");
    for l in &lines {
        assert!(
            l.starts_with("decay") || l.starts_with("resonance Q") || l.starts_with("length"),
            "only the decay changed: {l}"
        );
    }
}

#[test]
fn where_an_event_sits_between_the_speakers() {
    let mut l = vec![0.0f32; at(1.5)];
    for t in [0.2, 0.6, 1.0] {
        ping(&mut l, t, 1200.0, 25.0, 0.4, 1.0, 0.0);
    }
    // Half as loud on the right: 6 dB to the left, and the same wave.
    let r: Vec<f32> = l.iter().map(|v| 0.5 * v).collect();
    let rep = measure(l.clone(), r, &Options::default(), true);
    for e in &rep.events {
        assert!((e.inside.pan_db - 6.02).abs() < 0.2, "pan {}", e.inside.pan_db);
        assert!(val(e.inside.corr, "corr") > 0.99, "correlation {:?}", e.inside.corr);
        // Side (0.25) over mid (0.75): -9.5 dB.
        assert!((val(e.inside.side_mid_db, "side") + 9.54).abs() < 0.3, "{:?}", e.inside.side_mid_db);
    }
    // Panned 3 dB left against centred: the side it has is the pan's, and
    // the advice says pan, not stereo spread.
    let centre = measure(l.clone(), l.clone(), &Options::default(), true);
    let right: Vec<f32> = l.iter().map(|v| v * 10f32.powf(-3.0 / 20.0)).collect();
    let panned = measure(l.clone(), right, &Options::default(), true);
    let lines = advice_of(&centre, &panned);
    assert!(lines.iter().any(|l| l.contains("off centre")), "{lines:?}");
    assert!(!lines.iter().any(|l| l.contains("stereo spread")), "{lines:?}");
    // The right a different ping at the same time: nothing shared.
    let mut other = vec![0.0f32; at(1.5)];
    for t in [0.2, 0.6, 1.0] {
        ping(&mut other, t, 1236.0, 25.0, 0.4, 1.0, 0.0);
    }
    let rep = measure(l.clone(), other, &Options::default(), true);
    assert!(val(rep.events[0].inside.corr, "corr").abs() < 0.5, "correlation {:?}", rep.events[0].inside.corr);
    // Hard left: no correlation to read, and side is as loud as mid.
    let rep = measure(l.clone(), vec![0.0; l.len()], &Options::default(), true);
    for e in &rep.events {
        assert!(matches!(e.inside.corr, Fig::No(_)), "{:?}", e.inside.corr);
        assert!(val(e.inside.side_mid_db, "side").abs() < 0.3, "{:?}", e.inside.side_mid_db);
    }
    // Left and right in antiphase: the sum is silence, the file is not.
    let neg: Vec<f32> = l.iter().map(|v| -v).collect();
    let rep = measure(l, neg, &Options::default(), true);
    assert_eq!(rep.events.len(), 3, "antiphase");
    assert!(rep.note.is_some_and(|n| n.contains("antiphase")));
    // All side and no mid: the width is past what a ratio can say.
    assert!(
        matches!(rep.events[0].inside.side_mid_db, Fig::AtLeast(v) if v >= 60.0),
        "{:?}",
        rep.events[0].inside.side_mid_db
    );
    // And the correlation of the two channels is -1, not "nothing to read".
    assert!(val(rep.events[0].inside.corr, "corr") < -0.99, "{:?}", rep.events[0].inside.corr);
}

fn echoed(interval: f32, gain: f32) -> Vec<f32> {
    let mut x = bed(6.0, 0.002, 0.0);
    for (k, t) in [0.3f32, 1.31, 2.07, 3.2, 4.15].iter().enumerate() {
        let mut amp = 0.4;
        for n in 0..5 {
            ping(&mut x, t + n as f32 * interval, 900.0 + 130.0 * k as f32, 12.0, amp, 0.5, 0.0);
            amp *= gain;
        }
    }
    x
}

#[test]
fn a_delay_gives_back_its_time_and_its_loss() {
    // Every 170 ms, half as loud each time: 6 dB a repeat.
    let r = run(echoed(0.170, 0.5));
    let d = r.delay.as_ref().expect("a delay");
    assert!((d.interval_ms - 170.0).abs() < 1.5, "interval {}", d.interval_ms);
    assert!((d.first_db + 6.02).abs() < 1.0, "first {}", d.first_db);
    assert!((d.per_repeat_db.unwrap() + 6.02).abs() < 1.0, "per repeat {:?}", d.per_repeat_db);
    assert!(d.ambiguous.is_none());
    // Another time and another loss read as themselves.
    let r = run(echoed(0.233, 0.708));
    let d = r.delay.as_ref().expect("a delay");
    assert!((d.interval_ms - 233.0).abs() < 1.5, "interval {}", d.interval_ms);
    assert!((d.first_db + 3.0).abs() < 1.0, "first {}", d.first_db);
    // And pings with no echo have none, however long they ring: a decay is
    // shaped like itself later on, and is no repeat.
    assert!(run(pings(18.0, 0.002, 0.0)).delay.is_none());
    for tau in [40.0, 120.0] {
        let r = run(pings(tau, 0.002, 0.0));
        assert!(r.delay.is_none(), "tau {tau}: {:?}", r.delay.as_ref().map(|d| (d.interval_ms, d.first_db)));
    }
    // What tells a repeat from the event's own decay: a dip before it. A
    // ping 50 ms on is 17 dB down and shaped like itself, and no repeat; the
    // same ping with an echo at 170 ms dips before the echo.
    for (echo, expect) in [(false, false), (true, true)] {
        let mut x = vec![0.0f32; at(2.0)];
        ping(&mut x, 0.5, 1000.0, 25.0, 0.4, 0.5, 0.0);
        if echo {
            ping(&mut x, 0.67, 1000.0, 25.0, 0.2, 0.5, 0.0);
        }
        let r = run(x.clone());
        let e = &r.events[0];
        let found = inside::Ctx {
            left: &x,
            right: &x,
            sign: 1.0,
            mono: &x,
            stft: &Stft::new(&x, 2048, 256),
            floor: &Floor::new(&Stft::new(&x, 2048, 256)),
        };
        let lag = if echo { 0.17 } else { 0.05 } * SR;
        let (a, b) = (at(0.5), at(0.5) + lag as usize);
        let level = |t: f32| 0.4 * (-t / 0.025).exp();
        let one = if echo { 0.2 } else { level(0.05) };
        let dips = inside::dips_before(&found, e, lag as f64, a, b, one, inside::cutoff_for(1000.0));
        assert_eq!(dips, expect, "echo {echo}");
    }
    // A dry snare: a 200 Hz body and a burst of wires, every 0.968 s.
    let mut x = vec![0.0f32; at(8.0)];
    let mut n = Noise(13);
    for k in 0..8 {
        let t0 = 0.48 + 0.968 * k as f32;
        ping(&mut x, t0, 200.0, 25.0, 0.4, 0.0, 0.0);
        for i in 0..at(0.3) {
            x[at(t0) + i] += 0.5 * (-(i as f32) / SR / 0.02).exp() * n.next();
        }
    }
    let r = run(x);
    assert!(r.delay.is_none(), "a dry snare: {:?}", r.delay.as_ref().map(|d| (d.interval_ms, d.first_db)));
}

#[test]
fn a_delay_on_one_pitch_on_a_grid_is_not_read_as_the_note_spacing() {
    // 127.2 BPM eighths of one note, a 353.7 ms delay (three sixteenths),
    // 6 dB a repeat: the notes and the repeats interleave.
    let s = 60.0 / 127.2 / 4.0;
    let mut x = bed(14.0, 0.002, 0.0);
    for i in (0..48).step_by(2) {
        let mut amp = 0.3;
        for n in 0..5 {
            ping(&mut x, 0.3 + i as f32 * s + n as f32 * 0.3537, 1000.0, 15.0, amp, 0.5, 0.0);
            amp *= 0.5;
        }
    }
    let r = run(x);
    if let Some(d) = &r.delay {
        assert!(d.ambiguous.is_some() || (d.interval_ms - 353.7).abs() < 2.0, "a wrong time given as a delay: {d:?}");
    }
    // On a grid of several pitches the delay is read as itself, or said to
    // be ambiguous; never its second repeat, nor the notes' own spacing.
    let phrase = |bpm: f32, steps: &[usize], pitches: &[f32]| {
        let s = 60.0 / bpm / 4.0;
        let last = 0.3 + *steps.last().unwrap() as f32 * s;
        let mut x = bed(last + 2.5, 0.002, 0.0);
        for (k, i) in steps.iter().enumerate() {
            let mut amp = 0.3;
            for n in 0..5 {
                ping(&mut x, 0.3 + *i as f32 * s + n as f32 * 0.3537, pitches[k % pitches.len()], 15.0, amp, 0.5, 0.0);
                amp *= 0.5;
            }
        }
        run(x)
    };
    let five = [900.0, 1030.0, 1160.0, 1290.0, 1420.0];
    let syncopated: Vec<usize> = (0..48).filter(|i| [0, 3, 6, 8, 11, 14].contains(&(i % 16))).collect();
    let eighths: Vec<usize> = (0..48).step_by(2).collect();
    for (bpm, steps, pitches, what) in [
        (140.0, &eighths, &five[..], "140 BPM eighths, five pitches"),
        (127.2, &syncopated, &five[..], "127.2 BPM syncopated, five pitches"),
        (127.2, &syncopated, &[1000.0][..], "127.2 BPM syncopated, one pitch"),
    ] {
        let r = phrase(bpm, steps, pitches);
        if let Some(d) = &r.delay {
            assert!(d.ambiguous.is_some() || (d.interval_ms - 353.7).abs() < 2.0, "{what}: {d:?}");
        }
    }
    // Two equal pings are not a note and its echo.
    let mut x = bed(3.0, 0.03, 0.0);
    for t in [0.4, 1.23] {
        ping(&mut x, t, 1000.0, 25.0, 0.3, 2.0, 0.0);
    }
    assert!(run(x).delay.is_none());
}

#[test]
fn a_tail_is_what_is_left_beyond_the_decay_and_a_dry_event_has_none() {
    // Dry: whatever the decay, no tail.
    for tau in [10.0, 25.0, 100.0, 400.0] {
        let gap = (tau * 0.014f32).max(0.5);
        let mut x = vec![0.0f32; at(0.3 + 4.0 * gap + 1.0)];
        for k in 0..4 {
            ping(&mut x, 0.3 + gap * k as f32, 1000.0, tau, 0.4, 0.5, 0.0);
        }
        for e in &run(x).events {
            assert!(!matches!(e.inside.tail_db, Fig::Value(_)), "tau {tau}: {:?}", e.inside.tail_db);
        }
    }
    // Dry pings four semitones apart, 600 ms apart: the next one's onset is
    // in each one's tail window, and no tail is read.
    let mut x = vec![0.0f32; at(7.5)];
    for k in 0..11 {
        ping(&mut x, 0.4 + 0.6 * k as f32, [800.0, 1000.0, 1260.0, 1587.0][k % 4], 100.0, 0.2, 2.0, 0.0);
    }
    for e in &run(x).events {
        assert!(!matches!(e.inside.tail_db, Fig::Value(_)), "at {}: {:?}", e.start_s, e.inside.tail_db);
    }
    // Wet: a tail at the ping's pitch, 20 and 30 dB down, falling over 300 ms.
    let tau = 15.0f32;
    for wet_gain in [0.1f32, 0.0316] {
        let mut x = vec![0.0f32; at(4.0)];
        let mut n = Noise(5);
        for t in [0.3f32, 1.5, 2.7] {
            ping(&mut x, t, 1000.0, tau, 0.4, 0.5, 0.0);
            let mut phase = 0.0f32;
            for i in 0..at(0.9) {
                let s = i as f32 / SR;
                phase = (phase + (1000.0 + 15.0 * n.next()) / SR).fract();
                // A reverb builds up: no step where it starts.
                let swell = (s * 100.0).min(1.0);
                x[at(t + 0.03) + i] += 0.4 * wet_gain * swell * (-s / 0.3).exp() * (TAU * phase).sin();
            }
        }
        let r = run(x);
        assert_eq!(
            r.events.len(),
            3,
            "{:?}",
            r.events.iter().map(|e| (e.start_s, e.end_s, e.inside.overlap)).collect::<Vec<_>>()
        );
        // The truth: the tail's energy from 50 to 400 ms after the ping's own
        // decay is 20 dB down, against the ping's energy up to there. Energy
        // of a e^(-t / tau): a^2 tau / 2 over all of it.
        let t20 = tau * 10f32.ln() * 0.001;
        let own = 0.16 * 0.5 * tau * 0.001 * (1.0 - 0.01);
        let w2 = 0.16 * wet_gain * wet_gain;
        let wet = |a: f32, b: f32| w2 * 0.15 * ((-(a - 0.03) / 0.15).exp() - (-(b - 0.03) / 0.15).exp());
        let truth = 10.0 * (wet(t20 + 0.05, t20 + 0.4) / own).log10();
        for e in &r.events {
            let v = val(e.inside.tail_db, "wet tail");
            assert!((v - truth).abs() < 1.5, "wet {wet_gain}: tail {v} dB for {truth}");
            // The ping's own decay is read under the tail, not the tail's.
            let rate = val(e.inside.decay_db_ms, "decay");
            assert!((rate / (8.686 / tau) - 1.0).abs() < 0.1, "wet {wet_gain}: decay {rate}");
        }
    }
}

#[test]
fn struck_again_before_it_dies_the_event_says_so() {
    // The same dry ping (Q 314) every 150 ms: its width is the window's,
    // its Q from the decay still holds.
    let mut x = vec![0.0f32; at(7.0)];
    for k in 0..40 {
        ping(&mut x, 0.4 + 0.15 * k as f32, 1000.0, 100.0, 0.2, 2.0, 0.0);
    }
    let r = run(x);
    assert!(!r.events.is_empty());
    for e in &r.events {
        assert!(e.inside.overlap.is_some(), "at {}", e.start_s);
        assert!(e.inside.q.value().is_none() && e.inside.tonal_db.value().is_none(), "{:?}", e.inside.q);
        if let Fig::Value(q) = e.inside.ring_q {
            assert!((q / 314.0 - 1.0).abs() < 0.1, "Q from the decay {q}");
        }
    }
    // A roll faster than the re-strike rule: what is found is not passed
    // off as one clean event.
    let mut x = vec![0.0f32; at(5.0)];
    for k in 0..40 {
        ping(&mut x, 0.4 + 0.08 * k as f32, 1000.0, 100.0, 0.2, 2.0, 0.0);
    }
    let r = run(x);
    for e in &r.events {
        assert!(e.inside.overlap.is_some(), "a roll read as one clean event at {}", e.start_s);
        assert!(e.inside.q.value().is_none() && e.inside.tail_db.value().is_none(), "{:?}", e.inside.q);
    }
}

#[test]
fn events_in_the_first_milliseconds_are_read_without_falling_over() {
    // In a debug build the echo search once subtracted past zero here.
    for (t0, hz) in [(0.0f32, 1000.0f32), (0.005, 1000.0), (0.010, 1000.0), (0.015, 1000.0), (0.03, 80.0)] {
        let mut x = vec![0.0f32; at(3.0)];
        for t in [t0, 1.0, 2.0] {
            ping(&mut x, t, hz, 40.0, 0.3, 0.0, 0.0);
        }
        let r = run(x);
        assert!(
            r.events.first().is_some_and(|e| e.start_s < t0 + 0.02),
            "{t0} s at {hz} Hz: {:?}",
            r.events.iter().map(|e| e.start_s).collect::<Vec<_>>()
        );
    }
}

#[test]
fn one_or_two_events_are_held_against_the_estimators_error_not_a_spread() {
    let one = |seed: u32, n: usize| {
        let mut x = vec![0.0f32; at(3.0)];
        for k in 0..n {
            ping(&mut x, 0.4 + 0.83 * k as f32, 1000.0, 25.0, 0.3, 2.0, 0.0);
        }
        hiss(&mut x, 0.03, seed);
        run(x)
    };
    let (a, b) = (one(1, 1), one(2, 1));
    assert!(advice_of(&a, &b).is_empty(), "{:?}", advice_of(&a, &b));
    let (a, b) = (one(3, 2), one(4, 2));
    assert!(advice_of(&a, &b).is_empty(), "{:?}", advice_of(&a, &b));
}

/// A held sawtooth with 50 ms fades: naive (it folds) or summed from its
/// harmonics up to 20 kHz (it does not).
fn saw(f0: f32, seconds: f32, naive: bool) -> Vec<f32> {
    let top = (20_000.0f32.min(0.45 * SR) / f0) as usize;
    (0..at(seconds))
        .map(|i| {
            let t = i as f32 / SR;
            let fade = (t * 20.0).min(1.0) * ((seconds - t) * 20.0).min(1.0);
            let v = if naive {
                2.0 * (f0 as f64 * i as f64 / SR as f64).fract() as f32 - 1.0
            } else {
                let ph = (f0 as f64 * i as f64 / SR as f64).fract() as f32;
                (1..=top).map(|k| (TAU * (ph * k as f32).fract()).sin() / k as f32).sum::<f32>() * -2.0
                    / std::f32::consts::PI
            };
            0.2 * fade * v
        })
        .collect()
}

#[test]
fn a_naive_saw_folds_and_a_band_limited_one_does_not() {
    for f0 in [55.0, 110.0, 220.0, 440.0, 1000.0, 1760.0, 3520.0] {
        let naive = run(saw(f0, 2.0, true));
        let c = &naive.clean;
        assert!(c.alias.is_some() || c.alias_unread.is_some(), "{f0} Hz naive: {:?}", c.lines());
        assert!(!c.lines().iter().any(|l| l.contains("nothing folded")), "{f0} Hz naive: {:?}", c.lines());
        assert!(c.alias.is_some(), "{f0} Hz naive: {:?}", c.lines());
        let clean = run(saw(f0, 2.0, false));
        assert!(clean.clean.alias.is_none(), "{f0} Hz band-limited: {:?}", clean.clean.alias);
    }
    // A naive square folds too; plucked naive saws, struck four times, as well.
    let square: Vec<f32> = saw(440.0, 2.0, true).iter().map(|v| if *v >= 0.0 { 0.2 } else { -0.2 }).collect();
    assert!(run(square).clean.alias.is_some(), "naive square");
    let mut plucks = vec![0.0f32; at(5.5)];
    for k in 0..4 {
        let s0 = at(0.3 + 1.25 * k as f32);
        for i in 0..at(0.8) {
            let t = i as f32 / SR;
            plucks[s0 + i] += 0.3 * (-t / 0.25).exp() * (2.0 * (440.0 * t).fract() - 1.0);
        }
    }
    let r = run(plucks);
    assert!(r.clean.alias.is_some(), "plucked naive saw: {:?}", r.clean.lines());
    // A file at another rate has been resampled on the way in: not looked at, and said.
    let x = saw(440.0, 2.0, true);
    let audio = crate::wav::Audio { rate: 48_000, left: x.clone(), right: x };
    let r = analyze(&audio, None, None, &Options::default()).unwrap();
    assert!(
        r.clean.alias.is_none() && r.clean.alias_unread.is_some_and(|w| w.contains("converted")),
        "{:?}",
        r.clean.lines()
    );
    // Five lines planted where folds land: too few to call aliasing, and
    // not "nothing folded back" either.
    let (sr, f0) = (SR as f64, SR as f64 / 42.3);
    let x: Vec<f32> = (0..at(3.0))
        .map(|i| {
            let t = i as f64 / sr;
            let fade = (t * 20.0).min(1.0) * ((3.0 - t) * 20.0).min(1.0);
            let tone = |hz: f64| (std::f64::consts::TAU * (hz * t).fract()).sin();
            let notes: f64 = (1..13).map(|k| tone(k as f64 * f0) / k as f64).sum();
            let folds: f64 = (24..29).map(|k| 0.0045 * tone(sr - k as f64 * f0)).sum();
            (0.15 * fade * (notes + folds)) as f32
        })
        .collect();
    let r = run(x);
    assert!(r.clean.alias.is_none(), "{:?}", r.clean.alias);
    assert!(
        r.clean.lines().iter().any(|l| l.contains("5 lines stand where folds would land")),
        "{:?}",
        r.clean.lines()
    );
    // Where a fold lands on the note's own harmonics nothing can be seen, and it is said.
    let r = run(saw(SR / 400.0, 2.0, true));
    assert!(
        r.clean.alias.is_some() || r.clean.alias_unread.is_some_and(|w| w.starts_with("not examined")),
        "{:?}",
        r.clean.lines()
    );
    // A pure sine has nothing to fold, even with lines planted where folds would land.
    let f0 = 1000.0f32;
    let x: Vec<f32> = (0..at(2.0))
        .map(|i| {
            let t = i as f32 / SR;
            let fade = (t * 20.0).min(1.0) * ((2.0 - t) * 20.0).min(1.0);
            let folds: f32 = (23..27).map(|k| 0.003 * (TAU * ((SR - k as f32 * f0) * t).fract()).sin()).sum();
            0.2 * fade * ((TAU * (f0 * t).fract()).sin() + folds)
        })
        .collect();
    assert!(run(x).clean.alias.is_none());
}

#[test]
fn a_steep_band_limited_edge_is_not_a_step_and_a_real_one_is() {
    for f0 in [55.0, 110.0, 220.0] {
        let r = run(saw(f0, 2.0, false));
        assert!(r.clean.steps.is_empty(), "{f0} Hz band-limited saw: {:?}", r.clean.steps);
    }
    // A naive saw's edges come round every period: they are not steps.
    assert!(run(saw(110.0, 2.0, true)).clean.steps.is_empty());
    // A jump in the middle of a band-limited saw is.
    let mut x = saw(110.0, 2.0, false);
    for v in x[at(1.3)..].iter_mut() {
        *v += 0.03;
    }
    let r = run(x);
    assert!(r.clean.steps.iter().any(|s| (s.at_s - 1.3).abs() < 0.001), "{:?}", r.clean.steps);
}

#[test]
fn a_step_dc_and_a_stepped_modulation_are_flagged_and_a_clean_signal_is_not() {
    let tone = |t: f32| 0.3 * (t * 4.0).min(1.0) * (TAU * 440.0 * t).sin();
    let clean: Vec<f32> = (0..at(3.0)).map(|i| tone(i as f32 / SR)).collect();
    let r = run(clean.clone());
    assert!(r.clean.findings().is_empty(), "{:?}", r.clean.findings());

    // The wave jumps by 0.03 at 1.7 s and stays there.
    let mut stepped = clean.clone();
    for v in stepped[at(1.7)..].iter_mut() {
        *v += 0.03;
    }
    let r = run(stepped);
    assert_eq!(r.clean.steps.len(), 1, "{:?}", r.clean.steps);
    assert!((r.clean.steps[0].at_s - 1.7).abs() < 0.001, "{:?}", r.clean.steps);
    // 0.03 against a tone of 0.3 (0.212 rms), and the interpolation misses
    // half the jump: -23 dB.
    assert!((r.clean.steps[0].db + 23.0).abs() < 3.0, "{:?}", r.clean.steps);
    // An offset that comes with the step for under half the file is the
    // step, not an offset of the render.
    assert!(!r.clean.findings().iter().any(|f| f.starts_with("DC")), "{:?}", r.clean.findings());

    // A tremolo worked out every 64 samples and held, against the same one smooth.
    let swell = |n: usize| 0.6 + 0.4 * (TAU * 3.0 * n as f32 / SR).sin();
    let smooth: Vec<f32> = clean.iter().enumerate().map(|(n, v)| v * swell(n)).collect();
    let held: Vec<f32> = clean.iter().enumerate().map(|(n, v)| v * swell(n / 64 * 64)).collect();
    assert!(run(smooth).clean.zipper.is_none());
    let z = run(held).clean.zipper.expect("a zipper");
    assert_eq!(z.period, 64);
    // The gain moves up to 0.4 * 2 pi * 3 * 64 / rate (0.011) a step, on a
    // tone of 0.3: about -40 dB under the signal's level.
    assert!(z.db < -34.0 && z.db > -48.0, "{} dB", z.db);
    // A wave whose period is exactly 64 or 128 samples is not a zipper at
    // that period, nor at half or a quarter of it.
    for period in [64.0, 128.0] {
        let z = run(saw(SR / period, 2.0, true)).clean.zipper;
        assert!(z.is_none(), "a period of {period} samples: {z:?}");
    }
}

#[test]
fn a_short_low_note_is_not_dc_and_a_real_offset_is() {
    // A 50 Hz thump in a second of file: the mean of its first half cycle is
    // not an offset in the render.
    let mut x = vec![0.0f32; at(1.0)];
    ping(&mut x, 0.1, 50.0, 60.0, 0.6, 0.0, 0.0);
    assert!(run(x).clean.dc_db < clean::DC_DB);
    let x: Vec<f32> = (0..at(2.0)).map(|i| 0.3 * (TAU * 220.0 * i as f32 / SR).sin() + 0.01).collect();
    let dc = run(x).clean.dc_db;
    assert!((dc + 26.5).abs() < 1.0, "{dc}");
    // Kicks with quiet between them on an offset of 0.003: found, at its level.
    let mut x = vec![0.003f32; at(3.0)];
    for t in [0.2, 1.2, 2.2] {
        ping(&mut x, t, 50.0, 60.0, 0.6, 0.0, 0.0);
    }
    let level = (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
    let dc = run(x).clean.dc_db;
    assert!((dc - 20.0 * (0.003 / level).log10()).abs() < 1.0, "{dc}");
}

#[test]
fn a_tone_left_in_the_quiet_is_found() {
    let mut x = vec![0.0f32; at(3.0)];
    ping(&mut x, 0.2, 800.0, 30.0, 0.5, 1.0, 0.0);
    assert!(run(x.clone()).clean.buzz.is_none());
    for (i, v) in x.iter_mut().enumerate() {
        *v += 2e-5 * (TAU * 120.0 * i as f32 / SR).sin();
    }
    let b = run(x).clean.buzz.expect("a buzz");
    assert!((b.hz - 120.0).abs() < 12.0, "{} Hz", b.hz);
}

#[test]
fn sustained_material_gives_back_its_partials_and_how_they_move() {
    // 200 Hz with a 5 Hz vibrato of 30 cents peak to peak, 500 Hz whose
    // level swings 6 dB at 3 Hz, and a pair 2 Hz apart at 900 Hz.
    let mut phase = 0.0f32;
    let x: Vec<f32> = (0..at(8.0))
        .map(|i| {
            let t = i as f32 / SR;
            phase = (phase + 200.0 * 2f32.powf(15.0 / 1200.0 * (TAU * 5.0 * t).sin()) / SR).fract();
            let swing = 10f32.powf(3.0 / 20.0 * (TAU * 3.0 * t).sin());
            0.2 * (TAU * phase).sin()
                + 0.1 * swing * (TAU * 500.0 * t).sin()
                + 0.05 * ((TAU * 900.0 * t).sin() + (TAU * 902.0 * t).sin())
        })
        .collect();
    let r = run(x);
    let s = r.sustained.as_ref().expect("sustained");
    let near = |hz: f32| {
        s.partials.iter().find(|p| (p.hz - hz).abs() < 3.0).unwrap_or_else(|| panic!("no {hz}: {:?}", s.partials))
    };
    let low = near(200.0);
    assert!((low.fm_hz.expect("a vibrato") - 5.0).abs() < 0.3, "{:?}", low);
    assert!((low.fm_cents - 30.0).abs() < 6.0 && low.am_db < 1.0, "{:?}", low);
    let mid = near(500.0);
    assert!((mid.am_hz.expect("a tremolo") - 3.0).abs() < 0.2, "{:?}", mid);
    assert!((mid.am_db - 6.0).abs() < 1.0 && mid.fm_cents < 5.0, "{:?}", mid);
    assert!((mid.db - low.db + 6.0).abs() < 2.5, "{:?} {:?}", low, mid);
    assert!(s.beats.iter().any(|b| (b - 2.0).abs() < 0.3), "{:?}", s.beats);
    assert!(s.beats.iter().any(|b| (b - 3.0).abs() < 0.3), "{:?}", s.beats);
    // The third octave around 200 Hz holds most of it.
    let top = s.thirds.iter().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
    assert!((top.0 / 200.0).log2().abs() < 0.17, "{:?}", top);
    // Partials that move spread past the few bins counted as theirs.
    assert!(s.hnr_db > 10.0, "{}", s.hnr_db);
    // The same in both channels: no side at all.
    assert!(s.width.iter().all(|w| w.1 < -60.0 || w.2 > 0.99), "{:?}", s.width);

    // Noise has no partials standing over it, and a held harmonic note names its note.
    let mut n = Noise(9);
    let hiss: Vec<f32> = (0..at(3.0)).map(|_| 0.1 * n.next()).collect();
    let r = run(hiss);
    let s = r.sustained.as_ref().expect("sustained");
    assert!(s.hnr_db < -5.0 && s.flux > 0.2, "{} {}", s.hnr_db, s.flux);
    let held: Vec<f32> = (0..at(4.0))
        .map(|i| {
            let t = i as f32 / SR;
            0.2 * (t * 2.0).min(1.0)
                * (1..=8).map(|k| (TAU * (110.0 * k as f32 * t).fract()).sin() / k as f32).sum::<f32>()
        })
        .collect();
    let r = run(held);
    let s = r.sustained.as_ref().expect("sustained");
    assert!((s.f0.expect("a note") - 110.0).abs() < 0.5, "{:?}", s.f0);
    assert!(s.flux < 0.02 && s.hnr_db > 30.0, "{} {}", s.flux, s.hnr_db);
}

#[test]
fn against_flags_what_was_changed_and_nothing_in_a_file_against_itself() {
    let a = run(pings(18.0, 0.002, 0.01));
    let same = run(pings(18.0, 0.002, 0.01));
    let rows = report::rows(&a, &same);
    assert!(
        rows.iter().all(|r| !r.flagged),
        "{:?}",
        rows.iter().filter(|r| r.flagged).map(|r| &r.key).collect::<Vec<_>>()
    );
    let text = report::against(&a, &same, "a.wav", "a.wav");
    assert!(text.contains("nothing differs by enough to matter"), "{text}");

    // The same pings ringing a third as long.
    let b = run(pings(6.0, 0.002, 0.01));
    let rows = report::rows(&a, &b);
    let decay = rows.iter().find(|r| r.key == "decay_db_ms").unwrap();
    assert!(decay.flagged && (decay.difference.unwrap() - 3.0).abs() < 0.3, "{:?}", decay.difference);
    assert!(rows.iter().find(|r| r.key == "ring_q").unwrap().flagged);
    // What was not changed is not flagged, and no line says it was.
    for key in [
        "start_hz",
        "pan_db",
        "per_second",
        "attack_ms",
        "tonal_db",
        "h2_db",
        "h3_db",
        "tail_db",
        "in_mix_db",
        "burst_db",
    ] {
        assert!(!rows.iter().find(|r| r.key == key).unwrap().flagged, "{key}");
    }
    let lines = report::advice(&a, &b, &rows, "b.wav");
    for l in &lines {
        assert!(
            ["decay", "resonance Q", "length", "the centroid moves"].iter().any(|p| l.starts_with(p)),
            "a decay change: {l}"
        );
    }
    let text = report::against(&a, &b, "a.wav", "b.wav");
    assert!(text.contains("resonance Q") && text.contains("raise the `resonance`"), "{text}");

    // And 9 dB quieter over the same bed.
    let quiet: Vec<f32> = {
        let (loud, bed) = (pings(18.0, 0.0, 0.0), bed(3.8, 0.02, 0.0));
        loud.iter().zip(&bed).map(|(p, b)| 0.355 * p + b).collect()
    };
    let loud: Vec<f32> = {
        let (loud, bed) = (pings(18.0, 0.0, 0.0), bed(3.8, 0.02, 0.0));
        loud.iter().zip(&bed).map(|(p, b)| p + b).collect()
    };
    let (a, b) = (run(loud), run(quiet));
    let rows = report::rows(&a, &b);
    let level = rows.iter().find(|r| r.key == "over_bed_db").unwrap();
    assert!(level.flagged && (level.difference.unwrap() + 9.0).abs() < 1.5, "{:?}", level.difference);
}

#[test]
fn the_centroid_advice_names_the_file_that_moves() {
    let empty = run(vec![0.0; at(1.0)]);
    let spread = |v: f32| Summary::Spread(Spread { median: v, low: v, high: v, n: 6 });
    let line = |x: f32, y: f32| {
        let rows = vec![report::Row {
            key: "centroid_move_st".into(),
            label: "centroid by -12 dB".into(),
            unit: "st",
            a: spread(x),
            b: spread(y),
            difference: Some(y - x),
            ratio: false,
            flagged: true,
            weight: 1.0,
        }];
        report::advice(&empty, &empty, &rows, "b.wav").remove(0)
    };
    // A holds, B brightens: nothing of A darkening.
    let l = line(0.25, 13.2);
    assert!(l.contains("B's events brighten") && l.contains("A's keep their brightness"), "{l}");
    assert!(l.contains("B brightens as it falls") && !l.contains("filter envelope"), "{l}");
    // Both brighten, B less: B does not "darken more", and no filter envelope is asked for.
    let l = line(6.0, 3.0);
    assert!(l.contains("B's events brighten") && l.contains("A's brighten"), "{l}");
    assert!(l.contains("both brighten, A more") && !l.contains("darker"), "{l}");
    let l = line(-3.5, 4.5);
    assert!(l.contains("B's events brighten") && l.contains("A's darken"), "{l}");
    assert!(l.contains("A darkens and B does not: a filter envelope on B"), "{l}");
    // Both darken, B more: less envelope on B, never more.
    let l = line(-2.0, -8.0);
    assert!(l.contains("both darken, B more: less filter envelope on B"), "{l}");
    let l = line(0.0, -6.0);
    assert!(l.contains("B darkens and A does not"), "{l}");
}

#[test]
fn the_text_the_json_and_the_sheet_come_out_whole() {
    let a = run(pings(18.0, 0.002, 0.01));
    let b = run(pings(6.0, 0.002, 0.01));
    let text = report::describe(&a, "a.wav", 5);
    assert!(text.contains("12 events") && text.contains("first 5 events") && text.contains("clean"), "{text}");
    for json in [report::json(&a, "a \"quoted\".wav"), report::json_against(&a, &b, "a.wav", "b.wav")] {
        let mut depth = 0i32;
        let mut in_string = false;
        let mut escaped = false;
        for c in json.chars() {
            match c {
                _ if escaped => escaped = false,
                '\\' if in_string => escaped = true,
                '"' => in_string = !in_string,
                '{' | '[' if !in_string => depth += 1,
                '}' | ']' if !in_string => depth -= 1,
                _ => {}
            }
            assert!(depth >= 0);
        }
        assert!(depth == 0 && !in_string, "{json}");
        assert!(json.contains("\"decay_db_ms\":{\"unit\":\"dB/ms\",\"median\":"), "{json}");
        // Every measure is there, with a number or the reason it has none.
        for me in report::MEASURES {
            assert!(json.contains(&format!("\"{}\":{{\"unit\"", me.key)), "{} missing", me.key);
        }
        assert!(!json.contains("NaN") && !json.contains("inf"), "{json}");
    }
    // With no events every measure still says why.
    let none = report::json(&run(vec![0.0; at(1.0)]), "s.wav");
    assert!(none.contains("\"tail_db\":{\"unit\":\"dB\",\"median\":null,\"why\":"), "{none}");
    // The same input gives the same report, to the byte.
    assert_eq!(report::json(&a, "a.wav"), report::json(&run(pings(18.0, 0.002, 0.01)), "a.wav"));
    let one = draw::sheet(&a, "a.wav", None, 6);
    let two = draw::sheet(&a, "a.wav", Some((&b, "b.wav")), 6);
    assert!(two.height > one.height && one.width == two.width && one.width > 6 * 220);
}

#[test]
fn a_dense_mix_is_read_without_falling_over() {
    // Pings, glides, hiss and a pad on top of each other, at every threshold:
    // whatever is found, every number is a number.
    let mut x = bed(12.0, 0.03, 0.08);
    let mut n = Noise(21);
    let mut t = 0.1f32;
    while t < 11.0 {
        let hz = 300.0 + 1500.0 * (0.5 + 0.5 * n.next());
        ping(
            &mut x,
            t,
            hz,
            4.0 + 60.0 * (0.5 + 0.5 * n.next()),
            0.05 + 0.2 * (0.5 + 0.5 * n.next()),
            8.0 * n.next().abs(),
            80.0 * n.next(),
        );
        t += 0.02 + 0.15 * n.next().abs();
    }
    for threshold_db in [6.0, 12.0, 24.0] {
        let right: Vec<f32> = x.iter().rev().copied().collect();
        let opts = Options { threshold_db, sustained: true, ..Options::default() };
        let r = measure(x.clone(), right, &opts, true);
        assert!(r.events.len() > 10, "{} events at {threshold_db}", r.events.len());
        for e in &r.events {
            assert!(e.end_s > e.start_s && e.inside.direct_end_s >= e.start_s - 0.05, "{} {}", e.start_s, e.end_s);
            for me in report::MEASURES {
                assert!((me.get)(e).any().is_none_or(f32::is_finite), "{} at {}", me.key, e.start_s);
            }
            // A centroid is somewhere in the spectrum; a decay is a line.
            assert!(e.inside.centroid_hz.any().is_none_or(|c| c > 0.0 && c < SR / 2.0), "{:?}", e.inside.centroid_hz);
            if e.inside.decay_db_ms.value().is_some() {
                assert!(e.inside.decay_r2.value().is_some_and(|r| r >= 0.5), "{:?}", e.inside.decay_r2);
            }
        }
        // In a mix this dense, events overlap: what needs one alone is not read.
        assert!(r.events.iter().any(|e| e.inside.overlap.is_some()));
        let text = report::against(&r, &r, "a", "a");
        assert!(text.contains("nothing differs"), "{text}");
        let _ = draw::sheet(&r, "a", Some((&r, "a")), 4);
    }
}

#[test]
fn a_long_glide_with_a_vibrato_is_one_event_with_its_vibrato() {
    // Two and a half seconds falling an octave a second, with 20 Hz of
    // vibrato 80 cents wide: it arrives in new bins all the time, and
    // nothing starts but once.
    let mut x = vec![0.0f32; at(4.0)];
    let mut phase = 0.0f32;
    for i in 0..at(2.5) {
        let t = i as f32 / SR;
        let cents = -1200.0 * t + 40.0 * (TAU * 20.0 * t).sin();
        phase = (phase + 3000.0 * 2f32.powf(cents / 1200.0) / SR).fract();
        x[at(0.5) + i] = 0.3 * (t * 100.0).min(1.0) * ((2.5 - t) * 4.0).min(1.0) * (TAU * phase).sin();
    }
    let r = run(x);
    assert_eq!(r.events.len(), 1, "{:?}", r.events.iter().map(|e| (e.start_s, e.end_s)).collect::<Vec<_>>());
    let e = &r.events[0];
    assert!((e.start_s - 0.5).abs() < 0.01 && e.end_s - e.start_s > 2.2, "{}..{}", e.start_s, e.end_s);
    let (rate, depth) = e.inside.wobble.expect("a vibrato");
    assert!((rate - 20.0).abs() < 0.5 && (depth - 80.0).abs() < 15.0, "{rate} Hz, {depth} cents");
    assert!((e.inside.bend_st + 28.0).abs() < 3.0, "bend {}", e.inside.bend_st);
    assert_eq!(e.inside.shape, inside::Shape::Straight);
}

#[test]
fn a_band_under_15_hz_is_refused() {
    let audio = crate::wav::Audio { rate: SR as u32, left: vec![0.0; at(1.0)], right: vec![0.0; at(1.0)] };
    let opts = Options { band: Some((10.0, 11.0)), ..Options::default() };
    assert!(analyze(&audio, None, None, &opts).is_err());
}
