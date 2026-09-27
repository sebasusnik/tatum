mod audit;
mod debug;
mod include;
mod live;
mod midi;
mod set;
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
        "audit" => audit::cmd(&args[2..]),
        "debug" => debug::cmd(&args[2..]),
        "params" => cmd_params(&args[2..]),
        "set" => set::cmd(&args[2..]),
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
    eprintln!("tatum - DSL-powered synthesizer

USAGE:
    tatum render <song.synth> [-o output.wav] [--bars N] [--solo a,b] [--mute c]
    tatum check <song.synth>
    tatum params [bass|fm|keys|beats|track|fx] [--json]
    tatum play <song.synth> [--device <name>] [--rate <hz>] [--midi <name>]
    tatum set render <dir> [-o out.wav] | set check <dir> | set next <dir> <file>
    tatum audit <song.synth> [--bars N] [--json] [--strict]
    tatum debug <song.synth> [--bars N | A-B] [--dry] [-o dir] [--solo a,b] [--mute c]
    tatum watch <song.synth> [--device <name>] [--rate <hz>] [--midi <name>]

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
              A MIDI controller plays the song through its `midi {{ }}` block:
              knobs and faders on parameters, the keys on a track, the pads on
              drums. Every MIDI input is read unless --midi names one;
              --list-midi shows them.
    audit     Render every tonal track on its own and dry, and report per
              note how much of its energy is NOT at a harmonic of the note
              the pattern asked for. Two comparisons come out of that: a note
              against the other notes of the same voice, which finds one note
              going wrong, and the same voice with and without one effect,
              which finds an effect that dirties all of it. The absolute
              number is a fingerprint of a timbre and means nothing next to
              another track's. --bars limits how much is rendered, --json
              prints the same numbers for a script, --strict exits 1 if
              anything is reported.
    debug     Render once with every part kept apart, as it sits in the mix,
              and write into test_output/debug/<song>/ (or -o): a WAV and a
              spectrogram per track, bus and send return; sheet.png with all
              of them stacked on one time axis over the mix; report.txt, which
              lists clicks (and which fall on a section change), tracks that
              do not go quiet between notes, energy below 25 Hz and fizz above
              16 kHz, with the bar each happens at, plus the mix per section:
              each part's level against the loudest, and which parts sit level
              with each other on top of the same range (sheet.png shows the
              same as a crowded strip); and zoom.*.png, the worst moment of
              each part up close, wave and spectrum. --bars 17-24
              zooms in; --dry adds each track before its insert chain.
    set       A live set: a directory of numbered .synth files, each the whole
              rig at a moment. `render` walks them with real hot swaps into one
              continuous WAV; `check` validates every step and reports the arc;
              `next` is the gate a proposed step has to pass. How long a step
              holds travels in the file: `# set: bars=32 phase=build energy=5`.
    help      Show this help

    render, play, watch and debug take --solo and --mute with track names
    (comma-separated, or the flag repeated). A muted track is taken to level 0
    everywhere, level automation included; a muted kick still drives the
    sidechain, so what is left pumps the way it does in the mix.
");
}

fn cmd_params(args: &[String]) {
    use tatum_core::params::{self, ModuleKind};

    let json = args.iter().any(|a| a == "--json");
    if args.iter().any(|a| a == "fx" || a == "nodes") {
        if json { print!("{}", tatum_core::nodes::json()); } else { print!("{}", tatum_core::nodes::markdown()); }
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
            print!("{}", tatum_core::nodes::markdown());
        }
    }
}

fn cmd_check(args: &[String]) {
    if args.is_empty() {
        eprintln!("error: missing input file");
        eprintln!("usage: tatum check <song.synth>");
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
    let ast = match tatum_core::dsl::parse(&source.text) {
        Ok(ast) => {
            eprintln!("  parse OK");
            ast
        }
        Err(errs) => {
            source.print_errors(&tatum_core::song_engine::DslError::Parse(errs));
            process::exit(1);
        }
    };

    // Compile
    match tatum_core::dsl::compiler::compile(&ast) {
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
            let lints = tatum_core::dsl::lint::lint_song(&ast);
            if !lints.is_empty() {
                eprintln!("design warnings:");
                for l in &lints {
                    eprintln!("  [{}] {}\n      {}", l.code, l.message, l.hint);
                }
            }
        }
        Err(errs) => {
            source.print_errors(&tatum_core::song_engine::DslError::Compile(errs));
            process::exit(1);
        }
    }

    eprintln!("{}: OK", path);
}

fn cmd_render(args: &[String]) {
    let mut args = args.to_vec();
    let isolation = debug::take_isolation(&mut args);
    if args.is_empty() {
        eprintln!("error: missing input file");
        eprintln!("usage: tatum render <song.synth> [-o output.wav] [--bars N]");
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

    let mut engine = tatum_core::song_engine::SongEngine::from_compiled(
        debug::compile_or_exit(&source, &isolation),
    );
    engine.set_output_gain(debug::output_gain_or_exit(&source));

    let render_bars = bars.unwrap_or_else(|| {
        let arr = engine.arrangement_bars();
        if arr > 0 { arr } else { 4 }
    });

    eprintln!("rendering {} bars at {} BPM...", render_bars, engine.tempo());

    engine.set_band_metering(true);
    engine.reset_meters();
    // Listened to as it renders: what `tatum debug` would find, said here.
    let mut listener = tatum_debug::Listener::new(&mut engine);
    let (out_l, out_r) = engine.render_each(render_bars, |e, l, r| listener.feed(e, l, r));
    let heard = listener.findings();

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
        eprintln!("  {:<12} {:>6} {:>8} {:>8} {:>8} {:>6} {:>6}  band",
            "track", "peak", "rms", "rms dB", "peak dB", "crest", "width");
        for i in 0..engine.track_count() {
            let rms = engine.track_rms(i);
            let peak = engine.track_peak(i);
            let rel = 20.0 * (rms.max(1e-6) / loudest).log10();
            let rel_peak = 20.0 * (peak.max(1e-6) / loudest_peak.max(1e-6)).log10();
            let flag = if rms <= 0.0 { "  SILENT" } else if rel < -30.0 { "  buried" } else { "" };
            let band = engine.track_dominant_band(i).map_or("-", |b| tatum_core::analysis::BAND_NAMES[b]);
            eprintln!("  {:<12} {:>6.3} {:>8.4} {:>+8.1} {:>+8.1} {:>6.1} {:>5.0}%  {}{}",
                engine.track_name(i), peak, rms, rel, rel_peak,
                tatum_core::analysis::crest(peak, rms),
                engine.track_width(i) * 100.0, band, flag);
        }
        for i in 0..engine.bus_count() {
            let (p, rms) = (engine.bus_peak(i), engine.bus_rms(i));
            eprintln!("  bus {:<8} {:>6.3} {:>8.4} {:>8} {:>8} {:>6.1}",
                engine.bus_name(i), p, rms, "", "", tatum_core::analysis::crest(p, rms));
        }
    }
    // Where two tracks are in each other's way. Balance is visible in the
    // table above; this is not. Two instruments in the same band cannot be
    // told apart however well their levels are set, and reading a column of
    // `mid` does not make that jump out.
    {
        use tatum_core::analysis::BAND_NAMES;
        let mut lines = Vec::new();
        for (b, name) in BAND_NAMES.iter().enumerate() {
            let mut here: Vec<(String, f32)> = (0..engine.track_count())
                .filter(|i| engine.track_rms(*i) > 0.0)
                .filter(|i| engine.track_dominant_band(*i) == Some(b))
                .map(|i| {
                    let db = 20.0 * (engine.track_rms(i).max(1e-6) / loudest).log10();
                    (engine.track_name(i).to_string(), db)
                })
                .collect();
            if here.len() < 2 { continue }
            here.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
            // Within 6 dB is where one stops sitting clearly behind the other.
            let close = here[0].1 - here[1].1 < 6.0;
            let who: Vec<String> = here.iter().map(|(n, d)| format!("{n} {d:+.1}")).collect();
            lines.push(format!("  {:<6} {}{}", name, who.join(", "),
                if close { "   <-- within 6 dB of each other" } else { "" }));
        }
        if !lines.is_empty() {
            eprintln!("sharing a band:");
            for l in lines { eprintln!("{l}"); }
        }
    }

    // The shape of the song. A drop that measures the same as the breakdown
    // before it is flat however good the parts are, and nothing in the table
    // above can show that -- it averages the whole render into one row.
    {
        let sections = engine.sections();
        if sections.len() > 1 {
            let beats_per_bar = engine.steps_per_bar() as f32 / 4.0;
            let mut rows: Vec<(String, u32, f32)> = Vec::new();
            let mut at = 0usize;
            for (name, bars, bpm) in &sections {
                let spb = (tatum_core::SAMPLE_RATE * 60.0 / bpm * beats_per_bar) as usize;
                let end = (at + spb * *bars as usize).min(out_l.len());
                if at >= end { break }
                let (_, rms) = tatum_core::analysis::peak_rms(&out_l[at..end], &out_r[at..end]);
                rows.push(((*name).to_string(), *bars, 20.0 * rms.max(1e-6).log10()));
                at = end;
            }
            if rows.len() > 1 {
                let hi = rows.iter().fold(f32::MIN, |a, r| a.max(r.2));
                let lo = rows.iter().fold(f32::MAX, |a, r| a.min(r.2));
                eprintln!("sections:");
                for (name, bars, db) in &rows {
                    let bar = "#".repeat((((db - lo) / (hi - lo).max(0.1)) * 24.0) as usize);
                    eprintln!("  {:<12} {:>3} bars {:>7.1} dB  {}", name, bars, db, bar);
                }
                eprintln!("  arc: {:.1} dB between the quietest section and the loudest{}",
                    hi - lo,
                    if hi - lo < 3.0 { "  -- that is flat" } else { "" });
                eprintln!("  (`width` above is how much of a track is NOT in the middle, full");
                eprintln!("   band -- on a drum track the mono kick holds it near zero.)");
            }
        }
    }

    // What the master chain costs in dynamics: raising the master gain looks
    // free on the peak meter because the limiter catches it, and the punch
    // leaves with the transients.
    let (in_peak, in_rms) = engine.master_input_peak_rms();
    let (out_peak, out_rms) = tatum_core::analysis::peak_rms(&out_l, &out_r);
    let (crest_in, crest_out) = (
        tatum_core::analysis::crest(in_peak, in_rms),
        tatum_core::analysis::crest(out_peak, out_rms),
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
    if heard.is_empty() {
        eprintln!("heard: no clicks, no noise between notes, nothing under 25 Hz or over 16 kHz.");
    } else {
        eprintln!("heard:");
        for line in &heard {
            eprintln!("  {line}");
        }
        eprintln!("  `tatum debug {path}` shows each one up close, with every part on its own.");
    }
    eprintln!("done.");
}

fn write_wav_stereo(path: &str, samples_l: &[f32], samples_r: &[f32], sample_rate: u32) {
    let bytes = tatum_core::wav::encode_stereo_16(samples_l, samples_r, sample_rate);
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
