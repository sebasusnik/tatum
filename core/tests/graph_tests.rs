mod test_helpers;

use tatum_core::graph::node::NodeSpec;
use tatum_core::graph::GraphBuilder;
use tatum_core::graph::voice::Instrument;
use tatum_core::primitives::oscillator::Waveform;
use tatum_core::primitives::filter::FilterType;
use test_helpers::{output_path, write_wav};
use tatum_core::{BLOCK_SIZE, SAMPLE_RATE};

/// Build a bass instrument graph:
///   osc saw(55) > ladder(900, 0.6) > VCA(audio, adsr) > out
fn build_bass_graph() -> Instrument {
    let mut b = GraphBuilder::new();

    let osc = b.add_node(NodeSpec::Osc {
        waveform: Waveform::Saw,
        freq: 55.0,
        drift_seed: 42,
        fixed: false,
        pitch_semitones: 0.0,
    });
    let filt = b.add_node(NodeSpec::ladder(900.0, 0.6));
    let env = b.add_node(NodeSpec::Env { a: 0.01, d: 0.2, s: 0.7, r: 0.3 });
    let vca = b.add_node(NodeSpec::Vca);
    let out = b.add_node(NodeSpec::Output);

    b.connect(osc, filt); // osc -> filter
    b.connect(filt, vca); // filter -> VCA input 0 (audio)
    b.connect(env, vca); // env -> VCA input 1 (control)
    b.connect(vca, out); // VCA -> output

    let template = b.build();
    Instrument::new(template)
}

/// Build a kick drum graph:
///   osc sine(50) > perc(0.001, 0.3) as body_env > VCA > saturate(0.5) > mix
///   noise > highpass(5000) > perc(0.001, 0.02) as click_env > VCA > mix
///   mix > out
fn build_kick_graph() -> Instrument {
    let mut b = GraphBuilder::new();

    // Body
    let body_osc = b.add_node(NodeSpec::Osc {
        waveform: Waveform::Sine,
        freq: 50.0,
        drift_seed: 42,
        fixed: false,
        pitch_semitones: 0.0,
    });
    let body_env = b.add_node(NodeSpec::Env { a: 0.001, d: 0.3, s: 0.0, r: 0.01 });
    let body_vca = b.add_node(NodeSpec::Vca);
    let sat = b.add_node(NodeSpec::Saturator { drive: 0.5 });

    // Click
    let noise = b.add_node(NodeSpec::Noise { seed: 42 });
    let hp = b.add_node(NodeSpec::biquad(FilterType::HighPass, 5000.0, 0.3));
    let click_env = b.add_node(NodeSpec::Env { a: 0.001, d: 0.02, s: 0.0, r: 0.01 });
    let click_vca = b.add_node(NodeSpec::Vca);

    // Mix
    let mix = b.add_node(NodeSpec::Mix);
    let out = b.add_node(NodeSpec::Output);

    // Body chain
    b.connect(body_osc, body_vca);
    b.connect(body_env, body_vca);
    b.connect(body_vca, sat);
    b.connect(sat, mix);

    // Click chain
    b.connect(noise, hp);
    b.connect(hp, click_vca);
    b.connect(click_env, click_vca);
    b.connect(click_vca, mix);

    b.connect(mix, out);

    let template = b.build();
    Instrument::new(template)
}

/// Build a pad instrument graph:
///   osc saw(220) > mix
///   osc square(221) > mix
///   mix > lowpass(2000, 0.3) > VCA(audio, adsr(0.5,0.3,0.8,1.0)) > out
fn build_pad_graph() -> Instrument {
    let mut b = GraphBuilder::new();

    let osc1 = b.add_node(NodeSpec::Osc {
        waveform: Waveform::Saw,
        freq: 220.0,
        drift_seed: 42,
        fixed: false,
        pitch_semitones: 0.0,
    });
    let osc2 = b.add_node(NodeSpec::Osc {
        waveform: Waveform::Square,
        freq: 221.0,
        drift_seed: 43,
        fixed: false,
        pitch_semitones: 0.0,
    });
    let mix = b.add_node(NodeSpec::Mix);
    let filt = b.add_node(NodeSpec::biquad(FilterType::LowPass, 2000.0, 0.3));
    let env = b.add_node(NodeSpec::Env { a: 0.5, d: 0.3, s: 0.8, r: 1.0 });
    let vca = b.add_node(NodeSpec::Vca);
    let out = b.add_node(NodeSpec::Output);

    b.connect(osc1, mix);
    b.connect(osc2, mix);
    b.connect(mix, filt);
    b.connect(filt, vca);
    b.connect(env, vca);
    b.connect(vca, out);

    let template = b.build();
    Instrument::new(template)
}

#[test]
fn test_graph_bass_note() {
    let mut bass = build_bass_graph();
    let sr = SAMPLE_RATE as u32;
    let total_samples = sr as usize * 2; // 2 seconds
    let mut output = vec![0.0f32; total_samples];

    // Play A1 (MIDI 33) for 1 second, then release
    bass.note_on(33, 0.8);

    let note_on_samples = sr as usize;
    let mut pos = 0;
    let mut block = [0.0f32; BLOCK_SIZE];

    // Note-on phase
    while pos < note_on_samples {
        let len = BLOCK_SIZE.min(note_on_samples - pos);
        bass.process_block(&mut block[..len]);
        output[pos..pos + len].copy_from_slice(&block[..len]);
        pos += len;
    }

    // Release
    bass.note_off(33);

    // Release phase
    while pos < total_samples {
        let len = BLOCK_SIZE.min(total_samples - pos);
        bass.process_block(&mut block[..len]);
        output[pos..pos + len].copy_from_slice(&block[..len]);
        pos += len;
    }

    // Verify non-silent output
    let max_val = output.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
    assert!(max_val > 0.01, "bass output should be audible, got max={}", max_val);

    write_wav(&output_path("test_graph_bass.wav"), &output, sr);
}

#[test]
fn test_graph_kick_drum() {
    let mut kick = build_kick_graph();
    let sr = SAMPLE_RATE as u32;
    let total_samples = sr as usize; // 1 second
    let mut output = vec![0.0f32; total_samples];

    // Trigger 4 kicks
    let mut block = [0.0f32; BLOCK_SIZE];
    let mut pos = 0;
    let kick_interval = sr as usize / 4;

    for beat in 0..4 {
        let trigger_at = beat * kick_interval;
        while pos < total_samples.min(trigger_at + kick_interval) {
            if pos == trigger_at {
                kick.note_on(36, 0.9);
            }
            let end = total_samples.min(trigger_at + kick_interval);
            let len = BLOCK_SIZE.min(end - pos);
            kick.process_block(&mut block[..len]);
            output[pos..pos + len].copy_from_slice(&block[..len]);
            pos += len;
        }
    }

    let max_val = output.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
    assert!(max_val > 0.01, "kick output should be audible, got max={}", max_val);

    write_wav(&output_path("test_graph_kick.wav"), &output, sr);
}

#[test]
fn test_graph_pad_polyphony() {
    let mut pad = build_pad_graph();
    let sr = SAMPLE_RATE as u32;
    let total_samples = sr as usize * 3; // 3 seconds
    let mut output = vec![0.0f32; total_samples];

    let mut block = [0.0f32; BLOCK_SIZE];
    let mut pos = 0;

    // Play chord: C4 E4 G4
    pad.note_on(60, 0.6); // C4
    pad.note_on(64, 0.6); // E4
    pad.note_on(67, 0.6); // G4

    let sustain_end = sr as usize * 2;
    while pos < sustain_end {
        let len = BLOCK_SIZE.min(sustain_end - pos);
        pad.process_block(&mut block[..len]);
        output[pos..pos + len].copy_from_slice(&block[..len]);
        pos += len;
    }

    // Release all
    pad.note_off(60);
    pad.note_off(64);
    pad.note_off(67);

    while pos < total_samples {
        let len = BLOCK_SIZE.min(total_samples - pos);
        pad.process_block(&mut block[..len]);
        output[pos..pos + len].copy_from_slice(&block[..len]);
        pos += len;
    }

    let max_val = output.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
    assert!(max_val > 0.01, "pad output should be audible, got max={}", max_val);

    write_wav(&output_path("test_graph_pad.wav"), &output, sr);
}

#[test]
fn test_graph_topological_sort() {
    // Verify that the graph processes nodes in correct dependency order
    let mut b = GraphBuilder::new();

    // Add nodes in reverse dependency order to test sorting
    let out = b.add_node(NodeSpec::Output);
    let vca = b.add_node(NodeSpec::Vca);
    let env = b.add_node(NodeSpec::Env { a: 0.01, d: 0.1, s: 0.5, r: 0.1 });
    let osc = b.add_node(NodeSpec::Osc {
        waveform: Waveform::Sine,
        freq: 440.0,
        drift_seed: 42,
        fixed: false,
        pitch_semitones: 0.0,
    });

    b.connect(osc, vca);
    b.connect(env, vca);
    b.connect(vca, out);

    let template = b.build();

    // Osc and Env should come before VCA, VCA before Output
    let order = &template.execution_order[..template.exec_len as usize];
    let osc_pos = order.iter().position(|&x| x == osc).unwrap();
    let env_pos = order.iter().position(|&x| x == env).unwrap();
    let vca_pos = order.iter().position(|&x| x == vca).unwrap();
    let out_pos = order.iter().position(|&x| x == out).unwrap();

    assert!(osc_pos < vca_pos, "osc must come before vca");
    assert!(env_pos < vca_pos, "env must come before vca");
    assert!(vca_pos < out_pos, "vca must come before output");
}

#[test]
fn test_graph_voice_stealing() {
    // With MAX_VOICES=8, playing 9 notes should steal the oldest
    let mut b = GraphBuilder::new();
    let osc = b.add_node(NodeSpec::Osc {
        waveform: Waveform::Sine,
        freq: 440.0,
        drift_seed: 42,
        fixed: false,
        pitch_semitones: 0.0,
    });
    let env = b.add_node(NodeSpec::Env { a: 0.01, d: 0.1, s: 1.0, r: 0.1 });
    let vca = b.add_node(NodeSpec::Vca);
    let out = b.add_node(NodeSpec::Output);
    b.connect(osc, vca);
    b.connect(env, vca);
    b.connect(vca, out);

    let template = b.build();
    let mut inst = Instrument::new(template);

    let mut block = [0.0f32; BLOCK_SIZE];

    // Play 8 notes
    for note in 60..68 {
        inst.note_on(note, 0.5);
        inst.process_block(&mut block); // advance age
    }

    // 9th note should steal voice 0 (oldest)
    inst.note_on(70, 0.5);
    inst.process_block(&mut block);

    // Output should still be audible
    let max_val = block.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
    assert!(max_val > 0.001, "should still produce sound after voice stealing");
}
