//! `tatum kick fit <reference.wav>`: the `kick(...)` line whose hit has the
//! pitch and the level over time of a record's kick.
//!
//! The record's hits are averaged on its beat grid (`tatum_debug::hit`), the
//! node is fitted to that average on its own (`tatum_debug::fit`), and then
//! the line it prints is rendered by the whole engine, measured against the
//! record again, and refined while that render is further off than the node
//! alone was. What is printed is the line that was rendered.

use std::path::Path;
use std::process;
use std::time::Instant;

use tatum_core::song_engine::SongEngine;
use tatum_core::SAMPLE_RATE;
use tatum_debug::fit::{self, Judge, KickArgs, Score, Target};

const USAGE: &str =
    "Usage: tatum kick fit <reference.wav> [--from S] [--to S] [--bpm N] [--ms 230] [--effort 1] [--hit hit.wav]
  The kick(...) line whose hit matches the record's: its hits between --from and
  --to (seconds or m:ss) are averaged on the beat grid (--bpm, else measured),
  the first --ms of each. Pick a stretch where the kick plays alone or nearly
  (an intro, an outro), and only as many ms as come before an offbeat bass.
  --effort scales how many kicks are tried (1 is a few thousand); --hit writes
  the averaged hit to a WAV, to hear what was fitted to.";

pub fn cmd(args: &[String]) {
    match args.first().map(String::as_str) {
        Some("fit") => cmd_fit(&args[1..]),
        _ => {
            eprintln!("{USAGE}");
            process::exit(1);
        }
    }
}

/// What `fit` is asked for.
pub struct FitRequest {
    pub from: Option<f32>,
    pub to: Option<f32>,
    pub bpm: Option<f32>,
    pub ms: f32,
    pub effort: f32,
    /// Where to write the averaged hit, to listen to it.
    pub hit: Option<String>,
}

fn parse(args: &[String]) -> Result<(String, FitRequest), String> {
    let mut path = None;
    let mut req = FitRequest { from: None, to: None, bpm: None, ms: 230.0, effort: 1.0, hit: None };
    let mut i = 0;
    while i < args.len() {
        let value = |i: usize| args.get(i + 1).cloned().ok_or_else(|| format!("{} needs a value", args[i]));
        let number = |i: usize| -> Result<f32, String> {
            let v = value(i)?;
            v.parse::<f32>()
                .ok()
                .filter(|v| v.is_finite() && *v > 0.0)
                .ok_or_else(|| format!("{}: '{v}' is not a positive number", args[i]))
        };
        match args[i].as_str() {
            "--from" => req.from = Some(crate::analyze::seconds(&value(i)?).ok_or("--from takes seconds or m:ss")?),
            "--to" => req.to = Some(crate::analyze::seconds(&value(i)?).ok_or("--to takes seconds or m:ss")?),
            "--bpm" => req.bpm = Some(number(i)?),
            "--ms" => req.ms = number(i)?.clamp(20.0, 2000.0),
            "--effort" => req.effort = number(i)?.clamp(0.05, 10.0),
            "--hit" => req.hit = Some(value(i)?),
            a if a.starts_with('-') => return Err(format!("unknown flag '{a}'")),
            a => {
                path = Some(a.to_string());
                i += 1;
                continue;
            }
        }
        i += 2;
    }
    Ok((path.ok_or("which WAV?")?, req))
}

/// The record's averaged hit, and the tempo it was averaged at.
pub fn reference_hit(audio: &tatum_debug::wav::Audio, req: &FitRequest) -> Result<(Vec<f32>, f32), String> {
    let (l, r) = tatum_debug::reference::stretch(audio, req.from, req.to, 15.0 * 60.0)?;
    let bpm = match req.bpm {
        Some(b) => b,
        None => {
            tatum_debug::reference::measure(&l, &r).tempo.ok_or("no steady beat in it; give the tempo with --bpm")?
        }
    };
    let mono: Vec<f32> = l.iter().zip(&r).map(|(a, b)| 0.5 * (a + b)).collect();
    let hit = tatum_debug::hit::average_hit(&mono, bpm, req.ms)
        .ok_or("fewer than four beats in that stretch; give it a few bars")?;
    Ok((hit, bpm))
}

/// The hit of `line` as the engine plays it: the instrument alone, before
/// the master and the loudness, cut where the hit starts.
pub fn engine_hit(line: &str, ms: f32) -> Result<Vec<f32>, String> {
    let src = format!(
        "tempo 60\ninstrument fit {{\n  {line} as k\n  k > out\n}}\npattern one {{ C3 - - -  - - - -  - - - -  - - - - }}\ntrack t {{ play one using fit }}\n"
    );
    let mut engine = SongEngine::from_source(&src)?;
    engine.set_taps(true);
    let mut out = Vec::new();
    let want = ((ms + 100.0) * 0.001 * SAMPLE_RATE) as usize;
    engine.render_each(1, |e, _, _| {
        if out.len() < want {
            if let Some(t) = e.taps() {
                out.extend_from_slice(&t.dry[0].l[..t.len]);
            }
        }
    });
    Ok(fit::cut_like_an_average(&out, ms))
}

/// The search with every kick rendered by the engine.
struct Engine<'a> {
    target: &'a Target,
}

impl Judge for Engine<'_> {
    fn judge(&mut self, k: &KickArgs) -> Score {
        match engine_hit(&k.line(), self.target.ms) {
            Ok(y) => fit::score(self.target, &y),
            Err(_) => Score { pitch_st: 99.0, attack_st: 99.0, level_db: 99.0, click_db: 0.0 },
        }
    }
}

/// What a fit ends with: the line, and how far its engine render is.
pub struct Fitted {
    pub args: KickArgs,
    pub score: Score,
    /// One line per round: what the engine's render measured.
    pub rounds: Vec<String>,
    pub tried: usize,
}

/// Fit, then render, measure and refine in a loop.
pub fn fit_kick(target: &Target, effort: f32) -> Result<Fitted, String> {
    let found = fit::fit(target, None, effort);
    let mut args = found.args;
    let mut tried = found.evals;
    let alone = found.score;
    let mut rounds = Vec::new();
    let mut score = Score::default();
    for round in 1..=3 {
        let y = engine_hit(&args.line(), target.ms)?;
        score = fit::score(target, &y);
        rounds.push(format!(
            "round {round}: the engine's render is off by {:.2} st in pitch ({:.2} st over the attack), {:.2} dB in level, click {:+.1} dB",
            score.pitch_st, score.attack_st, score.level_db, score.click_db
        ));
        // The engine plays the node as it is; a render further off than the
        // node alone was is a difference the search has to see.
        if score.cost() <= alone.cost() + 0.02 {
            break;
        }
        let evals = (300.0 * effort) as usize;
        args = fit::refine(&mut Engine { target }, &args, evals);
        tried += evals;
    }
    Ok(Fitted { args, score, rounds, tried })
}

fn cmd_fit(args: &[String]) {
    let (path, req) = parse(args).unwrap_or_else(|e| {
        eprintln!("error: {e}\n{USAGE}");
        process::exit(1);
    });
    let audio = tatum_debug::wav::read(Path::new(&path)).unwrap_or_else(|e| {
        eprintln!("{path}: {e}");
        process::exit(1);
    });
    let (hit, bpm) = reference_hit(&audio, &req).unwrap_or_else(|e| {
        eprintln!("{path}: {e}");
        process::exit(1);
    });
    if let Some(out) = &req.hit {
        let wav = tatum_core::wav::encode_stereo_16(&hit, &hit, SAMPLE_RATE as u32);
        if let Err(e) = std::fs::write(out, wav) {
            eprintln!("{out}: {e}");
            process::exit(1);
        }
        eprintln!("the averaged hit is in {out}");
    }
    let target = Target::new(hit);
    eprintln!("{path}: hits averaged at {bpm:.2} BPM, the first {:.0} ms of each", target.ms);
    let t0 = Instant::now();
    let fitted = fit_kick(&target, req.effort).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        process::exit(1);
    });
    eprintln!("{} kicks tried in {:.1} s", fitted.tried, t0.elapsed().as_secs_f32());
    for r in &fitted.rounds {
        eprintln!("  {r}");
    }
    println!("{}", fitted.args.line());
    let s = fitted.score;
    println!(
        "# left: pitch {:.2} st (attack {:.2} st), level {:.2} dB, click {:+.1} dB ({})",
        s.pitch_st,
        s.attack_st,
        s.level_db,
        s.click_db,
        if s.click_db > 0.0 { "the record's has more" } else { "ours has more or the same" }
    );
    // The two side by side, where the ear follows a kick.
    let y = engine_hit(&fitted.args.line(), target.ms).unwrap_or_default();
    let ours = tatum_debug::hit::curves(&y);
    let at = |c: &[(f32, f32)], t: f32| c.iter().find(|p| p.0 >= t).map_or("-".to_string(), |p| format!("{:.0}", p.1));
    println!("#   ms      pitch Hz (record / ours)    level dB (record / ours)");
    for t in [1.0f32, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 150.0, 200.0] {
        if t > target.ms {
            break;
        }
        println!(
            "#  {t:4.0}   {:>8} / {:<8}            {:>5} / {:<5}",
            at(&target.curves.pitch, t),
            at(&ours.pitch, t),
            at(&target.curves.level, t),
            at(&ours.level, t)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_are_read_and_checked() {
        let a = |v: &[&str]| parse(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        let (p, r) = a(&["ref.wav", "--from", "1:04", "--to", "96", "--bpm", "124", "--ms", "200"]).expect("reads");
        assert_eq!(p, "ref.wav");
        assert_eq!((r.from, r.to, r.bpm, r.ms), (Some(64.0), Some(96.0), Some(124.0), 200.0));
        assert!(a(&["ref.wav", "--bpm", "NaN"]).is_err());
        assert!(a(&["ref.wav", "--bpm"]).is_err());
        assert!(a(&["ref.wav", "--nope"]).is_err());
        assert!(a(&[]).is_err());
    }

    /// A whole song -- the kick, an offbeat bass, the master and the
    /// loudness -- made with a kick of known arguments, and fitted back.
    #[test]
    fn a_rendered_song_gives_back_the_kick_it_was_made_with() {
        let src = "tempo 124
instrument deep {
  kick(F#1, start=1.1khz, knee=170hz, snap=2.5ms, chirp=4khz, chirp_time=0.6ms, body=80ms, hold=95ms, sag=-2db, release=210ms, phase=25) as k
  k > out
}
instrument sub {
  osc sine(110) as wave
  wave > adsr(0.005, 0.1, 0.8, 0.05) as env
  env > out
}
pattern four { C3 - - -  C3 - - -  C3 - - -  C3 - - - }
pattern off { - - A1 -  - - C2 -  - - G1 -  - - D2 - }
track kick { play four using deep }
track bass { play off using sub level 0.5 }
";
        let mut engine = SongEngine::from_source(src).expect("compiles");
        let (l, r) = engine.render(8);
        let mono: Vec<f32> = l.iter().zip(&r).map(|(a, b)| 0.5 * (a + b)).collect();
        let hit = tatum_debug::hit::average_hit(&mono, 124.0, 200.0).expect("a hit");
        let target = Target::new(hit);
        let f = fit_kick(&target, 1.0).expect("fits");
        let (a, s) = (&f.args, f.score);
        assert!(s.pitch_st < 0.2 && s.attack_st < 1.0 && s.level_db < 0.5, "{s:?} {}", a.line());
        assert!(a.line().starts_with("kick(F#1,"), "{}", a.line());
        assert!((a.body - 80.0).abs() < 8.0, "body {}", a.body);
        assert!((a.hold - 95.0).abs() < 8.0, "hold {}", a.hold);
        assert!((a.release - 210.0).abs() < 25.0, "release {}", a.release);
        assert!((a.sag + 2.0).abs() < 1.0, "sag {}", a.sag);
        assert!(!f.rounds.is_empty());
    }

    #[test]
    fn the_engine_plays_the_line_as_the_node_alone_does() {
        let k =
            KickArgs { note: 46.25, chirp: 3500.0, chirp_time: 0.6, phase: 20.0, click: -40.0, ..KickArgs::default() }
                .rounded();
        let alone = k.render(230.0).expect("renders");
        let engine = engine_hit(&k.line(), 230.0).expect("renders");
        let target = Target::new(alone);
        let s = fit::score(&target, &engine);
        assert!(s.pitch_st < 0.01 && s.level_db < 0.05, "{s:?}");
    }
}
