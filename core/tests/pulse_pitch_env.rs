//! A pulse of any width, and a pitch envelope on an oscillator that follows
//! the note: `osc pulse(55, width=0.42)` and `osc saw(55, pitch_env=150hz,
//! pitch_decay=5ms)`.
//!
//! The parser accepted `pulse` before either existed, and the compiler turned
//! it into a sine without a word, so a patch written with one played a
//! different instrument than the one on the page.

use tatum_core::dsl;
use tatum_core::graph::node::NodeSpec;
use tatum_core::graph::voice::Instrument;
use tatum_core::graph::GraphTemplate;
use tatum_core::primitives::oscillator::Waveform;
use tatum_core::{BLOCK_SIZE, SAMPLE_RATE};

/// One instrument, held at full level so what comes out is the oscillator.
fn graph(osc: &str) -> GraphTemplate {
    let src = format!(
        "tempo 120\n\
         instrument i {{\n  {osc} as src\n  src > adsr(0.001, 0.1, 1.0, 0.1) as amp\n  amp > out\n}}\n\
         pattern p {{ C2 }}\n\
         track t {{ play p using i out > master }}\n"
    );
    let ast = dsl::parse(&src).unwrap_or_else(|e| panic!("{e:?}"));
    let song = dsl::compiler::compile(&ast).unwrap_or_else(|e| panic!("{e:?}"));
    song.instruments[0].as_graph().expect("an instrument graph").clone()
}

fn compile_error(osc: &str) -> String {
    let src = format!(
        "tempo 120\n\
         instrument i {{\n  {osc} as src\n  src > adsr(0.001, 0.1, 1.0, 0.1) as amp\n  amp > out\n}}\n\
         pattern p {{ C2 }}\n\
         track t {{ play p using i out > master }}\n"
    );
    let ast = dsl::parse(&src).unwrap_or_else(|e| panic!("{e:?}"));
    match dsl::compiler::compile(&ast) {
        Ok(_) => panic!("`{osc}` compiled"),
        Err(e) => format!("{e:?}"),
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

/// Frequency from the upward zero crossings over a window, in Hz.
fn pitch(buf: &[f32]) -> f32 {
    let mut ups = Vec::new();
    for i in 1..buf.len() {
        if buf[i - 1] < 0.0 && buf[i] >= 0.0 {
            ups.push(i);
        }
    }
    assert!(ups.len() >= 2, "no full cycle in the window");
    let span = (ups[ups.len() - 1] - ups[0]) as f32;
    (ups.len() - 1) as f32 * SAMPLE_RATE / span
}

#[test]
fn pulse_compiles_to_a_pulse_not_a_sine() {
    let t = graph("osc pulse(55, width=0.42)");
    let wf = (0..t.node_count as usize).find_map(|i| match t.specs[i] {
        NodeSpec::Osc { waveform, .. } => Some(waveform),
        _ => None,
    });
    assert_eq!(wf, Some(Waveform::Pulse(0.42)));
    let t = graph("osc pulse(55)");
    let wf = (0..t.node_count as usize).find_map(|i| match t.specs[i] {
        NodeSpec::Osc { waveform, .. } => Some(waveform),
        _ => None,
    });
    assert_eq!(wf, Some(Waveform::Pulse(0.5)), "a pulse with no width is a square");
}

#[test]
fn width_sets_the_share_of_the_cycle_spent_high_and_leaves_no_dc() {
    // After the attack, a second of a 110 Hz pulse.
    let buf = play(graph("osc pulse(55, width=0.42)"), 45, SAMPLE_RATE as usize * 2);
    let held = &buf[SAMPLE_RATE as usize..];
    let high = held.iter().filter(|&&s| s > 0.0).count() as f32 / held.len() as f32;
    assert!((high - 0.42).abs() < 0.01, "spent {:.3} of the cycle high, asked for 0.42", high);
    let mean = held.iter().sum::<f32>() / held.len() as f32;
    assert!(mean.abs() < 0.01, "a 42% pulse should sit on zero, its mean is {mean:.4}");
}

#[test]
fn width_on_a_shape_that_is_not_a_pulse_is_an_error() {
    let err = compile_error("osc saw(55, width=0.42)");
    assert!(err.contains("pulse"), "{err}");
}

#[test]
fn pitch_env_starts_the_note_high_and_falls_back_onto_it() {
    // A2 is 110 Hz; 400 Hz on top of it, falling with a 5 ms time constant.
    let buf = play(graph("osc saw(55, pitch_env=400hz, pitch_decay=5ms)"), 45, SAMPLE_RATE as usize / 2);
    let ms = |m: f32| (m * SAMPLE_RATE / 1000.0) as usize;
    let early = pitch(&buf[..ms(6.0)]);
    let settled = pitch(&buf[ms(100.0)..ms(400.0)]);
    assert!(early > 250.0, "the first 6 ms should be well above the note, measured {early:.0} Hz");
    assert!((settled - 110.0).abs() < 1.5, "after it falls the note is A2, measured {settled:.1} Hz");
}

#[test]
fn no_pitch_env_is_the_plain_note_from_the_first_cycle() {
    let buf = play(graph("osc saw(55)"), 45, SAMPLE_RATE as usize / 2);
    let first = pitch(&buf[..(SAMPLE_RATE * 0.03) as usize]);
    assert!((first - 110.0).abs() < 2.0, "measured {first:.1} Hz");
}
