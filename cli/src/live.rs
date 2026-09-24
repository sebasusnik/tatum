//! `tatum play` and `tatum watch`: the native live session.
//!
//! Three threads. The audio callback owns the `LivePlayer` and does nothing
//! but apply plans it receives, render, and hand retired engines back. The
//! main thread watches the file, plans (parse, compile, diff, build the new
//! engine) and sends plans down a bounded channel; it also drops what the
//! callback retires, so no engine is ever freed on the audio thread. A third
//! thread reads stdin for `q`.

use std::io::BufRead;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tatum_core::live::{Applied, LivePlanner, LivePlayer, Plan, Retired};
use tatum_core::{BLOCK_SIZE, SAMPLE_RATE};

use crate::include::Source;
use crate::resample::Resampler;

/// What the audio thread tells the main thread.
enum Event {
    Applied(Applied),
    Swapped { bar: usize },
    Finished,
    /// A retired engine could not be handed back and was dropped in place.
    DroppedInPlace,
}

/// Counters the callback updates and the main thread reads.
struct Stats {
    /// Generation the player is on, so the planner knows what landed.
    generation: AtomicU64,
    callbacks: AtomicU64,
    worst_ns: AtomicU64,
    /// Callbacks that took longer than the audio they produced.
    late: AtomicU64,
    frames: AtomicU64,
}

pub fn cmd(args: &[String], watch: bool) {
    let verb = if watch { "watch" } else { "play" };
    let mut path: Option<&str> = None;
    let mut device: Option<&str> = None;
    let mut rate: Option<u32> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--device" | "-d" => {
                i += 1;
                device = args.get(i).map(|s| s.as_str());
                if device.is_none() {
                    eprintln!("error: --device needs a name");
                    std::process::exit(1);
                }
            }
            "--rate" => {
                i += 1;
                rate = args.get(i).and_then(|s| s.parse().ok());
                if rate.is_none() {
                    eprintln!("error: --rate needs a number in Hz (for example 48000)");
                    std::process::exit(1);
                }
            }
            "--list-devices" => {
                list_devices();
                return;
            }
            other if other.starts_with('-') => {
                eprintln!("error: unknown flag '{}'", other);
                std::process::exit(1);
            }
            other => path = Some(other),
        }
        i += 1;
    }
    let Some(path) = path else {
        eprintln!("error: missing input file");
        eprintln!("usage: tatum {} <song.synth> [--device <name>]", verb);
        std::process::exit(1);
    };
    if let Err(msg) = run(path, watch, device, rate) {
        eprintln!("error: {}", msg);
        std::process::exit(1);
    }
}

fn list_devices() {
    let host = cpal::default_host();
    let default = host.default_output_device().and_then(|d| d.description().ok()).map(|d| d.to_string());
    match host.output_devices() {
        Ok(devices) => {
            for d in devices {
                let name = d.description().map(|x| x.to_string()).unwrap_or_else(|_| "?".into());
                let note = match pick_config(&d, None) {
                    Ok((_, rate)) if rate == SAMPLE_RATE as u32 => String::new(),
                    Ok((_, rate)) => format!("  [resampled to {} Hz]", rate),
                    Err(_) => "  [no stereo f32 output]".to_string(),
                };
                println!("{}{}{}", name,
                    if Some(&name) == default.as_ref() { "  (default)" } else { "" }, note);
            }
        }
        Err(e) => eprintln!("error: cannot list devices: {}", e),
    }
}

/// The engine renders 44.1 kHz stereo. Take that rate when the device offers
/// it (no conversion, sample-exact), otherwise the device's own default rate
/// if it comes in stereo f32, otherwise the highest stereo f32 rate it has;
/// the output is resampled to it. Bluetooth headphones are the usual case:
/// they offer 48 kHz and 24 kHz and nothing else.
fn pick_config(device: &cpal::Device, forced: Option<u32>) -> Result<(cpal::StreamConfig, u32), String> {
    let engine = SAMPLE_RATE as u32;
    let ranges: Vec<_> = device.supported_output_configs().map_err(|e| e.to_string())?
        .filter(|r| r.channels() == 2 && r.sample_format() == cpal::SampleFormat::F32)
        .collect();
    let at = |rate: u32| ranges.iter()
        .find(|r| r.min_sample_rate() <= rate && r.max_sample_rate() >= rate)
        .and_then(|r| (*r).try_with_sample_rate(rate))
        .map(|c| (c.config(), rate));
    // `--rate` forces the device rate, to hear the converter on a device
    // that would not otherwise need it.
    if let Some(rate) = forced {
        return at(rate).ok_or_else(|| format!("the device does not offer {} Hz stereo f32", rate));
    }
    if let Some(found) = at(engine) {
        return Ok(found);
    }
    if let Ok(default) = device.default_output_config() {
        if let Some(found) = at(default.sample_rate()) {
            return Ok(found);
        }
    }
    let best = ranges.iter().map(|r| r.max_sample_rate()).filter(|&r| r <= 96_000).max();
    match best.and_then(at) {
        Some(found) => Ok(found),
        None => Err("the device has no stereo f32 output configuration".to_string()),
    }
}

fn open_device(wanted: Option<&str>) -> Result<cpal::Device, String> {
    let host = cpal::default_host();
    match wanted {
        None => host.default_output_device().ok_or_else(|| "no default output device".to_string()),
        Some(name) => {
            let needle = name.to_lowercase();
            let mut names = Vec::new();
            for d in host.output_devices().map_err(|e| e.to_string())? {
                let desc = d.description().map(|x| x.to_string()).unwrap_or_default();
                if desc.to_lowercase().contains(&needle) {
                    return Ok(d);
                }
                names.push(desc);
            }
            Err(format!("no output device matching '{}'. Available: {}", name, names.join(", ")))
        }
    }
}

fn run(path: &str, watch: bool, device_name: Option<&str>, forced_rate: Option<u32>) -> Result<(), String> {
    let mut source = Source::load(std::path::Path::new(path))?;
    let mut planner = LivePlanner::new();
    let mut player = LivePlayer::new();
    match planner.plan(&source.text, player.generation()) {
        Ok(plan) => { player.apply(plan); }
        Err(err) => {
            source.print_errors(&err);
            return Err("the song does not compile".into());
        }
    }
    let (tempo, bars, tracks) = {
        let e = player.engine().expect("loaded");
        (e.tempo(), e.arrangement_bars(), e.track_count())
    };

    let device = open_device(device_name)?;
    let (config, rate) = pick_config(&device, forced_rate)?;
    // None at the engine's own rate: the samples reach the device untouched.
    let mut resampler = if rate == SAMPLE_RATE as u32 { None } else { Some(Resampler::new(SAMPLE_RATE as u32, rate)) };
    let device_desc = device.description().map(|d| d.to_string()).unwrap_or_else(|_| "?".into());

    let (plan_tx, plan_rx): (SyncSender<Plan>, Receiver<Plan>) = sync_channel(4);
    let (event_tx, event_rx): (SyncSender<Event>, Receiver<Event>) = sync_channel(64);
    let (retired_tx, retired_rx): (SyncSender<Retired>, Receiver<Retired>) = sync_channel(16);
    let stats = Arc::new(Stats {
        generation: AtomicU64::new(player.generation()),
        callbacks: AtomicU64::new(0),
        worst_ns: AtomicU64::new(0),
        late: AtomicU64::new(0),
        frames: AtomicU64::new(0),
    });
    let stats_cb = Arc::clone(&stats);

    player.start();
    let mut was_running = true;
    let stream = device.build_output_stream(
        config,
        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            let t0 = Instant::now();
            // Plans first, so an edit lands in this callback, not the next.
            while let Ok(plan) = plan_rx.try_recv() {
                let applied = player.apply(plan);
                if applied == Applied::Loaded && !player.running() {
                    // Nothing was playing (the arrangement had ended):
                    // the new song starts from the top.
                    player.start();
                    was_running = true;
                }
                let _ = event_tx.try_send(Event::Applied(applied));
            }
            let mut l = [0.0f32; BLOCK_SIZE];
            let mut r = [0.0f32; BLOCK_SIZE];
            for chunk in data.chunks_mut(BLOCK_SIZE * 2) {
                let frames = chunk.len() / 2;
                match resampler.as_mut() {
                    None => {
                        if let Some(bar) = player.process(&mut l[..frames], &mut r[..frames]) {
                            let _ = event_tx.try_send(Event::Swapped { bar });
                        }
                    }
                    Some(rs) => {
                        // Render engine blocks until the converter has enough
                        // input for this chunk of device frames, then pull.
                        while rs.needed(frames) > 0 && rs.can_push() {
                            let mut bl = [0.0f32; BLOCK_SIZE];
                            let mut br = [0.0f32; BLOCK_SIZE];
                            if let Some(bar) = player.process(&mut bl, &mut br) {
                                let _ = event_tx.try_send(Event::Swapped { bar });
                            }
                            rs.push(&bl, &br);
                        }
                        rs.pull(&mut l[..frames], &mut r[..frames]);
                    }
                }
                for (i, frame) in chunk.chunks_mut(2).enumerate() {
                    frame[0] = l[i];
                    if frame.len() > 1 { frame[1] = r[i]; }
                }
            }
            while let Some(retired) = player.take_retired() {
                if let Err(TrySendError::Full(_)) = retired_tx.try_send(retired) {
                    let _ = event_tx.try_send(Event::DroppedInPlace);
                }
            }
            if was_running && !player.running() {
                was_running = false;
                let _ = event_tx.try_send(Event::Finished);
            }
            stats_cb.generation.store(player.generation(), Ordering::Relaxed);
            stats_cb.callbacks.fetch_add(1, Ordering::Relaxed);
            stats_cb.frames.fetch_add((data.len() / 2) as u64, Ordering::Relaxed);
            let ns = t0.elapsed().as_nanos() as u64;
            stats_cb.worst_ns.fetch_max(ns, Ordering::Relaxed);
            let budget_ns = (data.len() / 2) as u64 * 1_000_000_000 / rate as u64;
            if ns > budget_ns {
                stats_cb.late.fetch_add(1, Ordering::Relaxed);
            }
        },
        |err| eprintln!("audio stream error: {}", err),
        None,
    ).map_err(|e| format!("cannot open output stream: {}", e))?;
    stream.play().map_err(|e| format!("cannot start output stream: {}", e))?;

    eprintln!("{} {} on {}{}", verb(watch), path, device_desc,
        if rate == SAMPLE_RATE as u32 { String::new() } else { format!(" (resampled {} -> {} Hz)", SAMPLE_RATE as u32, rate) });
    eprintln!("  {} BPM, {} tracks, {}", tempo, tracks,
        if bars > 0 { format!("{} bars arranged", bars) } else { "no arrangement: loops until you stop it".to_string() });
    eprintln!("  {}q + Enter to quit", if watch { "save the file to re-evaluate it; " } else { "" });

    // stdin reader: `q` quits.
    let (quit_tx, quit_rx) = sync_channel::<()>(1);
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            match line {
                Ok(l) if l.trim() == "q" => { let _ = quit_tx.send(()); break; }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });

    let started = Instant::now();
    let mut last_mtime: Option<SystemTime> = source.newest_mtime();
    let mut swaps = 0u32;
    let mut fast = 0u32;
    let mut rejected = 0u32;
    let mut dropped_in_place = 0u32;
    let mut finished = false;
    loop {
        std::thread::sleep(Duration::from_millis(50));
        // Everything the callback retired is freed here.
        while let Ok(_retired) = retired_rx.try_recv() {}
        while let Ok(ev) = event_rx.try_recv() {
            let t = started.elapsed().as_secs_f32();
            match ev {
                Event::Applied(Applied::Fast) => { fast += 1; eprintln!("[{:7.2}s] applied instantly", t); }
                Event::Applied(Applied::Queued) => eprintln!("[{:7.2}s] queued for the next bar", t),
                Event::Applied(Applied::Loaded) => eprintln!("[{:7.2}s] loaded (nothing was playing)", t),
                Event::Applied(Applied::Unchanged) => eprintln!("[{:7.2}s] unchanged", t),
                Event::Applied(Applied::Stale) => eprintln!("[{:7.2}s] BUG: plan was stale, edit lost; save again", t),
                Event::Swapped { bar } => { swaps += 1; eprintln!("[{:7.2}s] swapped at bar {}", t, bar); }
                Event::Finished => { finished = true; eprintln!("[{:7.2}s] arrangement finished", t); }
                Event::DroppedInPlace => { dropped_in_place += 1; }
            }
        }
        if quit_rx.try_recv().is_ok() { break; }
        if finished && !watch { break; }
        if !watch { continue; }
        // Any file the song `use`s counts: editing the rig re-evaluates too.
        let now = source.newest_mtime();
        if now == last_mtime { continue; }
        last_mtime = now;
        // Editors write through a temporary file: a file missing for a tick
        // is retried, not reported. A `use` that no longer resolves is.
        let new_source = match Source::load(std::path::Path::new(path)) {
            Ok(s) => s,
            Err(e) if e.starts_with("cannot read") && !e.contains(':') => continue,
            Err(e) => {
                rejected += 1;
                eprintln!("{}\n  (still playing the last good version)", e);
                continue;
            }
        };
        if new_source.text == source.text { continue; }
        source = new_source;
        let generation = stats.generation.load(Ordering::Relaxed);
        match planner.plan(&source.text, generation) {
            Ok(plan) => {
                let kind = plan.describe();
                if plan_tx.send(plan).is_err() { break; }
                eprintln!("[{:7.2}s] saved: {}", started.elapsed().as_secs_f32(), kind);
            }
            Err(err) => {
                rejected += 1;
                source.print_errors(&err);
                eprintln!("  (still playing the last good version)");
            }
        }
    }
    drop(stream);

    let callbacks = stats.callbacks.load(Ordering::Relaxed);
    let frames = stats.frames.load(Ordering::Relaxed);
    let worst = stats.worst_ns.load(Ordering::Relaxed) as f64 / 1e6;
    let per_cb = if callbacks > 0 { frames as f64 / callbacks as f64 } else { 0.0 };
    let budget = per_cb / rate as f64 * 1e3;
    eprintln!("played {:.1} s in {} callbacks of ~{:.0} frames ({:.2} ms each)",
        frames as f64 / rate as f64, callbacks, per_cb, budget);
    eprintln!("worst callback {:.3} ms of {:.2} ms budget ({:.0}%), {} late",
        worst, budget, if budget > 0.0 { worst / budget * 100.0 } else { 0.0 },
        stats.late.load(Ordering::Relaxed));
    if watch {
        eprintln!("{} swaps, {} instant edits, {} rejected saves", swaps, fast, rejected);
    }
    if dropped_in_place > 0 {
        eprintln!("{} retired engines were dropped on the audio thread (main thread fell behind)", dropped_in_place);
    }
    Ok(())
}

fn verb(watch: bool) -> &'static str {
    if watch { "watching" } else { "playing" }
}
