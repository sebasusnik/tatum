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
/// period. The master is tuned so that its period is a whole number of
/// samples at whatever rate the engine is built for (441 at 44.1 kHz and 320
/// at 32 kHz, both 100 Hz; 221 at 22.05 kHz, 99.8 Hz), give or take its
/// analog drift, which moves the reset by a fraction of a sample: the share
/// of samples that come back a period later is what is measured. The period
/// was written as 441 samples whatever the rate, which at 32 kHz is 1.38
/// periods: a locked slave read as not repeating at all.
#[test]
fn sync_locks_the_slave_to_the_master() {
    let period = (SAMPLE_RATE / 100.0).round() as usize;
    let hz = SAMPLE_RATE / period as f32;
    let body = |wire: &str| format!("  fixosc saw({hz}) as m\n  osc saw(55, pitch=7) as src\n{wire}\n{AMP}");
    let synced = play(graph(&body("  m > src.sync")), 48, 44100);
    let free = play(graph(&body("")), 48, 44100);
    let repeats = |b: &[f32], lag: usize| {
        (10_000..30_000).filter(|&i| (b[i] - b[i + lag]).abs() < 0.05).count() as f32 / 20_000.0
    };
    assert!(repeats(&synced, period) > 0.9, "synced: {} of it repeats", repeats(&synced, period));
    assert!(repeats(&free, period) < 0.3, "free: {} of it repeats", repeats(&free, period));
    // And at the master's period only: a lag that is no multiple of it does
    // not line up, so the measure can tell a locked slave from an unlocked one.
    let off = period * 11 / 8;
    assert!(repeats(&synced, off) < 0.3, "synced, {off} samples on: {} of it repeats", repeats(&synced, off));
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

/// Three octaves on the `cutoff` port open a filter from 200 Hz to 1600, on
/// both filters that have one.
#[test]
fn cutoff_moves_the_filter_by_octaves() {
    for filter in ["svf(200, 0.01)", "ladder(200, 0.01)"] {
        let body = |wire: &str| format!("  osc saw(55) as src0\n{ONE}\n  src0 > {filter} as src\n{wire}\n{AMP}");
        let shut = play(graph(&body("")), 36, 44100);
        let open = play(graph(&body("  one > gain(3) > src.cutoff")), 36, 44100);
        // The 18th harmonic of C2, about 1180 Hz.
        let at = 18.0 * 65.406;
        assert!(
            energy_at(&open[4410..], at) > 100.0 * energy_at(&shut[4410..], at),
            "{filter}: nothing opened at {at} Hz"
        );
    }
}

/// A biquad retuned every sample runs off to infinity (noise two octaves deep
/// into a `lowpass(800, 0.5)` did within 20 samples), so the three of them
/// have no `cutoff` port: wiring into one is an error on that line, which
/// names the filters that do.
#[test]
fn a_biquad_has_no_cutoff_port() {
    for kind in ["lowpass", "highpass", "bandpass"] {
        let src = song(&format!(
            "  osc saw(55) as a\n  noise as m\n  a > {kind}(800, 0.5) as src\n  m > gain(2) > src.cutoff\n{AMP}"
        ));
        let ast = dsl::parse(&src).unwrap_or_else(|e| panic!("{e:?}"));
        let errors = match dsl::compiler::compile(&ast) {
            Ok(_) => panic!("{kind} took a cutoff port"),
            Err(e) => e,
        };
        let e = &errors[0];
        assert!(e.message.contains("`svf`") && e.message.contains("`ladder`"), "{}", e.message);
        assert!(e.message.contains(kind), "{}", e.message);
        // `tempo`, `instrument`, then the fourth line of the body.
        assert_eq!(e.line, 6, "{}", e.message);
    }
}

/// The filters that keep the port stay filters under the worst that can be
/// wired into it: full-scale noise and a fast oscillator, eight octaves either
/// way, at the top of the resonance.
#[test]
fn every_cutoff_port_stays_bounded_at_audio_rate() {
    for filter in
        ["svf(800, 1.0)", "svf(800, 0.9, bandpass)", "svf(18000, 1.0)", "ladder(800, 1.0)", "ladder(18000, 0.9)"]
    {
        for modulator in ["noise as m", "osc saw(55, pitch=48) as m", "osc sine(55, pitch=7) as m"] {
            let out = play(
                graph(&format!(
                    "  osc saw(55) as a\n  {modulator}\n  a > {filter} as src\n  m > gain(8) > src.cutoff\n{AMP}"
                )),
                36,
                44100,
            );
            assert!(out.iter().all(|x| x.is_finite()), "{filter} under {modulator} left the numbers");
            let peak = out.iter().fold(0.0f32, |m, x| m.max(x.abs()));
            assert!(peak < 8.0, "{filter} under {modulator}: peak {peak}");
        }
    }
}

/// Whatever goes wrong inside a voice, nothing that is not a number comes out
/// of the instrument: the voice is dropped, and the next note plays clean.
/// A NaN let out would settle in the track's inserts, the sends and the
/// master, and silence the song for good.
#[test]
fn a_voice_that_leaves_the_numbers_is_dropped() {
    use tatum_core::graph::node::NodeSpec;
    let template = graph(&format!("  osc saw(55) as a\n  a > gain(1) as src\n{AMP}"));
    let gain = (0..template.node_count as usize)
        .find(|&i| matches!(template.specs[i], NodeSpec::Gain { .. }))
        .expect("the gain");
    for poison in [f32::NAN, f32::INFINITY] {
        let mut broken = template.clone();
        broken.specs[gain] = NodeSpec::Gain { amount: poison };
        let mut inst = Instrument::new(broken);
        inst.note_on(48, 1.0);
        inst.note_on(55, 0.8);
        let mut block = [0.0f32; BLOCK_SIZE];
        for _ in 0..8 {
            inst.process_block(&mut block);
            assert!(block.iter().all(|&x| x == 0.0), "{poison} got out: {:?}", &block[..4]);
        }
        assert!(inst.is_idle(), "the voices are still running");
        // The same instrument, mended: its voices hold nothing of what broke.
        inst.template.specs[gain] = NodeSpec::Gain { amount: 1.0 };
        inst.note_on(48, 1.0);
        let mut out = Vec::new();
        for _ in 0..40 {
            inst.process_block(&mut block);
            out.extend_from_slice(&block);
        }
        assert!(out.iter().all(|x| x.is_finite()));
        let mut fresh = play(template.clone(), 48, out.len());
        fresh.truncate(out.len());
        assert!(out.iter().zip(fresh.iter()).all(|(a, b)| a.to_bits() == b.to_bits()), "not the note a new one plays");
    }
}

/// An envelope nothing flows into is a control source only on its way to a
/// port. Wired towards `out` it is what it always was, a VCA with nothing to
/// shape: it adds nothing, where its curve would be full-scale DC on every
/// note. And one envelope can be both, each connection by where it goes.
#[test]
fn an_unfed_envelope_towards_out_adds_nothing() {
    let body = |stray: &str| {
        format!(
            "  osc saw(55) as a\n  adsr(0.001, 10, 1.0, 0.1) as e\n  a > mix\n{stray}  mix > adsr(0.001, 0.1, 1.0, 0.1) as amp\n  amp > out"
        )
    };
    let without = play(graph(&body("")), 48, 8000);
    let mean = |b: &[f32]| b[4000..].iter().sum::<f32>() / 4000.0;
    assert!(mean(&without).abs() < 0.05, "the saw alone has DC {}", mean(&without));
    for stray in ["  e > mix\n", "  e > gain(0.5) as half\n  half > mix\n"] {
        let with = play(graph(&body(stray)), 48, 8000);
        assert!(mean(&with).abs() < 0.05, "{stray:?}: DC of {}", mean(&with));
        assert!(with.iter().zip(without.iter()).all(|(a, b)| a == b), "{stray:?} changed the sound");
    }
    // No port anywhere: no mask, and the envelope is nobody's control source.
    assert!(graph(&body("  e > mix\n")).mod_masks.iter().all(|&m| m == 0));

    // Both at once: into `mix` it is still silent, into the port it bends
    // the note an octave up.
    let both = play(
        graph(
            "  osc sine(55) as a\n  adsr(0.001, 10, 1.0, 0.1) as e\n  a > mix\n  e > mix\n  e > gain(1) > a.pitch\n  mix > adsr(0.001, 0.1, 1.0, 0.1) as amp\n  amp > out",
        ),
        48,
        22050,
    );
    let m = both[11025..].iter().sum::<f32>() / 11025.0;
    assert!(m.abs() < 0.05, "DC of {m}");
    let hz = pitch(&both[11025..]);
    assert!((hz / 261.63 - 1.0).abs() < 0.01, "{hz} Hz");
}

/// Ports cannot push an oscillator past the top of the band, where its phase
/// would move more than a cycle a sample and a saw read 1e10: thirty octaves
/// of `pitch` and ten carriers of `fm` leave it an oscillator.
#[test]
fn a_modulated_oscillator_stops_under_half_the_sample_rate() {
    for wave in ["sine", "saw", "square", "triangle", "pulse"] {
        for wire in ["one > gain(10) > gain(3) > src.pitch", "one > gain(10) > gain(10) > gain(10) > src.fm"] {
            let out = play(graph(&format!("  osc {wave}(55) as src\n{ONE}\n  {wire}\n{AMP}")), 48, 22050);
            let peak = out.iter().fold(0.0f32, |m, x| m.max(x.abs()));
            assert!(out.iter().all(|x| x.is_finite()) && peak < 2.0, "{wave}, {wire}: peak {peak}");
        }
    }
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

/// In a chain too, `morph` has nothing to turn on an `svf` written as the
/// bandpass: a lane or a knob on one is an error, not a sweep of nothing.
#[test]
fn morph_is_refused_on_a_bandpass_svf_in_a_chain() {
    let song = |track: &str, master: &str, rest: &str| {
        format!(
            "tempo 120\n\
             module bass low {{ cutoff 0.5 }}\n\
             pattern line {{ 1.1 - 1.3 - }}\n\
             track bass {{ play line using low level 0.6 out > {track}master }}\n\
             master {{ in > {master} > out }}\n{rest}\n"
        )
    };
    let check = |src: String| {
        let ast = dsl::parse(&src).unwrap_or_else(|e| panic!("{e:?}\n{src}"));
        dsl::compiler::compile(&ast).map(|_| ()).map_err(|e| format!("{e:?}"))
    };
    let bp = "svf(900, 0.3, bandpass)";
    for (track, master, rest) in [
        ("", bp, "auto master morph 0 > 1 over 2"),
        ("", "svf(900, 0.3, bandpass) as f", "auto master f morph 0 > 1 over 2"),
        ("", bp, "midi { cc 74 > master morph }"),
        ("svf(900, 0.3, bandpass) as f > ", "gain(1)", "midi { cc 74 > bass f morph }"),
    ] {
        let e = check(song(track, master, rest)).expect_err(rest);
        assert!(e.contains("morph"), "{rest}: {e}");
        // The same line with the lowpass-to-highpass `svf` is fine.
        let (track, master) = (track.replace(", bandpass", ""), master.replace(", bandpass", ""));
        check(song(&track, &master, rest)).unwrap_or_else(|e| panic!("{rest}: {e}"));
    }
    // With both on the master, the lane reaches the one that has the knob.
    check(song("", "svf(900, 0.3, bandpass) > svf(3000, 0.1)", "auto master morph 0 > 1 over 2")).unwrap();
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

// ── With what main added to a voice: the kick node and notes on their sample ──

/// Every port there is, an `svf`, a `ladder` and a swaying kick in one graph.
const EVERYTHING: &str = "  osc saw(55) as a\n  osc pulse(55, width=0.4, pitch=-12) as b\n  osc saw(55, pitch=7) as slave\n  lfo(3, 0.3) as wob\n  adsr(0.001, 0.2, 0.0, 0.1) as fenv\n  kick(A1, body=40ms, hold=30ms, release=80ms, vary=0.2, vary_pitch=0.05) as k\n  b > gain(0.4) > a.fm\n  wob > b.width\n  wob > slave.pitch\n  a > slave.sync\n  a > mix\n  b > mix\n  slave > mix\n  mix > svf(300, 0.5, morph=0.2) as filt\n  fenv > gain(2.5) > filt.cutoff\n  wob > filt.morph\n  filt > ladder(2000, 0.3) as lad\n  b > gain(0.5) > lad.cutoff\n  lad > adsr(0.002, 0.15, 0.5, 0.1) as amp\n  amp > out\n  k > gain(0.5) as kg\n  kg > out";

/// The engine starts a note on its own sample by rendering a block in parts,
/// up to each sample where something changes. However a block is cut, a graph
/// with ports plays the same samples: nothing in the port machinery counts
/// on a part being a whole block, the one-sample delay of a synced slave
/// included.
#[test]
fn a_block_rendered_in_parts_plays_the_same_samples() {
    let template = graph(EVERYTHING);
    assert!(template.mod_masks.iter().filter(|&&m| m != 0).count() >= 5);
    // A second and a third note land mid-block, 1000 and 5003 samples in.
    let notes = [(0usize, 45u8), (1000, 52), (5003, 40)];
    let total = 12_000;
    let run = |cuts: &[usize]| {
        let mut inst = Instrument::new(template.clone());
        let mut out = vec![0.0f32; total];
        let (mut at, mut c) = (0usize, 0usize);
        while at < total {
            for &(when, note) in &notes {
                if when == at {
                    inst.note_on(note, 0.9);
                }
            }
            let next_note = notes.iter().map(|n| n.0).find(|&w| w > at).unwrap_or(total);
            let len = cuts[c % cuts.len()].min(total - at).min(next_note - at);
            c += 1;
            inst.process_block(&mut out[at..at + len]);
            at += len;
        }
        out
    };
    let whole = run(&[BLOCK_SIZE]);
    let peak = whole.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    assert!(whole.iter().all(|x| x.is_finite()) && peak > 0.05, "peak {peak}");
    for cuts in [&[1usize][..], &[37, 128, 5, 90, 1, 127], &[64], &[3, 125]] {
        let parts = run(cuts);
        let differs = whole.iter().zip(parts.iter()).position(|(a, b)| a.to_bits() != b.to_bits());
        assert_eq!(differs, None, "cut at {cuts:?}: {:?}", differs.map(|i| (whole[i], parts[i])));
    }
}

/// A voice dropped for leaving the numbers is renewed from its specs, and a
/// swaying kick is made again for its hit number on the next note: the hit
/// after a dropped one is the hit it would have been, not hit 0 again and
/// not the dropped one's.
#[test]
fn a_dropped_voice_leaves_the_kicks_hit_count_alone() {
    use tatum_core::graph::node::NodeSpec;
    let template = graph("  kick(A1, body=40ms, hold=30ms, release=80ms, vary=0.2, vary_pitch=0.05) as k\n  k > gain(1) as trim\n  trim > out");
    let gain = (0..template.node_count as usize)
        .find(|&i| matches!(template.specs[i], NodeSpec::Gain { .. }))
        .expect("the gain");
    // One hit, to its end: the kick is what the voice waits on.
    let hit = |inst: &mut Instrument| {
        inst.note_on(45, 1.0);
        let mut out = Vec::new();
        let mut block = [0.0f32; BLOCK_SIZE];
        for _ in 0..400 {
            inst.process_block(&mut block);
            out.extend_from_slice(&block);
            if inst.is_idle() {
                break;
            }
        }
        assert!(inst.is_idle(), "the kick never ended");
        out
    };
    let mut clean = Instrument::new(template.clone());
    let hits: Vec<Vec<f32>> = (0..4).map(|_| hit(&mut clean)).collect();
    assert!(hits[2] != hits[3] && hits[0] != hits[3], "the kick does not sway, so the test proves nothing");

    let mut inst = Instrument::new(template.clone());
    assert_eq!(hit(&mut inst), hits[0]);
    assert_eq!(hit(&mut inst), hits[1]);
    for poison in [f32::NAN, f32::NEG_INFINITY] {
        let mut inst = Instrument::new(template.clone());
        hit(&mut inst);
        hit(&mut inst);
        // Hit 2 goes wrong and is dropped whole.
        inst.template.specs[gain] = NodeSpec::Gain { amount: poison };
        let dropped = hit(&mut inst);
        assert!(dropped.iter().all(|&x| x == 0.0), "{poison} got out");
        assert_eq!(dropped.len(), BLOCK_SIZE, "dropped on its first block");
        inst.template.specs[gain] = NodeSpec::Gain { amount: 1.0 };
        let after = hit(&mut inst);
        assert!(after.iter().zip(hits[3].iter()).all(|(a, b)| a.to_bits() == b.to_bits()), "not hit 3");
        assert_eq!(after.len(), hits[3].len());
    }
}

/// The kick has no ports, and says so; it can still drive one.
#[test]
fn a_kick_has_no_ports_and_can_feed_one() {
    let e = error("  kick(A1) as k\n  osc sine(55) as a\n  a > k.pitch\n  k > out");
    assert!(e.contains("'k' is a kick, which has no ports to modulate"), "{e}");
    let plain = play(
        graph("  kick(A1) as k\n  osc sine(220) as a\n  a > mix\n  mix > perc(0.001, 0.2) as amp\n  amp > out"),
        57,
        4000,
    );
    let bent = play(
        graph("  kick(A1) as k\n  osc sine(220) as a\n  k > gain(0.5) > a.fm\n  a > mix\n  mix > perc(0.001, 0.2) as amp\n  amp > out"),
        57,
        4000,
    );
    assert!(bent.iter().all(|x| x.is_finite()) && bent != plain);
}
