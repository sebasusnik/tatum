//! Connections that used to compile and then not be heard.
//!
//! Every node of an instrument graph reads its first input. Two connections
//! into `out`, a filter or an envelope played the first and dropped the
//! second in silence; they add up now, as they do into `mix` and into a port.
//! A connection into a node that takes nothing in (an oscillator, the noise,
//! a whole drum) was dropped the same way, and such a node in a chain of
//! effects replaced the track's sound: both are errors that say what to write.

use tatum_core::graph::node::{port, MAX_NODE_INPUTS};
use tatum_core::graph::{Graph, GraphTemplate};
use tatum_core::song_engine::SongEngine;
use tatum_core::BLOCK_SIZE;

fn song(instrument: &str, chain: &str, extra: &str) -> String {
    format!(
        "tempo 120\nscale C major\n\
         instrument v {{\n{instrument}\n}}\n\
         {extra}\n\
         pattern p {{ C3 .. .. .. - - - -  E3 .. .. .. - - - - }}\n\
         track t {{ play p using v out > {chain} }}\n\
         master {{ in > out }}\n\
         scene a {{ track t {{ play p using v }} }}\narrange {{ a x1 }}\n"
    )
}

fn render(instrument: &str) -> Vec<f32> {
    let src = song(instrument, "master", "");
    SongEngine::from_source(&src).unwrap_or_else(|e| panic!("{e}\n{src}")).render(1).0
}

/// The instrument `v` of `song`, compiled to its graph.
fn template(instrument: &str) -> GraphTemplate {
    let ast = tatum_core::dsl::parse(&song(instrument, "master", "")).unwrap_or_else(|e| panic!("{e:?}"));
    let song = tatum_core::dsl::compiler::compile(&ast).unwrap_or_else(|e| panic!("{e:?}"));
    song.instruments[0].as_graph().expect("an instrument graph").clone()
}

/// The error `src` is refused with, its line included.
fn error(src: &str) -> String {
    match tatum_core::dsl::parse(src) {
        Err(e) => panic!("{e:?}\n{src}"),
        Ok(ast) => match tatum_core::dsl::compiler::compile(&ast) {
            Ok(_) => panic!("compiled:\n{src}"),
            Err(e) => format!("{e:?}"),
        },
    }
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| (v * v) as f64).sum::<f64>() / x.len().max(1) as f64).sqrt() as f32
}

/// How far `a` is from `b`, against `b`'s own level.
fn apart(a: &[f32], b: &[f32]) -> f32 {
    let d: Vec<f32> = a.iter().zip(b.iter()).map(|(x, y)| x - y).collect();
    rms(&d) / rms(b).max(1e-9)
}

const A: &str = "osc saw > adsr(0.005, 0.1, 0.8, 0.1) as a";
const B: &str = "osc square(pitch=7) > adsr(0.005, 0.1, 0.8, 0.1) as b";

#[test]
fn two_wires_into_out_are_both_heard() {
    let both = render(&format!("{A}\n{B}\na > out\nb > out"));
    let mixed = render(&format!("{A}\n{B}\na > mix\nb > mix\nmix > out"));
    let first = render(&format!("{A}\na > out"));
    assert!(rms(&first) > 0.01);
    assert!(
        apart(&both, &mixed) < 1e-6,
        "two wires into out are {} away from the same through mix",
        apart(&both, &mixed)
    );
    assert!(apart(&both, &first) > 0.3, "the second wire into out is not heard");
    // In either order, and three of them.
    let swapped = render(&format!("{A}\n{B}\nb > out\na > out"));
    assert!(apart(&swapped, &mixed) < 1e-5, "the order of the wires changed the sound");
    let c = "osc sine(pitch=12) > adsr(0.005, 0.1, 0.8, 0.1) as c";
    let three = render(&format!("{A}\n{B}\n{c}\na > out\nb > out\nc > out"));
    let mixed3 = render(&format!("{A}\n{B}\n{c}\na > mix\nb > mix\nc > mix\nmix > out"));
    assert!(apart(&three, &mixed3) < 1e-6);
    assert!(apart(&three, &mixed) > 0.1, "the third wire into out is not heard");
}

#[test]
fn two_wires_into_a_filter_or_an_effect_are_both_heard() {
    for node in ["lowpass(900, 0.3)", "ladder(900, 0.4)", "svf(900, 0.3)", "saturate(0.8)", "gain(0.5)"] {
        let tail = format!("{node} as f\nf > adsr(0.005, 0.1, 0.8, 0.1) as e\ne > out");
        let both = render(&format!("osc saw as a\nosc square(pitch=7) as b\na > {tail}\nb > f"));
        let mixed = render(&format!("osc saw as a\nosc square(pitch=7) as b\na > mix\nb > mix\nmix > {tail}"));
        let first = render(&format!("osc saw as a\na > {tail}"));
        assert!(apart(&both, &mixed) < 1e-6, "{node}: {} away from the same through mix", apart(&both, &mixed));
        assert!(apart(&both, &first) > 0.2, "{node}: the second wire is not heard");
    }
}

#[test]
fn two_wires_into_an_envelope_are_both_shaped() {
    let tail = "adsr(0.005, 0.1, 0.8, 0.1) as e\ne > out";
    let both = render(&format!("osc saw as a\nosc square(pitch=7) as b\na > {tail}\nb > e"));
    let mixed = render(&format!("osc saw as a\nosc square(pitch=7) as b\na > mix\nb > mix\nmix > {tail}"));
    let first = render(&format!("osc saw as a\na > {tail}"));
    assert!(apart(&both, &mixed) < 1e-6, "{} away from the same through mix", apart(&both, &mixed));
    assert!(apart(&both, &first) > 0.2, "the second wire into the envelope is not heard");
    // Shaped, not let through: silent in the rests.
    let step = both.len() / 16;
    assert!(rms(&both[step * 6..step * 8]) < rms(&both[..step * 4]) * 0.001, "the second wire went round the envelope");
}

/// The frame loop as it was before connections added up: each node reads
/// what was wired into it as it came, with nothing summed before it runs.
/// The voice's own loop (`GraphTemplate::run_frame`) must play a graph with
/// one connection into each node exactly like this.
fn render_unsummed(t: &GraphTemplate, freq: f32, samples: usize) -> Vec<f32> {
    let mut g = Graph::from_template(t);
    g.set_frequency(t, freq);
    g.gate_on(t);
    let mut out = Vec::with_capacity(samples);
    while out.len() < samples {
        for s in 0..BLOCK_SIZE {
            for &idx in &t.execution_order[..t.exec_len as usize] {
                let i = idx as usize;
                let mut audio = [0.0f32; MAX_NODE_INPUTS];
                let mut mods = [0.0f32; port::COUNT];
                let n = t.gather(i, &g.buffers, s, &mut audio, &mut mods);
                if let Some(node) = g.nodes[i].as_mut() {
                    if t.mod_masks[i] != 0 {
                        node.modulate(&mods, t.mod_masks[i]);
                    }
                    g.buffers[i][s] = node.process(&audio, n);
                }
            }
            out.push(g.buffers[t.output_node as usize][s]);
        }
    }
    out.truncate(samples);
    out
}

/// The voice's loop: `Graph::process_block`, through `run_frame`.
fn render_voice(t: &GraphTemplate, freq: f32, samples: usize) -> Vec<f32> {
    let mut g = Graph::from_template(t);
    g.set_frequency(t, freq);
    g.gate_on(t);
    let mut out = Vec::with_capacity(samples);
    let mut block = [0.0f32; BLOCK_SIZE];
    while out.len() < samples {
        g.process_block(t, &mut block);
        out.extend_from_slice(&block);
    }
    out.truncate(samples);
    out
}

#[test]
fn one_wire_sounds_as_it_did() {
    // Pins that adding up touches nothing when each node has one connection:
    // a `mix` of two (which sums them itself), a port, an LFO on a pitch, and
    // every envelope's VCA, whose two inputs are the envelope and the sound
    // and must not be added together. The voice's loop has to match the old
    // loop above sample for sample, bit for bit; a sum run on any of these
    // nodes changes the samples and fails here.
    for body in [
        "osc saw as a\nosc sine(pitch=-12) as s\nlfo(3, 0.3) > a.pitch\na > mix\ns > mix\nmix > ladder(900, 0.4) as f\nf > adsr(0.005, 0.1, 0.8, 0.1) as e\ne > out",
        "osc saw > lowpass(1200, 0.3) > adsr(0.001, 0.2, 0.5, 0.1) > out",
        "osc square as a\nosc saw(pitch=7) as b\nb > gain(0.4) > a.fm\na > svf(700, 0.5) as f\nadsr(0.001, 0.3, 0.0, 0.1) as sweep\nsweep > f.cutoff\nf > perc(0.001, 0.4) as e\ne > out",
    ] {
        let t = template(body);
        let old = render_unsummed(&t, 110.0, 8192);
        assert!(rms(&old) > 0.01 && old.iter().all(|v| v.is_finite()), "{body}");
        let now = render_voice(&t, 110.0, 8192);
        let first = now.iter().zip(old.iter()).position(|(a, b)| a.to_bits() != b.to_bits());
        assert!(first.is_none(), "sample {first:?} differs from the loop before summing:\n{body}");
    }
}

#[test]
fn a_wire_into_a_node_that_takes_nothing_in_is_an_error() {
    for (kind, word) in [
        ("osc square", "osc"),
        ("fixosc sine(200)", "fixosc"),
        ("noise", "noise"),
        ("lfo(2, 0.5)", "lfo"),
        ("kick(A1)", "kick"),
        ("tom(A2)", "tom"),
        ("hat()", "hat"),
        ("snare()", "snare"),
        ("clap()", "clap"),
        ("pitch_osc sine(300, 50, 0.999)", "pitch_osc"),
    ] {
        let src = song(&format!("{A}\n{kind} as n\na > n\nn > out"), "master", "");
        let e = error(&src);
        assert!(e.contains("line: 6"), "{kind}: no line in: {e}");
        assert!(e.contains("'a' goes into 'n'") && e.contains(&format!("{word} makes its own signal")), "{kind}: {e}");
        assert!(e.contains("`mix`"), "{kind}: does not say what to write: {e}");
        // Inline in a chain, where the node has no name.
        let e = error(&song(&format!("{A}\na > {kind} > out"), "master", ""));
        assert!(e.contains(&format!("goes into {word},")), "{kind} inline: {e}");
    }
    // An oscillator has ports, and that may be what was meant.
    let e = error(&song(&format!("{A}\nosc square as n\na > n\nn > out"), "master", ""));
    assert!(e.contains("`n.fm`"), "{e}");
    // Into a port it is what it always was.
    let src =
        song(&format!("{A}\nosc square as n\na > n.fm\nn > adsr(0.005, 0.1, 0.8, 0.1) as e\ne > out"), "master", "");
    SongEngine::from_source(&src).unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn a_source_in_a_chain_of_effects_is_an_error() {
    let inst = format!("{A}\na > out");
    for kind in
        ["noise", "kick(A1)", "hat()", "snare()", "clap()", "tom(A2)", "lfo(2, 0.5)", "pitch_osc(300, 50, 0.999)"]
    {
        let word = kind.split(['(', ' ']).next().unwrap();
        // A track's inserts, a bus, the master and a send return.
        let cases = [
            (song(&inst, &format!("{kind} > master"), ""), "track 't'"),
            (song(&inst, "grp", &format!("bus grp\ngrp {{ in > {kind} > master }}")), "grp"),
            (
                song(&inst, "master", "").replace("master { in > out }", &format!("master {{ in > {kind} > out }}")),
                "master",
            ),
            (song(&inst, "master", &format!("reverb_return {{ in > {kind} > out }}")), "reverb_return"),
        ];
        for (src, owner) in cases {
            let e = error(&src);
            assert!(
                e.contains(owner) && e.contains(&format!("`{word}` makes its own signal")) && e.contains("replace"),
                "{kind} in {owner}: {e}"
            );
        }
    }
    let e = error(&song(&inst, "adsr(0.01, 0.1, 0.5, 0.1) > master", ""));
    assert!(e.contains("`adsr` is an envelope") && e.contains("silence"), "{e}");
    // Effects are what a chain holds.
    let src = song(&inst, "lowpass(900, 0.2) > saturate(0.3) > reverb(0.4) > master", "");
    SongEngine::from_source(&src).unwrap_or_else(|e| panic!("{e}"));
}
