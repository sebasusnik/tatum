//! `tatum events`: the events of a WAV read from the inside, alone or
//! against a second file. The work is in `tatum_debug::events`, which the
//! MCP server shares; this is the command line around it.

use std::path::Path;
use std::process;

use tatum_debug::events::{self, Options, Report};

use crate::analyze::seconds;

const USAGE: &str = "usage: tatum events <a.wav> [--against <b.wav>] [--from 1:04] [--to 1:36] [--b-from S] [--b-to S]
                    [--band LO HI] [--min-gap MS] [--threshold DB] [--sustained] [--first N]
                    [--png sheet.png] [--show N] [--json]
  --against    measure a second file the same way: a table of both, and what differs most
               (A is the reference; --from/--to trim A, --b-from/--b-to trim B)
  --band       look for events between two frequencies in Hz (default: where the onsets are)
  --min-gap    onsets closer than this are one event (default 30 ms)
  --threshold  how far over its bin's floor an event has to stand (default 24 dB)
  --sustained  measure what holds too (pads, held notes): done anyway when under 3 events are found
  --first      rows of the per-event table (default 12)
  --png        write the sheet: --show events of each file up close (default 6), and their curves
  --json       the same numbers, for a script";

fn fail(message: &str) -> ! {
    eprintln!("error: {message}");
    process::exit(1);
}

pub fn cmd(args: &[String]) {
    let mut opts = Options::default();
    let (mut paths, mut against, mut png) = (Vec::new(), None, None);
    let (mut from, mut to, mut b_from, mut b_to) = (None, None, None, None);
    let (mut json, mut show) = (false, 6usize);
    let mut i = 0;
    while i < args.len() {
        let value =
            |k: usize| args.get(i + k).cloned().unwrap_or_else(|| fail(&format!("{} needs a value\n{USAGE}", args[i])));
        let number = |k: usize| -> f32 {
            let v = value(k);
            v.parse()
                .ok()
                .filter(|n: &f32| n.is_finite())
                .unwrap_or_else(|| fail(&format!("{} takes a number, not '{v}'", args[i])))
        };
        let time = |k: usize| {
            let v = value(k);
            seconds(&v).unwrap_or_else(|| {
                fail(&format!("{} takes seconds (64) or minutes and seconds (1:04), not '{v}'", args[i]))
            })
        };
        match args[i].as_str() {
            "--against" => (against, i) = (Some(value(1)), i + 1),
            "--png" => (png, i) = (Some(value(1)), i + 1),
            "--from" => (from, i) = (Some(time(1)), i + 1),
            "--to" => (to, i) = (Some(time(1)), i + 1),
            "--b-from" => (b_from, i) = (Some(time(1)), i + 1),
            "--b-to" => (b_to, i) = (Some(time(1)), i + 1),
            "--band" => (opts.band, i) = (Some((number(1), number(2))), i + 2),
            "--min-gap" => (opts.min_gap_ms, i) = (number(1).clamp(1.0, 5000.0), i + 1),
            "--threshold" => (opts.threshold_db, i) = (number(1).clamp(3.0, 80.0), i + 1),
            "--first" => (opts.first, i) = (number(1) as usize, i + 1),
            "--show" => (show, i) = (number(1) as usize, i + 1),
            "--sustained" => opts.sustained = true,
            "--json" => json = true,
            "--help" | "-h" => {
                eprintln!("{USAGE}");
                return;
            }
            other if other.starts_with('-') => fail(&format!("unknown flag '{other}'\n{USAGE}")),
            other => paths.push(other.to_string()),
        }
        i += 1;
    }
    let [path] = paths.as_slice() else {
        eprintln!("{USAGE}");
        process::exit(1);
    };
    if against.is_none() && (b_from.is_some() || b_to.is_some()) {
        fail("--b-from and --b-to trim the file given with --against");
    }
    let a = measure(path, from, to, &opts);
    let b = against.as_ref().map(|p| measure(p, b_from, b_to, &opts));
    let out = match (&b, &against, json) {
        (Some(b), Some(name), false) => events::report::against(&a, b, path, name),
        (Some(b), Some(name), true) => events::report::json_against(&a, b, path, name),
        (_, _, false) => events::report::describe(&a, path, opts.first),
        (_, _, true) => events::report::json(&a, path),
    };
    print!("{out}");
    if let Some(png) = png {
        if a.events.is_empty() && b.as_ref().is_none_or(|b| b.events.is_empty()) {
            eprintln!("no sheet: no events were found to draw");
            return;
        }
        let sheet = events::draw::sheet(&a, path, b.as_ref().zip(against.as_deref()), show);
        if let Err(e) = sheet.save(Path::new(&png)) {
            fail(&e);
        }
        // Out of the way of a script reading the JSON.
        if json {
            eprintln!("sheet: {png}");
        } else {
            println!("sheet: {png}");
        }
    }
}

fn measure(path: &str, from: Option<f32>, to: Option<f32>, opts: &Options) -> Report {
    let audio = tatum_debug::wav::read(Path::new(path)).unwrap_or_else(|e| fail(&e));
    eprintln!("measuring {path}...");
    events::analyze(&audio, from, to, opts).unwrap_or_else(|e| fail(&format!("{path}: {e}")))
}
