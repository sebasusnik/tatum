//! Modulation ports inside an instrument graph, and the `svf` filter: what an
//! Oberheim does with its cross-mod, sync, PWM and SEM filter.
//!
//! `src > a.fm` lands `src` on one of `a`'s knobs instead of its input. Each
//! test drives a port with something whose value it knows -- an envelope held
//! at its sustain of 1, scaled by a `gain` -- and measures what came out.

use tatum_core::dsl;
use tatum_core::graph::node::port;
use tatum_core::graph::voice::Instrument;
use tatum_core::graph::GraphTemplate;
use tatum_core::primitives::filter::SvfFilter;
use tatum_core::{BLOCK_SIZE, SAMPLE_RATE};

fn song(body: &str) -> String {
    format!(
        "tempo 120\n\
         instrument i {{\n{body}\n}}\n\
         pattern p {{ C3 }}\n\
         track t {{ play p using i out > master }}\n"
    )
}

fn graph(body: &str) -> GraphTemplate {
    let src = song(body);
    let ast = dsl::parse(&src).unwrap_or_else(|e| panic!("{e:?}"));
    let song = dsl::compiler::compile(&ast).unwrap_or_else(|e| panic!("{e:?}"));
    song.instruments[0].as_graph().expect("an instrument graph").clone()
}

fn error(body: &str) -> String {
    let src = song(body);
    match dsl::parse(&src) {
        Err(e) => format!("{e:?}"),
        Ok(ast) => match dsl::compiler::compile(&ast) {
            Ok(_) => panic!("compiled:\n{body}"),
            Err(e) => format!("{e:?}"),
        },
    }
}

fn play(template: GraphTemplate, note: u8, samples: usize) -> Vec<f32> {
    let mut inst = Instrument::new(template);
    inst.note_on(note, 1.0);
    let mut out = Vec::with_capacity(samples);
    let mut block = [0.0f32; BLOCK_SIZE];
    while out.len() < samples {
        inst.process_block(&mut block);
        out.extend_from_slice(&block);
    }
    out.truncate(samples);
    out
}

/// Frequency from the upward zero crossings, in Hz.
fn pitch(buf: &[f32]) -> f32 {
    let ups: Vec<usize> = (1..buf.len()).filter(|&i| buf[i - 1] < 0.0 && buf[i] >= 0.0).collect();
    assert!(ups.len() >= 2, "no full cycle in the window");
    (ups.len() - 1) as f32 * SAMPLE_RATE / (ups[ups.len() - 1] - ups[0]) as f32
}

/// Energy at one frequency, over the whole buffer, with a Hann window.
fn energy_at(buf: &[f32], hz: f32) -> f32 {
    let n = buf.len() as f32;
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for (i, &x) in buf.iter().enumerate() {
        let w = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n).cos();
        let ph = std::f64::consts::TAU * hz as f64 * i as f64 / SAMPLE_RATE as f64;
        re += (x * w) as f64 * ph.cos();
        im += (x * w) as f64 * ph.sin();
    }
    (re * re + im * im) as f32
}

/// An envelope held at 1 once its 1 ms attack is over: a known constant to
/// drive a port with.
const ONE: &str = "  adsr(0.001, 10, 1.0, 0.1) as one";
const AMP: &str = "  src > adsr(0.001, 0.1, 1.0, 0.1) as amp\n  amp > out";

#[test]
fn a_port_is_written_with_a_dot_or_a_space() {
    for wiring in ["one > gain(1) > src.pitch", "one > gain(1) > src pitch"] {
        let t = graph(&format!("  osc sine(55) as src\n{ONE}\n  {wiring}\n{AMP}"));
        let masks: Vec<u8> = t.mod_masks[..t.node_count as usize].iter().copied().filter(|&m| m != 0).collect();
        assert_eq!(masks, vec![port::bit(port::PITCH)], "{wiring}");
    }
}

#[test]
fn a_graph_with_no_ports_has_no_masks() {
    let t = graph(&format!("  osc saw(55) as src\n{AMP}"));
    assert!(t.mod_masks.iter().all(|&m| m == 0));
}

/// An octave on the `pitch` port is twice the note. The envelope it comes
/// from is a control source because nothing flows into it.
#[test]
fn pitch_moves_the_note_by_octaves() {
    let plain = play(graph(&format!("  osc sine(55) as src\n{AMP}")), 48, 22050);
    let up = play(graph(&format!("  osc sine(55) as src\n{ONE}\n  one > gain(1) > src.pitch\n{AMP}")), 48, 22050);
    let (p0, p1) = (pitch(&plain[2000..]), pitch(&up[2000..]));
    assert!((p1 / p0 - 2.0).abs() < 0.01, "{p0} Hz to {p1} Hz");
}

/// Linear FM adds Hz in proportion to the carrier: half a carrier up is 1.5
/// times the note.
#[test]
fn fm_adds_a_share_of_the_carrier() {
    let plain = play(graph(&format!("  osc sine(55) as src\n{AMP}")), 48, 22050);
    let fm = play(graph(&format!("  osc sine(55) as src\n{ONE}\n  one > gain(0.5) > src.fm\n{AMP}")), 48, 22050);
    let (p0, p1) = (pitch(&plain[2000..]), pitch(&fm[2000..]));
    assert!((p1 / p0 - 1.5).abs() < 0.01, "{p0} Hz to {p1} Hz");
}

/// An oscillator into another's `fm` puts sidebands either side of the
/// carrier: at an octave above, the first lands at three times the note.
#[test]
fn an_oscillator_into_fm_makes_sidebands() {
    let body = |wire: &str| format!("  osc sine(55) as src\n  osc sine(55, pitch=12) as m\n{wire}\n{AMP}");
    let plain = play(graph(&body("")), 45, 44100);
    let fm = play(graph(&body("  m > gain(0.8) > src.fm")), 45, 44100);
    let side = 3.0 * 110.0;
    assert!(energy_at(&fm[4410..], side) > 1000.0 * energy_at(&plain[4410..], side), "no sideband at {side} Hz");
}

/// Hard sync: whatever the slave's own pitch, it repeats at the master's
/// period. 100 Hz is 441 samples, give or take the master's analog drift,
/// which moves the reset by a fraction of a sample: the share of samples
/// that come back a period later is what is measured.
#[test]
fn sync_locks_the_slave_to_the_master() {
    let body = |wire: &str| format!("  fixosc saw(100) as m\n  osc saw(55, pitch=7) as src\n{wire}\n{AMP}");
    let synced = play(graph(&body("  m > src.sync")), 48, 44100);
    let free = play(graph(&body("")), 48, 44100);
    let repeats = |b: &[f32]| (10_000..30_000).filter(|&i| (b[i] - b[i + 441]).abs() < 0.05).count() as f32 / 20_000.0;
    assert!(repeats(&synced) > 0.9, "synced: {} of it repeats", repeats(&synced));
    assert!(repeats(&free) < 0.3, "free: {} of it repeats", repeats(&free));
}

/// The jump at each reset is smoothed. A 1 kHz master over a slave a fifth
/// above C6 aliases hard if it is not: left as a bare jump, what lands off
/// the master's harmonics reads -67 dB against what lands on them. Smoothed
/// on both sides it reads -90.
#[test]
fn sync_does_not_alias() {
    let out = play(
        graph(&format!("  fixosc saw(1000) as m\n  osc saw(55, pitch=7) as src\n  m > src.sync\n{AMP}")),
        84,
        44100,
    );
    let w = &out[4410..];
    let on: f32 = (1..20).map(|k| energy_at(w, k as f32 * 1000.0)).sum();
    let off: f32 = (1..20).map(|k| energy_at(w, (k as f32 + 0.5) * 1000.0)).sum();
    let db = 10.0 * (off / on).log10();
    assert!(db < -80.0, "off-harmonic energy {db:.1} dB");
}

/// PWM: 0.3 on the `width` port of a half-width pulse is a 0.8 duty cycle.
#[test]
fn width_moves_the_duty_cycle() {
    let out = play(
        graph(&format!("  osc pulse(55, width=0.5) as src\n{ONE}\n  one > gain(0.3) > src.width\n{AMP}")),
        36,
        44100,
    );
    let w = &out[22050..];
    let mean = w.iter().sum::<f32>() / w.len() as f32;
    let high = w.iter().filter(|&&x| x > mean).count() as f32 / w.len() as f32;
    assert!((high - 0.8).abs() < 0.02, "high {high} of the time");
}

/// Three octaves on a lowpass's `cutoff` port open it from 200 Hz to 1600.
#[test]
fn cutoff_moves_the_filter_by_octaves() {
    let body = |wire: &str| format!("  osc saw(55) as src0\n{ONE}\n  src0 > lowpass(200, 0.01) as src\n{wire}\n{AMP}");
    let shut = play(graph(&body("")), 36, 44100);
    let open = play(graph(&body("  one > gain(3) > src.cutoff")), 36, 44100);
    // The 18th harmonic of C2, about 1180 Hz.
    let at = 18.0 * 65.406;
    assert!(energy_at(&open[4410..], at) > 100.0 * energy_at(&shut[4410..], at), "nothing opened at {at} Hz");
}

/// An oscillator sweeping a resonant `svf` at audio rate stays a filter.
#[test]
fn svf_takes_audio_rate_cutoff_without_blowing_up() {
    let out = play(
        graph(
            "  osc saw(55) as a\n  osc sine(55, pitch=7) as m\n  a > svf(800, 0.9) as f\n  m > gain(4) > f.cutoff\n  f > adsr(0.001, 0.1, 1.0, 0.1) as amp\n  amp > out",
        ),
        36,
        88200,
    );
    assert!(out.iter().all(|x| x.is_finite()));
    let peak = out.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    assert!(peak < 8.0, "peak {peak}");
}

fn svf_gain(morph: Option<f32>, hz: f32) -> f32 {
    let mut f = SvfFilter::new(SAMPLE_RATE);
    f.set_params(1000.0, 0.01);
    let mut peak = 0.0f32;
    for i in 0..44100 {
        let x = (std::f32::consts::TAU * hz * i as f32 / SAMPLE_RATE).sin();
        let o = f.process(x);
        let y = match morph {
            Some(m) => SvfFilter::morph(o, m),
            None => f.band(o),
        };
        if i > 22050 {
            peak = peak.max(y.abs());
        }
    }
    20.0 * peak.log10()
}

/// The SEM's mode knob: lowpass, a notch at the cutoff with both sides at
/// unity, highpass; and the bandpass at unity on its peak.
#[test]
fn svf_morphs_from_lowpass_through_notch_to_highpass() {
    assert!(svf_gain(Some(0.0), 100.0).abs() < 0.5);
    assert!(svf_gain(Some(0.0), 10_000.0) < -35.0);
    assert!(svf_gain(Some(1.0), 10_000.0).abs() < 0.5);
    assert!(svf_gain(Some(1.0), 100.0) < -35.0);
    assert!(svf_gain(Some(0.5), 1000.0) < -30.0, "notch {}", svf_gain(Some(0.5), 1000.0));
    assert!(svf_gain(Some(0.5), 50.0).abs() < 0.5, "under the notch {}", svf_gain(Some(0.5), 50.0));
    assert!(svf_gain(Some(0.5), 15_000.0).abs() < 0.5, "over the notch {}", svf_gain(Some(0.5), 15_000.0));
    assert!(svf_gain(None, 1000.0).abs() < 0.5, "bandpass peak {}", svf_gain(None, 1000.0));
}

#[test]
fn svf_sits_in_a_chain_too() {
    let src = "tempo 120\n\
        instrument i {\n  osc saw(55) as a\n  a > adsr(0.001, 0.1, 1.0, 0.1) as amp\n  amp > out\n}\n\
        pattern p { C3 }\n\
        track t { play p using i out > svf(900, 0.2, morph=0.5) > svf(3000, 0.3, bandpass) > master }\n";
    let ast = dsl::parse(src).unwrap_or_else(|e| panic!("{e:?}"));
    dsl::compiler::compile(&ast).unwrap_or_else(|e| panic!("{e:?}"));
}

#[test]
fn ports_are_checked_against_the_node() {
    let e = error(&format!("  osc saw(55) as src\n{ONE}\n  one > src.width\n{AMP}"));
    assert!(e.contains("is not a pulse"), "{e}");
    let e = error(&format!("  osc saw(55) as a\n  a > gain(0.5) as src\n{ONE}\n  one > src.cutoff\n{AMP}"));
    assert!(e.contains("is a gain, which has no ports"), "{e}");
    let e = error(&format!("  osc saw(55) as a\n  a > ladder(400) as src\n{ONE}\n  one > src.fm\n{AMP}"));
    assert!(e.contains("has no `fm` port (it has cutoff)"), "{e}");
    let e = error(&format!("  osc saw(55) as a\n  a > svf(400, bandpass) as src\n{ONE}\n  one > src.morph\n{AMP}"));
    assert!(e.contains("is the bandpass"), "{e}");
    let e = error(&format!("  osc saw(55) as src\n{ONE}\n  one > src.fm > out\n{AMP}"));
    assert!(e.contains("a port ends the chain"), "{e}");
    let e = error(&format!("  osc saw(55) as src\n  src.fm\n{AMP}"));
    assert!(e.contains("wire something into it"), "{e}");
}

/// `auto master morph` turns an `svf` on the master from lowpass to highpass:
/// a bass under its 3 kHz cutoff passes at the start and is gone at the end,
/// where it sits as quiet as one written at `morph=1`.
#[test]
fn a_morph_lane_turns_the_master_svf() {
    const RIG: &str = "tempo 120\n\
        module bass low { cutoff 0.5 }\n\
        pattern line { 1.1 - 1.3 - 1.5 - 1.3 - 1.1 - 1.3 - 1.5 - 1.3 - }\n\
        track bass { play line using low level 0.6 out > master }\n\
        master { in > svf(3000, 0.01, morph=0) > out }\n";
    let render = |src: &str| {
        let mut e = tatum_core::song_engine::SongEngine::from_source(src).unwrap_or_else(|e| panic!("{e}"));
        e.set_output_gain(1.0);
        e.start();
        e.render_steps(16 * 4).0
    };
    let bar = (SAMPLE_RATE * 2.0) as usize;
    let rms = |b: &[f32], n: usize| {
        let w = &b[n * bar..(n + 1) * bar];
        (w.iter().map(|x| x * x).sum::<f32>() / w.len() as f32).sqrt()
    };
    let swept = render(&format!("{RIG}auto master morph 0 > 1 over 2\n"));
    let high = render(&RIG.replace("morph=0", "morph=1"));
    let (open, shut, there) = (rms(&swept, 0), rms(&swept, 3), rms(&high, 3));
    assert!(open > 4.0 * shut, "the bass went from {open} to only {shut}");
    assert!((shut / there - 1.0).abs() < 0.2, "{shut} where the lane ends against {there} written");
}
