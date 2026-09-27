//! `tatum set` — a live set as a sequence of states.
//!
//! A set is a directory of numbered `.synth` files. Each one is the whole rig
//! at a moment; the set is the walk from one to the next. The engine moves
//! between them with the same hot swap `tatum watch` uses, so a state that
//! kept a voice's name keeps that voice: nothing restarts at a transition.
//!
//! There is no manifest. How long a step lasts travels in the file, on a
//! comment the DSL already ignores:
//!
//!     # set: bars=48 phase=warm-up energy=2
//!     # set-note: brought the reese in under the pad
//!
//! A manifest beside the files is a second thing to keep in sync, and the
//! first time it drifts the set plays in the wrong order for reasons nobody
//! can see. The header cannot drift from the file it is in.

use std::fs;
use std::path::{Path, PathBuf};

use tatum_core::live::{LivePlanner, LivePlayer, Plan};
use tatum_core::song_engine::SongEngine;
use tatum_core::{BLOCK_SIZE, SAMPLE_RATE};

const DEFAULT_BARS: u32 = 32;

pub struct Step {
    pub path: PathBuf,
    pub src: String,
    pub bars: u32,
    pub phase: String,
    pub energy: Option<f32>,
    pub note: String,
    /// `blend=8`: coming into this step in `set play`, the step before keeps
    /// playing under it for this many bars, DJ style.
    pub blend: Option<f32>,
}

impl Step {
    pub fn name(&self) -> String {
        self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }
}

/// `# set: bars=48 phase=warm-up energy=2` and `# set-note: ...`, anywhere in
/// the file. Unknown keys are ignored rather than rejected: the header is a
/// note to the tooling, not part of the language, and a set written by hand
/// should not fail to play because someone added a word.
fn parse_header(src: &str, default_bars: u32) -> (u32, String, Option<f32>, String, Option<f32>) {
    let (mut bars, mut phase, mut energy, mut note) = (default_bars, String::new(), None, String::new());
    let mut blend = None;
    for line in src.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("# set-note:").or_else(|| t.strip_prefix("#set-note:")) {
            note = rest.trim().to_string();
            continue;
        }
        let Some(rest) = t.strip_prefix("# set:").or_else(|| t.strip_prefix("#set:")) else { continue };
        for field in rest.split_whitespace() {
            let Some((k, v)) = field.split_once('=') else { continue };
            match k {
                "bars" => {
                    if let Ok(n) = v.parse::<u32>() {
                        if n > 0 {
                            bars = n;
                        }
                    }
                }
                "phase" => phase = v.to_string(),
                "energy" => energy = v.parse::<f32>().ok(),
                "blend" => blend = v.parse::<f32>().ok().filter(|b| *b >= 0.0),
                _ => {}
            }
        }
    }
    (bars, phase, energy, note, blend)
}

pub fn load(dir: &Path, default_bars: u32) -> Result<Vec<Step>, String> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|e| format!("{}: {}", dir.display(), e))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "synth"))
        // A leading underscore means "not a step". `use` resolves relative to
        // the file that writes it, so a candidate has to sit in the set's own
        // directory for the rig path to work -- and then it must not also be
        // read as part of the set.
        .filter(|p| !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('_')))
        .collect();
    files.sort();
    if files.is_empty() {
        return Err(format!("{}: no .synth files", dir.display()));
    }
    files
        .into_iter()
        .map(|path| {
            // Resolved through `use`, so a step can be the rig plus its overrides
            // rather than a copy of the whole rig. The header is read from the raw
            // file, not the expansion: `bars` belongs to the step, and a rig that
            // carried one would otherwise leak into every step that includes it.
            let raw = fs::read_to_string(&path).map_err(|e| format!("{}: {}", path.display(), e))?;
            let src = crate::include::Source::load(&path)?.text;
            let (bars, phase, energy, note, blend) = parse_header(&raw, default_bars);
            Ok(Step { path, src, bars, phase, energy, note, blend })
        })
        .collect()
}

/// The output gain that brings the loudest step of the set to the engine's
/// target. Every step plays at it, so a quiet intro stays quiet and the peak
/// of the set is where one song would be, instead of each step levelled on
/// its own or all of them on the first.
pub fn set_gain(steps: &[Step]) -> Result<f32, String> {
    let mut loudest: Option<f32> = None;
    for step in steps {
        let ast = tatum_core::dsl::parse(&step.src)
            .map_err(|e| format!("{}: {}", step.name(), describe(&tatum_core::song_engine::DslError::Parse(e))))?;
        let compiled = tatum_core::dsl::compiler::compile(&ast)
            .map_err(|e| format!("{}: {}", step.name(), describe(&tatum_core::song_engine::DslError::Compile(e))))?;
        if let Some(lufs) = SongEngine::loudness(&compiled) {
            loudest = Some(loudest.map_or(lufs, |l: f32| l.max(lufs)));
        }
    }
    Ok(tatum_core::output::gain_for(loudest))
}

/// Bars are the unit a set is written in, and a step can change the tempo, so
/// the length of a bar has to come from the engine that is about to play it.
fn samples_per_bar(e: &SongEngine) -> f64 {
    SAMPLE_RATE as f64 * 60.0 / e.tempo() as f64 * e.steps_per_bar() as f64 / 4.0
}

pub struct Rendered {
    pub l: Vec<f32>,
    pub r: Vec<f32>,
    /// Per step: how it was applied, and where it starts in the output.
    pub applied: Vec<(String, usize)>,
}

/// Walk the set, handing over between states the way the live path does.
///
/// A swap lands on the next bar line, so it is submitted one bar before the
/// step boundary and arrives exactly on it. A fast edit applies the instant it
/// is submitted, so it is submitted on the boundary itself. Getting this wrong
/// does not error, it just puts the change a bar out, which is the kind of
/// thing nobody notices until the set is an hour long.
pub fn render_set(steps: &[Step]) -> Result<Rendered, String> {
    let mut planner = LivePlanner::new();
    planner.set_output_gain(set_gain(steps)?);
    let mut player = LivePlayer::new();
    let first = planner
        .plan(&steps[0].src, player.generation())
        .map_err(|e| format!("{}: {}", steps[0].name(), describe(&e)))?;
    player.apply(first);
    player.start();

    let mut l = Vec::new();
    let mut r = Vec::new();
    let mut applied = Vec::new();
    let mut bl = [0.0f32; BLOCK_SIZE];
    let mut br = [0.0f32; BLOCK_SIZE];

    let mut render_samples = |player: &mut LivePlayer, n: usize, l: &mut Vec<f32>, r: &mut Vec<f32>| {
        let mut done = 0;
        while done < n {
            player.process(&mut bl, &mut br);
            let take = BLOCK_SIZE.min(n - done);
            l.extend_from_slice(&bl[..take]);
            r.extend_from_slice(&br[..take]);
            done += take;
            while player.take_retired().is_some() {}
        }
    };

    for (i, step) in steps.iter().enumerate() {
        applied.push((step.name(), l.len()));
        let spb = player.engine().map(samples_per_bar).unwrap_or(SAMPLE_RATE as f64 * 2.0);
        let total = (spb * step.bars as f64) as usize;

        let Some(next) = steps.get(i + 1) else {
            render_samples(&mut player, total, &mut l, &mut r);
            break;
        };
        let plan =
            planner.plan(&next.src, player.generation()).map_err(|e| format!("{}: {}", next.name(), describe(&e)))?;
        match plan {
            Plan::Swap { .. } => {
                let lead = (spb as usize).min(total);
                render_samples(&mut player, total - lead, &mut l, &mut r);
                player.apply(plan);
                render_samples(&mut player, lead, &mut l, &mut r);
            }
            _ => {
                render_samples(&mut player, total, &mut l, &mut r);
                player.apply(plan);
            }
        }
    }
    Ok(Rendered { l, r, applied })
}

fn describe(e: &tatum_core::song_engine::DslError) -> String {
    use tatum_core::song_engine::DslError;
    match e {
        DslError::Parse(errs) => {
            errs.iter().map(|x| format!("line {}: {}", x.line, x.message)).collect::<Vec<_>>().join("; ")
        }
        DslError::Compile(errs) => errs.iter().map(|x| x.message.clone()).collect::<Vec<_>>().join("; "),
    }
}

// ── Measurement ──────────────────────────────────────────────────────────────

pub struct Probe {
    pub peak: f32,
    pub rms_db: f32,
    pub clipped: usize,
    pub silent_tracks: usize,
    pub tracks: usize,
    /// Tracks actually making sound. A set that keeps stacking voices without
    /// taking any away ends up with everything on at once, and then the kick
    /// is one of ten things instead of the thing the rest hangs off.
    pub voices: usize,
    /// Energy below 140 Hz against 300 Hz-3 kHz, in dB. How far the floor
    /// stands over everything else. Measured, not judged: what counts as
    /// right is the genre's business, and the number is here to be compared.
    pub floor_db: f32,
    /// What the mix hands the master chain. Over 1.0 the limiter is not
    /// catching peaks any more, it is deciding how loud the step is -- and
    /// once that happens every step comes out the same loudness however the
    /// faders move, which takes the set's dynamics away from the set.
    pub master_in: f32,
}

/// What a step sounds like on its own. The gate uses it to reject a state that
/// is silent, clipping, or the same as the one before it -- the three ways an
/// agent's turn can be wasted without anything reporting an error.
pub fn probe(src: &str, bars: u32) -> Result<Probe, String> {
    let mut e = SongEngine::from_source(src)?;
    e.start();
    let (l, r) = e.render(bars);
    let voices = (0..e.track_count()).filter(|i| e.track_level(*i) > 0.0 && e.track_rms(*i) > 1e-5).count();
    let peak = l.iter().chain(r.iter()).fold(0.0f32, |m, v| m.max(v.abs()));
    let n = (l.len() + r.len()).max(1) as f64;
    let sum: f64 = l.iter().chain(r.iter()).map(|v| (*v as f64) * (*v as f64)).sum();
    let clipped = l.iter().chain(r.iter()).filter(|v| v.abs() >= 0.999).count();
    Ok(Probe {
        peak,
        rms_db: 10.0 * (sum / n).max(1e-20).log10() as f32,
        clipped,
        silent_tracks: e.silent_track_count(),
        tracks: e.track_count(),
        voices,
        floor_db: band_ratio(&l),
        master_in: e.master_input_peak_rms().0,
    })
}

/// Energy below 140 Hz against 300 Hz-3 kHz, in dB. A naive DFT at a handful
/// of probe frequencies would be cheaper, but the whole band is what the ear
/// weighs, so this walks it: one pass, no allocation beyond the accumulators.
fn band_ratio(x: &[f32]) -> f32 {
    // Two one-pole filters is enough to separate a floor from a midrange.
    let (mut lo, mut hi_lp, mut hi_hp) = (0.0f32, 0.0f32, 0.0f32);
    let (a_lo, a_hi) = (0.019f32, 0.38f32); // ~140 Hz and ~3 kHz at 44.1k
    let (mut e_lo, mut e_hi) = (0.0f64, 0.0f64);
    for &v in x {
        lo += a_lo * (v - lo);
        hi_lp += a_hi * (v - hi_lp);
        hi_hp += 0.041 * (v - hi_hp); // ~300 Hz
        let mid = hi_lp - hi_hp;
        e_lo += (lo * lo) as f64;
        e_hi += (mid * mid) as f64;
    }
    10.0 * (e_lo.max(1e-20) / e_hi.max(1e-20)).log10() as f32
}

/// How different two renders are, in dB. -inf is identical. The gate uses it
/// to catch a step that compiles, differs in the text, and changes nothing you
/// can hear -- an agent editing a parameter that is not reachable, say.
fn difference_db(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    if n == 0 {
        return f32::NEG_INFINITY;
    }
    let d: f64 = (0..n)
        .map(|i| {
            let x = (a[i] - b[i]) as f64;
            x * x
        })
        .sum();
    let e: f64 = (0..n).map(|i| (a[i] as f64) * (a[i] as f64)).sum();
    if e <= 0.0 {
        return f32::NEG_INFINITY;
    }
    10.0 * (d / e).max(1e-20).log10() as f32
}

// ── Commands ─────────────────────────────────────────────────────────────────

pub fn cmd(args: &[String]) {
    let sub = args.first().map(|s| s.as_str()).unwrap_or("");
    let rest = if args.is_empty() { &[][..] } else { &args[1..] };
    match sub {
        "render" => run(cmd_render(rest)),
        "check" => run(cmd_check(rest)),
        "next" => run(cmd_next(rest)),
        "play" => run(cmd_play(rest)),
        _ => {
            eprintln!("usage:");
            eprintln!("    tatum set render <dir> [-o out.wav] [--bars N]");
            eprintln!("    tatum set check  <dir> [--bars N] [--json]");
            eprintln!("    tatum set next   <dir> <candidate.synth> [--json]");
            eprintln!(
                "    tatum set play   <dir> [--phrase 8] [--ramp 4] [--blend 0] [--device <name>] [--midi <name>]"
            );
            eprintln!();
            eprintln!("A set is a directory of numbered .synth files, each the whole rig at a");
            eprintln!("moment. `render` walks them with real hot swaps. `check` validates the");
            eprintln!("whole set. `next` validates one proposed step against the last one, which");
            eprintln!("is the gate an agent writing a set has to pass. `play` plays it live: the");
            eprintln!("space bar or a pad moves to the next step on the next phrase line.");
            std::process::exit(2);
        }
    }
}

fn run(r: Result<(), String>) {
    if let Err(e) = r {
        eprintln!("error: {}", e);
        std::process::exit(1);
    }
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(|s| s.as_str())
}

/// Play a set live. Steps move on a key or a pad, landing on the next
/// multiple of `--phrase` bars; a tempo change between steps ramps over
/// `--ramp` bars (0 jumps).
fn cmd_play(args: &[String]) -> Result<(), String> {
    let (mut dir, mut phrase, mut ramp, mut blend) = (None, 8usize, 4.0f32, 0.0f32);
    let (mut device, mut rate, mut midi) = (None, None, None);
    let mut i = 0;
    while i < args.len() {
        let value = |i: usize| args.get(i + 1).map(|s| s.as_str());
        match args[i].as_str() {
            "--phrase" => {
                phrase = value(i)
                    .and_then(|v| v.parse().ok())
                    .filter(|&n| n > 0)
                    .ok_or("--phrase needs a number of bars")?;
                i += 1;
            }
            "--ramp" => {
                ramp = value(i)
                    .and_then(|v| v.parse().ok())
                    .filter(|&n: &f32| n >= 0.0)
                    .ok_or("--ramp needs a number of bars")?;
                i += 1;
            }
            "--blend" => {
                blend = value(i)
                    .and_then(|v| v.parse().ok())
                    .filter(|&n: &f32| n >= 0.0)
                    .ok_or("--blend needs a number of bars")?;
                i += 1;
            }
            "--device" | "-d" => {
                device = Some(value(i).ok_or("--device needs a name")?);
                i += 1;
            }
            "--rate" => {
                rate = Some(value(i).and_then(|v| v.parse().ok()).ok_or("--rate needs a number in Hz")?);
                i += 1;
            }
            "--midi" => {
                midi = Some(value(i).ok_or("--midi needs part of an input's name")?);
                i += 1;
            }
            other if other.starts_with('-') => return Err(format!("unknown flag '{}'", other)),
            other => dir = Some(other),
        }
        i += 1;
    }
    let dir = dir.ok_or("usage: tatum set play <dir> [--phrase 8] [--ramp 4]")?;
    let steps = load(Path::new(dir), DEFAULT_BARS)?;
    let first = steps[0].path.to_string_lossy().into_owned();
    let mut nav = crate::setnav::SetNav::new(steps, phrase, ramp);
    nav.blend_bars = blend;
    crate::live::run(&first, true, device, rate, midi, Default::default(), Some(nav))
}

fn cmd_render(args: &[String]) -> Result<(), String> {
    let dir = args.first().ok_or("usage: tatum set render <dir> [-o out.wav]")?;
    let bars = flag(args, "--bars").and_then(|v| v.parse().ok()).unwrap_or(DEFAULT_BARS);
    let out = flag(args, "-o").unwrap_or("set.wav").to_string();
    let steps = load(Path::new(dir), bars)?;

    let total_bars: u32 = steps.iter().map(|s| s.bars).sum();
    eprintln!("{}: {} steps, {} bars", dir, steps.len(), total_bars);
    let rendered = render_set(&steps)?;
    let secs = rendered.l.len() as f32 / SAMPLE_RATE;
    for (i, step) in steps.iter().enumerate() {
        let at = rendered.applied.get(i).map(|a| a.1).unwrap_or(0) as f32 / SAMPLE_RATE;
        eprintln!(
            "  {:>3}:{:02}  {:<14} {:>3} bars  {:<10} {}",
            (at / 60.0) as u32,
            (at % 60.0) as u32,
            step.name(),
            step.bars,
            step.phase,
            step.note
        );
    }
    let bytes = tatum_core::wav::encode_stereo_16(&rendered.l, &rendered.r, SAMPLE_RATE as u32);
    fs::write(&out, bytes).map_err(|e| format!("{}: {}", out, e))?;
    eprintln!("wrote {} ({:.0}:{:02.0})", out, secs / 60.0, secs % 60.0);
    Ok(())
}

fn cmd_check(args: &[String]) -> Result<(), String> {
    let dir = args.first().ok_or("usage: tatum set check <dir>")?;
    let bars = flag(args, "--bars").and_then(|v| v.parse().ok()).unwrap_or(DEFAULT_BARS);
    let json = args.iter().any(|a| a == "--json");
    let steps = load(Path::new(dir), bars)?;

    let mut planner = LivePlanner::new();
    let mut player = LivePlayer::new();
    let mut rows = Vec::new();
    let mut bad = 0;

    for (i, step) in steps.iter().enumerate() {
        let kind = match planner.plan(&step.src, player.generation()) {
            Ok(p) => {
                let k = p.describe();
                player.apply(p);
                if i == 0 {
                    player.start();
                }
                k
            }
            Err(e) => {
                bad += 1;
                rows.push((step, String::from("error"), None, describe(&e)));
                continue;
            }
        };
        let pr = probe(&step.src, 2)?;
        let mut why = String::new();
        if i > 0 && kind == "unchanged" {
            bad += 1;
            why = String::from("changes nothing");
        }
        if pr.peak < 1e-4 {
            bad += 1;
            why = String::from("silent");
        }
        if pr.clipped > 0 {
            bad += 1;
            why = format!("{} clipped samples", pr.clipped);
        }
        rows.push((step, String::from(kind), Some(pr), why));
    }

    if json {
        println!("{{\"steps\":[");
        for (i, (s, kind, pr, why)) in rows.iter().enumerate() {
            let (peak, rms, silent) = pr.as_ref().map_or((0.0, 0.0, 0), |p| (p.peak, p.rms_db, p.silent_tracks));
            println!("  {{\"file\":\"{}\",\"bars\":{},\"phase\":\"{}\",\"energy\":{},\"transition\":\"{}\",\"peak\":{:.4},\"rms_db\":{:.2},\"muted_tracks\":{},\"problem\":\"{}\"}}{}",
                s.name(), s.bars, s.phase,
                s.energy.map(|e| format!("{:.1}", e)).unwrap_or_else(|| String::from("null")),
                kind, peak, rms, silent, why, if i + 1 == rows.len() { "" } else { "," });
        }
        println!("],\"problems\":{}}}", bad);
    } else {
        println!(
            "{:<14} {:>5} {:<10} {:>6} {:<10} {:>7} {:>8} {:>6}  problem",
            "step", "bars", "phase", "energy", "transition", "peak", "rms dB", "muted"
        );
        for (s, kind, pr, why) in &rows {
            let (peak, rms, silent) = pr.as_ref().map_or((0.0, 0.0, 0), |p| (p.peak, p.rms_db, p.silent_tracks));
            let en = s.energy.map(|e| format!("{:.0}", e)).unwrap_or_else(|| String::from("-"));
            println!(
                "{:<14} {:>5} {:<10} {:>6} {:<10} {:>7.3} {:>8.1} {:>6}  {}",
                s.name(),
                s.bars,
                s.phase,
                en,
                kind,
                peak,
                rms,
                silent,
                why
            );
        }
    }
    if bad > 0 {
        return Err(format!("{} problems", bad));
    }
    Ok(())
}

/// True when a step is a build: a drum part whose name says it is a fill or a
/// roll. A build is a promise, and the only thing that can follow it is the
/// thing it promised. Two builds in a row means neither one explodes -- the
/// second steals the first one's arrival and then has nothing to arrive into.
fn is_a_build(src: &str) -> bool {
    src.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| l.split("play ").nth(1))
        .filter_map(|r| r.split_whitespace().next())
        .any(|p| p.contains("fill") || p.contains("roll_up") || p.contains("riser"))
}

/// The gate. An agent proposes the next state; this says yes or no and why.
fn cmd_next(args: &[String]) -> Result<(), String> {
    let dir = args.first().ok_or("usage: tatum set next <dir> <candidate.synth>")?;
    let cand_path = args.get(1).ok_or("usage: tatum set next <dir> <candidate.synth>")?;
    let json = args.iter().any(|a| a == "--json");
    let max_voices: Option<usize> = flag(args, "--max-voices").and_then(|v| v.parse().ok());
    let steps = load(Path::new(dir), DEFAULT_BARS)?;
    let prev = steps.last().ok_or("the set is empty")?;
    let cand = crate::include::Source::load(Path::new(cand_path))?.text;

    let mut problems: Vec<String> = Vec::new();

    // 1. Does it compile at all?
    let pr = match probe(&cand, 2) {
        Ok(p) => p,
        Err(e) => {
            emit(json, "rejected", "", &[format!("does not compile: {}", e)], None, f32::NEG_INFINITY);
            return Err(String::from("rejected"));
        }
    };

    // 2. How would the engine get there from the state before it?
    let mut planner = LivePlanner::new();
    let mut player = LivePlayer::new();
    let p0 = planner.plan(&prev.src, player.generation()).map_err(|e| describe(&e))?;
    player.apply(p0);
    player.start();
    let transition = match planner.plan(&cand, player.generation()) {
        Ok(p) => p.describe(),
        Err(e) => {
            emit(json, "rejected", "", &[format!("does not compile: {}", describe(&e))], None, f32::NEG_INFINITY);
            return Err(String::from("rejected"));
        }
    };
    if transition == "unchanged" {
        problems.push(String::from("changes nothing the engine can hear"));
    }

    // 3. Is it playable?
    if pr.peak < 1e-4 {
        problems.push(String::from("silent"));
    }
    if pr.clipped > 0 {
        problems.push(format!("{} clipped samples", pr.clipped));
    }
    if pr.silent_tracks == pr.tracks {
        problems.push(String::from("every track is muted"));
    }
    // Headroom. Once the mix arrives over full scale the limiter decides how
    // loud the step is, not the faders, and every step lands at the same
    // loudness however the set is arranged. Measured on one set: at 1.36 in,
    // the master crushed the crest from 4.7 to 3.4 and the whole second half
    // sat within 0.2 dB. Scaling the same steps to 0.6 recovered the crest to
    // 4.6 and reopened the gap between the quiet step and the loud one.
    if pr.master_in > 1.0 {
        problems.push(format!(
            "the mix hands the master {:.2}, over full scale: the limiter is arranging the set, not you. Pull levels or saturation back",
            pr.master_in
        ));
    }
    if let Some(max) = max_voices {
        if pr.voices > max {
            problems.push(format!(
                "{} voices sounding at once, more than {}: bring one in by taking one out",
                pr.voices, max
            ));
        }
    }

    // 4. Is the difference audible? A step that compiles, differs in the text
    //    and sounds the same has spent a turn for nothing.
    let a = SongEngine::from_source(&prev.src)?;
    let b = SongEngine::from_source(&cand)?;
    let (mut a, mut b) = (a, b);
    a.start();
    b.start();
    let (al, _) = a.render(2);
    let (bl, _) = b.render(2);
    let diff = difference_db(&al, &bl);
    if diff < -40.0 {
        problems.push(format!("inaudible: {:.0} dB from the step before it", diff));
    }

    // 5. A build has to explode. If the step before this one was a fill or a
    //    roll, it promised an arrival, and this step is the arrival.
    if is_a_build(&prev.src) {
        let before = probe(&prev.src, 2)?;
        if is_a_build(&cand) {
            problems.push(String::from(
                "the step before this one was already a build: two in a row means neither explodes",
            ));
        } else if pr.rms_db < before.rms_db + 1.5 {
            problems.push(format!(
                "the step before was a build and this does not land: {:.1} dB against {:.1} dB, needs at least 1.5 dB more",
                pr.rms_db, before.rms_db
            ));
        }
    }

    let verdict = if problems.is_empty() { "accepted" } else { "rejected" };
    emit(json, verdict, transition, &problems, Some(&pr), diff);
    if problems.is_empty() {
        Ok(())
    } else {
        Err(String::from("rejected"))
    }
}

fn emit(json: bool, verdict: &str, transition: &str, problems: &[String], pr: Option<&Probe>, diff: f32) {
    let (peak, rms, muted, voices, floor, min) = pr
        .map_or((0.0, 0.0, 0, 0, 0.0, 0.0), |p| (p.peak, p.rms_db, p.silent_tracks, p.voices, p.floor_db, p.master_in));
    if json {
        println!("{{\"verdict\":\"{}\",\"transition\":\"{}\",\"peak\":{:.4},\"rms_db\":{:.2},\"voices\":{},\"floor_db\":{:.1},\"master_in\":{:.2},\"muted_tracks\":{},\"difference_db\":{:.1},\"problems\":[{}]}}",
            verdict, transition, peak, rms, voices, floor, min, muted, diff,
            problems.iter().map(|p| format!("\"{}\"", p)).collect::<Vec<_>>().join(","));
    } else {
        println!("{}: {} transition, rms {:.1} dB, {} voices, floor {:+.1} dB over the mids, master in {:.2}, {:.0} dB from the step before",
            verdict, transition, rms, voices, floor, min, diff);
        for p in problems {
            println!("  - {}", p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RIG: &str = r#"
tempo 120
scale C minor
module bass low { cutoff 0.5 sustain 0.8 }
pattern line { C2:0.9 - - -  C2:0.9 - - -  C2:0.9 - - -  C2:0.9 - - - }
track bass { play line using low level 0.5 out > lowpass(600, 0.1, wet=0.0) as lp > master }
master { in > out }
"#;

    #[test]
    fn the_header_travels_in_the_file() {
        let src = format!("# set: bars=48 phase=build energy=5\n# set-note: reese in\n{RIG}");
        let (bars, phase, energy, note, _) = parse_header(&src, 32);
        assert_eq!(bars, 48);
        assert_eq!(phase, "build");
        assert_eq!(energy, Some(5.0));
        assert_eq!(note, "reese in");
    }

    /// The header is a note to the tooling, not part of the language. A set
    /// someone wrote by hand should not fail to play because of a stray word.
    #[test]
    fn a_missing_or_odd_header_falls_back_instead_of_failing() {
        let (bars, phase, energy, note, _) = parse_header(RIG, 32);
        assert_eq!((bars, energy), (32, None));
        assert!(phase.is_empty() && note.is_empty());

        let (bars, ..) = parse_header("# set: bars=nope colour=blue\n", 32);
        assert_eq!(bars, 32, "an unreadable value keeps the default");
    }

    /// The whole point of a set: bringing a voice in or switching an effect on
    /// hands over without rebuilding the engine, so nothing restarts.
    #[test]
    fn level_and_wet_steps_stay_on_the_fast_path() {
        let steps = [RIG.to_string(), RIG.replace("wet=0.0", "wet=1.0"), RIG.replace("level 0.5", "level 0.2")];
        let mut planner = LivePlanner::new();
        let mut player = LivePlayer::new();
        player.apply(planner.plan(&steps[0], player.generation()).expect("first"));
        player.start();
        for s in &steps[1..] {
            let plan = planner.plan(s, player.generation()).expect("plans");
            assert_eq!(plan.describe(), "fast", "a level or wet step must not need a swap");
            player.apply(plan);
        }
    }

    /// And the walk produces one continuous render whose length is the bars
    /// the steps asked for, not the bars of whichever state happened to load.
    #[test]
    fn the_walk_lasts_as_long_as_the_steps_say() {
        let mk = |n: u32, src: String| Step {
            path: PathBuf::from(format!("{n:03}.synth")),
            src,
            bars: 2,
            phase: String::new(),
            energy: None,
            note: String::new(),
            blend: None,
        };
        let steps = vec![mk(0, RIG.to_string()), mk(1, RIG.replace("wet=0.0", "wet=1.0"))];
        let out = render_set(&steps).expect("renders");
        // Two steps of two bars at 120 BPM in 4/4 is eight seconds.
        let secs = out.l.len() as f32 / SAMPLE_RATE;
        assert!((secs - 8.0).abs() < 0.2, "expected about 8 s, got {secs:.2}");
        assert_eq!(out.l.len(), out.r.len());
        assert!(out.l.iter().any(|v| v.abs() > 0.01), "the walk rendered silence");
    }
}
