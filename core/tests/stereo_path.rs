//! The stereo path, measured. Five of the six bugs behind the liquid_dnb pad
//! were here: a filter clocked twice per stereo frame ran an octave high, one
//! chorus delay line shared by both channels was a comb, two tracks on one
//! module advanced it twice per block, and keys and FM were rendered in
//! stereo and then downmixed. These are the numbers that would have caught
//! each of them.

use tatum_core::analysis::stereo_width;
use tatum_core::graph::node::{FilterLfo, NodeSpec};
use tatum_core::primitives::filter::FilterType;
use tatum_core::song_engine::SongEngine;
use tatum_core::{BLOCK_SIZE, SAMPLE_RATE};

fn lowpass(cutoff: f32, resonance: f32) -> NodeSpec {
    NodeSpec::Biquad {
        filter_type: FilterType::LowPass, cutoff, resonance,
        env_attack: 0.0, env_decay: 0.0, env_sustain: 0.0, env_release: 0.0, env_depth: 0.0,
        lfo: FilterLfo { hz: 0.0, bars: 0.0, depth: 0.0 },
    }
}

/// Gain of a node at `hz`, stereo path, after the filter settles.
fn stereo_gain_at(spec: &NodeSpec, hz: f32) -> f32 {
    let mut node = spec.instantiate();
    let n = SAMPLE_RATE as usize;
    let (mut in_sq, mut out_sq) = (0.0f64, 0.0f64);
    for i in 0..n {
        let x = tatum_core::math::sin(2.0 * tatum_core::math::PI * hz * i as f32 / SAMPLE_RATE);
        let (l, _r) = node.process_stereo(x, x);
        if i >= n / 2 {
            in_sq += (x * x) as f64;
            out_sq += (l * l) as f64;
        }
    }
    (out_sq / in_sq).sqrt() as f32
}

#[test]
fn a_stereo_lowpass_sits_at_its_written_corner() {
    // -3 dB at the corner is what a lowpass means (at the flat Q the lowest
    // resonance gives). Clocked twice per frame the same filter measured its
    // corner at 7681 Hz for a written 3400.
    let spec = lowpass(3400.0, 0.01);
    let at_corner = stereo_gain_at(&spec, 3400.0);
    let octave_up = stereo_gain_at(&spec, 6800.0);
    let db = |g: f32| 20.0 * g.max(1e-9).log10();
    assert!((db(at_corner) + 3.0).abs() < 1.0, "corner gain {:.1} dB, expected -3", db(at_corner));
    assert!(db(octave_up) < -9.0, "one octave above the corner reads {:.1} dB; the filter is not where it says", db(octave_up));
}

#[test]
fn a_chain_filter_resonance_is_what_the_docs_say() {
    // The chain filter maps `resonance` 0..1 to Q 0.5..20. At 0.35 that is
    // a Q of 7.3, a +17 dB peak: a recipe that wrote 0.3 meant "a little",
    // and now that the filter resonates where it says, it screams. This pins
    // the mapping so the docs can be written against it.
    let peak = |res: f32| 20.0 * stereo_gain_at(&lowpass(1000.0, res), 1000.0).log10();
    assert!(peak(0.35) > 14.0, "resonance 0.35 peaks {:.1} dB", peak(0.35));
    assert!(peak(0.05) < 5.0, "resonance 0.05 peaks {:.1} dB", peak(0.05));
}

// gain_comp 0: the automatic 1/sqrt(active tracks) would otherwise change
// the level the moment a second track exists, which is not what is measured.
const PAD: &str = "tempo 120\nscale A minor\ngain_comp 0\nmodule keys pad { voice_mode poly attack 300ms release 200ms cutoff 2khz chorus_mix 0.0 }\npattern hold { [1.3 3.3 5.3]:0.8 ..*15 }\npattern rest { - - - - - - - - - - - - - - - - }\ntrack a { play hold using pad level 0.5 out > master }\nmaster { in > out }\n";

#[test]
fn a_second_track_on_the_same_module_does_not_change_the_first() {
    // Two tracks naming one module used to share one instance, advanced once
    // per track per block: each track got every other block, envelopes ran
    // at twice their rate, and the discarded half combed at 344.5 Hz. A
    // silent second track must leave the first sample-identical.
    let alone = { let mut e = SongEngine::from_source(PAD).unwrap(); e.start(); e.render_steps(32).0 };
    let shared_src = PAD.replace("master { in > out }", "track b { play rest using pad level 0.5 out > master }\nmaster { in > out }");
    let shared = { let mut e = SongEngine::from_source(&shared_src).unwrap(); e.start(); e.render_steps(32).0 };
    assert!(alone.iter().any(|v| v.abs() > 0.01), "the pad is silent, the test proves nothing");
    let first = alone.iter().zip(&shared).position(|(x, y)| x != y);
    assert_eq!(first, None, "a silent track on the same module changed the other one at sample {:?}", first);
}

fn render_stereo(src: &str) -> (Vec<f32>, Vec<f32>) {
    let mut e = SongEngine::from_source(src).unwrap();
    e.start();
    e.render_steps(16)
}

#[test]
fn keys_chorus_makes_the_two_channels_differ() {
    // One delay line for both channels with L == R is a comb, not width: the
    // pad measured +1.000 correlation, -240 dB of side. With the chorus off
    // a centred chord is mono; with it on, the channels must differ.
    let (l, r) = render_stereo(PAD);
    assert!(stereo_width(&l, &r) < 1e-6, "with chorus_mix 0 a poly chord is centred");
    let (l, r) = render_stereo(&PAD.replace("chorus_mix 0.0", "chorus_mix 0.8"));
    let w = stereo_width(&l, &r);
    assert!(w > 0.02, "chorus_mix 0.8 leaves width at {:.4}; the chorus is still mono", w);
}

#[test]
fn fm_chorus_makes_the_two_channels_differ() {
    // FM had no stereo path at all; the chorus is its whole width budget.
    let src = PAD.replace("module keys pad { voice_mode poly attack 300ms release 200ms cutoff 2khz chorus_mix 0.0 }",
                          "module fm pad { chorus_mix 0.8 }");
    let (l, r) = render_stereo(&src);
    assert!(l.iter().any(|v| v.abs() > 0.01), "silent");
    let w = stereo_width(&l, &r);
    assert!(w > 0.02, "fm with chorus_mix 0.8 is mono: width {:.4}", w);
}

#[test]
fn autowah_opens_with_the_signal_and_closes_after_it() {
    // A resonant filter swept by the input's own envelope. Loud for 300 ms
    // must push the cutoff well above its base; a second of silence must
    // bring it back down. Every other filter here has an LFO; this is the
    // one that follows the playing.
    let spec = NodeSpec::AutoWah { sens: 0.8, base: 300.0, range: 2500.0, q: 0.6,
        attack_ms: 8.0, release_ms: 200.0, mode: 0, down: false, wobble: 0.0, wobble_hz: 5.0 };
    let mut node = spec.instantiate();
    let cutoff = |node: &tatum_core::graph::node::NodeKind| match node {
        tatum_core::graph::node::NodeKind::AutoWah { cutoff, .. } => *cutoff,
        _ => unreachable!(),
    };
    let loud = (0.3 * SAMPLE_RATE) as usize;
    for i in 0..loud {
        let x = 0.5 * tatum_core::math::sin(2.0 * tatum_core::math::PI * 220.0 * i as f32 / SAMPLE_RATE);
        node.process_stereo(x, x);
    }
    let open = cutoff(&node);
    assert!(open > 300.0 + 2500.0 * 0.5, "after 300 ms of signal the cutoff is at {:.0} Hz", open);
    for _ in 0..SAMPLE_RATE as usize {
        node.process_stereo(0.0, 0.0);
    }
    let closed = cutoff(&node);
    assert!(closed < 300.0 + 2500.0 * 0.1, "a second after the signal stopped the cutoff is still at {:.0} Hz", closed);
    let _ = BLOCK_SIZE;
}
