use tatum_core::modules::bass::{BassModule, BassParam};
use tatum_core::modules::fm::{FmModule, FmParam};
use tatum_core::modules::keys::{KeysModule, KeysParam};
use tatum_core::modules::beats::{BeatsModule, BeatsParam};
use tatum_core::{Module, SAMPLE_RATE, BLOCK_SIZE};
use tatum_core::math;

fn analyze(name: &str, samples: &[f32]) {
    let max_val = samples.iter().fold(0.0f32, |a, &b| a.max(math::abs(b)));
    let max_idx = samples.iter().position(|&s| math::abs(s) >= max_val - 0.0001).unwrap_or(0);
    let clip = samples.iter().filter(|&&s| math::abs(s) > 0.95).count();

    // RMS in windows
    let windows = [
        (0, 10, "attack 0-10ms"),
        (10, 50, "early 10-50ms"),
        (50, 200, "body 50-200ms"),
        (200, 500, "sustain 200-500ms"),
        (500, 1000, "late 500ms-1s"),
    ];

    println!("\n=== {} ===", name);
    print!("  Peak: {:.4} at {:.1}ms", max_val, max_idx as f32 / SAMPLE_RATE * 1000.0);
    if clip > 0 {
        print!("  *** {} samples > 0.95 ***", clip);
    }
    println!();

    for (start_ms, end_ms, label) in &windows {
        let s = (*start_ms as f32 / 1000.0 * SAMPLE_RATE) as usize;
        let e = ((*end_ms as f32 / 1000.0 * SAMPLE_RATE) as usize).min(samples.len());
        if s < samples.len() && e > s {
            let rms: f32 = (samples[s..e].iter().map(|x| x * x).sum::<f32>() / (e - s) as f32).sqrt();
            let bar_len = (rms * 40.0) as usize;
            let bar: String = "=".repeat(bar_len);
            println!("  {:18} RMS={:.4} {}", label, rms, bar);
        }
    }
}

fn render_module(module: &mut dyn Module, note: u8, vel: f32, dur_s: f32) -> Vec<f32> {
    let total = (SAMPLE_RATE * dur_s) as usize;
    let mut samples = vec![0.0f32; total];
    module.note_on(note, vel);
    let mut pos = 0;
    while pos < total {
        let bl = BLOCK_SIZE.min(total - pos);
        module.process_block(&mut samples[pos..pos + bl]);
        pos += bl;
    }
    samples
}

fn render_with_noteoff(module: &mut dyn Module, note: u8, vel: f32, hold_s: f32, release_s: f32) -> Vec<f32> {
    let hold = (SAMPLE_RATE * hold_s) as usize;
    let release = (SAMPLE_RATE * release_s) as usize;
    let total = hold + release;
    let mut samples = vec![0.0f32; total];
    module.note_on(note, vel);
    let mut pos = 0;
    while pos < hold {
        let bl = BLOCK_SIZE.min(hold - pos);
        module.process_block(&mut samples[pos..pos + bl]);
        pos += bl;
    }
    module.note_off(note);
    while pos < total {
        let bl = BLOCK_SIZE.min(total - pos);
        module.process_block(&mut samples[pos..pos + bl]);
        pos += bl;
    }
    samples
}

fn main() {
    println!("╔══════════════════════════════════════════════════════╗");
    println!("║          SYNTH-CORE MODULE LEVEL ANALYSIS           ║");
    println!("╚══════════════════════════════════════════════════════╝");

    // ── BASS MODULE ──
    println!("\n── BASS MODULE ──");

    // Bass basic: A2, vel 0.9, open filter
    let mut bass = BassModule::new();
    bass.set_param(BassParam::Cutoff, 0.8);
    bass.set_param(BassParam::CutoffEnv, 0.2);
    bass.set_param(BassParam::Resonance, 0.2);
    let s = render_with_noteoff(&mut bass, 45, 0.9, 1.5, 0.5);
    analyze("Bass basic (A2, vel=0.9, cutoff=0.8)", &s);

    // Bass acid: high reso, cutoff env
    let mut bass = BassModule::new();
    bass.set_param(BassParam::Cutoff, 0.15);
    bass.set_param(BassParam::CutoffEnv, 0.9);
    bass.set_param(BassParam::Resonance, 0.85);
    let s = render_with_noteoff(&mut bass, 45, 1.0, 1.5, 0.5);
    analyze("Bass acid (A2, vel=1.0, reso=0.85)", &s);

    // Bass 3osc
    let mut bass = BassModule::new();
    bass.set_param(BassParam::Cutoff, 0.6);
    bass.set_param(BassParam::CutoffEnv, 0.3);
    bass.set_param(BassParam::Resonance, 0.3);
    bass.set_param(BassParam::Osc2Pitch, 0.3); // -12st
    bass.set_param(BassParam::Osc3Pitch, 0.65); // +7st
    let s = render_with_noteoff(&mut bass, 45, 0.9, 1.5, 0.5);
    analyze("Bass 3osc (A2, vel=0.9, 3 oscs)", &s);

    // ── KEYS MODULE ──
    println!("\n── KEYS MODULE ──");

    // Keys pad: Am chord
    let mut keys = KeysModule::new();
    keys.set_param(KeysParam::Cutoff, 0.25);
    keys.set_param(KeysParam::Detune, 0.15);
    keys.set_param(KeysParam::ChorusMix, 0.4);
    keys.note_on(57, 0.7);
    keys.note_on(60, 0.7);
    keys.note_on(64, 0.7);
    let total = (SAMPLE_RATE * 2.0) as usize;
    let mut s = vec![0.0f32; total];
    let mut pos = 0;
    while pos < total {
        let bl = BLOCK_SIZE.min(total - pos);
        keys.process_block(&mut s[pos..pos + bl]);
        pos += bl;
    }
    analyze("Keys pad (Am chord, vel=0.7, 3 notes)", &s);

    // Keys single note for fair comparison
    let mut keys = KeysModule::new();
    keys.set_param(KeysParam::Cutoff, 0.4);
    let s = render_with_noteoff(&mut keys, 60, 0.8, 1.5, 0.5);
    analyze("Keys single (C4, vel=0.8)", &s);

    // ── FM MODULE ──
    println!("\n── FM MODULE ──");

    // FM bell
    let mut fm = FmModule::new();
    fm.set_ratios([1.0, 3.0, 1.0, 1.0]);
    fm.set_op_envelope(0, 0.005, 3.0, 0.0, 0.5);
    fm.set_op_envelope(1, 0.01, 2.0, 0.0, 0.5);
    fm.set_param(FmParam::Algorithm, 0.3);
    fm.set_param(FmParam::ModIndex, 0.125);
    let s = render_module(&mut fm, 72, 0.8, 2.0);
    analyze("FM bell (C5, vel=0.8, mi=1.0)", &s);

    // FM bass
    let mut fm = FmModule::new();
    fm.set_ratios([1.0, 1.0, 1.0, 1.0]);
    fm.set_op_envelope(0, 0.005, 0.5, 0.3, 0.2);
    fm.set_op_envelope(1, 0.01, 0.3, 0.1, 0.1);
    fm.set_param(FmParam::Algorithm, 0.3);
    fm.set_param(FmParam::ModIndex, 0.125);
    let s = render_module(&mut fm, 40, 0.8, 2.0);
    analyze("FM bass (E2, vel=0.8, mi=1.0)", &s);

    // FM feedback
    let mut fm = FmModule::new();
    fm.set_ratios([1.0, 1.0, 1.0, 1.0]);
    fm.set_op_envelope(0, 0.005, 2.0, 0.3, 0.5);
    fm.set_op_envelope(1, 0.005, 0.01, 0.0, 0.01);
    fm.set_param(FmParam::Algorithm, 1.0);
    fm.set_param(FmParam::ModIndex, 0.0);
    fm.set_op_feedback(0, 0.5);
    let s = render_module(&mut fm, 60, 0.8, 2.0);
    analyze("FM feedback (C4, vel=0.8, fb=0.5)", &s);

    // FM ADSR
    let mut fm = FmModule::new();
    fm.set_ratios([1.0, 2.0, 1.0, 1.0]);
    fm.set_op_envelope(0, 0.05, 0.3, 0.5, 0.4);
    fm.set_op_envelope(1, 0.08, 0.5, 0.3, 0.3);
    fm.set_param(FmParam::Algorithm, 0.3);
    fm.set_param(FmParam::ModIndex, 0.125);
    let s = render_with_noteoff(&mut fm, 60, 0.8, 1.5, 0.5);
    analyze("FM ADSR (C4, vel=0.8, A=50ms D=300ms S=0.5 R=400ms)", &s);

    // ── BEATS MODULE ──
    println!("\n── BEATS MODULE ──");

    // Individual drum hits
    let mut beats = BeatsModule::new();
    let s = render_module(&mut beats, 36, 1.0, 0.5);
    analyze("Beats kick (vel=1.0)", &s);

    let mut beats = BeatsModule::new();
    let s = render_module(&mut beats, 38, 0.9, 0.5);
    analyze("Beats snare (vel=0.9)", &s);

    let mut beats = BeatsModule::new();
    let s = render_module(&mut beats, 42, 0.6, 0.5);
    analyze("Beats hihat (vel=0.6)", &s);

    // Kick with click transient
    let mut beats = BeatsModule::new();
    beats.set_param(BeatsParam::KickClick, 1.0);
    let s = render_module(&mut beats, 36, 1.0, 0.5);
    analyze("Beats kick+click (vel=1.0, click=1.0)", &s);

    // Toms
    let mut beats = BeatsModule::new();
    let s = render_module(&mut beats, 43, 0.9, 0.5);
    analyze("Beats low tom (vel=0.9)", &s);

    let mut beats = BeatsModule::new();
    let s = render_module(&mut beats, 45, 0.9, 0.5);
    analyze("Beats mid tom (vel=0.9)", &s);

    let mut beats = BeatsModule::new();
    let s = render_module(&mut beats, 47, 0.9, 0.5);
    analyze("Beats high tom (vel=0.9)", &s);

    // Crash
    let mut beats = BeatsModule::new();
    let s = render_module(&mut beats, 49, 0.9, 1.5);
    analyze("Beats crash (vel=0.9)", &s);

    // Full pattern for comparison
    let mut beats = BeatsModule::new();
    let step_samples = (SAMPLE_RATE * 60.0 / 120.0 / 4.0) as usize;
    let total_steps = 16;
    let total = step_samples * total_steps;
    let mut s = vec![0.0f32; total];
    let kick_pattern:  [bool; 16] = [true,false,false,false,true,false,false,false,true,false,false,false,true,false,false,false];
    let snare_pattern: [bool; 16] = [false,false,false,false,true,false,false,false,false,false,false,false,true,false,false,false];
    let hh_pattern:    [bool; 16] = [true,false,true,false,true,false,true,false,true,false,true,false,true,false,true,false];
    for step in 0..total_steps {
        let start = step * step_samples;
        if kick_pattern[step] { beats.note_on(36, 1.0); }
        if snare_pattern[step] { beats.note_on(38, 0.9); }
        if hh_pattern[step] { beats.note_on(42, 0.6); }
        let end = (start + step_samples).min(total);
        let mut p = start;
        while p < end {
            let bl = BLOCK_SIZE.min(end - p);
            beats.process_block(&mut s[p..p + bl]);
            p += bl;
        }
    }
    analyze("Beats full pattern (kick+snare+hh)", &s);

    // ── SUMMARY ──
    println!("\n╔══════════════════════════════════════════════════════╗");
    println!("║                    LEVEL SUMMARY                    ║");
    println!("╠══════════════════════════════════════════════════════╣");

    // Re-render all at same conditions for fair comparison: single note, vel=0.8, C4 (60)
    println!("║  Fair comparison: single note C4, vel=0.8, 1.5s    ║");
    println!("╚══════════════════════════════════════════════════════╝");

    let mut bass = BassModule::new();
    bass.set_param(BassParam::Cutoff, 0.5);
    bass.set_param(BassParam::CutoffEnv, 0.2);
    bass.set_param(BassParam::Resonance, 0.2);
    let s_bass = render_module(&mut bass, 60, 0.8, 1.5);

    let mut keys = KeysModule::new();
    keys.set_param(KeysParam::Cutoff, 0.4);
    let s_keys = render_module(&mut keys, 60, 0.8, 1.5);

    let mut fm = FmModule::new();
    fm.set_ratios([1.0, 2.0, 1.0, 1.0]);
    fm.set_op_envelope(0, 0.005, 2.0, 0.3, 0.5);
    fm.set_op_envelope(1, 0.01, 1.5, 0.2, 0.5);
    fm.set_param(FmParam::Algorithm, 0.3);
    fm.set_param(FmParam::ModIndex, 0.125);
    let s_fm = render_module(&mut fm, 60, 0.8, 1.5);

    let mut beats = BeatsModule::new();
    let s_kick = render_module(&mut beats, 36, 0.8, 0.5);

    let mut beats = BeatsModule::new();
    let s_tom = render_module(&mut beats, 45, 0.8, 0.5);

    let mut beats = BeatsModule::new();
    let s_crash = render_module(&mut beats, 49, 0.8, 1.5);

    let modules: [(&str, &[f32]); 6] = [
        ("Bass", &s_bass),
        ("Keys", &s_keys),
        ("FM", &s_fm),
        ("Kick", &s_kick),
        ("Tom", &s_tom),
        ("Crash", &s_crash),
    ];

    for (name, samples) in &modules {
        let peak = samples.iter().fold(0.0f32, |a, &b| a.max(math::abs(b)));
        let rms: f32 = (samples.iter().map(|x| x * x).sum::<f32>() / samples.len() as f32).sqrt();
        let clip = samples.iter().filter(|&&s| math::abs(s) > 0.95).count();
        let peak_bar_len = (peak * 40.0) as usize;
        let rms_bar_len = (rms * 40.0) as usize;
        println!("  {:6} Peak={:.4} {} RMS={:.4} {}{}",
            name, peak, "#".repeat(peak_bar_len),
            rms, "=".repeat(rms_bar_len),
            if clip > 0 { format!("  *** {} > 0.95 ***", clip) } else { String::new() }
        );
    }
}
