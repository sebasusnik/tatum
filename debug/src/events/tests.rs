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

fn bed(seconds: f32, noise: f32, pad: f32) -> Vec<f32> {
    let mut n = Noise(7);
    (0..at(seconds))
        .map(|i| {
            let t = i as f32 / SR;
            noise * n.next() + pad * ((TAU * 220.0 * t).sin() + (TAU * 331.0 * t).sin() + (TAU * 443.0 * t).sin())
        })
        .collect()
}

fn run(x: Vec<f32>) -> Report {
    measure(x.clone(), x, &Options::default(), true)
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
        assert!((i.attack_ms - 4.0).abs() < 1.0, "attack {}", i.attack_ms);
        let rate = i.decay_db_ms.expect("a decay");
        assert!((rate / 0.347 - 1.0).abs() < 0.05, "decay {rate} dB/ms");
        assert!(i.decay_r2.unwrap() > 0.99, "an exponential is straight in dB: {:?}", i.decay_r2);
        assert!((i.t20_ms.unwrap() - 57.6).abs() < 4.0, "-20 dB at {:?}", i.t20_ms);
        assert!((i.start_hz - 1000.0).abs() < 2.0 && i.bend_st.abs() < 0.05, "{} Hz, bend {}", i.start_hz, i.bend_st);
        assert_eq!(i.shape, inside::Shape::Flat);
        assert!((i.bandwidth_hz / 12.73 - 1.0).abs() < 0.2, "width {} Hz", i.bandwidth_hz);
        assert!((i.q / 78.5 - 1.0).abs() < 0.2, "Q {}", i.q);
        assert!((i.ring_q.unwrap() / 78.5 - 1.0).abs() < 0.06, "Q from the decay {:?}", i.ring_q);
    }
    // Struck with no rise at all, it reads under a millisecond.
    let mut x = vec![0.0f32; at(1.0)];
    ping(&mut x, 0.3, 1000.0, 25.0, 0.5, 0.0, 0.0);
    let r = run(x);
    assert!(r.events[0].inside.attack_ms < 1.0, "attack {}", r.events[0].inside.attack_ms);
    // And a quarter of the time constant is four times the decay rate.
    let mut x = vec![0.0f32; at(1.0)];
    ping(&mut x, 0.3, 1000.0, 6.25, 0.5, 0.0, 0.0);
    let rate = run(x).events[0].inside.decay_db_ms.unwrap();
    assert!((rate / 1.39 - 1.0).abs() < 0.08, "decay {rate} dB/ms");
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
    assert!(e.inside.bandwidth_hz < 16.0, "width {}", e.inside.bandwidth_hz);
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
        spread(r.events.iter().map(|e| e.inside.tonal_db)).unwrap().median
    };
    let clean = make(0.0);
    assert!(clean > 12.0, "a damped sine alone: {clean} dB");
    // Noise 3 dB under the sine: (0.867^2 / 3) / 0.5 = 0.5.
    let noisy = make(0.867);
    assert!((noisy - 3.0).abs() < 1.5, "{noisy} dB with noise 3 dB under");
    // And 10 dB under.
    let less = make(0.387);
    assert!((less - 10.0).abs() < 2.5 && less < clean, "{less} dB with noise 10 dB under");
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
    assert!((e.inside.h2_db.unwrap() + 20.0).abs() < 1.5, "2nd {:?}", e.inside.h2_db);
    assert!((e.inside.h3_db.unwrap() + 30.0).abs() < 2.0, "3rd {:?}", e.inside.h3_db);
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
        assert!(e.inside.in_mix_db < -2.0 && e.inside.in_mix_db > -5.0, "in the mix {}", e.inside.in_mix_db);
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
        assert!(e.inside.corr > 0.99, "correlation {}", e.inside.corr);
    }
    // The right a different ping at the same time: nothing shared.
    let mut other = vec![0.0f32; at(1.5)];
    for t in [0.2, 0.6, 1.0] {
        ping(&mut other, t, 1236.0, 25.0, 0.4, 1.0, 0.0);
    }
    let rep = measure(l, other, &Options::default(), true);
    assert!(rep.events[0].inside.corr.abs() < 0.5, "correlation {}", rep.events[0].inside.corr);
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
    // Another time and another loss read as themselves.
    let r = run(echoed(0.233, 0.708));
    let d = r.delay.as_ref().expect("a delay");
    assert!((d.interval_ms - 233.0).abs() < 1.5, "interval {}", d.interval_ms);
    assert!((d.first_db + 3.0).abs() < 1.0, "first {}", d.first_db);
    // And pings with no echo have none.
    assert!(run(pings(18.0, 0.002, 0.0)).delay.is_none());
}

#[test]
fn a_tail_after_the_events_is_read_and_a_dry_event_has_none() {
    let make = |wet: f32| {
        let mut x = vec![0.0f32; at(4.0)];
        let mut n = Noise(5);
        for t in [0.3f32, 1.5, 2.7] {
            ping(&mut x, t, 1000.0, 15.0, 0.4, 0.5, 0.0);
            // A tail at the ping's own pitch, wandering in phase, falling over 300 ms.
            let mut phase = 0.0f32;
            for i in 0..at(0.9) {
                let s = i as f32 / SR;
                phase = (phase + (1000.0 + 15.0 * n.next()) / SR).fract();
                x[at(t + 0.03) + i] += 0.4 * wet * (-s / 0.3).exp() * (TAU * phase).sin();
            }
        }
        let r = run(x);
        spread(r.events.iter().filter_map(|e| e.inside.tail_db)).expect("tails").median
    };
    let dry = make(0.0);
    assert!(dry < -30.0, "dry {dry} dB");
    let wet = make(0.1);
    assert!(wet > -12.0 && wet < 0.0, "a tail a tenth as loud, held twenty times longer: {wet} dB");
}

/// A held note at `f0` with `harmonics` partials at 1/k, a second long fade
/// in, and whatever `also` adds per sample.
fn held(seconds: f32, f0: f32, harmonics: usize, also: impl Fn(f32) -> f32) -> Vec<f32> {
    (0..at(seconds))
        .map(|i| {
            let t = i as f32 / SR;
            let tone: f32 = (1..=harmonics).map(|k| (TAU * (f0 * k as f32 * t).fract()).sin() / k as f32).sum();
            0.2 * (t * 2.0).min(1.0) * (tone + also(t))
        })
        .collect()
}

#[test]
fn partials_folded_back_are_found_at_their_level_and_a_clean_note_has_none() {
    let f0 = SR / 42.3;
    let harmonics = 12;
    // Five harmonics from above Nyquist, folded: each 47 dB under the
    // fundamental, together 40 dB under it.
    let first = (SR / 2.0 / f0) as usize + 3;
    let folded = |t: f32| -> f32 {
        (first..first + 5).map(|k| 10f32.powf(-47.0 / 20.0) * (TAU * ((SR - k as f32 * f0) * t).fract()).sin()).sum()
    };
    let r = run(held(4.0, f0, harmonics, folded));
    let a = r.clean.alias.as_ref().expect("aliasing");
    assert!((a.db + 40.0).abs() < 3.0, "{} dB", a.db);
    assert!((first..first + 5).contains(&a.worst.0), "harmonic {}", a.worst.0);
    assert!((a.f0 / f0 - 1.0).abs() < 0.01, "f0 {}", a.f0);
    let clean = run(held(4.0, f0, harmonics, |_| 0.0));
    assert!(clean.clean.alias.is_none(), "{:?}", clean.clean.alias);
    assert!(clean.clean.findings().is_empty(), "{:?}", clean.clean.findings());
    // A file converted from another rate is not looked at for folds.
    let x = held(4.0, f0, harmonics, folded);
    let other = measure(x.clone(), x, &Options::default(), false);
    assert!(other.clean.alias.is_none() && other.clean.alias_unread.is_some());
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
    // 0.03 against a tone of 0.3 (0.212 rms): -17 dB.
    assert!((r.clean.steps[0].db + 17.0).abs() < 3.0, "{:?}", r.clean.steps);
    assert!(r.clean.dc_db > -40.0 && r.clean.findings().iter().any(|f| f.starts_with("DC")));

    // A tremolo worked out every 64 samples and held, against the same one smooth.
    let swell = |n: usize| 0.6 + 0.4 * (TAU * 3.0 * n as f32 / SR).sin();
    let smooth: Vec<f32> = clean.iter().enumerate().map(|(n, v)| v * swell(n)).collect();
    let held: Vec<f32> = clean.iter().enumerate().map(|(n, v)| v * swell(n / 64 * 64)).collect();
    assert!(run(smooth).clean.zipper.is_none());
    let z = run(held).clean.zipper.expect("a zipper");
    assert_eq!(z.period, 64);
    assert!(z.db < -30.0 && z.db > -80.0, "{} dB", z.db);
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
    let r = run(held(4.0, 110.0, 8, |_| 0.0));
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
    assert!(decay.flagged && (decay.difference.unwrap() - 3.0).abs() < 0.4, "{:?}", decay.difference);
    assert!(rows.iter().find(|r| r.key == "ring_q").unwrap().flagged);
    // What was not changed is not flagged.
    for key in ["start_hz", "pan_db", "per_second", "attack_ms"] {
        assert!(!rows.iter().find(|r| r.key == key).unwrap().flagged, "{key}");
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
        assert!(!json.contains("NaN") && !json.contains("inf"), "{json}");
    }
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
        assert!(threshold_db > 12.0 || r.events.len() > 10, "{} events at {threshold_db}", r.events.len());
        for e in &r.events {
            assert!(e.end_s > e.start_s && e.inside.direct_end_s >= e.start_s - 0.05, "{} {}", e.start_s, e.end_s);
            for me in report::MEASURES {
                assert!((me.get)(e).is_none_or(f32::is_finite), "{} at {}", me.key, e.start_s);
            }
        }
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
