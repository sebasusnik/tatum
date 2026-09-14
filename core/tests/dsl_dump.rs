//! Dump the compiled DSL output for funk_replica.synth
//! to compare with what song_funk_envelope_filter.rs does directly.
//!
//! Run: cargo test --test dsl_dump -- --nocapture

use synth_core::dsl;
use synth_core::dsl::compiler::{compile, CompiledStep};

const FUNK_SOURCE: &str = include_str!("../../examples/funk_replica.synth");

#[test]
fn dump_funk_dsl_compiled() {
    let ast = dsl::parse(FUNK_SOURCE).expect("parse failed");
    let compiled = compile(&ast).expect("compile failed");

    println!("═══════════════════════════════════════════════");
    println!("  DSL COMPILED OUTPUT: funk_replica.synth");
    println!("═══════════════════════════════════════════════");

    // Globals
    println!("\n── GLOBALS ──");
    println!("  tempo:     {}", compiled.globals.tempo);
    println!("  meter:     {}/{}", compiled.globals.meter.0, compiled.globals.meter.1);
    println!("  sidechain: {}", compiled.globals.sidechain);

    // Instruments
    println!("\n── INSTRUMENTS ({}) ──", compiled.instruments.len());
    for (i, name) in compiled.instrument_names.iter().enumerate() {
        let kind = &compiled.instruments[i];
        println!("  [{}] \"{}\"", i, name);
        if let Some(tmpl) = kind.as_graph() {
            println!("      type:        graph");
            println!("      nodes:       {}", tmpl.node_count);
            println!("      output_gain: {}", tmpl.output_gain);
            println!("      osc_count:   {}", tmpl.osc_count);
            println!("      env_count:   {}", tmpl.env_count);
            println!("      filter_count:{}", tmpl.filter_count);
            for ni in 0..tmpl.node_count as usize {
                println!("        node[{}]: {:?}", ni, tmpl.specs[ni]);
            }
        } else {
            println!("      type:        module");
        }
    }

    // Patterns — show step data
    println!("\n── PATTERNS ({}) ──", compiled.patterns.len());
    for pat in &compiled.patterns {
        println!("  \"{}\" ({} steps, {} per row)", pat.name, pat.steps.len(), pat.steps_per_row);
        for (si, step) in pat.steps.iter().enumerate() {
            let _row = si / pat.steps_per_row;
            let col = si % pat.steps_per_row;
            if col == 0 && si > 0 { println!(); }
            match step {
                CompiledStep::NoteOn { midi_note, velocity, plock, .. } => {
                    print!("    [{:2}] NoteOn(midi={:3}, vel={:.2}", si, midi_note, velocity);
                    if plock.cutoff.is_some() || plock.env_depth.is_some() || plock.resonance.is_some() || plock.gate.is_some() {
                        print!(" plock[");
                        if let Some(c) = plock.cutoff { print!("cutoff={}", c); }
                        if let Some(e) = plock.env_depth { print!(" edepth={}", e); }
                        if let Some(r) = plock.resonance { print!(" res={}", r); }
                        if let Some(g) = plock.gate { print!(" gate={}", g); }
                        print!("]");
                    }
                    print!(")");
                }
                CompiledStep::DrumHit { velocity, plock, .. } => {
                    print!("    [{:2}] DrumHit(vel={:.2}", si, velocity);
                    if plock.cutoff.is_some() || plock.env_depth.is_some() || plock.resonance.is_some() || plock.gate.is_some() {
                        print!(" plock[");
                        if let Some(c) = plock.cutoff { print!("cutoff={}", c); }
                        if let Some(e) = plock.env_depth { print!(" edepth={}", e); }
                        if let Some(r) = plock.resonance { print!(" res={}", r); }
                        if let Some(g) = plock.gate { print!(" gate={}", g); }
                        print!("]");
                    }
                    print!(")");
                }
                CompiledStep::Rest => {
                    print!("    [{:2}] Rest", si);
                }
                CompiledStep::Tie => {
                    print!("    [{:2}] Tie", si);
                }
                CompiledStep::Chord { notes, count, .. } => {
                    let chord_str: Vec<String> = (0..*count as usize)
                        .map(|i| format!("{}@{:.2}", notes[i].midi_note, notes[i].velocity))
                        .collect();
                    print!("    [{:2}] Chord [{}]", si, chord_str.join(" "));
                }
            }
        }
        println!();
    }

    // Tracks
    println!("\n── TRACKS ({}) ──", compiled.tracks.len());
    for t in &compiled.tracks {
        println!("  \"{}\"", t.name);
        println!("      instrument: [{}] \"{}\"", t.instrument_idx, compiled.instrument_names[t.instrument_idx]);
        println!("      pattern:    [{}] \"{}\"", t.pattern_idx, compiled.patterns[t.pattern_idx].name);
        println!("      velocity:   {}", t.velocity);
        println!("      level:      {}", t.level);
        println!("      pan:        {}", t.pan);
        println!("      gate:       {}", t.gate);
        println!("      insert_fx:  {:?}", t.insert_fx);
        println!("      bus_send:   {:?}", t.bus_send);
        println!("      to_master:  {}", t.to_master);
    }

    // Buses
    println!("\n── BUSES ({}) ──", compiled.buses.len());
    for b in &compiled.buses {
        println!("  \"{}\"", b.name);
        for spec in &b.fx_chain {
            println!("      {:?}", spec);
        }
    }

    // Master
    println!("\n── MASTER FX ──");
    for spec in &compiled.master.fx_chain {
        println!("      {:?}", spec);
    }

    // Scenes
    println!("\n── SCENES ({}) ──", compiled.scenes.len());
    for scene in &compiled.scenes {
        println!("  \"{}\" (tempo: {:?})", scene.name, scene.tempo);
        for t in &scene.tracks {
            println!("      track \"{}\" → inst[{}] pat[{}] vel={} lvl={} gate={} pan={}",
                t.name, t.instrument_idx, t.pattern_idx, t.velocity, t.level, t.gate, t.pan);
        }
    }

    // Arrangement
    println!("\n── ARRANGEMENT ──");
    for (scene_idx, repeat) in &compiled.arrangement {
        println!("  scene[{}] \"{}\" x{}", scene_idx, compiled.scenes[*scene_idx].name, repeat);
    }

    // Now print what the RUST funk test does for comparison
    println!("\n\n═══════════════════════════════════════════════");
    println!("  RUST FUNK TEST COMPARISON (hardcoded values)");
    println!("═══════════════════════════════════════════════");

    println!("\n── BASS MODULE (Rust test) ──");
    println!("  Uses BassModule (NOT graph instrument)");
    println!("  - 3 oscillators, osc1=saw, osc2=sub octave");
    println!("  - Cutoff:    0.08 (normalized, ~= 80 Hz)");
    println!("  - CutoffEnv: 0.7 (BIG envelope sweep)");
    println!("  - Resonance: 0.75");
    println!("  - Glide:     0.15");
    println!("  - Keytrack:  0.5");
    println!("  - Attack:    0.0 (instant)");
    println!("  - Output multiplier: 1.8x (hardcoded in BassModule)");
    println!("  - PER-STEP parameter modulation:");
    println!("    step[0]: A1(21) vel=0.90 gate=0.40 cutoff_env=0.75 res=0.80");
    println!("    step[3]: A1(21) vel=0.55 gate=0.25 cutoff_env=0.50 res=0.65");
    println!("    step[5]: G1(19) vel=0.70 gate=0.30 cutoff_env=0.65 res=0.75");
    println!("    step[8]: A1(21) vel=0.80 gate=0.35 cutoff_env=0.70 res=0.78");
    println!("    step[10]:A1(21) vel=0.40 gate=0.20 cutoff_env=0.40 res=0.55");
    println!("    step[12]:C2(24) vel=0.50 gate=0.25 cutoff_env=0.55 res=0.70");
    println!("    step[14]:A1(21) vel=0.65 gate=0.30 cutoff_env=0.60 res=0.72");
    println!("  → Each step changes BOTH velocity AND filter character!");
    println!("  → Rests release the note completely (staccato)");

    println!("\n── DSL BASS INSTRUMENT ──");
    let bass_idx = compiled.instrument_names.iter().position(|n| n == "bass" || n == "funk_bass").unwrap();
    let bass_kind = &compiled.instruments[bass_idx];
    if let Some(bass_tmpl) = bass_kind.as_graph() {
        println!("  Type: graph");
        println!("  Graph nodes:");
        for ni in 0..bass_tmpl.node_count as usize {
            println!("    [{:2}] {:?}", ni, bass_tmpl.specs[ni]);
        }
        println!("  output_gain: {}", bass_tmpl.output_gain);
        println!("  → Filter params are FIXED at compile time");
        println!("  → Only velocity varies per step (no filter modulation)");
        println!("  → Gate is same for ALL steps (track-level, not per-step)");
    } else {
        println!("  Type: module (BassModule/FmModule/KeysModule)");
    }

    println!("\n── KEY DIFFERENCES ──");
    println!("  1. BassModule has per-step filter modulation (cutoff_env + resonance)");
    println!("     DSL instrument has FIXED filter params (ladder(400, 0.75, edepth=5000))");
    println!("  2. BassModule has per-step gate length (0.2-0.4)");
    println!("     DSL has single gate per track (0.35)");
    println!("  3. BassModule has glide/portamento between notes");
    println!("     DSL graph has no glide support");
    println!("  4. BassModule uses normalized cutoff (0.08 = ~80Hz base, swept up by env)");
    println!("     DSL ladder uses absolute Hz (400 Hz cutoff + 5000 Hz env depth)");
    println!("  5. Rust test does real-time automation (cutoff sweep in bridge, wah closing in outro)");
    println!("     DSL has no automation — all params frozen at compile time");
    println!("  6. Drums: Rust test uses Beats module (multi-voice per track: kick+snare+hat in one)");
    println!("     DSL needs separate instrument + track per drum sound");
}
