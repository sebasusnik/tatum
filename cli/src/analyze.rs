//! `tatum analyze` and `tatum compare`: a reference track measured, and a
//! song measured next to it. The work is in `tatum_debug::reference`, which
//! the MCP server shares; this is the command line around it.

use std::path::Path;
use std::process;

use tatum_debug::reference::{self, Profile};

use crate::debug::{compile_or_exit, output_gain_or_exit, parse_bars, take_isolation};
use crate::include::Source;

const ANALYZE_USAGE: &str = "usage: tatum analyze <reference.wav> [--from 1:04] [--to 1:36]";
const COMPARE_USAGE: &str = "usage: tatum compare <reference.wav> <song.synth | other.wav> [--from 1:04] [--to 1:36] \
     [--bars N | A-B] [--solo a,b] [--mute c]";

pub fn cmd_analyze(args: &[String]) {
    let (paths, from, to, _) = flags(args, ANALYZE_USAGE);
    let [path] = paths.as_slice() else {
        eprintln!("{ANALYZE_USAGE}");
        process::exit(1);
    };
    let profile = wav_profile(path, from, to);
    print!("{}", reference::describe(&profile, path));
}

pub fn cmd_compare(args: &[String]) {
    let mut args = args.to_vec();
    let iso = take_isolation(&mut args);
    let (paths, from, to, bars) = flags(&args, COMPARE_USAGE);
    let [reference_path, yours] = paths.as_slice() else {
        eprintln!("{COMPARE_USAGE}");
        process::exit(1);
    };
    let other_wav = yours.to_lowercase().ends_with(".wav");
    if other_wav && (bars.is_some() || !iso.is_empty()) {
        eprintln!("error: --bars, --solo and --mute pick parts of a song; {yours} is a WAV, which is measured whole");
        process::exit(1);
    }
    let reference = wav_profile(reference_path, from, to);
    let mine = if other_wav {
        wav_profile(yours, None, None)
    } else {
        let source = Source::load(Path::new(yours)).unwrap_or_else(|e| {
            eprintln!("error: {e}");
            process::exit(1);
        });
        let song = compile_or_exit(&source, &iso);
        let gain = output_gain_or_exit(&source);
        eprintln!("rendering {yours}...");
        let (l, r) = reference::render(song, gain, bars).unwrap_or_else(|e| {
            eprintln!("error: {yours}: {e}");
            process::exit(1);
        });
        if l.len() < tatum_core::SAMPLE_RATE as usize * 2 {
            eprintln!("error: the render is under two seconds; give --bars more of the song");
            process::exit(1);
        }
        reference::measure(&l, &r)
    };
    print!("{}", reference::compare(&reference, &mine, reference_path, yours));
}

fn wav_profile(path: &str, from: Option<f32>, to: Option<f32>) -> Profile {
    let audio = tatum_debug::wav::read(Path::new(path)).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        process::exit(1);
    });
    eprintln!("measuring {path}...");
    reference::analyze(&audio, from, to).unwrap_or_else(|e| {
        eprintln!("error: {path}: {e}");
        process::exit(1);
    })
}

type Flags = (Vec<String>, Option<f32>, Option<f32>, Option<(u32, u32)>);

fn flags(args: &[String], usage: &str) -> Flags {
    let (mut paths, mut from, mut to, mut bars) = (Vec::new(), None, None, None);
    let mut i = 0;
    while i < args.len() {
        let value = |i: usize| {
            args.get(i + 1).cloned().unwrap_or_else(|| {
                eprintln!("error: {} needs a value\n{usage}", args[i]);
                process::exit(1);
            })
        };
        match args[i].as_str() {
            "--from" | "--to" => {
                let v = value(i);
                let Some(s) = seconds(&v) else {
                    eprintln!("error: {} takes seconds (64) or minutes and seconds (1:04), not '{v}'", args[i]);
                    process::exit(1);
                };
                if args[i] == "--from" {
                    from = Some(s);
                } else {
                    to = Some(s);
                }
                i += 1;
            }
            "--bars" => {
                bars = parse_bars(&value(i));
                if bars.is_none() {
                    eprintln!("error: --bars takes a count (16) or a range (17-24)");
                    process::exit(1);
                }
                i += 1;
            }
            other if other.starts_with('-') => {
                eprintln!("error: unknown flag '{other}'\n{usage}");
                process::exit(1);
            }
            other => paths.push(other.to_string()),
        }
        i += 1;
    }
    (paths, from, to, bars)
}

/// `64`, `64.5` or `1:04`.
fn seconds(v: &str) -> Option<f32> {
    let s = match v.split_once(':') {
        Some((m, s)) => m.parse::<f32>().ok()? * 60.0 + s.parse::<f32>().ok()?,
        None => v.parse().ok()?,
    };
    (s.is_finite() && s >= 0.0).then_some(s)
}

#[cfg(test)]
mod tests {
    use super::seconds;

    #[test]
    fn times_read_as_seconds_or_minutes() {
        assert_eq!(seconds("64"), Some(64.0));
        assert_eq!(seconds("1:04"), Some(64.0));
        assert_eq!(seconds("0:30.5"), Some(30.5));
        assert_eq!(seconds("-3"), None);
        assert_eq!(seconds("abc"), None);
    }
}
