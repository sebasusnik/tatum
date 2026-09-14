use std::fs;
use std::process;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        print_usage();
        process::exit(1);
    }

    match args[1].as_str() {
        "render" => cmd_render(&args[2..]),
        "check" => cmd_check(&args[2..]),
        "params" => cmd_params(&args[2..]),
        "help" | "--help" | "-h" => print_usage(),
        other => {
            eprintln!("unknown command: {}", other);
            print_usage();
            process::exit(1);
        }
    }
}

fn print_usage() {
    eprintln!("synth - DSL-powered synthesizer

USAGE:
    synth render <song.synth> [-o output.wav] [--bars N]
    synth check <song.synth>
    synth params [bass|fm|keys|beats|track|fx] [--json]

COMMANDS:
    render    Parse, compile, and render a .synth file to WAV
    check     Parse and validate a .synth file (no audio output)
    params    Print the module parameter reference (markdown, or JSON with --json)
    help      Show this help
");
}

fn cmd_params(args: &[String]) {
    use synth_core::params::{self, ModuleKind};

    let json = args.iter().any(|a| a == "--json");
    if args.iter().any(|a| a == "fx" || a == "nodes") {
        if json { print!("{}", synth_core::nodes::json()); } else { print!("{}", synth_core::nodes::markdown()); }
        return;
    }
    if args.iter().any(|a| a == "track") {
        if json { print!("{}", params::track_json()); } else { print!("{}", params::track_markdown()); }
        return;
    }
    let kinds: Vec<ModuleKind> = match args.iter().find(|a| !a.starts_with("--")) {
        Some(name) => match ModuleKind::from_str(name) {
            Some(k) => vec![k],
            None => {
                eprintln!("error: unknown module '{}' (expected bass, fm, keys, or beats)", name);
                std::process::exit(2);
            }
        },
        None => ModuleKind::ALL.to_vec(),
    };
    if json {
        print!("{}", params::json(&kinds));
    } else {
        print!("{}", params::markdown(&kinds));
        if args.iter().all(|a| a.starts_with("--")) {
            print!("{}", params::track_markdown());
            print!("{}", synth_core::nodes::markdown());
        }
    }
}

fn cmd_check(args: &[String]) {
    if args.is_empty() {
        eprintln!("error: missing input file");
        eprintln!("usage: synth check <song.synth>");
        process::exit(1);
    }

    let path = &args[0];
    let source = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot read '{}': {}", path, e);
            process::exit(1);
        }
    };

    // Parse
    let ast = match synth_core::dsl::parse(&source) {
        Ok(ast) => {
            eprintln!("  parse OK");
            ast
        }
        Err(errs) => {
            eprintln!("parse errors:");
            for e in &errs {
                eprintln!("  line {}: {}", e.line, e.message);
            }
            process::exit(1);
        }
    };

    // Compile
    match synth_core::dsl::compiler::compile(&ast) {
        Ok(compiled) => {
            eprintln!("  compile OK");
            eprintln!("  {} instruments, {} patterns, {} tracks, {} buses",
                compiled.instruments.len(),
                compiled.patterns.len(),
                compiled.tracks.len(),
                compiled.buses.len(),
            );
            if !compiled.scenes.is_empty() {
                eprintln!("  {} scenes, arrangement: {} entries",
                    compiled.scenes.len(),
                    compiled.arrangement.len(),
                );
            }
            eprintln!("  tempo: {} BPM", compiled.globals.tempo);
            let lints = synth_core::dsl::lint::lint_song(&ast);
            if !lints.is_empty() {
                eprintln!("design warnings:");
                for l in &lints {
                    eprintln!("  [{}] {}\n      {}", l.code, l.message, l.hint);
                }
            }
        }
        Err(errs) => {
            eprintln!("compile errors:");
            for e in &errs {
                eprintln!("  {}", e);
            }
            process::exit(1);
        }
    }

    eprintln!("{}: OK", path);
}

fn cmd_render(args: &[String]) {
    if args.is_empty() {
        eprintln!("error: missing input file");
        eprintln!("usage: synth render <song.synth> [-o output.wav] [--bars N]");
        process::exit(1);
    }

    let path = &args[0];
    let mut output_path = String::from("output.wav");
    let mut bars: Option<u32> = None;

    // Parse flags
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-o" | "--output" => {
                i += 1;
                if i < args.len() {
                    output_path = args[i].clone();
                }
            }
            "--bars" => {
                i += 1;
                if i < args.len() {
                    bars = args[i].parse().ok();
                }
            }
            _ => {}
        }
        i += 1;
    }

    let source = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot read '{}': {}", path, e);
            process::exit(1);
        }
    };

    eprintln!("loading {}...", path);

    let mut engine = match synth_core::song_engine::SongEngine::from_source(&source) {
        Ok(e) => e,
        Err(msg) => {
            eprintln!("{}", msg);
            process::exit(1);
        }
    };

    let render_bars = bars.unwrap_or_else(|| {
        let arr = engine.arrangement_bars();
        if arr > 0 { arr } else { 4 }
    });

    eprintln!("rendering {} bars at {} BPM...", render_bars, engine.tempo());

    let (out_l, out_r) = engine.render(render_bars);

    if out_l.iter().chain(out_r.iter()).any(|v| !v.is_finite()) {
        eprintln!("error: render produced non-finite samples (an effect is unstable)");
        process::exit(1);
    }
    eprintln!("writing {} ({} samples, {:.1}s)...",
        output_path,
        out_l.len(),
        out_l.len() as f32 / 44100.0,
    );

    write_wav_stereo(&output_path, &out_l, &out_r, 44100);
    eprintln!("done.");
}

fn write_wav_stereo(path: &str, samples_l: &[f32], samples_r: &[f32], sample_rate: u32) {
    let bytes = synth_core::wav::encode_stereo_16(samples_l, samples_r, sample_rate);
    fs::write(path, bytes).expect("Failed to write WAV file");
}
