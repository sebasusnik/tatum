mod include;
mod live;
mod resample;


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
        "play" => live::cmd(&args[2..], false),
        "watch" => live::cmd(&args[2..], true),
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
    synth play <song.synth> [--device <name>] [--rate <hz>]
    synth watch <song.synth> [--device <name>] [--rate <hz>]

COMMANDS:
    render    Parse, compile, and render a .synth file to WAV
    check     Parse and validate a .synth file (no audio output)
    params    Print the module parameter reference (markdown, or JSON with --json)
    play      Play a .synth file on the audio device until it ends or you type q
    watch     Play, and re-evaluate the file every time it is saved: value edits
              apply at once, anything else takes over on the next bar; a save
              that does not compile is reported and the last good version keeps
              playing. --list-devices shows the output devices. The engine runs
              at 44.1 kHz; a device that only offers another rate (Bluetooth:
              48 kHz) gets the output resampled. --rate forces the device rate.
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
    let source = match include::Source::load(std::path::Path::new(path)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {}", e);
            process::exit(1);
        }
    };
    if source.files.len() > 1 {
        eprintln!("  {} files via `use`", source.files.len());
    }

    // Parse
    let ast = match synth_core::dsl::parse(&source.text) {
        Ok(ast) => {
            eprintln!("  parse OK");
            ast
        }
        Err(errs) => {
            source.print_errors(&synth_core::song_engine::DslError::Parse(errs));
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
            source.print_errors(&synth_core::song_engine::DslError::Compile(errs));
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

    let source = match include::Source::load(std::path::Path::new(path)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {}", e);
            process::exit(1);
        }
    };

    eprintln!("loading {}...", path);

    let mut engine = match synth_core::song_engine::SongEngine::try_from_source(&source.text) {
        Ok(e) => e,
        Err(err) => {
            source.print_errors(&err);
            process::exit(1);
        }
    };

    let render_bars = bars.unwrap_or_else(|| {
        let arr = engine.arrangement_bars();
        if arr > 0 { arr } else { 4 }
    });

    eprintln!("rendering {} bars at {} BPM...", render_bars, engine.tempo());

    engine.set_band_metering(true);
    engine.reset_meters();
    let (out_l, out_r) = engine.render(render_bars);

    if out_l.iter().chain(out_r.iter()).any(|v| !v.is_finite()) {
        eprintln!("error: render produced non-finite samples (an effect is unstable)");
        process::exit(1);
    }
    // Mix report: per-track levels, so a buried or silent track is visible.
    // Peak and RMS both: they rank tracks differently, and reading only RMS
    // hides a sparse bass under a continuous pad.
    let loudest = (0..engine.track_count()).fold(0.0f32, |a, i| a.max(engine.track_rms(i)));
    let loudest_peak = (0..engine.track_count()).fold(0.0f32, |a, i| a.max(engine.track_peak(i)));
    if engine.track_count() > 0 && loudest > 0.0 {
        eprintln!("mix (post level/pan, pre master):");
        eprintln!("  {:<12} {:>6} {:>8} {:>8} {:>8} {:>6}  band",
            "track", "peak", "rms", "rms dB", "peak dB", "crest");
        for i in 0..engine.track_count() {
            let rms = engine.track_rms(i);
            let peak = engine.track_peak(i);
            let rel = 20.0 * (rms.max(1e-6) / loudest).log10();
            let rel_peak = 20.0 * (peak.max(1e-6) / loudest_peak.max(1e-6)).log10();
            let flag = if rms <= 0.0 { "  SILENT" } else if rel < -30.0 { "  buried" } else { "" };
            let band = engine.track_dominant_band(i).map_or("-", |b| synth_core::analysis::BAND_NAMES[b]);
            eprintln!("  {:<12} {:>6.3} {:>8.4} {:>+8.1} {:>+8.1} {:>6.1}  {}{}",
                engine.track_name(i), peak, rms, rel, rel_peak,
                synth_core::analysis::crest(peak, rms), band, flag);
        }
        for i in 0..engine.bus_count() {
            let (p, rms) = (engine.bus_peak(i), engine.bus_rms(i));
            eprintln!("  bus {:<8} {:>6.3} {:>8.4} {:>8} {:>8} {:>6.1}",
                engine.bus_name(i), p, rms, "", "", synth_core::analysis::crest(p, rms));
        }
    }
    // What the master chain costs in dynamics: raising the master gain looks
    // free on the peak meter because the limiter catches it, and the punch
    // leaves with the transients.
    let (in_peak, in_rms) = engine.master_input_peak_rms();
    let (out_peak, out_rms) = synth_core::analysis::peak_rms(&out_l, &out_r);
    let (crest_in, crest_out) = (
        synth_core::analysis::crest(in_peak, in_rms),
        synth_core::analysis::crest(out_peak, out_rms),
    );
    if crest_in > 0.0 {
        let change = 20.0 * (crest_out / crest_in).log10();
        eprintln!("master: peak {:.2} in -> {:.2} out | crest {:.1} -> {:.1} ({:+.1} dB){}",
            in_peak, out_peak, crest_in, crest_out, change,
            if change < -3.0 { "  the master chain is eating transients" } else { "" });
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

/// A counting allocator for the tests of what runs in the audio callback.
/// Counts per thread: tests run in parallel, and another test's `Vec` must
/// not be charged to the one under measurement.
#[cfg(test)]
mod test_alloc {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    thread_local! {
        pub static ALLOCS: Cell<usize> = const { Cell::new(0) };
        pub static COUNTING: Cell<bool> = const { Cell::new(false) };
    }

    struct Counting;

    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            COUNTING.with(|c| if c.get() { ALLOCS.with(|a| a.set(a.get() + 1)); });
            unsafe { System.alloc(layout) }
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            unsafe { System.dealloc(ptr, layout) }
        }
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            COUNTING.with(|c| if c.get() { ALLOCS.with(|a| a.set(a.get() + 1)); });
            unsafe { System.realloc(ptr, layout, new_size) }
        }
    }

    #[global_allocator]
    static A: Counting = Counting;
}
