//! Engine unit-level integration tests — modules, effects, sequencer, pitch bend, etc.
//!
//! Moved out of `engine.rs` to keep the core module lean.
//! Run with: `cargo test --test engine_tests`

use synth_core::engine::Engine;
use synth_core::modules::bass::BassModule;
use synth_core::modules::fm::FmModule;
use synth_core::modules::keys::KeysModule;
use synth_core::modules::beats::BeatsModule;
use synth_core::harmony::{HarmonyContext, Scale, Progressions};
use synth_core::sequencer::{Step, PARAM_BASS_CUTOFF, PARAM_BEATS_KICK_DECAY};
use synth_core::modules::bass::BassParam;
use synth_core::modules::fm::FmParam;
use synth_core::modules::keys::KeysParam;
use synth_core::effects::reverb::{Reverb, DattorroReverb};
use synth_core::effects::delay::{Delay, DelaySync};
use synth_core::effects::limiter::Limiter;
use synth_core::effects::compressor::Compressor;
use synth_core::effects::eq::{TiltEq, ThreeBandEq};
use synth_core::effects::saturator::Saturator;
use synth_core::primitives::envelope::EnvStage;
use synth_core::primitives::fm_operator::FmWaveform;
use synth_core::{Module, SAMPLE_RATE, BLOCK_SIZE};

use std::f32::consts::TAU;

mod test_helpers;
use test_helpers::{write_wav, write_wav_stereo, output_path};

#[test]
fn test_bass_basic() {
    let mut bass = BassModule::new();
    bass.set_param(BassParam::Cutoff, 0.8);
    bass.set_param(BassParam::CutoffEnv, 0.2);
    bass.set_param(BassParam::Resonance, 0.2);

    let duration = 3.0;
    let total_samples = (SAMPLE_RATE * duration) as usize;
    let mut samples = vec![0.0f32; total_samples];

    bass.note_on(45, 0.9);

    let mut pos = 0;
    while pos < total_samples {
        let block_len = BLOCK_SIZE.min(total_samples - pos);
        bass.process_block(&mut samples[pos..pos + block_len]);
        pos += block_len;
    }

    bass.note_off(45);
    let extra = (SAMPLE_RATE * 0.5) as usize;
    let mut tail = vec![0.0f32; extra];
    let mut tpos = 0;
    while tpos < extra {
        let bl = BLOCK_SIZE.min(extra - tpos);
        bass.process_block(&mut tail[tpos..tpos + bl]);
        tpos += bl;
    }
    samples.extend_from_slice(&tail);

    write_wav(&output_path("test_bass_basic.wav"), &samples, SAMPLE_RATE as u32);
}

#[test]
fn test_bass_acid() {
    let mut bass = BassModule::new();
    bass.set_param(BassParam::Cutoff, 0.15);
    bass.set_param(BassParam::CutoffEnv, 0.9);
    bass.set_param(BassParam::Resonance, 0.85);
    bass.set_param(BassParam::Glide, 0.3);

    let sr = SAMPLE_RATE as usize;
    let total = sr * 4;
    let mut samples = vec![0.0f32; total];

    let notes = [(45u8, 0, sr), (52, sr, sr * 2), (45, sr * 2, sr * 3)];

    for &(note, start, _end) in &notes {
        if start == 0 {
            bass.note_on(note, 1.0);
        }
    }

    let mut pos = 0;
    let mut next_note_idx = 1;
    while pos < total {
        if next_note_idx < notes.len() && pos >= notes[next_note_idx].1 {
            bass.note_on(notes[next_note_idx].0, 1.0);
            next_note_idx += 1;
        }

        let block_len = BLOCK_SIZE.min(total - pos);
        bass.process_block(&mut samples[pos..pos + block_len]);
        pos += block_len;
    }

    write_wav(&output_path("test_bass_acid.wav"), &samples, SAMPLE_RATE as u32);
}

#[test]
fn test_fm_bell() {
    let mut fm = FmModule::new();
    fm.set_ratios([1.0, 3.0, 1.0, 1.0]);
    fm.set_op_envelope(0, 0.001, 3.0, 0.0, 0.5);
    fm.set_op_envelope(1, 0.01, 2.0, 0.0, 0.5);
    fm.set_op_envelope(2, 0.001, 0.01, 0.0, 0.01);
    fm.set_op_envelope(3, 0.001, 0.01, 0.0, 0.01);
    fm.set_param(FmParam::Algorithm, 0.3);
    fm.set_param(FmParam::ModIndex, 0.125);

    let total = (SAMPLE_RATE * 3.0) as usize;
    let mut samples = vec![0.0f32; total];

    fm.note_on(72, 0.8);

    let mut pos = 0;
    while pos < total {
        let bl = BLOCK_SIZE.min(total - pos);
        fm.process_block(&mut samples[pos..pos + bl]);
        pos += bl;
    }

    write_wav(&output_path("test_fm_bell.wav"), &samples, SAMPLE_RATE as u32);
}

#[test]
fn test_fm_bass() {
    let mut fm = FmModule::new();
    fm.set_ratios([1.0, 1.0, 1.0, 1.0]);
    fm.set_op_envelope(0, 0.005, 0.5, 0.3, 0.2);
    fm.set_op_envelope(1, 0.01, 0.3, 0.1, 0.1);
    fm.set_op_envelope(2, 0.001, 0.01, 0.0, 0.01);
    fm.set_op_envelope(3, 0.001, 0.01, 0.0, 0.01);
    fm.set_param(FmParam::Algorithm, 0.3);
    fm.set_param(FmParam::ModIndex, 0.125);

    let total = (SAMPLE_RATE * 3.0) as usize;
    let mut samples = vec![0.0f32; total];

    fm.note_on(40, 0.8);

    let mut pos = 0;
    while pos < total {
        let bl = BLOCK_SIZE.min(total - pos);
        fm.process_block(&mut samples[pos..pos + bl]);
        pos += bl;
    }

    write_wav(&output_path("test_fm_bass.wav"), &samples, SAMPLE_RATE as u32);
}

#[test]
fn test_beats_full() {
    let mut beats = BeatsModule::new();
    let bpm = 120.0;
    let step_samples = (SAMPLE_RATE * 60.0 / bpm / 4.0) as usize;
    let total_steps = 64;
    let total = step_samples * total_steps;
    let mut samples = vec![0.0f32; total];

    let kick_pattern:  [bool; 16] = [true,false,false,false,true,false,false,false,true,false,false,false,true,false,false,false];
    let snare_pattern: [bool; 16] = [false,false,false,false,true,false,false,false,false,false,false,false,true,false,false,false];
    let hh_pattern:    [bool; 16] = [true,false,true,false,true,false,true,false,true,false,true,false,true,false,true,false];

    for step in 0..total_steps {
        let pattern_step = step % 16;
        let start = step * step_samples;

        if kick_pattern[pattern_step] {
            beats.note_on(36, 1.0);
        }
        if snare_pattern[pattern_step] {
            beats.note_on(38, 0.9);
        }
        if hh_pattern[pattern_step] {
            beats.note_on(42, 0.6);
        }

        let end = (start + step_samples).min(total);
        let mut pos = start;
        while pos < end {
            let bl = BLOCK_SIZE.min(end - pos);
            beats.process_block(&mut samples[pos..pos + bl]);
            pos += bl;
        }
    }

    write_wav(&output_path("test_beats_full.wav"), &samples, SAMPLE_RATE as u32);
}

#[test]
fn test_keys_pad() {
    let mut keys = KeysModule::new();
    keys.set_param(KeysParam::Cutoff, 0.25);
    keys.set_param(KeysParam::Detune, 0.15);
    keys.set_param(KeysParam::ChorusMix, 0.4);

    let mut reverb = Reverb::new(SAMPLE_RATE);
    reverb.set_room_size(0.8);
    reverb.set_mix(0.35);

    let total = (SAMPLE_RATE * 4.0) as usize;
    let mut samples = vec![0.0f32; total];

    keys.note_on(57, 0.7);
    keys.note_on(60, 0.7);
    keys.note_on(64, 0.7);

    let mut pos = 0;
    while pos < total {
        let bl = BLOCK_SIZE.min(total - pos);
        keys.process_block(&mut samples[pos..pos + bl]);
        pos += bl;
    }

    keys.note_off(57);
    keys.note_off(60);
    keys.note_off(64);
    let extra = (SAMPLE_RATE * 1.5) as usize;
    let mut tail = vec![0.0f32; extra];
    let mut tpos = 0;
    while tpos < extra {
        let bl = BLOCK_SIZE.min(extra - tpos);
        keys.process_block(&mut tail[tpos..tpos + bl]);
        tpos += bl;
    }
    samples.extend_from_slice(&tail);

    for s in samples.iter_mut() {
        *s = reverb.process(*s);
    }

    write_wav(&output_path("test_keys_pad.wav"), &samples, SAMPLE_RATE as u32);
}

#[test]
fn test_harmony_progression() {
    let mut bass = BassModule::new();
    let mut keys = KeysModule::new();
    keys.set_param(KeysParam::Cutoff, 0.3);
    keys.set_param(KeysParam::Detune, 0.1);

    let mut reverb = Reverb::new(SAMPLE_RATE);
    reverb.set_room_size(0.6);
    reverb.set_mix(0.25);

    let saturator = Saturator::new(1.2);

    let bpm = 120.0;
    let beats_per_chord = 4.0;
    let samples_per_chord = (SAMPLE_RATE * 60.0 / bpm * beats_per_chord) as usize;

    let mut ctx = HarmonyContext::new(57, Scale::Minor);
    let progression = Progressions::minor_pop();

    let total = samples_per_chord * 4;
    let mut samples = vec![0.0f32; total];

    for (chord_idx, &degree) in progression.iter().enumerate() {
        ctx.set_chord_degree(degree);
        let chord_notes = ctx.chord_notes();
        let bass_note = ctx.bass_note();

        let bass_midi = if bass_note >= 12 { bass_note - 12 } else { bass_note };
        bass.note_on(bass_midi, 0.9);

        for note_opt in &chord_notes {
            if let Some(note) = note_opt {
                keys.note_on(*note, 0.7);
            }
        }

        let start = chord_idx * samples_per_chord;
        let end = start + samples_per_chord;
        let mut pos = start;
        let mut bass_buf = [0.0f32; BLOCK_SIZE];
        let mut keys_buf = [0.0f32; BLOCK_SIZE];

        while pos < end {
            let bl = BLOCK_SIZE.min(end - pos);
            bass.process_block(&mut bass_buf[..bl]);
            keys.process_block(&mut keys_buf[..bl]);
            for i in 0..bl {
                samples[pos + i] = bass_buf[i] * 0.6 + keys_buf[i] * 0.4;
            }
            pos += bl;
        }

        bass.note_off(bass_midi);
        for note_opt in &chord_notes {
            if let Some(note) = note_opt {
                keys.note_off(*note);
            }
        }
    }

    for s in samples.iter_mut() {
        *s = reverb.process(*s);
        *s = saturator.process(*s);
    }

    write_wav(&output_path("test_harmony_progression.wav"), &samples, SAMPLE_RATE as u32);
}

#[test]
fn test_engine_full() {
    let mut engine = Engine::new();
    engine.set_bpm(120.0);

    let mut ctx = HarmonyContext::new(57, Scale::Minor);
    ctx.set_chord_degree(0);
    engine.set_harmony(ctx);

    for i in 0..16 {
        let step = match i {
            4 | 12 => Step::new(38, 0.9, 0.5),
            0 | 8 => Step::new(36, 1.0, 0.5),
            2 | 6 | 10 | 14 => Step::new(42, 0.6, 0.3),
            _ => Step::empty(),
        };
        engine.sequencer.steps[i] = step;
    }

    engine.sequencer.start();

    engine.bass.note_on(45, 0.8);
    engine.keys.note_on(57, 0.7);
    engine.keys.note_on(60, 0.7);
    engine.keys.note_on(64, 0.7);
    engine.fm.note_on(72, 0.3);

    let total = (SAMPLE_RATE * 8.0) as usize;
    let mut samples_l = vec![0.0f32; total];
    let mut samples_r = vec![0.0f32; total];

    let mut pos = 0;
    while pos < total {
        let bl = BLOCK_SIZE.min(total - pos);
        engine.process_block_stereo(
            &mut samples_l[pos..pos + bl],
            &mut samples_r[pos..pos + bl],
        );
        pos += bl;
    }

    write_wav_stereo(&output_path("test_engine_full.wav"), &samples_l, &samples_r, SAMPLE_RATE as u32);
}

#[test]
fn test_reverb_stereo() {
    let mut reverb = Reverb::new(SAMPLE_RATE);
    reverb.set_room_size(0.8);
    reverb.set_damping(0.4);
    reverb.set_mix(0.4);

    let total = (SAMPLE_RATE * 3.0) as usize;
    let mut samples_l = vec![0.0f32; total];
    let mut samples_r = vec![0.0f32; total];

    let mut keys = KeysModule::new();
    keys.set_param(KeysParam::Cutoff, 0.3);
    keys.set_param(KeysParam::ChorusMix, 0.2);
    keys.note_on(57, 0.8);
    keys.note_on(60, 0.8);
    keys.note_on(64, 0.8);

    let mut pos = 0;
    while pos < total {
        let bl = BLOCK_SIZE.min(total - pos);
        let mut buf = vec![0.0f32; bl];
        keys.process_block(&mut buf);

        if pos < (SAMPLE_RATE as usize) && pos + bl >= (SAMPLE_RATE as usize) {
            keys.note_off(57);
            keys.note_off(60);
            keys.note_off(64);
        }

        for i in 0..bl {
            let (l, r) = reverb.process_stereo(buf[i]);
            samples_l[pos + i] = l;
            samples_r[pos + i] = r;
        }
        pos += bl;
    }

    write_wav_stereo(&output_path("test_reverb_stereo.wav"), &samples_l, &samples_r, SAMPLE_RATE as u32);
}

#[test]
fn test_bass_lfo_cutoff() {
    let mut bass = BassModule::new();
    bass.set_param(BassParam::Cutoff, 0.3);
    bass.set_param(BassParam::CutoffEnv, 0.1);
    bass.set_param(BassParam::Resonance, 0.5);
    bass.set_param(BassParam::LfoRate, (4.0 - 0.1) / 19.9);
    bass.set_param(BassParam::LfoDepth, 0.7);
    bass.set_param(BassParam::LfoWaveform, 0.0);
    bass.set_param(BassParam::LfoTarget, 0.0);
    bass.set_param(BassParam::LfoSync, 0.0);

    let total = (SAMPLE_RATE * 4.0) as usize;
    let mut samples = vec![0.0f32; total];

    bass.note_on(45, 0.9);

    let mut pos = 0;
    while pos < total {
        let bl = BLOCK_SIZE.min(total - pos);
        bass.process_block(&mut samples[pos..pos + bl]);
        pos += bl;
    }

    let first_quarter = &samples[0..(total / 4)];
    let max_val = first_quarter.iter().fold(0.0f32, |a, &b| if b.abs() > a { b.abs() } else { a });
    assert!(max_val > 0.01, "Bass LFO cutoff test should produce audible output");

    write_wav(&output_path("test_bass_lfo_cutoff.wav"), &samples, SAMPLE_RATE as u32);
}

#[test]
fn test_keys_lfo_pitch() {
    let mut keys = KeysModule::new();
    keys.set_param(KeysParam::Cutoff, 0.4);
    keys.set_param(KeysParam::Detune, 0.05);
    keys.set_param(KeysParam::ChorusMix, 0.2);
    keys.set_param(KeysParam::LfoRate, (1.0 - 0.1) / 19.9);
    keys.set_param(KeysParam::LfoDepth, 0.1);
    keys.set_param(KeysParam::LfoWaveform, 0.25);
    keys.set_param(KeysParam::LfoTarget, 0.5);
    keys.set_param(KeysParam::LfoSync, 0.0);

    let total = (SAMPLE_RATE * 4.0) as usize;
    let mut samples = vec![0.0f32; total];

    keys.note_on(57, 0.5);
    keys.note_on(60, 0.5);
    keys.note_on(64, 0.5);

    let mut pos = 0;
    while pos < total {
        let bl = BLOCK_SIZE.min(total - pos);
        keys.process_block(&mut samples[pos..pos + bl]);
        pos += bl;
    }

    let max_val = samples.iter().fold(0.0f32, |a, &b| if b.abs() > a { b.abs() } else { a });
    assert!(max_val > 0.01, "Keys LFO pitch test should produce audible output");

    write_wav(&output_path("test_keys_lfo_pitch.wav"), &samples, SAMPLE_RATE as u32);
}

#[test]
fn test_fm_lfo_mod_index() {
    let mut fm = FmModule::new();
    fm.set_ratios([1.0, 3.0, 1.0, 1.0]);
    fm.set_op_envelope(0, 0.001, 3.0, 0.3, 0.5);
    fm.set_op_envelope(1, 0.01, 2.5, 0.2, 0.5);
    fm.set_op_envelope(2, 0.001, 0.01, 0.0, 0.01);
    fm.set_op_envelope(3, 0.001, 0.01, 0.0, 0.01);
    fm.set_param(FmParam::Algorithm, 0.3);
    fm.set_param(FmParam::ModIndex, 0.1875);
    fm.set_param(FmParam::LfoRate, (0.5 - 0.1) / 19.9);
    fm.set_param(FmParam::LfoDepth, 0.5);
    fm.set_param(FmParam::LfoWaveform, 0.0);
    fm.set_param(FmParam::LfoTarget, 1.0);
    fm.set_param(FmParam::LfoSync, 0.0);

    let total = (SAMPLE_RATE * 4.0) as usize;
    let mut samples = vec![0.0f32; total];

    fm.note_on(72, 0.8);

    let mut pos = 0;
    while pos < total {
        let bl = BLOCK_SIZE.min(total - pos);
        fm.process_block(&mut samples[pos..pos + bl]);
        pos += bl;
    }

    let max_val = samples.iter().fold(0.0f32, |a, &b| if b.abs() > a { b.abs() } else { a });
    assert!(max_val > 0.01, "FM LFO mod_index test should produce audible output");

    write_wav(&output_path("test_fm_lfo_mod_index.wav"), &samples, SAMPLE_RATE as u32);
}

#[test]
fn test_engine_lfo_delay() {
    let mut engine = Engine::new();
    engine.set_bpm(120.0);

    let mut ctx = HarmonyContext::new(57, Scale::Minor);
    ctx.set_chord_degree(0);
    engine.set_harmony(ctx);

    engine.set_delay_lfo_rate(0.3);
    engine.set_delay_lfo_depth(0.6);
    engine.set_delay_time(0.3);

    engine.bass.note_on(45, 0.8);
    engine.keys.note_on(57, 0.7);
    engine.keys.note_on(60, 0.7);
    engine.keys.note_on(64, 0.7);

    let total = (SAMPLE_RATE * 4.0) as usize;
    let mut samples_l = vec![0.0f32; total];
    let mut samples_r = vec![0.0f32; total];

    let mut pos = 0;
    while pos < total {
        let bl = BLOCK_SIZE.min(total - pos);
        engine.process_block_stereo(
            &mut samples_l[pos..pos + bl],
            &mut samples_r[pos..pos + bl],
        );
        pos += bl;
    }

    let max_l = samples_l.iter().fold(0.0f32, |a, &b| if b.abs() > a { b.abs() } else { a });
    assert!(max_l > 0.01, "Engine delay LFO test should produce audible output");

    write_wav_stereo(&output_path("test_engine_lfo_delay.wav"), &samples_l, &samples_r, SAMPLE_RATE as u32);
}

#[test]
fn test_motion_seq_bass_cutoff() {
    let mut engine = Engine::new();
    engine.set_bpm(120.0);

    engine.bass.set_param(BassParam::Cutoff, 0.3);
    engine.bass.set_param(BassParam::CutoffEnv, 0.6);
    engine.bass.set_param(BassParam::Resonance, 0.6);

    let cutoffs = [0.1, 0.8, 0.3, 0.9];
    for i in 0..4 {
        engine.sequencer.tracks[0].steps[i] = Step::new(33, 0.9, 0.7)
            .with_lock(PARAM_BASS_CUTOFF, cutoffs[i]);
    }
    engine.sequencer.num_steps = 4;
    engine.sequencer.start();

    let total = (SAMPLE_RATE * 4.0) as usize;
    let mut samples_l = vec![0.0f32; total];
    let mut samples_r = vec![0.0f32; total];

    let mut pos = 0;
    while pos < total {
        let bl = BLOCK_SIZE.min(total - pos);
        engine.process_block_stereo(
            &mut samples_l[pos..pos + bl],
            &mut samples_r[pos..pos + bl],
        );
        pos += bl;
    }

    let max_val = samples_l.iter().fold(0.0f32, |a, &b| if b.abs() > a { b.abs() } else { a });
    assert!(max_val > 0.01, "Motion seq bass cutoff should produce audible output");

    write_wav_stereo(&output_path("test_motion_seq_bass_cutoff.wav"), &samples_l, &samples_r, SAMPLE_RATE as u32);
}

#[test]
fn test_slide_acid_bass() {
    let mut engine = Engine::new();
    engine.set_bpm(140.0);

    engine.bass.set_param(BassParam::Cutoff, 0.1);
    engine.bass.set_param(BassParam::CutoffEnv, 0.95);
    engine.bass.set_param(BassParam::Resonance, 0.85);
    engine.bass.set_param(BassParam::Glide, 0.3);

    engine.sequencer.tracks[0].steps[0] = Step::new(33, 1.0, 0.8);
    engine.sequencer.tracks[0].steps[1] = Step::new(28, 1.0, 0.8).with_slide();
    engine.sequencer.tracks[0].steps[2] = Step::new(33, 1.0, 0.8);
    engine.sequencer.tracks[0].steps[3] = Step::new(35, 1.0, 0.8).with_slide();
    engine.sequencer.tracks[0].steps[4] = Step::new(33, 1.0, 0.8);
    engine.sequencer.tracks[0].steps[5] = Step::new(28, 1.0, 0.8).with_slide()
        .with_lock(PARAM_BASS_CUTOFF, 0.6);
    engine.sequencer.tracks[0].steps[6] = Step::empty();
    engine.sequencer.tracks[0].steps[7] = Step::new(21, 0.8, 0.5);

    engine.sequencer.num_steps = 8;
    engine.sequencer.start();

    let total = (SAMPLE_RATE * 4.0) as usize;
    let mut samples_l = vec![0.0f32; total];
    let mut samples_r = vec![0.0f32; total];

    let mut pos = 0;
    while pos < total {
        let bl = BLOCK_SIZE.min(total - pos);
        engine.process_block_stereo(
            &mut samples_l[pos..pos + bl],
            &mut samples_r[pos..pos + bl],
        );
        pos += bl;
    }

    let max_val = samples_l.iter().fold(0.0f32, |a, &b| if b.abs() > a { b.abs() } else { a });
    assert!(max_val > 0.01, "Slide acid bass should produce audible output");

    write_wav_stereo(&output_path("test_slide_acid_bass.wav"), &samples_l, &samples_r, SAMPLE_RATE as u32);
}

#[test]
fn test_motion_seq_beats_decay() {
    let mut engine = Engine::new();
    engine.set_bpm(120.0);

    for i in 0..16 {
        if i % 4 == 0 {
            let decay = if (i / 4) % 2 == 0 { 0.2 } else { 0.9 };
            engine.sequencer.steps[i] = Step::new(36, 1.0, 0.5)
                .with_lock(PARAM_BEATS_KICK_DECAY, decay);
        } else {
            engine.sequencer.steps[i] = Step::empty();
        }
    }

    engine.sequencer.num_steps = 16;
    engine.sequencer.start();

    let total = (SAMPLE_RATE * 4.0) as usize;
    let mut samples_l = vec![0.0f32; total];
    let mut samples_r = vec![0.0f32; total];

    let mut pos = 0;
    while pos < total {
        let bl = BLOCK_SIZE.min(total - pos);
        engine.process_block_stereo(
            &mut samples_l[pos..pos + bl],
            &mut samples_r[pos..pos + bl],
        );
        pos += bl;
    }

    let max_val = samples_l.iter().fold(0.0f32, |a, &b| if b.abs() > a { b.abs() } else { a });
    assert!(max_val > 0.01, "Motion seq beats decay should produce audible output");

    write_wav_stereo(&output_path("test_motion_seq_beats_decay.wav"), &samples_l, &samples_r, SAMPLE_RATE as u32);
}

#[test]
fn test_motion_seq_empty_step_locks() {
    let mut engine = Engine::new();
    engine.set_bpm(120.0);

    engine.bass.set_param(BassParam::Cutoff, 0.1);
    engine.bass.set_param(BassParam::CutoffEnv, 0.0);
    engine.bass.set_param(BassParam::Resonance, 0.7);
    engine.bass.note_on(45, 0.9);

    let cutoffs = [0.1, 0.3, 0.6, 0.9, 0.9, 0.6, 0.3, 0.1];
    for i in 0..8 {
        engine.sequencer.steps[i] = Step::empty()
            .with_lock(PARAM_BASS_CUTOFF, cutoffs[i]);
    }

    engine.sequencer.num_steps = 8;
    engine.sequencer.start();

    let total = (SAMPLE_RATE * 4.0) as usize;
    let mut samples_l = vec![0.0f32; total];
    let mut samples_r = vec![0.0f32; total];

    let mut pos = 0;
    while pos < total {
        let bl = BLOCK_SIZE.min(total - pos);
        engine.process_block_stereo(
            &mut samples_l[pos..pos + bl],
            &mut samples_r[pos..pos + bl],
        );
        pos += bl;
    }

    let max_val = samples_l.iter().fold(0.0f32, |a, &b| if b.abs() > a { b.abs() } else { a });
    assert!(max_val > 0.01, "Empty step locks should still produce audible output via Tick");

    write_wav_stereo(&output_path("test_motion_seq_empty_step_locks.wav"), &samples_l, &samples_r, SAMPLE_RATE as u32);
}

#[test]
fn test_delay_feedback_filter() {
    let mut delay = Delay::new(SAMPLE_RATE, 2.0);
    delay.set_time(0.2, SAMPLE_RATE);
    delay.set_feedback(0.7);
    delay.set_mix(0.5);
    delay.set_filter(0.8);

    let burst_len = (SAMPLE_RATE * 0.03) as usize;
    let total = (SAMPLE_RATE * 2.0) as usize;
    let mut out_l = vec![0.0f32; total];
    let mut out_r = vec![0.0f32; total];

    for i in 0..total {
        let input = if i < burst_len {
            let t = i as f32 / SAMPLE_RATE;
            let env = 1.0 - (i as f32 / burst_len as f32);
            ((t * 440.0 * TAU).sin()
                + 0.5 * (t * 880.0 * TAU).sin()
                + 0.3 * (t * 1760.0 * TAU).sin()) * env * 0.5
        } else {
            0.0
        };
        let (l, r) = delay.process_stereo(input, input);
        out_l[i] = l;
        out_r[i] = r;
    }

    let delay_samps = (SAMPLE_RATE * 0.2) as usize;
    let window = delay_samps / 2;

    let early_start = delay_samps;
    let early_end = (early_start + window).min(total);
    let mut early_crossings = 0u32;
    for i in (early_start + 1)..early_end {
        if (out_l[i] > 0.0) != (out_l[i - 1] > 0.0) {
            early_crossings += 1;
        }
    }

    let late_start = delay_samps * 4;
    let late_end = (late_start + window).min(total);
    let mut late_crossings = 0u32;
    for i in (late_start + 1)..late_end {
        if (out_l[i] > 0.0) != (out_l[i - 1] > 0.0) {
            late_crossings += 1;
        }
    }

    assert!(
        late_crossings < early_crossings,
        "Later delay taps should be darker: early_crossings={}, late_crossings={}",
        early_crossings, late_crossings
    );

    write_wav_stereo(
        &output_path("test_delay_feedback_filter.wav"),
        &out_l, &out_r, SAMPLE_RATE as u32,
    );
}

#[test]
fn test_delay_tempo_sync() {
    let mut delay = Delay::new(SAMPLE_RATE, 2.0);
    delay.set_sync(DelaySync::Eighth, 120.0, SAMPLE_RATE);

    let expected_samples = (SAMPLE_RATE * 0.25) as usize;
    let total = expected_samples * 3;
    let mut out = vec![0.0f32; total];

    delay.set_feedback(0.0);
    delay.set_mix(1.0);

    let (first, _) = delay.process_stereo(1.0, 1.0);
    out[0] = first;
    for i in 1..total {
        let (l, _) = delay.process_stereo(0.0, 0.0);
        out[i] = l;
    }

    let mut peak_idx = 0;
    let mut peak_val = 0.0f32;
    for i in (expected_samples - 100)..(expected_samples + 100) {
        if out[i].abs() > peak_val {
            peak_val = out[i].abs();
            peak_idx = i;
        }
    }

    let tolerance = 2;
    assert!(
        (peak_idx as i64 - expected_samples as i64).unsigned_abs() <= tolerance as u64,
        "Delay echo should appear at ~{} samples, found peak at {} (val={})",
        expected_samples, peak_idx, peak_val
    );
    assert!(peak_val > 0.5, "Echo peak should be significant, got {}", peak_val);
}

#[test]
fn test_delay_filter_bypass() {
    let mut delay_filtered = Delay::new(SAMPLE_RATE, 2.0);
    delay_filtered.set_time(0.1, SAMPLE_RATE);
    delay_filtered.set_feedback(0.5);
    delay_filtered.set_mix(0.5);
    delay_filtered.set_filter(0.0);

    let mut delay_reference = Delay::new(SAMPLE_RATE, 2.0);
    delay_reference.set_time(0.1, SAMPLE_RATE);
    delay_reference.set_feedback(0.5);
    delay_reference.set_mix(0.5);
    delay_reference.set_filter(0.0);

    let total = (SAMPLE_RATE * 0.5) as usize;
    let mut max_diff = 0.0f32;

    for i in 0..total {
        let input = if i < 100 {
            if i % 3 == 0 { 0.5 } else { -0.3 }
        } else {
            0.0
        };
        let (fl, fr) = delay_filtered.process_stereo(input, input);
        let (rl, rr) = delay_reference.process_stereo(input, input);
        let diff_l = (fl - rl).abs();
        let diff_r = (fr - rr).abs();
        if diff_l > max_diff { max_diff = diff_l; }
        if diff_r > max_diff { max_diff = diff_r; }
    }

    assert!(
        max_diff < 1e-6,
        "Filter bypass (0.0) should produce identical output, max diff was {}",
        max_diff
    );
}

#[test]
fn test_limiter_prevents_clipping() {
    let mut limiter = Limiter::new(SAMPLE_RATE);
    limiter.set_threshold(0.95);

    let mut max_out = 0.0f32;
    for i in 0..4000 {
        let t = i as f32 / SAMPLE_RATE;
        let loud = (t * 440.0 * TAU).sin() * 3.0;
        let (ol, or) = limiter.process_stereo(loud, loud * 0.8);
        let peak = if ol.abs() > or.abs() { ol.abs() } else { or.abs() };
        if peak > max_out {
            max_out = peak;
        }
    }

    assert!(
        max_out <= 0.96,
        "Limiter output should not exceed threshold, got {}",
        max_out
    );
}

#[test]
fn test_limiter_transparent_below_threshold() {
    let mut limiter = Limiter::new(SAMPLE_RATE);
    limiter.set_threshold(0.95);

    for _ in 0..44 {
        limiter.process_stereo(0.0, 0.0);
    }

    const N: usize = 2000;
    let mut inputs = [0.0f32; N];
    let mut outputs = [0.0f32; N];
    for i in 0..N {
        let t = i as f32 / SAMPLE_RATE;
        let quiet = (t * 440.0 * TAU).sin() * 0.3;
        inputs[i] = quiet;
        let (ol, _) = limiter.process_stereo(quiet, quiet);
        outputs[i] = ol;
    }

    let mut max_diff = 0.0f32;
    for i in 43..N {
        let diff = (outputs[i] - inputs[i - 43]).abs();
        if diff > max_diff {
            max_diff = diff;
        }
    }

    assert!(
        max_diff < 0.001,
        "Below-threshold signal should pass transparently, max diff was {}",
        max_diff
    );
}

#[test]
fn test_limiter_stereo_linked() {
    let mut limiter = Limiter::new(SAMPLE_RATE);
    limiter.set_threshold(0.95);

    for _ in 0..44 {
        limiter.process_stereo(0.0, 0.0);
    }

    let loud = 2.0f32;
    let quiet = 0.2f32;

    for _ in 0..200 {
        limiter.process_stereo(loud, quiet);
    }

    let (ol, or) = limiter.process_stereo(loud, quiet);

    let ratio_in = quiet / loud;
    let ratio_out = or / ol;

    assert!(
        (ratio_in - ratio_out).abs() < 0.05,
        "Stereo-linked: L/R ratio should be preserved. in={}, out={}",
        ratio_in, ratio_out
    );
}

#[test]
fn test_limiter_release_envelope() {
    let mut limiter = Limiter::new(SAMPLE_RATE);
    limiter.set_threshold(0.95);
    limiter.set_release(100.0, SAMPLE_RATE);

    for _ in 0..500 {
        limiter.process_stereo(3.0, 3.0);
    }

    const M: usize = 8000;
    let mut gains = [0.0f32; M];
    for i in 0..M {
        let probe = 0.5;
        let (ol, _) = limiter.process_stereo(probe, probe);
        gains[i] = ol / probe;
    }

    let early_gain = gains[100];
    let mid_gain = gains[2000];
    let late_gain = gains[7000];

    assert!(
        early_gain < mid_gain && mid_gain < late_gain,
        "Gain should recover gradually: early={}, mid={}, late={}",
        early_gain, mid_gain, late_gain
    );
    assert!(
        early_gain < 0.95,
        "Early gain should still be reduced, got {}",
        early_gain
    );
}

#[test]
fn test_keys_unison_stereo() {
    let mut keys = KeysModule::new();
    keys.set_param(KeysParam::VoiceMode, 0.25);
    keys.set_param(KeysParam::Cutoff, 0.4);

    keys.note_on(60, 0.8);

    let active_count = keys.voices.iter().filter(|v| v.active).count();
    assert_eq!(active_count, 8, "Unison should activate all 8 voices");

    let root_notes: Vec<u8> = keys.voices.iter().filter(|v| v.active).map(|v| v.root_note).collect();
    assert!(root_notes.iter().all(|&r| r == 60), "All voices should have root_note=60");

    let total = (SAMPLE_RATE * 0.5) as usize;
    let mut samples_l = vec![0.0f32; total];
    let mut samples_r = vec![0.0f32; total];
    let mut pos = 0;
    while pos < total {
        let bl = BLOCK_SIZE.min(total - pos);
        keys.process_block_stereo(&mut samples_l[pos..pos + bl], &mut samples_r[pos..pos + bl]);
        pos += bl;
    }

    let mut diff_sum = 0.0f32;
    for i in 0..total {
        diff_sum += (samples_l[i] - samples_r[i]).abs();
    }
    assert!(diff_sum > 1.0, "Unison stereo should produce L != R, diff_sum={}", diff_sum);

    write_wav_stereo(&output_path("test_keys_unison.wav"), &samples_l, &samples_r, SAMPLE_RATE as u32);
}

#[test]
fn test_keys_octave_voices() {
    let mut keys = KeysModule::new();
    keys.set_param(KeysParam::VoiceMode, 0.5);

    keys.note_on(60, 0.8);

    let active: Vec<u8> = keys.voices.iter().filter(|v| v.active).map(|v| v.note).collect();
    assert_eq!(active.len(), 3, "Octave should activate 3 voices, got {}", active.len());
    assert!(active.contains(&48), "Should have note 48 (octave below)");
    assert!(active.contains(&60), "Should have note 60 (root)");
    assert!(active.contains(&72), "Should have note 72 (octave above)");

    for v in &keys.voices {
        if v.active {
            assert_eq!(v.root_note, 60, "All voices should have root_note=60");
        }
    }

    keys.note_off(60);
    for v in &keys.voices {
        if v.note == 48 || v.note == 60 || v.note == 72 {
            assert!(
                v.env.stage() == EnvStage::Release || !v.active,
                "note_off(60) should release all octave voices"
            );
        }
    }
}

#[test]
fn test_keys_fifth_voices() {
    let mut keys = KeysModule::new();
    keys.set_param(KeysParam::VoiceMode, 0.75);

    keys.note_on(60, 0.8);

    let active: Vec<u8> = keys.voices.iter().filter(|v| v.active).map(|v| v.note).collect();
    assert_eq!(active.len(), 2, "Fifth should activate 2 voices, got {}", active.len());
    assert!(active.contains(&60), "Should have note 60");
    assert!(active.contains(&67), "Should have note 67 (fifth above)");

    keys.note_on(64, 0.8);
    let active_count = keys.voices.iter().filter(|v| v.active).count();
    assert_eq!(active_count, 4, "Two fifths should use 4 voices, got {}", active_count);

    keys.note_off(60);
    let still_active: Vec<u8> = keys.voices.iter()
        .filter(|v| v.active && v.env.stage() != EnvStage::Release)
        .map(|v| v.root_note)
        .collect();
    assert!(still_active.iter().all(|&r| r == 64), "Only the E4 pair should remain active");
}

#[test]
fn test_bass_3osc_default() {
    let mut bass = BassModule::new();
    bass.set_param(BassParam::Cutoff, 0.8);
    bass.set_param(BassParam::CutoffEnv, 0.2);
    bass.set_param(BassParam::Resonance, 0.2);

    let total = (SAMPLE_RATE * 2.0) as usize;
    let mut samples = vec![0.0f32; total];

    bass.note_on(45, 0.9);

    let mut pos = 0;
    while pos < total {
        let bl = BLOCK_SIZE.min(total - pos);
        bass.process_block(&mut samples[pos..pos + bl]);
        pos += bl;
    }

    let max_val = samples.iter().fold(0.0f32, |a, &b| if b.abs() > a { b.abs() } else { a });
    assert!(max_val > 0.01, "3-osc default should produce audible output, max={}", max_val);

    write_wav(&output_path("test_bass_3osc_default.wav"), &samples, SAMPLE_RATE as u32);
}

#[test]
fn test_bass_3osc_pitch_offsets() {
    let mut bass = BassModule::new();
    bass.set_param(BassParam::Cutoff, 0.6);
    bass.set_param(BassParam::CutoffEnv, 0.3);
    bass.set_param(BassParam::Resonance, 0.3);
    bass.set_param(BassParam::Osc2Pitch, (-12.0 + 24.0) / 48.0);
    bass.set_param(BassParam::Osc3Pitch, (7.0 + 24.0) / 48.0);

    let total = (SAMPLE_RATE * 2.0) as usize;
    let mut samples = vec![0.0f32; total];

    bass.note_on(45, 0.9);

    let mut pos = 0;
    while pos < total {
        let bl = BLOCK_SIZE.min(total - pos);
        bass.process_block(&mut samples[pos..pos + bl]);
        pos += bl;
    }

    let max_val = samples.iter().fold(0.0f32, |a, &b| if b.abs() > a { b.abs() } else { a });
    assert!(max_val > 0.01, "3-osc pitch offsets should produce audible output, max={}", max_val);

    write_wav(&output_path("test_bass_3osc_pitch_offsets.wav"), &samples, SAMPLE_RATE as u32);
}

#[test]
fn test_bass_3osc_waveforms() {
    let mut bass = BassModule::new();
    bass.set_param(BassParam::Cutoff, 0.5);
    bass.set_param(BassParam::CutoffEnv, 0.4);
    bass.set_param(BassParam::Resonance, 0.4);
    bass.set_param(BassParam::Osc1Wave, 0.0);
    bass.set_param(BassParam::Osc2Wave, 1.0);
    bass.set_param(BassParam::Osc3Wave, 1.0);
    bass.set_param(BassParam::Osc3Pitch, (12.0 + 24.0) / 48.0);

    let total = (SAMPLE_RATE * 2.0) as usize;
    let mut samples = vec![0.0f32; total];

    bass.note_on(45, 0.9);

    let mut pos = 0;
    while pos < total {
        let bl = BLOCK_SIZE.min(total - pos);
        bass.process_block(&mut samples[pos..pos + bl]);
        pos += bl;
    }

    let max_val = samples.iter().fold(0.0f32, |a, &b| if b.abs() > a { b.abs() } else { a });
    assert!(max_val > 0.01, "3-osc waveforms should produce audible output, max={}", max_val);

    write_wav(&output_path("test_bass_3osc_waveforms.wav"), &samples, SAMPLE_RATE as u32);
}

#[test]
fn test_bass_3osc_acid_slide() {
    let mut bass = BassModule::new();
    bass.set_param(BassParam::Cutoff, 0.15);
    bass.set_param(BassParam::CutoffEnv, 0.9);
    bass.set_param(BassParam::Resonance, 0.85);
    bass.set_param(BassParam::Glide, 0.3);
    bass.set_param(BassParam::Osc2Pitch, (-12.0 + 24.0) / 48.0);
    bass.set_param(BassParam::Osc3Pitch, (7.0 + 24.0) / 48.0);

    let sr = SAMPLE_RATE as usize;
    let total = sr * 4;
    let mut samples = vec![0.0f32; total];

    bass.note_on(45, 1.0);

    let mut pos = 0;
    while pos < total {
        if pos == sr {
            bass.slide_to(52, 1.0);
        }
        if pos == sr * 2 {
            bass.slide_to(45, 1.0);
        }

        let bl = BLOCK_SIZE.min(total - pos);
        bass.process_block(&mut samples[pos..pos + bl]);
        pos += bl;
    }

    let max_val = samples.iter().fold(0.0f32, |a, &b| if b.abs() > a { b.abs() } else { a });
    assert!(max_val > 0.01, "3-osc acid slide should produce audible output, max={}", max_val);

    write_wav(&output_path("test_bass_3osc_acid_slide.wav"), &samples, SAMPLE_RATE as u32);
}

#[test]
fn test_keys_ring_mod() {
    let mut keys = KeysModule::new();
    keys.set_param(KeysParam::VoiceMode, 1.0);
    keys.set_param(KeysParam::Cutoff, 0.6);

    keys.note_on(60, 0.8);

    let active_voice = keys.voices.iter().find(|v| v.active).unwrap();
    assert!(active_voice.ring_mod, "RingMod voice should have ring_mod=true");

    let total = (SAMPLE_RATE * 0.5) as usize;
    let mut samples = vec![0.0f32; total];
    let mut pos = 0;
    while pos < total {
        let bl = BLOCK_SIZE.min(total - pos);
        keys.process_block(&mut samples[pos..pos + bl]);
        pos += bl;
    }

    let max_val = samples.iter().fold(0.0f32, |a, &b| if b.abs() > a { b.abs() } else { a });
    assert!(max_val > 0.01, "RingMod should produce audible output, max={}", max_val);
}

#[test]
fn test_keys_unison_note_off() {
    let mut keys = KeysModule::new();
    keys.set_param(KeysParam::VoiceMode, 0.25);

    keys.note_on(60, 0.8);
    assert_eq!(keys.voices.iter().filter(|v| v.active).count(), 8);

    keys.note_off(60);

    for v in &keys.voices {
        if v.root_note == 60 {
            assert!(
                v.env.stage() == EnvStage::Release || !v.active,
                "All unison voices should be released after note_off"
            );
        }
    }
}

#[test]
fn test_fm_feedback() {
    let mut fm_fb = FmModule::new();
    fm_fb.set_ratios([1.0, 1.0, 1.0, 1.0]);
    fm_fb.set_op_envelope(0, 0.005, 2.0, 0.3, 0.5);
    fm_fb.set_op_envelope(1, 0.005, 0.01, 0.0, 0.01);
    fm_fb.set_op_envelope(2, 0.005, 0.01, 0.0, 0.01);
    fm_fb.set_op_envelope(3, 0.005, 0.01, 0.0, 0.01);
    fm_fb.set_param(FmParam::Algorithm, 1.0);
    fm_fb.set_param(FmParam::ModIndex, 0.0);
    fm_fb.set_op_feedback(0, 0.5);

    let mut fm_no = FmModule::new();
    fm_no.set_ratios([1.0, 1.0, 1.0, 1.0]);
    fm_no.set_op_envelope(0, 0.005, 2.0, 0.3, 0.5);
    fm_no.set_op_envelope(1, 0.005, 0.01, 0.0, 0.01);
    fm_no.set_op_envelope(2, 0.005, 0.01, 0.0, 0.01);
    fm_no.set_op_envelope(3, 0.005, 0.01, 0.0, 0.01);
    fm_no.set_param(FmParam::Algorithm, 1.0);
    fm_no.set_param(FmParam::ModIndex, 0.0);

    let total = (SAMPLE_RATE * 2.0) as usize;
    let mut samples_fb = vec![0.0f32; total];
    let mut samples_no = vec![0.0f32; total];

    fm_fb.note_on(60, 0.8);
    fm_no.note_on(60, 0.8);

    let mut pos = 0;
    while pos < total {
        let bl = BLOCK_SIZE.min(total - pos);
        fm_fb.process_block(&mut samples_fb[pos..pos + bl]);
        fm_no.process_block(&mut samples_no[pos..pos + bl]);
        pos += bl;
    }

    let mut diff_sum = 0.0f32;
    for i in 0..total {
        diff_sum += (samples_fb[i] - samples_no[i]).abs();
    }
    assert!(diff_sum > 1.0, "Feedback should produce different output, diff_sum={}", diff_sum);

    let max_fb = samples_fb.iter().fold(0.0f32, |a, &b| if b.abs() > a { b.abs() } else { a });
    assert!(max_fb > 0.01, "FM with feedback should produce audible output, max={}", max_fb);

    write_wav(&output_path("test_fm_feedback.wav"), &samples_fb, SAMPLE_RATE as u32);
}

#[test]
fn test_fm_velocity_mod() {
    let mut fm_low = FmModule::new();
    fm_low.set_ratios([1.0, 3.0, 1.0, 1.0]);
    fm_low.set_op_envelope(0, 0.005, 2.0, 0.3, 0.5);
    fm_low.set_op_envelope(1, 0.01, 1.5, 0.2, 0.5);
    fm_low.set_op_envelope(2, 0.005, 0.01, 0.0, 0.01);
    fm_low.set_op_envelope(3, 0.005, 0.01, 0.0, 0.01);
    fm_low.set_param(FmParam::Algorithm, 0.3);
    fm_low.set_param(FmParam::ModIndex, 0.125);

    let mut fm_high = FmModule::new();
    fm_high.set_ratios([1.0, 3.0, 1.0, 1.0]);
    fm_high.set_op_envelope(0, 0.005, 2.0, 0.3, 0.5);
    fm_high.set_op_envelope(1, 0.01, 1.5, 0.2, 0.5);
    fm_high.set_op_envelope(2, 0.005, 0.01, 0.0, 0.01);
    fm_high.set_op_envelope(3, 0.005, 0.01, 0.0, 0.01);
    fm_high.set_param(FmParam::Algorithm, 0.3);
    fm_high.set_param(FmParam::ModIndex, 0.125);

    let total = (SAMPLE_RATE * 2.0) as usize;
    let mut samples_low = vec![0.0f32; total];
    let mut samples_high = vec![0.0f32; total];

    fm_low.note_on(60, 0.3);
    fm_high.note_on(60, 1.0);

    let mut pos = 0;
    while pos < total {
        let bl = BLOCK_SIZE.min(total - pos);
        fm_low.process_block(&mut samples_low[pos..pos + bl]);
        fm_high.process_block(&mut samples_high[pos..pos + bl]);
        pos += bl;
    }

    let energy_low: f32 = samples_low.iter().map(|s| s * s).sum();
    let energy_high: f32 = samples_high.iter().map(|s| s * s).sum();
    assert!(
        energy_high > energy_low * 2.0,
        "High velocity should have significantly more energy: low={}, high={}",
        energy_low, energy_high
    );

    let max_low = samples_low.iter().fold(0.0f32, |a, &b| if b.abs() > a { b.abs() } else { a });
    assert!(max_low > 0.01, "Low velocity should still produce audible output, max={}", max_low);

    write_wav(&output_path("test_fm_velocity_mod.wav"), &samples_high, SAMPLE_RATE as u32);
}

#[test]
fn test_fm_algorithms_4_to_7() {
    let total = (SAMPLE_RATE * 1.5) as usize;
    let mut all_samples = Vec::new();

    for algo in 4..=7 {
        let mut fm = FmModule::new();
        fm.set_ratios([1.0, 2.0, 3.0, 4.0]);
        fm.set_op_envelope(0, 0.001, 1.5, 0.2, 0.3);
        fm.set_op_envelope(1, 0.005, 1.5, 0.2, 0.3);
        fm.set_op_envelope(2, 0.005, 1.5, 0.2, 0.3);
        fm.set_op_envelope(3, 0.008, 1.5, 0.2, 0.3);
        fm.set_param(FmParam::Algorithm, (algo as f32 + 0.1) / 7.0);
        fm.set_param(FmParam::ModIndex, 0.1875);

        let mut samples = vec![0.0f32; total];
        fm.note_on(60, 0.8);

        let mut pos = 0;
        while pos < total {
            let bl = BLOCK_SIZE.min(total - pos);
            fm.process_block(&mut samples[pos..pos + bl]);
            pos += bl;
        }

        let max_val = samples.iter().fold(0.0f32, |a, &b| if b.abs() > a { b.abs() } else { a });
        assert!(max_val > 0.01, "Algorithm {} should produce audible output, max={}", algo, max_val);

        all_samples.extend_from_slice(&samples);
    }

    write_wav(&output_path("test_fm_algorithms_4_to_7.wav"), &all_samples, SAMPLE_RATE as u32);
}

#[test]
fn test_fm_waveforms() {
    let total = (SAMPLE_RATE * 1.5) as usize;
    let mut all_samples = Vec::new();

    let waveforms = [
        (FmWaveform::Sine, "Sine"),
        (FmWaveform::HalfSine, "HalfSine"),
        (FmWaveform::AbsSine, "AbsSine"),
        (FmWaveform::QuarterSine, "QuarterSine"),
    ];

    let mut prev_samples: Option<Vec<f32>> = None;

    for (wf_val, name) in &waveforms {
        let mut fm = FmModule::new();
        fm.set_ratios([1.0, 2.0, 1.0, 1.0]);
        fm.set_op_envelope(0, 0.005, 2.0, 0.3, 0.5);
        fm.set_op_envelope(1, 0.01, 1.5, 0.2, 0.5);
        fm.set_op_envelope(2, 0.005, 0.01, 0.0, 0.01);
        fm.set_op_envelope(3, 0.005, 0.01, 0.0, 0.01);
        fm.set_param(FmParam::Algorithm, 0.3);
        fm.set_param(FmParam::ModIndex, 0.1875);

        let wf_norm = match wf_val {
            FmWaveform::Sine => 0.0,
            FmWaveform::HalfSine => 0.5,
            FmWaveform::AbsSine => 0.75,
            FmWaveform::QuarterSine => 1.0,
        };
        fm.set_param(FmParam::Waveform, wf_norm);

        let mut samples = vec![0.0f32; total];
        fm.note_on(60, 0.8);

        let mut pos = 0;
        while pos < total {
            let bl = BLOCK_SIZE.min(total - pos);
            fm.process_block(&mut samples[pos..pos + bl]);
            pos += bl;
        }

        let max_val = samples.iter().fold(0.0f32, |a, &b| if b.abs() > a { b.abs() } else { a });
        assert!(max_val > 0.01, "{} waveform should produce audible output, max={}", name, max_val);

        if let Some(ref prev) = prev_samples {
            let mut diff_sum = 0.0f32;
            for i in 0..total {
                diff_sum += (samples[i] - prev[i]).abs();
            }
            assert!(diff_sum > 0.1, "{} should differ from previous waveform, diff_sum={}", name, diff_sum);
        }
        prev_samples = Some(samples.clone());
        all_samples.extend_from_slice(&samples);
    }

    write_wav(&output_path("test_fm_waveforms.wav"), &all_samples, SAMPLE_RATE as u32);
}

#[test]
fn test_fm_chorus() {
    let mut fm = FmModule::new();
    fm.set_ratios([1.0, 3.0, 1.0, 1.0]);
    fm.set_op_envelope(0, 0.001, 3.0, 0.0, 0.5);
    fm.set_op_envelope(1, 0.01, 2.5, 0.0, 0.5);
    fm.set_op_envelope(2, 0.001, 0.01, 0.0, 0.01);
    fm.set_op_envelope(3, 0.001, 0.01, 0.0, 0.01);
    fm.set_param(FmParam::Algorithm, 0.3);
    fm.set_param(FmParam::ModIndex, 0.1875);
    fm.set_param(FmParam::ChorusMix, 0.5);

    let mut fm_dry = FmModule::new();
    fm_dry.set_ratios([1.0, 3.0, 1.0, 1.0]);
    fm_dry.set_op_envelope(0, 0.001, 3.0, 0.0, 0.5);
    fm_dry.set_op_envelope(1, 0.01, 2.5, 0.0, 0.5);
    fm_dry.set_op_envelope(2, 0.001, 0.01, 0.0, 0.01);
    fm_dry.set_op_envelope(3, 0.001, 0.01, 0.0, 0.01);
    fm_dry.set_param(FmParam::Algorithm, 0.3);
    fm_dry.set_param(FmParam::ModIndex, 0.1875);

    let total = (SAMPLE_RATE * 3.0) as usize;
    let mut samples_chorus = vec![0.0f32; total];
    let mut samples_dry = vec![0.0f32; total];

    fm.note_on(72, 0.8);
    fm_dry.note_on(72, 0.8);

    let mut pos = 0;
    while pos < total {
        let bl = BLOCK_SIZE.min(total - pos);
        fm.process_block(&mut samples_chorus[pos..pos + bl]);
        fm_dry.process_block(&mut samples_dry[pos..pos + bl]);
        pos += bl;
    }

    let mut diff_sum = 0.0f32;
    for i in 0..total {
        diff_sum += (samples_chorus[i] - samples_dry[i]).abs();
    }
    assert!(diff_sum > 1.0, "Chorus should produce different output, diff_sum={}", diff_sum);

    let max_val = samples_chorus.iter().fold(0.0f32, |a, &b| if b.abs() > a { b.abs() } else { a });
    assert!(max_val > 0.01, "FM with chorus should produce audible output, max={}", max_val);

    write_wav(&output_path("test_fm_chorus.wav"), &samples_chorus, SAMPLE_RATE as u32);
}

#[test]
fn test_fm_adsr_lifecycle() {
    let mut fm = FmModule::new();
    fm.set_ratios([1.0, 2.0, 1.0, 1.0]);
    fm.set_op_envelope(0, 0.05, 0.3, 0.5, 0.4);
    fm.set_op_envelope(1, 0.08, 0.5, 0.3, 0.3);
    fm.set_op_envelope(2, 0.005, 0.01, 0.0, 0.01);
    fm.set_op_envelope(3, 0.005, 0.01, 0.0, 0.01);
    fm.set_param(FmParam::Algorithm, 0.3);
    fm.set_param(FmParam::ModIndex, 0.125);

    let note_off_sample = (SAMPLE_RATE * 1.5) as usize;
    let total = (SAMPLE_RATE * 3.0) as usize;
    let mut samples = vec![0.0f32; total];

    fm.note_on(60, 0.8);

    let mut pos = 0;
    while pos < total {
        if pos <= note_off_sample && pos + BLOCK_SIZE > note_off_sample {
            fm.note_off(60);
        }
        let bl = BLOCK_SIZE.min(total - pos);
        fm.process_block(&mut samples[pos..pos + bl]);
        pos += bl;
    }

    let spm = SAMPLE_RATE as usize;

    let attack_start = samples[0..spm/100].iter().fold(0.0f32, |a, &b| a.max(b.abs()));
    let attack_peak = samples[spm/40..spm/10].iter().fold(0.0f32, |a, &b| a.max(b.abs()));
    assert!(attack_peak > attack_start * 2.0,
        "Attack should ramp up: start={:.4}, peak region={:.4}", attack_start, attack_peak);

    let sustain_rms_1: f32 = {
        let s = (0.8 * SAMPLE_RATE) as usize;
        let e = (1.0 * SAMPLE_RATE) as usize;
        (samples[s..e].iter().map(|x| x * x).sum::<f32>() / (e - s) as f32).sqrt()
    };
    let sustain_rms_2: f32 = {
        let s = (1.2 * SAMPLE_RATE) as usize;
        let e = (1.4 * SAMPLE_RATE) as usize;
        (samples[s..e].iter().map(|x| x * x).sum::<f32>() / (e - s) as f32).sqrt()
    };
    let sustain_ratio = if sustain_rms_1 > sustain_rms_2 {
        sustain_rms_1 / sustain_rms_2
    } else {
        sustain_rms_2 / sustain_rms_1
    };
    assert!(sustain_ratio < 1.5,
        "Sustain should be stable: rms_1={:.4}, rms_2={:.4}, ratio={:.2}", sustain_rms_1, sustain_rms_2, sustain_ratio);

    let pre_release_rms: f32 = {
        let s = (1.3 * SAMPLE_RATE) as usize;
        let e = (1.5 * SAMPLE_RATE) as usize;
        (samples[s..e].iter().map(|x| x * x).sum::<f32>() / (e - s) as f32).sqrt()
    };
    let post_release_rms: f32 = {
        let s = (2.0 * SAMPLE_RATE) as usize;
        let e = (2.5 * SAMPLE_RATE) as usize;
        (samples[s..e].iter().map(|x| x * x).sum::<f32>() / (e - s) as f32).sqrt()
    };
    assert!(post_release_rms < pre_release_rms * 0.3,
        "Release should decay: pre={:.4}, post={:.4}", pre_release_rms, post_release_rms);

    let tail_rms: f32 = {
        let s = (2.5 * SAMPLE_RATE) as usize;
        let e = (3.0 * SAMPLE_RATE) as usize;
        (samples[s..e].iter().map(|x| x * x).sum::<f32>() / (e - s) as f32).sqrt()
    };
    assert!(tail_rms < 0.01,
        "Tail should be near silence after release: rms={:.4}", tail_rms);

    write_wav(&output_path("test_fm_adsr_lifecycle.wav"), &samples, SAMPLE_RATE as u32);
}

#[test]
fn test_sequencer_swing_audio() {
    let mut engine = Engine::new();
    engine.set_bpm(120.0);
    engine.sequencer.set_swing(0.62);

    for i in 0..16 {
        engine.sequencer.steps[i] = Step::new(36, 1.0, 0.5);
    }
    engine.sequencer.num_steps = 16;
    engine.sequencer.start();

    let total = (SAMPLE_RATE * 4.0) as usize;
    let mut samples_l = vec![0.0f32; total];
    let mut samples_r = vec![0.0f32; total];

    let mut pos = 0;
    while pos < total {
        let bl = BLOCK_SIZE.min(total - pos);
        engine.process_block_stereo(
            &mut samples_l[pos..pos + bl],
            &mut samples_r[pos..pos + bl],
        );
        pos += bl;
    }

    let max_val = samples_l.iter().fold(0.0f32, |a, &b| if b.abs() > a { b.abs() } else { a });
    assert!(max_val > 0.01, "Swing audio test should produce audible output, max={}", max_val);

    write_wav_stereo(&output_path("test_sequencer_swing.wav"), &samples_l, &samples_r, SAMPLE_RATE as u32);
}

#[test]
fn test_reverb_pre_delay_wav() {
    let mut reverb = Reverb::new(SAMPLE_RATE);
    reverb.set_room_size(0.7);
    reverb.set_damping(0.4);
    reverb.set_mix(1.0);
    reverb.set_pre_delay(20.0);

    let pre_delay_samples = (20.0 * 44.1) as usize;

    let total = (SAMPLE_RATE * 2.0) as usize;
    let mut samples_l = vec![0.0f32; total];
    let mut samples_r = vec![0.0f32; total];

    for i in 0..total {
        let input = if i < 10 { 0.8 } else { 0.0 };
        let (l, r) = reverb.process_stereo(input);
        samples_l[i] = l;
        samples_r[i] = r;
    }

    let check_end = pre_delay_samples.saturating_sub(50);
    let mut max_early = 0.0f32;
    for i in 10..check_end {
        if samples_l[i].abs() > max_early { max_early = samples_l[i].abs(); }
        if samples_r[i].abs() > max_early { max_early = samples_r[i].abs(); }
    }
    assert!(
        max_early < 0.01,
        "First {} samples should be quiet with 20ms pre-delay, max={}",
        check_end, max_early
    );

    let mut max_late = 0.0f32;
    for i in (pre_delay_samples + 2000)..(pre_delay_samples + 8000).min(total) {
        if samples_l[i].abs() > max_late { max_late = samples_l[i].abs(); }
    }
    assert!(max_late > 0.001, "Should have reverb tail after pre-delay, max={}", max_late);

    write_wav_stereo(&output_path("test_reverb_pre_delay.wav"), &samples_l, &samples_r, SAMPLE_RATE as u32);
}

#[test]
fn test_dattorro_reverb_wav() {
    let mut reverb = DattorroReverb::new(SAMPLE_RATE);
    reverb.set_room_size(0.6);
    reverb.set_damping(0.6);
    reverb.set_mix(0.35);

    let mut keys = KeysModule::new();
    keys.set_param(KeysParam::Cutoff, 0.35);
    keys.set_param(KeysParam::ChorusMix, 0.2);
    keys.note_on(57, 0.7);
    keys.note_on(60, 0.7);
    keys.note_on(64, 0.7);

    let total = (SAMPLE_RATE * 4.0) as usize;
    let mut samples_l = vec![0.0f32; total];
    let mut samples_r = vec![0.0f32; total];

    let mut pos = 0;
    while pos < total {
        let bl = BLOCK_SIZE.min(total - pos);
        let mut buf = vec![0.0f32; bl];
        keys.process_block(&mut buf);

        if pos < (SAMPLE_RATE as usize) && pos + bl >= (SAMPLE_RATE as usize) {
            keys.note_off(57);
            keys.note_off(60);
            keys.note_off(64);
        }

        for i in 0..bl {
            let (l, r) = reverb.process_stereo(buf[i]);
            samples_l[pos + i] = l;
            samples_r[pos + i] = r;
        }
        pos += bl;
    }

    let mut diff_sum = 0.0f32;
    let check_start = (SAMPLE_RATE * 1.5) as usize;
    let check_end = (SAMPLE_RATE * 2.5) as usize;
    for i in check_start..check_end.min(total) {
        diff_sum += (samples_l[i] - samples_r[i]).abs();
    }
    let avg_diff = diff_sum / (check_end - check_start) as f32;
    assert!(
        avg_diff > 0.0001,
        "Dattorro should produce stereo decorrelation, avg L-R diff={}",
        avg_diff
    );

    write_wav_stereo(&output_path("test_dattorro_reverb.wav"), &samples_l, &samples_r, SAMPLE_RATE as u32);
}

#[test]
fn test_bass_keytrack_brighter() {
    fn render_bass_note(note: u8, keytrack: f32) -> Vec<f32> {
        let mut bass = BassModule::new();
        bass.set_param(BassParam::Cutoff, 0.15);
        bass.set_param(BassParam::CutoffEnv, 0.0);
        bass.set_param(BassParam::Resonance, 0.3);
        bass.set_param(BassParam::Keytrack, keytrack);

        let total = (SAMPLE_RATE * 0.5) as usize;
        let mut samples = vec![0.0f32; total];
        bass.note_on(note, 0.9);

        let mut pos = 0;
        while pos < total {
            let bl = BLOCK_SIZE.min(total - pos);
            bass.process_block(&mut samples[pos..pos + bl]);
            pos += bl;
        }
        samples
    }

    fn zero_crossings(buf: &[f32]) -> u32 {
        let mut count = 0u32;
        for i in 1..buf.len() {
            if (buf[i] > 0.0) != (buf[i - 1] > 0.0) {
                count += 1;
            }
        }
        count
    }

    let low_note = 33;
    let high_note = 57;

    let low_samples = render_bass_note(low_note, 1.0);
    let high_samples = render_bass_note(high_note, 1.0);

    let start = (SAMPLE_RATE * 0.1) as usize;
    let end = (SAMPLE_RATE * 0.4) as usize;

    let low_zc = zero_crossings(&low_samples[start..end]);
    let high_zc = zero_crossings(&high_samples[start..end]);

    assert!(
        high_zc > low_zc,
        "Keytrack=1.0: high note (A3) should have more zero crossings than low note (A1): high={}, low={}",
        high_zc, low_zc
    );
}

#[test]
fn test_bass_keytrack_audio() {
    let mut bass = BassModule::new();
    bass.set_param(BassParam::Cutoff, 0.15);
    bass.set_param(BassParam::CutoffEnv, 0.5);
    bass.set_param(BassParam::Resonance, 0.5);
    bass.set_param(BassParam::Keytrack, 0.7);

    let notes = [33u8, 45, 57, 69];
    let note_dur = (SAMPLE_RATE * 1.0) as usize;
    let total = note_dur * notes.len();
    let mut samples = vec![0.0f32; total];

    for (idx, &note) in notes.iter().enumerate() {
        bass.note_on(note, 0.9);
        let start = idx * note_dur;
        let end = start + note_dur;
        let mut pos = start;
        while pos < end {
            let bl = BLOCK_SIZE.min(end - pos);
            bass.process_block(&mut samples[pos..pos + bl]);
            pos += bl;
        }
        bass.note_off(note);
    }

    let max_val = samples.iter().fold(0.0f32, |a, &b| if b.abs() > a { b.abs() } else { a });
    assert!(max_val > 0.01, "Keytrack audio test should produce audible output, max={}", max_val);

    write_wav(&output_path("test_bass_keytrack.wav"), &samples, SAMPLE_RATE as u32);
}

#[test]
fn test_tilt_eq_spectral() {
    let len = (SAMPLE_RATE * 0.5) as usize;

    let mut signal = vec![0.0f32; len];
    for i in 0..len {
        let t = i as f32 / SAMPLE_RATE;
        signal[i] = (TAU * 200.0 * t).sin() * 0.5
                  + (TAU * 2000.0 * t).sin() * 0.3
                  + (TAU * 8000.0 * t).sin() * 0.2;
    }

    let mut dark_eq = TiltEq::new(SAMPLE_RATE);
    dark_eq.set_tilt(1.0);
    let mut dark_crossings = 0u32;
    let mut prev = 0.0f32;
    for i in 0..len {
        let out = dark_eq.process_stereo(signal[i], signal[i]).0;
        if i > 100 {
            if (out >= 0.0) != (prev >= 0.0) {
                dark_crossings += 1;
            }
        }
        prev = out;
    }

    let mut bright_eq = TiltEq::new(SAMPLE_RATE);
    bright_eq.set_tilt(-1.0);
    let mut bright_crossings = 0u32;
    prev = 0.0;
    for i in 0..len {
        let out = bright_eq.process_stereo(signal[i], signal[i]).0;
        if i > 100 {
            if (out >= 0.0) != (prev >= 0.0) {
                bright_crossings += 1;
            }
        }
        prev = out;
    }

    assert!(
        bright_crossings > dark_crossings,
        "Bright tilt should have more zero crossings ({}) than dark ({})",
        bright_crossings, dark_crossings
    );
}

#[test]
fn test_three_band_eq_boost() {
    let len = (SAMPLE_RATE * 0.2) as usize;

    let mut signal = vec![0.0f32; len];
    for i in 0..len {
        let t = i as f32 / SAMPLE_RATE;
        signal[i] = (TAU * 100.0 * t).sin() * 0.5;
    }

    let mut flat_eq = ThreeBandEq::new(SAMPLE_RATE);
    let mut flat_energy = 0.0f32;
    for i in 0..len {
        let out = flat_eq.process_stereo(signal[i], signal[i]).0;
        if i > 200 { flat_energy += out * out; }
    }

    let mut boost_eq = ThreeBandEq::new(SAMPLE_RATE);
    boost_eq.set_low(12.0);
    let mut boost_energy = 0.0f32;
    for i in 0..len {
        let out = boost_eq.process_stereo(signal[i], signal[i]).0;
        if i > 200 { boost_energy += out * out; }
    }

    assert!(
        boost_energy > flat_energy * 2.0,
        "Low shelf +12dB should significantly boost bass: boosted={:.4}, flat={:.4}",
        boost_energy, flat_energy
    );
}

#[test]
fn test_eq_flat_transparent() {
    let len = (SAMPLE_RATE * 0.1) as usize;

    let mut signal = vec![0.0f32; len];
    for i in 0..len {
        let t = i as f32 / SAMPLE_RATE;
        signal[i] = (TAU * 440.0 * t).sin() * 0.5
                  + (TAU * 1000.0 * t).sin() * 0.3;
    }

    let mut tilt = TiltEq::new(SAMPLE_RATE);
    let mut eq3 = ThreeBandEq::new(SAMPLE_RATE);

    let warmup = 200;
    let mut max_diff = 0.0f32;
    for i in 0..len {
        let (tl, _) = tilt.process_stereo(signal[i], signal[i]);
        let (out, _) = eq3.process_stereo(tl, tl);
        if i >= warmup {
            let diff = (out - signal[i]).abs();
            if diff > max_diff { max_diff = diff; }
        }
    }

    assert!(
        max_diff < 0.01,
        "Flat EQ should be transparent, max diff = {:.6}",
        max_diff
    );
}

#[test]
fn test_eq_audio() {
    let mut engine = Engine::new();
    engine.set_bpm(120.0);

    engine.bass.set_param(BassParam::Cutoff, 0.4);
    engine.bass.set_param(BassParam::CutoffEnv, 0.3);
    engine.bass.set_param(BassParam::Resonance, 0.3);

    engine.keys.set_param(KeysParam::Cutoff, 0.5);
    engine.keys.note_on(57, 0.7);
    engine.keys.note_on(60, 0.7);
    engine.keys.note_on(64, 0.7);
    engine.bass.note_on(45, 0.9);

    let sr = SAMPLE_RATE as usize;
    let total = sr * 6;

    let mut samples_l = vec![0.0f32; total];
    let mut samples_r = vec![0.0f32; total];

    let mut pos = 0;
    while pos < total {
        let t = pos as f32 / SAMPLE_RATE;

        let tilt = if t < 1.5 {
            t / 1.5
        } else if t < 3.0 {
            1.0 - (t - 1.5) / 0.75
        } else if t < 4.5 {
            -1.0 + (t - 3.0) / 1.5
        } else {
            0.0
        };
        engine.set_tilt_eq(tilt);

        if t >= 2.0 && t < 3.0 {
            engine.set_eq_low(8.0);
        } else {
            engine.set_eq_low(0.0);
        }
        if t >= 4.0 && t < 5.0 {
            engine.set_eq_high(8.0);
        } else {
            engine.set_eq_high(0.0);
        }

        let bl = BLOCK_SIZE.min(total - pos);
        engine.process_block_stereo(
            &mut samples_l[pos..pos + bl],
            &mut samples_r[pos..pos + bl],
        );
        pos += bl;
    }

    let max_val = samples_l.iter().fold(0.0f32, |a, &b| if b.abs() > a { b.abs() } else { a });
    assert!(max_val > 0.01, "EQ audio test should produce audible output, max={}", max_val);

    write_wav_stereo(&output_path("test_eq_audio.wav"), &samples_l, &samples_r, SAMPLE_RATE as u32);
}

#[test]
fn test_compressor_gain_reduction() {
    let mut comp = Compressor::new(SAMPLE_RATE);
    comp.set_threshold(-12.0);
    comp.set_ratio(4.0);

    let input_amp = 0.8;

    for i in 0..4000 {
        let t = i as f32 / SAMPLE_RATE;
        let signal = (t * 440.0 * TAU).sin() * input_amp;
        comp.process_stereo(signal, signal);
    }

    let mut max_out = 0.0f32;
    for i in 4000..8000 {
        let t = i as f32 / SAMPLE_RATE;
        let signal = (t * 440.0 * TAU).sin() * input_amp;
        let (ol, _) = comp.process_stereo(signal, signal);
        let abs_ol = ol.abs();
        if abs_ol > max_out {
            max_out = abs_ol;
        }
    }

    assert!(
        max_out < input_amp * 0.9,
        "Compressor should reduce gain: input_amp={}, max_out={}",
        input_amp, max_out
    );
    assert!(
        max_out > 0.05,
        "Compressor should still pass signal, got {}",
        max_out
    );
}

#[test]
fn test_compressor_transparent_below_threshold() {
    let mut comp = Compressor::new(SAMPLE_RATE);
    comp.set_threshold(-6.0);

    for _ in 0..100 {
        comp.process_stereo(0.0, 0.0);
    }

    let input_amp = 0.1;
    let mut max_diff = 0.0f32;
    for i in 0..2000 {
        let t = i as f32 / SAMPLE_RATE;
        let signal = (t * 440.0 * TAU).sin() * input_amp;
        let (ol, _) = comp.process_stereo(signal, signal);
        let diff = (ol - signal).abs();
        if diff > max_diff {
            max_diff = diff;
        }
    }

    assert!(
        max_diff < 0.001,
        "Below-threshold signal should pass transparently, max diff was {}",
        max_diff
    );
}

#[test]
fn test_sidechain_bass_ducking() {
    let mut engine_sc = Engine::new();
    engine_sc.set_bpm(120.0);
    engine_sc.set_sidechain_amount(1.0);
    engine_sc.bass.note_on(33, 0.9);
    engine_sc.beats.note_on(36, 1.0);

    let mut engine_no_sc = Engine::new();
    engine_no_sc.set_bpm(120.0);
    engine_no_sc.set_sidechain_amount(0.0);
    engine_no_sc.bass.note_on(33, 0.9);
    engine_no_sc.beats.note_on(36, 1.0);

    let total = (SAMPLE_RATE * 0.5) as usize;
    let mut sc_l = vec![0.0f32; total];
    let mut sc_r = vec![0.0f32; total];
    let mut no_sc_l = vec![0.0f32; total];
    let mut no_sc_r = vec![0.0f32; total];

    let mut pos = 0;
    while pos < total {
        let bl = BLOCK_SIZE.min(total - pos);
        engine_sc.process_block_stereo(&mut sc_l[pos..pos+bl], &mut sc_r[pos..pos+bl]);
        engine_no_sc.process_block_stereo(&mut no_sc_l[pos..pos+bl], &mut no_sc_r[pos..pos+bl]);
        pos += bl;
    }

    let region_start = 100;
    let region_end = 2000;
    let mut diff_sum = 0.0f32;
    let mut energy_sum = 0.0f32;
    for i in region_start..region_end {
        diff_sum += (sc_l[i] - no_sc_l[i]) * (sc_l[i] - no_sc_l[i]);
        energy_sum += no_sc_l[i] * no_sc_l[i];
    }
    let relative_diff = diff_sum / (energy_sum + 1e-10);

    assert!(
        relative_diff > 0.005,
        "Sidechain should alter bass waveform during kick: relative_diff={}",
        relative_diff
    );
}

#[test]
fn test_compressor_sidechain_audio() {
    let mut engine = Engine::new();
    engine.set_bpm(120.0);
    engine.set_compressor_threshold(-12.0);
    engine.set_compressor_ratio(4.0);

    let mut ctx = HarmonyContext::new(45, Scale::Minor);
    ctx.set_chord_degree(0);
    engine.set_harmony(ctx);

    engine.bass.set_param(BassParam::Cutoff, 0.4);
    engine.bass.note_on(33, 0.9);

    for i in 0..16 {
        let step = if i % 4 == 0 {
            Step::new(36, 1.0, 0.5)
        } else {
            Step::empty()
        };
        engine.sequencer.steps[i] = step;
    }
    engine.sequencer.num_steps = 16;
    engine.sequencer.start();

    let total = (SAMPLE_RATE * 8.0) as usize;
    let mut samples_l = vec![0.0f32; total];
    let mut samples_r = vec![0.0f32; total];

    let mut pos = 0;
    while pos < total {
        let t = pos as f32 / SAMPLE_RATE;
        engine.set_sidechain_amount(t / 8.0);

        let bl = BLOCK_SIZE.min(total - pos);
        engine.process_block_stereo(
            &mut samples_l[pos..pos + bl],
            &mut samples_r[pos..pos + bl],
        );
        pos += bl;
    }

    let max_val = samples_l.iter().fold(0.0f32, |a, &b| if b.abs() > a { b.abs() } else { a });
    assert!(max_val > 0.01, "Compressor/sidechain audio should produce output, max={}", max_val);

    write_wav_stereo(&output_path("test_compressor_sidechain.wav"), &samples_l, &samples_r, SAMPLE_RATE as u32);
}

#[test]
fn test_pitch_bend_shifts_frequency() {
    fn zero_crossings(buf: &[f32]) -> u32 {
        let mut count = 0u32;
        for i in 1..buf.len() {
            if (buf[i] > 0.0) != (buf[i - 1] > 0.0) {
                count += 1;
            }
        }
        count
    }

    let total = (SAMPLE_RATE * 1.0) as usize;

    let mut engine_flat = Engine::new();
    engine_flat.bass.note_on(45, 0.9);
    let mut flat_buf = vec![0.0f32; total];
    let mut pos = 0;
    while pos < total {
        let bl = BLOCK_SIZE.min(total - pos);
        engine_flat.bass.process_block(&mut flat_buf[pos..pos + bl]);
        pos += bl;
    }

    let mut engine_bent = Engine::new();
    engine_bent.set_pitch_bend(1.0);
    let pb_ratio = 2.0f32.powf(1.0 * 2.0 / 12.0);
    engine_bent.bass.pitch_bend_ratio = pb_ratio;
    engine_bent.bass.note_on(45, 0.9);
    let mut bent_buf = vec![0.0f32; total];
    pos = 0;
    while pos < total {
        let bl = BLOCK_SIZE.min(total - pos);
        engine_bent.bass.process_block(&mut bent_buf[pos..pos + bl]);
        pos += bl;
    }

    let start = (SAMPLE_RATE * 0.2) as usize;
    let end = (SAMPLE_RATE * 0.8) as usize;
    let flat_zc = zero_crossings(&flat_buf[start..end]);
    let bent_zc = zero_crossings(&bent_buf[start..end]);

    let ratio = bent_zc as f32 / flat_zc as f32;
    assert!(
        ratio > 1.08 && ratio < 1.18,
        "Bent (+2st) should have ~12% more ZC: flat={}, bent={}, ratio={:.3}",
        flat_zc, bent_zc, ratio
    );
}

#[test]
fn test_pitch_bend_zero_transparent() {
    let total = (SAMPLE_RATE * 0.5) as usize;

    let mut engine_a = Engine::new();
    engine_a.bass.note_on(45, 0.9);
    let mut buf_a = vec![0.0f32; total];
    let mut pos = 0;
    while pos < total {
        let bl = BLOCK_SIZE.min(total - pos);
        engine_a.bass.process_block(&mut buf_a[pos..pos + bl]);
        pos += bl;
    }

    let mut engine_b = Engine::new();
    engine_b.set_pitch_bend(0.0);
    engine_b.bass.pitch_bend_ratio = 1.0;
    engine_b.bass.note_on(45, 0.9);
    let mut buf_b = vec![0.0f32; total];
    pos = 0;
    while pos < total {
        let bl = BLOCK_SIZE.min(total - pos);
        engine_b.bass.process_block(&mut buf_b[pos..pos + bl]);
        pos += bl;
    }

    let mut max_diff = 0.0f32;
    for i in 0..total {
        let diff = (buf_a[i] - buf_b[i]).abs();
        if diff > max_diff { max_diff = diff; }
    }
    assert!(
        max_diff < 1e-6,
        "Pitch bend=0 should be transparent, max diff={}",
        max_diff
    );
}

#[test]
fn test_vibrato_delayed_onset() {
    fn zero_crossings(buf: &[f32]) -> u32 {
        let mut count = 0u32;
        for i in 1..buf.len() {
            if (buf[i] > 0.0) != (buf[i - 1] > 0.0) {
                count += 1;
            }
        }
        count
    }

    let mut bass = BassModule::new();
    bass.set_param(BassParam::Cutoff, 0.8);
    bass.set_param(BassParam::CutoffEnv, 0.0);
    bass.set_param(BassParam::VibratoDepth, 0.6);
    bass.set_param(BassParam::VibratoRate, 0.47);
    bass.note_on(45, 0.9);

    let total = (SAMPLE_RATE * 1.0) as usize;
    let mut samples = vec![0.0f32; total];
    let mut pos = 0;
    while pos < total {
        let bl = BLOCK_SIZE.min(total - pos);
        bass.process_block(&mut samples[pos..pos + bl]);
        pos += bl;
    }

    let window = (SAMPLE_RATE * 0.05) as usize;
    let early_end = (SAMPLE_RATE * 0.25) as usize;
    let late_start = (SAMPLE_RATE * 0.4) as usize;
    let late_end = (SAMPLE_RATE * 0.9) as usize;

    let mut early_zcrs = Vec::new();
    let mut i = (SAMPLE_RATE * 0.05) as usize;
    while i + window <= early_end {
        early_zcrs.push(zero_crossings(&samples[i..i + window]));
        i += window;
    }

    let mut late_zcrs = Vec::new();
    i = late_start;
    while i + window <= late_end {
        late_zcrs.push(zero_crossings(&samples[i..i + window]));
        i += window;
    }

    fn variance(vals: &[u32]) -> f32 {
        if vals.is_empty() { return 0.0; }
        let mean = vals.iter().sum::<u32>() as f32 / vals.len() as f32;
        vals.iter().map(|&v| { let d = v as f32 - mean; d * d }).sum::<f32>() / vals.len() as f32
    }

    let early_var = variance(&early_zcrs);
    let late_var = variance(&late_zcrs);

    assert!(
        late_var > early_var,
        "Late vibrato should have more ZCR variance (pitch variation): early_var={:.1}, late_var={:.1}",
        early_var, late_var
    );
}

#[test]
fn test_pitch_bend_vibrato_audio() {
    let mut engine = Engine::new();
    engine.set_bpm(120.0);
    engine.bass.set_param(BassParam::Cutoff, 0.5);
    engine.bass.set_param(BassParam::CutoffEnv, 0.2);
    engine.bass.set_param(BassParam::VibratoDepth, 0.6);
    engine.bass.set_param(BassParam::VibratoRate, 0.47);
    engine.bass.note_on(45, 0.9);

    let total = (SAMPLE_RATE * 4.0) as usize;
    let mut samples_l = vec![0.0f32; total];
    let mut samples_r = vec![0.0f32; total];

    let mut pos = 0;
    while pos < total {
        let t = pos as f32 / SAMPLE_RATE;
        engine.set_pitch_bend(t / 2.0 - 1.0);

        let bl = BLOCK_SIZE.min(total - pos);
        engine.process_block_stereo(
            &mut samples_l[pos..pos + bl],
            &mut samples_r[pos..pos + bl],
        );
        pos += bl;
    }

    let max_val = samples_l.iter().fold(0.0f32, |a, &b| if b.abs() > a { b.abs() } else { a });
    assert!(max_val > 0.01, "Pitch bend + vibrato audio should be audible, max={}", max_val);

    write_wav_stereo(&output_path("test_pitch_bend_vibrato.wav"), &samples_l, &samples_r, SAMPLE_RATE as u32);
}
