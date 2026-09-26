//! `tatum debug`, and the `--solo` / `--mute` flags every command that plays
//! a song takes. The work is in the `tatum-debug` crate, which the MCP server
//! shares; this is the command line around it.

use std::path::{Path, PathBuf};
use std::process;

use tatum_core::dsl::isolate::{self, Isolation};
use tatum_debug::Options;

use crate::include::Source;

/// Take `--solo a,b` and `--mute c` out of `args`, wherever they are. Both can
/// be given more than once.
pub fn take_isolation(args: &mut Vec<String>) -> Isolation {
    let mut iso = Isolation::default();
    let mut i = 0;
    while i < args.len() {
        let list = match args[i].as_str() {
            "--solo" => &mut iso.solo,
            "--mute" => &mut iso.mute,
            _ => { i += 1; continue }
        };
        let Some(names) = args.get(i + 1).filter(|v| !v.starts_with('-')).cloned() else {
            eprintln!("error: {} needs track names, separated by commas", args[i]);
            process::exit(1);
        };
        list.extend(names.split(',').map(str::trim).filter(|n| !n.is_empty()).map(String::from));
        args.drain(i..i + 2);
    }
    iso
}

/// Load, isolate and compile, or print what is wrong and exit.
pub fn compile_or_exit(source: &Source, iso: &Isolation) -> tatum_core::dsl::compiler::CompiledSong {
    match isolate::compile(&source.text, iso) {
        Ok(song) => song,
        Err(err) => {
            source.print_errors(&err);
            process::exit(1);
        }
    }
}

/// The engine's output gain for the song, measured whole, or exit.
pub fn output_gain_or_exit(source: &Source) -> f32 {
    match isolate::output_gain(&source.text) {
        Ok(g) => g,
        Err(err) => {
            source.print_errors(&err);
            process::exit(1);
        }
    }
}

const USAGE: &str = "usage: tatum debug <song.synth> [--solo a,b] [--mute c] [--bars N | --bars A-B] [--dry] [-o dir]";

pub fn cmd(args: &[String]) {
    let mut args = args.to_vec();
    let iso = take_isolation(&mut args);
    let mut path: Option<String> = None;
    let mut out: Option<PathBuf> = None;
    let mut dry = false;
    let mut bars: Option<(u32, u32)> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-o" | "--output" => {
                i += 1;
                out = args.get(i).map(PathBuf::from);
            }
            "--bars" => {
                i += 1;
                bars = args.get(i).and_then(|v| parse_bars(v));
                if bars.is_none() {
                    eprintln!("error: --bars takes a count (16) or a range (17-24)");
                    process::exit(1);
                }
            }
            "--dry" => dry = true,
            other if other.starts_with('-') => {
                eprintln!("error: unknown flag '{other}'\n{USAGE}");
                process::exit(1);
            }
            other => path = Some(other.to_string()),
        }
        i += 1;
    }
    let Some(path) = path else {
        eprintln!("{USAGE}");
        process::exit(1);
    };
    let source = match Source::load(Path::new(&path)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            process::exit(1);
        }
    };
    let song = compile_or_exit(&source, &iso);
    let stem = Path::new(&path).file_stem().map_or("song".into(), |s| s.to_string_lossy().into_owned());
    let mut opts = Options::new(out.unwrap_or_else(|| PathBuf::from("test_output/debug").join(&stem)));
    opts.dry = dry;
    opts.bars = bars;
    opts.gain = output_gain_or_exit(&source);
    eprintln!("rendering {path} with every part kept apart...");
    match tatum_debug::run(song, &stem, &iso, &opts) {
        Ok(outcome) => print!("{}", outcome.report),
        Err(e) => {
            eprintln!("error: {e}");
            process::exit(1);
        }
    }
}

/// `16` is the first sixteen bars; `17-24` is bars 17 to 24.
fn parse_bars(v: &str) -> Option<(u32, u32)> {
    match v.split_once('-') {
        Some((a, b)) => {
            let (a, b) = (a.parse().ok()?, b.parse().ok()?);
            (a >= 1 && b >= a).then_some((a, b))
        }
        None => {
            let n: u32 = v.parse().ok()?;
            (n >= 1).then_some((1, n))
        }
    }
}
