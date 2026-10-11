//! `tatum set controls <dir>`: every control of every step, tried. A knob is
//! turned from one end to the other, a pad and a key held, the strip and
//! the wheel thrown, and what comes out is compared with what came out
//! without it. A control that changes nothing anywhere a hand could reach it
//! is reported: on stage it would be a knob that does nothing.
//!
//! A knob on a voice only a pad or a key plays (a grinder on B1, the lead
//! on the keyboard) is tried with that pad or key held too, so it counts as
//! working when it works in the only place it can.

use std::path::Path;

use tatum_core::dsl::ast::{MidiSource, Song, VoiceMove};
use tatum_core::live::{LivePlanner, LivePlayer, Plan};
use tatum_core::{BLOCK_SIZE, SAMPLE_RATE};

use crate::set::Step;

/// Something a hand does, applied as the step starts and kept.
#[derive(Clone, Debug)]
enum Touch {
    Knob(u8, u8),
    Pad(u8),
    Key(u8),
    Bend(u16),
    Voice(String),
    Scene(String),
}

/// Under this, relative to the louder of the two takes, a control changed
/// nothing: -60 dB is a thousandth.
const NOTHING_DB: f32 = -60.0;

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

/// How different two takes are, in dB below the louder of them.
fn difference(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    let d: f32 = (a[..n].iter().zip(&b[..n]).map(|(x, y)| (x - y) * (x - y)).sum::<f32>() / n.max(1) as f32).sqrt();
    let level = rms(&a[..n]).max(rms(&b[..n])).max(1e-9);
    20.0 * (d.max(1e-12) / level).log10()
}

/// The step played for `bars` with `touches` done on its first sample.
fn take(src: &str, touches: &[Touch], bars: f32) -> Result<Vec<f32>, String> {
    let mut planner = LivePlanner::new();
    planner.set_output_gain(1.0);
    planner.restart_lanes();
    let mut player = LivePlayer::new();
    let plan = planner.plan(src, player.generation()).map_err(|e| format!("{:?}", e))?;
    player.apply(plan);
    player.start();
    let g = player.generation();
    for t in touches {
        let plans: Vec<Plan> = match t {
            Touch::Knob(cc, v) => planner.knob(*cc, *v, g).plans,
            Touch::Pad(n) => planner.pad(*n, 110, g).unwrap_or_default(),
            Touch::Key(n) => planner.key(*n, 110, g).unwrap_or_default(),
            Touch::Bend(v) => planner.bend(*v, g),
            Touch::Voice(p) => {
                planner.voice(&VoiceMove::To(p.clone()), g);
                Vec::new()
            }
            Touch::Scene(name) => {
                let mut plans = planner.enter_scene(name, g).unwrap_or_default();
                plans.extend(planner.scene_values(name, None, g).unwrap_or_default());
                plans
            }
        };
        for p in plans {
            player.apply(p);
        }
    }
    let tempo = player.engine().map_or(138.0, |e| e.tempo());
    let n = (bars * 4.0 * 60.0 / tempo * SAMPLE_RATE) as usize;
    let (mut l, mut r) = ([0.0f32; BLOCK_SIZE], [0.0f32; BLOCK_SIZE]);
    let mut out = Vec::with_capacity(2 * n);
    while out.len() < 2 * n {
        player.process(&mut l, &mut r);
        // Left and right side by side: a pan moves one against the other.
        for (a, b) in l.iter().zip(&r) {
            out.push(*a);
            out.push(*b);
        }
        while player.take_retired().is_some() {}
    }
    Ok(out)
}

/// A knob turned all the way, to both ends, after a sweep that takes it
/// over wherever its target sits.
fn knob(page: &Option<String>, cc: u8, value: u8) -> Vec<Touch> {
    let mut t: Vec<Touch> = page.iter().map(|p| Touch::Voice(p.clone())).collect();
    t.extend((0..=16).map(|i| Touch::Knob(cc, (i * 8).min(127) as u8)));
    t.extend((0..=16).rev().map(|i| Touch::Knob(cc, (i * 8).min(127) as u8)));
    t.push(Touch::Knob(cc, value));
    t
}

/// A page by name (`None`: the `midi` block's own lines) and its knob
/// lines, by controller and what they move.
type PageLines = (Option<String>, Vec<(u8, String)>);

/// What one control did in one step.
#[derive(Clone, Debug)]
pub struct Verdict {
    pub control: String,
    pub what: String,
    /// `None`: it changed nothing anywhere it was tried. Otherwise how much,
    /// in dB, and with what held.
    pub heard: Option<(f32, String)>,
}

/// The lead and bass keys and the pads that play a voice: the places a
/// knob on a voice nobody plays by itself can be heard.
fn contexts(song: &Song) -> Vec<(String, Vec<Touch>)> {
    let mut out: Vec<(String, Vec<Touch>)> = vec![(String::new(), Vec::new())];
    use tatum_core::dsl::ast::ZoneKind;
    for z in &song.perform.zones {
        match z.kind {
            ZoneKind::Lead => {
                out.push((String::from("a lead key held"), vec![Touch::Key(z.low + 4)]));
                // Each scene puts another voice on the lead zone.
                for sc in song.perform.scenes.iter().skip(1) {
                    out.push((
                        format!("scene {}, a lead key held", sc.name),
                        vec![Touch::Scene(sc.name.clone()), Touch::Key(z.low + 4)],
                    ));
                }
            }
            ZoneKind::Bass => out.push((String::from("a bass key held"), vec![Touch::Key(z.low + 4)])),
            ZoneKind::Triggers => {}
        }
    }
    for m in &song.midi {
        if let MidiSource::Pad(n) = m.source {
            if m.target.starts_with("play.") || m.target.starts_with("hold.") {
                out.push((format!("pad {} held", n), vec![Touch::Pad(n)]));
            }
        }
    }
    out
}

fn try_in(src: &str, ctxs: &[(String, Vec<Touch>)], a: &[Touch], b: &[Touch], bars: f32) -> Option<(f32, String)> {
    for (name, ctx) in ctxs {
        let with = |t: &[Touch]| -> Vec<Touch> { ctx.iter().cloned().chain(t.iter().cloned()).collect() };
        let (Ok(x), Ok(y)) = (take(src, &with(a), bars), take(src, &with(b), bars)) else { continue };
        let d = difference(&x, &y);
        if d > NOTHING_DB {
            return Some((d, name.clone()));
        }
    }
    None
}

/// Every control of one step, tried.
pub fn audit_step(step: &Step, bars: f32) -> Result<Vec<Verdict>, String> {
    let src = crate::include::Source::load(&step.path)?.text;
    let mut probe = LivePlanner::new();
    probe.set_output_gain(1.0);
    probe.plan(&src, 0).map_err(|e| format!("{}: {:?}", step.name(), e))?;
    let song = probe.song().cloned().ok_or("no song")?;
    let ctxs = contexts(&song);
    let mut out = Vec::new();

    // Knobs and faders: the `midi` block's, page by page.
    let pages: Vec<PageLines> = {
        let block: Vec<(u8, String)> = song
            .midi
            .iter()
            .filter_map(|m| match m.source {
                MidiSource::Cc(cc) if !m.target.starts_with("voice") => Some((cc, m.target.replace('.', " "))),
                _ => None,
            })
            .collect();
        let first = song.perform.pages.first().map(|p| p.name.clone());
        let mut v = vec![(first.clone(), block)];
        for p in song.perform.pages.iter().skip(1) {
            let lines = p
                .knobs
                .iter()
                .filter_map(|m| match m.source {
                    MidiSource::Cc(cc) => Some((cc, m.target.replace('.', " "))),
                    _ => None,
                })
                .collect();
            v.push((Some(p.name.clone()), lines));
        }
        v
    };
    for (page, lines) in &pages {
        let mut ccs: Vec<u8> = lines.iter().map(|l| l.0).collect();
        ccs.sort();
        ccs.dedup();
        // On the first page a cc the page itself names comes from the page.
        for cc in ccs {
            let what: Vec<&str> = lines.iter().filter(|l| l.0 == cc).map(|l| l.1.as_str()).collect();
            let heard = try_in(&src, &ctxs, &knob(page, cc, 0), &knob(page, cc, 127), bars);
            let control = match page {
                Some(p) => format!("cc {} ({})", cc, p),
                None => format!("cc {}", cc),
            };
            out.push(Verdict { control, what: what.join(" + "), heard });
        }
    }

    // Pads and trigger keys: held, against nothing held.
    let none: [Touch; 0] = [];
    for m in &song.midi {
        let (control, touch) = match m.source {
            MidiSource::Pad(n) => (format!("pad {}", n), Touch::Pad(n)),
            MidiSource::Key(n) => (format!("key {}", n), Touch::Key(n)),
            _ => continue,
        };
        if out.iter().any(|v: &Verdict| v.control == control) {
            // A macro: several lines on one pad, tried once.
            if let Some(v) = out.iter_mut().find(|v| v.control == control) {
                v.what = format!("{} + {}", v.what, m.target.replace('.', " "));
            }
            continue;
        }
        let heard = try_in(&src, &ctxs, &none, &[touch], bars);
        out.push(Verdict { control, what: m.target.replace('.', " "), heard });
    }

    // The zones, the strip and the wheel, with the hands off and on.
    use tatum_core::dsl::ast::ZoneKind;
    for z in &song.perform.zones {
        if z.kind == ZoneKind::Triggers {
            continue;
        }
        let heard = try_in(&src, &ctxs[..1], &none, &[Touch::Key(z.low + 4)], bars);
        out.push(Verdict { control: format!("{} zone", z.kind.word()), what: String::from("a key held"), heard });
    }
    let strip = try_in(&src, &ctxs, &[Touch::Bend(0)], &[Touch::Bend(16383)], bars);
    out.push(Verdict { control: String::from("pitch strip"), what: String::from("down vs up"), heard: strip });
    let wheel = try_in(&src, &ctxs, &knob(&None, 1, 0), &knob(&None, 1, 127), bars);
    out.push(Verdict { control: String::from("mod wheel"), what: String::from("cc 1"), heard: wheel });
    Ok(out)
}

pub fn cmd(args: &[String]) -> Result<(), String> {
    let mut dir: Option<&str> = None;
    let mut only: Option<usize> = None;
    let mut bars = 2.0f32;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--step" => {
                i += 1;
                only = args.get(i).and_then(|v| v.parse().ok());
            }
            "--bars" => {
                i += 1;
                bars = args.get(i).and_then(|v| v.parse().ok()).unwrap_or(2.0);
            }
            other => dir = Some(other),
        }
        i += 1;
    }
    let dir = dir.ok_or("usage: tatum set controls <dir> [--step N] [--bars 2]")?;
    let steps = crate::set::load(Path::new(dir), crate::set::DEFAULT_BARS)?;
    let chosen: Vec<&Step> = match only {
        Some(n) => steps.get(n.saturating_sub(1)).into_iter().collect(),
        None => steps.iter().collect(),
    };
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let results: Vec<(String, Result<Vec<Verdict>, String>)> = std::thread::scope(|s| {
        let chunks: Vec<Vec<&Step>> =
            chosen.chunks(chosen.len().div_ceil(threads).max(1)).map(|c| c.to_vec()).collect();
        let handles: Vec<_> = chunks
            .into_iter()
            .map(|chunk| {
                s.spawn(move || chunk.into_iter().map(|st| (st.name(), audit_step(st, bars))).collect::<Vec<_>>())
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
    });
    for (name, r) in &results {
        match r {
            Err(e) => println!("{}\terror\t{}", name, e),
            Ok(vs) => {
                for v in vs {
                    match &v.heard {
                        None => println!("{}\tNOTHING\t{}\t{}", name, v.control, v.what),
                        Some((db, ctx)) => println!(
                            "{}\tok {:.0} dB{}\t{}\t{}",
                            name,
                            db,
                            if ctx.is_empty() { String::new() } else { format!(" ({})", ctx) },
                            v.control,
                            v.what
                        ),
                    }
                }
            }
        }
    }
    Ok(())
}
