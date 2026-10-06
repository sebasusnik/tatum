//! `tatum play` and `tatum watch`: the native live session.
//!
//! Three threads, plus MIDI's own. The audio callback owns the `LivePlayer`
//! and does nothing but apply plans it receives, render, and hand retired
//! engines back. The main thread watches the file, turns knobs, keys and pads
//! into plans, plans saves (parse, compile, diff, build the new engine) and
//! sends it all down a bounded channel; it also drops what the callback
//! retires, so no engine is ever freed on the audio thread. A third thread
//! reads stdin for `q`. MIDI arrives on a thread `midir` owns and is only
//! forwarded from there.

use std::io::{BufRead, IsTerminal};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, sync_channel, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tatum_core::live::{Applied, LivePlanner, LivePlayer, Plan, Retired};
use tatum_core::{BLOCK_SIZE, SAMPLE_RATE};

use crate::include::Source;
use crate::midi::{Event as Midi, DRUM_CHANNEL};
use tatum_core::dsl::ast::KeyAction;
use crate::resample::Resampler;

/// What the audio thread tells the main thread.
enum Event {
    Applied(Applied),
    Swapped {
        bar: usize,
    },
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
    /// The bar the engine is in, and its tempo as f32 bits, for a set to
    /// time its moves on.
    bar: AtomicU64,
    /// The step the engine is on since it started, for the screen's beat.
    step: AtomicU64,
    tempo: AtomicU64,
}

pub fn cmd(args: &[String], watch: bool) {
    let mut args = args.to_vec();
    let isolation = crate::debug::take_isolation(&mut args);
    let verb = if watch { "watch" } else { "play" };
    let mut path: Option<&str> = None;
    let mut device: Option<&str> = None;
    let mut rate: Option<u32> = None;
    let mut midi: Option<&str> = None;
    let mut tui = false;
    let mut glass = false;
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
            "--midi" => {
                i += 1;
                midi = args.get(i).map(|s| s.as_str());
                if midi.is_none() {
                    eprintln!("error: --midi needs part of an input's name (--list-midi shows them)");
                    std::process::exit(1);
                }
            }
            "--list-midi" => {
                crate::midi::list();
                return;
            }
            "--tui" => tui = true,
            "--buffer" => {
                i += 1;
                match args.get(i).and_then(|v| v.parse::<u32>().ok()).filter(|n| (64..=8192).contains(n)) {
                    Some(n) => {
                        let _ = BUFFER.set(n);
                    }
                    None => {
                        eprintln!("error: --buffer needs a number of frames, 64 to 8192 (like 1024)");
                        std::process::exit(1);
                    }
                }
            }
            // Sound painted with cell backgrounds only, for a terminal that
            // makes them translucent (Ghostty: `background-opacity-cells`).
            "--glass" => {
                tui = true;
                glass = true;
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
        eprintln!("usage: tatum {} <song.synth> [--device <name>] [--midi <name>] [--tui | --glass]", verb);
        std::process::exit(1);
    };
    if let Err(msg) = run(path, watch, device, rate, midi, isolation, None, tui, glass) {
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
                println!("{}{}{}", name, if Some(&name) == default.as_ref() { "  (default)" } else { "" }, note);
            }
        }
        Err(e) => eprintln!("error: cannot list devices: {}", e),
    }
}

/// `--buffer`: the frames the audio device is asked for per callback.
pub static BUFFER: std::sync::OnceLock<u32> = std::sync::OnceLock::new();

/// Errors the audio stream reported: dropouts, as a rule.
static STREAM_ERRORS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// The engine renders 44.1 kHz stereo. Take that rate when the device offers
/// it (no conversion, sample-exact), otherwise the device's own default rate
/// if it comes in stereo f32, otherwise the highest stereo f32 rate it has;
/// the output is resampled to it. Bluetooth headphones are the usual case:
/// they offer 48 kHz and 24 kHz and nothing else.
fn pick_config(device: &cpal::Device, forced: Option<u32>) -> Result<(cpal::StreamConfig, u32), String> {
    let engine = SAMPLE_RATE as u32;
    let ranges: Vec<_> = device
        .supported_output_configs()
        .map_err(|e| e.to_string())?
        .filter(|r| r.channels() == 2 && r.sample_format() == cpal::SampleFormat::F32)
        .collect();
    let at = |rate: u32| {
        ranges
            .iter()
            .find(|r| r.min_sample_rate() <= rate && r.max_sample_rate() >= rate)
            .and_then(|r| (*r).try_with_sample_rate(rate))
            .map(|c| (c.config(), rate))
    };
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

/// The live session. With `set`, `path` is its first step, the file watched
/// is whichever step plays, and the keys and pads move through the steps.
/// With `tui` it runs on one full screen instead of printing lines.
#[allow(clippy::too_many_arguments)]
pub fn run(
    path: &str,
    watch: bool,
    device_name: Option<&str>,
    forced_rate: Option<u32>,
    midi_name: Option<&str>,
    isolation: tatum_core::dsl::isolate::Isolation,
    mut set: Option<crate::setnav::SetNav>,
    tui: bool,
    glass: bool,
) -> Result<(), String> {
    let mut source = Source::load(std::path::Path::new(path))?;
    if !isolation.is_empty() {
        // Checked once, so a misspelt name stops here instead of silencing
        // everything. Later saves keep the same names.
        crate::debug::compile_or_exit(&source, &isolation);
    }
    let mut planner = LivePlanner::new();
    planner.isolate(isolation);
    if let Some(nav) = &set {
        planner.set_output_gain(crate::set::set_gain(&nav.steps)?);
        // Each step's `auto ... over N` starts on the step's first bar.
        planner.restart_lanes();
    }
    let mut player = LivePlayer::new();
    match planner.plan(&source.text, player.generation()) {
        Ok(plan) => {
            player.apply(plan);
        }
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
    let (mut config, rate) = pick_config(&device, forced_rate)?;
    // `--buffer 1024`: a bigger block per callback, for headroom when the
    // device's default leaves too little (a blend runs two engines at once).
    if let Some(&frames) = BUFFER.get() {
        config.buffer_size = cpal::BufferSize::Fixed(frames);
    }
    // None at the engine's own rate: the samples reach the device untouched.
    let mut resampler = if rate == SAMPLE_RATE as u32 { None } else { Some(Resampler::new(SAMPLE_RATE as u32, rate)) };
    let device_desc = device.description().map(|d| d.to_string()).unwrap_or_else(|_| "?".into());

    // Room for a burst of knob values on top of saves: a knob sends one plan
    // per thing it moves, per engine.
    let (plan_tx, plan_rx): (SyncSender<Plan>, Receiver<Plan>) = sync_channel(256);
    let (event_tx, event_rx): (SyncSender<Event>, Receiver<Event>) = sync_channel(64);
    let (retired_tx, retired_rx): (SyncSender<Retired>, Receiver<Retired>) = sync_channel(16);
    let stats = Arc::new(Stats {
        generation: AtomicU64::new(player.generation()),
        callbacks: AtomicU64::new(0),
        worst_ns: AtomicU64::new(0),
        late: AtomicU64::new(0),
        frames: AtomicU64::new(0),
        bar: AtomicU64::new(0),
        step: AtomicU64::new(0),
        tempo: AtomicU64::new(tempo.to_bits() as u64),
    });
    let stats_cb = Arc::clone(&stats);
    // What the screen draws, written by the callback. Only with a screen:
    // the per-band meters cost the audio thread work nobody would see.
    let telemetry = tui.then(|| Arc::new(crate::tui::Telemetry::new()));
    let telemetry_cb = telemetry.clone();

    player.start();
    let mut was_running = true;
    let stream = device
        .build_output_stream(
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
                    // Knob values are shown when they are sent, not here.
                    if applied != Applied::Control {
                        let _ = event_tx.try_send(Event::Applied(applied));
                    }
                }
                if let (Some(_), Some(e)) = (&telemetry_cb, player.engine_mut()) {
                    // A new engine arrives with it off; setting it is a store.
                    e.set_band_metering(true);
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
                            if let Some(t) = &telemetry_cb {
                                t.push(&l[..frames], &r[..frames]);
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
                                if let Some(t) = &telemetry_cb {
                                    t.push(&bl, &br);
                                }
                                rs.push(&bl, &br);
                            }
                            rs.pull(&mut l[..frames], &mut r[..frames]);
                        }
                    }
                    for (i, frame) in chunk.chunks_mut(2).enumerate() {
                        frame[0] = l[i];
                        if frame.len() > 1 {
                            frame[1] = r[i];
                        }
                    }
                }
                if let (Some(t), Some(e)) = (&telemetry_cb, player.engine_mut()) {
                    t.meter(e);
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
                if let Some(e) = player.engine() {
                    stats_cb.bar.store(e.current_bar() as u64, Ordering::Relaxed);
                    stats_cb.step.store(e.global_step() as u64, Ordering::Relaxed);
                    stats_cb.tempo.store(e.tempo().to_bits() as u64, Ordering::Relaxed);
                }
                stats_cb.callbacks.fetch_add(1, Ordering::Relaxed);
                stats_cb.frames.fetch_add((data.len() / 2) as u64, Ordering::Relaxed);
                let ns = t0.elapsed().as_nanos() as u64;
                stats_cb.worst_ns.fetch_max(ns, Ordering::Relaxed);
                let budget_ns = (data.len() / 2) as u64 * 1_000_000_000 / rate as u64;
                if ns > budget_ns {
                    stats_cb.late.fetch_add(1, Ordering::Relaxed);
                }
            },
            // Counted, not printed: a line written over the screen would tear
            // it. The summary at the end says how many, and how to get room.
            |_err| {
                STREAM_ERRORS.fetch_add(1, Ordering::Relaxed);
            },
            None,
        )
        .map_err(|e| format!("cannot open output stream: {}", e))?;
    stream.play().map_err(|e| format!("cannot start output stream: {}", e))?;

    let resampled = if rate == SAMPLE_RATE as u32 {
        String::new()
    } else {
        format!(" (resampled {} -> {} Hz)", SAMPLE_RATE as u32, rate)
    };
    // Every input is read even when the song maps no knob yet, so adding a
    // `midi` block while watching works without a restart.
    let (midi_tx, midi_rx) = channel::<Midi>();
    let inputs = crate::midi::open(midi_name, &midi_tx)?;
    let (key_tx, key_rx) = channel::<crate::keys::Key>();
    let (quit_tx, quit_rx) = sync_channel::<()>(1);

    let mut ui = match &telemetry {
        Some(t) => {
            let mut screen =
                crate::tui::Screen::new(Arc::clone(t), format!("{}{}", device_desc, resampled), inputs.names.clone())
                    .map_err(|e| format!("cannot open the screen: {}", e))?;
            screen.glass = glass;
            screen.say(format!("{} {}", verb(watch), path), crate::tui::Tone::Info);
            Ui::Screen(Box::new(screen))
        }
        None => Ui::Plain(Status::new()),
    };
    let _keys = if let Ui::Plain(_) = ui {
        eprintln!("{} {} on {}{}", verb(watch), path, device_desc, resampled);
        eprintln!(
            "  {} BPM, {} tracks, {}",
            tempo,
            tracks,
            if bars > 0 {
                format!("{} bars arranged", bars)
            } else {
                "no arrangement: loops until you stop it".to_string()
            }
        );
        if !inputs.names.is_empty() && (planner.has_knobs() || midi_name.is_some()) {
            eprintln!("  midi: {}", inputs.names.join(", "));
        } else if inputs.names.is_empty() && planner.has_knobs() {
            eprintln!("  midi: no input found; the knobs in this song are waiting for one");
        }
        match &set {
            Some(nav) => {
                eprintln!("  steps, {} bars to a phrase; a move lands on the next phrase line:", nav.phrase);
                for i in 0..nav.steps.len() {
                    eprintln!("    {}", nav.describe(i));
                }
                eprintln!("  space or → next, ← back, 1-9 a step, q quit; save the playing step to re-evaluate it");
                print_keyboard(&planner);
                Some(crate::keys::spawn(key_tx))
            }
            None if planner.has_keyboard() => {
                eprintln!("  q quits");
                print_keyboard(&planner);
                Some(crate::keys::spawn(key_tx))
            }
            None => {
                eprintln!("  {}q + Enter to quit", if watch { "save the file to re-evaluate it; " } else { "" });
                // stdin reader: `q` quits.
                std::thread::spawn(move || {
                    let stdin = std::io::stdin();
                    for line in stdin.lock().lines() {
                        match line {
                            Ok(l) if l.trim() == "q" => {
                                let _ = quit_tx.send(());
                                break;
                            }
                            Ok(_) => {}
                            Err(_) => break,
                        }
                    }
                });
                None
            }
        }
    } else {
        None
    };
    let title = |source: &Source, set: &Option<crate::setnav::SetNav>| match set {
        Some(nav) => nav.describe(nav.current),
        None => source.files[0].file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default(),
    };
    ui.song(&title(&source, &set), &source);

    // The first step of a set may name the scene it starts in.
    if let Some(scene) = set.as_ref().and_then(|n| n.steps[n.current].perform.clone()) {
        let generation = stats.generation.load(Ordering::Relaxed);
        let plans = planner
            .enter_scene(&scene, generation)
            .into_iter()
            .flatten()
            .chain(planner.scene_values(&scene, None, generation).into_iter().flatten());
        for plan in plans {
            let _ = plan_tx.send(plan);
        }
    }
    let started = Instant::now();
    let mut last_mtime: Option<SystemTime> = source.newest_mtime();
    let mut swaps = 0u32;
    let mut fast = 0u32;
    let mut rejected = 0u32;
    let mut dropped_in_place = 0u32;
    let mut finished = false;
    let mut knob_moves = 0u32;
    let mut notes_played = 0u32;
    // The newest value per controller not yet sent. A knob turned fast sends
    // far more values than anyone hears; only the last one of a burst goes,
    // and the same for the pitch strip. Notes are never merged: every key and
    // every hit is sent, in the order it was played.
    let mut unsent: [Option<u8>; 128] = [None; 128];
    let mut unsent_bend: Option<u16> = None;
    // A scene called from the computer's keys, and the bar it lands on.
    let mut pending_scene: Option<(String, usize)> = None;
    // What the screen's keys changed, newest last, to take back: every file
    // one key touched, as it was before.
    let mut undo: Vec<Vec<(std::path::PathBuf, String)>> = Vec::new();
    // The screen redraws at about 30 frames a second; lines need no hurry.
    let poll_every = Duration::from_millis(if ui.is_screen() { 33 } else { 50 });
    let mut next_poll = Instant::now() + poll_every;
    loop {
        // MIDI wakes the loop at once; otherwise it runs every poll.
        let first = match midi_rx.recv_timeout(next_poll.saturating_duration_since(Instant::now())) {
            Ok(event) => Some(event),
            Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => None,
        };
        let now_s = || started.elapsed().as_secs_f32();
        let mut readings = Vec::new();
        for event in first.into_iter().chain(std::iter::from_fn(|| midi_rx.try_recv().ok())) {
            // `i` on the screen: every message as it arrives, and what the
            // song does with it.
            if let Ui::Screen(screen) = &mut ui {
                let what = planner.song().and_then(|s| crate::midi::meaning(s, Some(&planner), &event));
                screen.midi_in(now_s(), crate::midi::describe(&event), what);
            }
            match event {
                Midi::Cc(cc, value) => unsent[cc as usize] = Some(value),
                Midi::Bend(value) => unsent_bend = Some(value),
                // How hard a held pad or key is pressed: a held effect follows it.
                Midi::Pressure { channel, note, value } => {
                    for plan in planner.pressure(channel == DRUM_CHANNEL, note, value) {
                        let _ = plan_tx.try_send(plan);
                    }
                }
                Midi::Other { .. } => {}
                Midi::Note { channel, note, velocity } => {
                    let generation = stats.generation.load(Ordering::Relaxed);
                    let pad = channel == DRUM_CHANNEL;
                    if pad && velocity > 0 {
                        if let Some(action) = planner.pad_navigation(note) {
                            match set.as_mut() {
                                Some(nav) => {
                                    let m = match action {
                                        tatum_core::midi::PadAction::Prev => crate::setnav::Move::Prev,
                                        tatum_core::midi::PadAction::Step(n) => crate::setnav::Move::To(n),
                                        _ => crate::setnav::Move::Next,
                                    };
                                    let bar = stats.bar.load(Ordering::Relaxed) as usize;
                                    let said = nav.ask(m, bar);
                                    ui.say(now_s(), &said, crate::tui::Tone::Info);
                                }
                                None => {
                                    readings.push(format!("pad {}: moves through a set, in `tatum set play`", note))
                                }
                            }
                            continue;
                        }
                    }
                    let plans = if pad {
                        planner.pad(note, velocity, generation)
                    } else {
                        planner.key(note, velocity, generation)
                    };
                    match plans {
                        Some(plans) => {
                            // Waits for room rather than drop one: a lost
                            // release is a note that never stops.
                            for plan in plans {
                                if plan_tx.send(plan).is_err() {
                                    break;
                                }
                            }
                            if velocity > 0 {
                                notes_played += 1;
                            }
                            // A quantized pad says it heard you, and when it
                            // will act: `pad 36 → bar`, on the way down and up.
                            if pad {
                                if let Some(q) = planner.pad_quantize(note) {
                                    let way = if velocity > 0 { "" } else { " (up)" };
                                    readings.push(format!("pad {}{} → {}", note, way, q.word()));
                                }
                            }
                        }
                        None if velocity > 0 => {
                            readings.push(format!("{} {} (not mapped)", if pad { "pad" } else { "key" }, note));
                        }
                        None => {}
                    }
                }
            }
        }
        if let Some(value) = unsent_bend {
            let generation = stats.generation.load(Ordering::Relaxed);
            let mut full = false;
            for plan in planner.bend(value, generation) {
                if let Err(TrySendError::Full(_)) = plan_tx.try_send(plan) {
                    full = true;
                }
            }
            if !full {
                unsent_bend = None;
                readings.push(planner.bend_reading());
            }
        }
        if unsent.iter().any(|v| v.is_some()) {
            let generation = stats.generation.load(Ordering::Relaxed);
            for (cc, slot) in unsent.iter_mut().enumerate() {
                let Some(value) = *slot else { continue };
                // With the knob list open, a knob tries the parameter the
                // list points at instead of whatever it is mapped to.
                let trying = match &ui {
                    Ui::Screen(s) => s.knob_target(),
                    Ui::Plain(_) => None,
                };
                let turn = match &trying {
                    Some(t) => planner.try_knob(&t.dotted(), value, generation),
                    None => planner.knob(cc as u8, value, generation),
                };
                if trying.is_some() {
                    if let Ui::Screen(s) = &mut ui {
                        s.knob_turned(cc as u8, turn.readings.first().cloned());
                    }
                }
                let mut full = false;
                for plan in turn.plans {
                    if let Err(TrySendError::Full(_)) = plan_tx.try_send(plan) {
                        full = true;
                    }
                }
                // A full channel keeps the value for the next round rather
                // than leave the knob somewhere it was only passing through.
                if !full {
                    *slot = None;
                }
                if turn.readings.is_empty() {
                    // Shown anyway: turning a knob and reading its number is
                    // how the `midi` block gets written in the first place.
                    readings.push(format!("cc {} = {} (not mapped)", cc, value));
                } else {
                    knob_moves += 1;
                    readings.extend(turn.readings);
                }
            }
        }
        if !readings.is_empty() {
            ui.readings(readings);
        }
        if Instant::now() < next_poll {
            continue;
        }
        next_poll = Instant::now() + poll_every;

        let mut quit = false;
        let mut keys: Vec<crate::keys::Key> = key_rx.try_iter().collect();
        if let Ui::Screen(screen) = &mut ui {
            let watched = set.as_ref().map(|n| n.path()).unwrap_or_else(|| std::path::PathBuf::from(path));
            for k in screen.keys() {
                match k {
                    crate::tui::Key::Quit => keys.push(crate::keys::Key::Quit),
                    crate::tui::Key::Next => keys.push(crate::keys::Key::Next),
                    crate::tui::Key::Prev => keys.push(crate::keys::Key::Prev),
                    crate::tui::Key::Step(n) => keys.push(crate::keys::Key::Step(n)),
                    crate::tui::Key::Named(n) => keys.push(crate::keys::Key::Named(n)),
                    // The screen writes the line in the file; the save is
                    // picked up below like one from the editor.
                    crate::tui::Key::Edit(op) => match edit_play(&watched, screen.selected_tracks(), op) {
                        Ok((before, said)) => {
                            undo.push(vec![(watched.clone(), before)]);
                            screen.say(said, crate::tui::Tone::Good);
                        }
                        Err(e) => screen.say(e, crate::tui::Tone::Bad),
                    },
                    // Mute and solo are a gesture, not part of the song: they
                    // go straight to the engine, at once, and a save keeps them.
                    crate::tui::Key::Mute | crate::tui::Key::Solo => {
                        let names: Vec<String> = screen.selected_tracks().into_iter().map(|(n, _)| n).collect();
                        let generation = stats.generation.load(Ordering::Relaxed);
                        let plans = if names.is_empty() {
                            Vec::new()
                        } else if k == crate::tui::Key::Mute {
                            // Each selected track flips on its own; each call
                            // returns every track's state, so the last is all
                            // that needs sending.
                            let mut plans = Vec::new();
                            for n in &names {
                                plans = planner.toggle_mute(n, generation);
                            }
                            plans
                        } else {
                            planner.toggle_solo_group(&names, generation)
                        };
                        for plan in plans {
                            if plan_tx.send(plan).is_err() {
                                break;
                            }
                        }
                        screen.soloed = planner.soloed().to_vec();
                    }
                    // The knob list keeps a knob on something by writing its
                    // `midi` line, where the song keeps its knobs.
                    crate::tui::Key::Knob(change) => match screen.knob_target() {
                        Some(target) => match crate::tui::knobs::write(&source.files, &target, change) {
                            Ok((before, said)) => {
                                undo.push(before);
                                screen.say(said, crate::tui::Tone::Good);
                            }
                            Err(e) => screen.say(e, crate::tui::Tone::Bad),
                        },
                        None => screen.say("nothing to put a knob on", crate::tui::Tone::Info),
                    },
                    crate::tui::Key::Undo => match undo.pop() {
                        Some(files) => {
                            let failed: Vec<String> = files
                                .into_iter()
                                .filter_map(|(file, before)| {
                                    std::fs::write(&file, before).err().map(|e| format!("{}: {}", file.display(), e))
                                })
                                .collect();
                            if failed.is_empty() {
                                screen.say("undone", crate::tui::Tone::Good);
                            } else {
                                screen.say(format!("cannot write {}", failed.join(", ")), crate::tui::Tone::Bad);
                            }
                        }
                        None => screen.say("nothing to undo", crate::tui::Tone::Info),
                    },
                }
            }
        }
        let (bindings, scene_bars) = planner.keyboard();
        for key in keys {
            // A key the song binds does what the `keyboard` block says;
            // any other keeps the set's own meaning.
            let key = match key {
                crate::keys::Key::Named(name) => {
                    let word = name.word();
                    match bindings.iter().find(|b| b.key == word).map(|b| &b.action) {
                        Some(KeyAction::Voice(mv)) => {
                            let generation = stats.generation.load(Ordering::Relaxed);
                            match planner.voice(mv, generation) {
                                Some(said) => ui.say(now_s(), &said, crate::tui::Tone::Good),
                                None => ui.say(now_s(), "no knob pages in this song", crate::tui::Tone::Bad),
                            }
                            continue;
                        }
                        Some(KeyAction::Perform(scene)) => {
                            let bar = stats.bar.load(Ordering::Relaxed) as usize;
                            // A phrase counts from the step's first bar.
                            let origin = set.as_ref().map_or(0, |n| bar + 1 - n.bar_in_step(bar));
                            let q = scene_bars as usize;
                            let target = origin + ((bar - origin) / q + 1) * q;
                            let generation = stats.generation.load(Ordering::Relaxed);
                            match planner.scene_values(scene, Some(target), generation) {
                                Some(plans) => {
                                    for plan in plans {
                                        if plan_tx.send(plan).is_err() {
                                            break;
                                        }
                                    }
                                    pending_scene = Some((scene.clone(), target));
                                    ui.say(
                                        now_s(),
                                        &format!("scene {} on bar {}", scene, target + 1),
                                        crate::tui::Tone::Info,
                                    );
                                }
                                None => ui.say(now_s(), &format!("no scene named {}", scene), crate::tui::Tone::Bad),
                            }
                            continue;
                        }
                        Some(KeyAction::Next) => crate::keys::Key::Next,
                        Some(KeyAction::Prev) => crate::keys::Key::Prev,
                        Some(KeyAction::Step(n)) => crate::keys::Key::Step(*n),
                        None => match name.default_move() {
                            Some(k) => k,
                            None => continue,
                        },
                    }
                }
                k => k,
            };
            let m = match key {
                crate::keys::Key::Quit => {
                    quit = true;
                    break;
                }
                crate::keys::Key::Next => crate::setnav::Move::Next,
                crate::keys::Key::Prev => crate::setnav::Move::Prev,
                crate::keys::Key::Step(n) => crate::setnav::Move::To(n),
                crate::keys::Key::Named(_) => continue,
            };
            if let Some(nav) = set.as_mut() {
                let bar = stats.bar.load(Ordering::Relaxed) as usize;
                let said = nav.ask(m, bar);
                ui.say(now_s(), &said, crate::tui::Tone::Info);
            }
        }
        if quit {
            break;
        }
        // A scene called from the keys takes the keyboard on its bar; its
        // values were sent when it was called and land there on their own.
        if let Some((scene, target)) = pending_scene.clone() {
            if stats.bar.load(Ordering::Relaxed) as usize >= target {
                pending_scene = None;
                let generation = stats.generation.load(Ordering::Relaxed);
                for plan in planner.enter_scene(&scene, generation).unwrap_or_default() {
                    if plan_tx.send(plan).is_err() {
                        break;
                    }
                }
                ui.say(now_s(), &format!("scene {}", planner.describe_scene(&scene)), crate::tui::Tone::Good);
            }
        }
        if let Some(nav) = set.as_mut() {
            let bar = stats.bar.load(Ordering::Relaxed) as usize;
            let tempo = f32::from_bits(stats.tempo.load(Ordering::Relaxed) as u32);
            let generation = stats.generation.load(Ordering::Relaxed);
            let before = nav.current;
            let (plans, said) = nav.tick(bar, tempo, generation, now_s(), &mut planner);
            for plan in plans {
                if plan_tx.send(plan).is_err() {
                    break;
                }
            }
            for line in said {
                ui.say(now_s(), &line, crate::tui::Tone::Info);
            }
            if nav.current != before {
                // A step whose header names a scene brings it in as it lands.
                if let Some(scene) = nav.steps[nav.current].perform.clone().filter(|s| planner.scene() != Some(s)) {
                    let generation = stats.generation.load(Ordering::Relaxed);
                    let plans = planner
                        .enter_scene(&scene, generation)
                        .into_iter()
                        .flatten()
                        .chain(planner.scene_values(&scene, None, generation).into_iter().flatten());
                    for plan in plans {
                        if plan_tx.send(plan).is_err() {
                            break;
                        }
                    }
                    pending_scene = None;
                    ui.say(now_s(), &format!("scene {}", planner.describe_scene(&scene)), crate::tui::Tone::Good);
                }
                if let Ok(s) = Source::load(&nav.path()) {
                    source = s;
                    last_mtime = source.newest_mtime();
                    ui.song(&nav.describe(nav.current), &source);
                }
            }
            if let Some((step, bars)) = nav.waiting(bar) {
                let line = format!(
                    "bar {} · next {} in {} bar{}",
                    bar + 1,
                    nav.describe(step),
                    bars,
                    if bars == 1 { "" } else { "s" }
                );
                ui.waiting(&line);
            }
        }

        // Everything the callback retired is freed here.
        while let Ok(_retired) = retired_rx.try_recv() {}
        while let Ok(ev) = event_rx.try_recv() {
            let t = now_s();
            match ev {
                Event::Applied(Applied::Fast) => {
                    fast += 1;
                    ui.say(t, "applied instantly", crate::tui::Tone::Good);
                }
                Event::Applied(Applied::Queued) => ui.say(t, "queued for the next bar", crate::tui::Tone::Info),
                Event::Applied(Applied::Loaded) => ui.say(t, "loaded (nothing was playing)", crate::tui::Tone::Info),
                Event::Applied(Applied::Unchanged) => ui.say(t, "unchanged", crate::tui::Tone::Info),
                Event::Applied(Applied::Control) => {}
                Event::Applied(Applied::Stale) => {
                    ui.say(t, "BUG: plan was stale, edit lost; save again", crate::tui::Tone::Bad)
                }
                Event::Swapped { bar } => {
                    swaps += 1;
                    let landed = set.as_mut().and_then(|nav| nav.landed(stats.generation.load(Ordering::Relaxed), t));
                    match landed {
                        Some(line) => {
                            ui.say(t, &format!("bar {}: {}", bar + 1, line), crate::tui::Tone::Good);
                            if let Some(nav) = set.as_ref() {
                                if let Ok(s) = Source::load(&nav.path()) {
                                    source = s;
                                    last_mtime = source.newest_mtime();
                                    ui.song(&nav.describe(nav.current), &source);
                                }
                            }
                        }
                        None => ui.say(t, &format!("swapped at bar {}", bar), crate::tui::Tone::Good),
                    }
                }
                Event::Finished => {
                    finished = true;
                    ui.say(t, "arrangement finished", crate::tui::Tone::Info);
                    if let Ui::Screen(screen) = &mut ui {
                        screen.finished();
                    }
                }
                Event::DroppedInPlace => {
                    dropped_in_place += 1;
                }
            }
        }
        if let Ui::Screen(screen) = &mut ui {
            screen.bound = bindings.iter().map(|b| b.key.clone()).collect();
            screen.voice = planner.voice_name();
            screen.perform = match (planner.scene(), &pending_scene) {
                (now, Some((next, bar))) => Some(format!("{} → {} @ bar {}", now.unwrap_or("—"), next, bar + 1)),
                (Some(now), None) => Some(now.to_string()),
                (None, None) => None,
            };
            if let Some(nav) = set.as_ref() {
                let bar = stats.bar.load(Ordering::Relaxed) as usize;
                screen.set = Some(crate::tui::SetView {
                    steps: nav.steps.iter().map(|s| s.name()).collect(),
                    current: nav.current,
                    next: nav.waiting(bar),
                    phrase: nav.phrase,
                    bar: nav.bar_in_step(bar),
                    bars: nav.steps[nav.current].bars as usize,
                    cues: nav.steps[nav.current].cues.clone(),
                });
            }
            screen.position(
                stats.bar.load(Ordering::Relaxed) as usize,
                stats.step.load(Ordering::Relaxed) as usize,
                f32::from_bits(stats.tempo.load(Ordering::Relaxed) as u32),
            );
            if let Err(e) = screen.draw() {
                drop(ui);
                return Err(format!("cannot draw the screen: {}", e));
            }
        }
        if quit_rx.try_recv().is_ok() {
            break;
        }
        if finished && !watch && !ui.is_screen() {
            break;
        }
        if !watch {
            continue;
        }
        // Any file the song `use`s counts: editing the rig re-evaluates too.
        let now = source.newest_mtime();
        if now == last_mtime {
            continue;
        }
        last_mtime = now;
        // Editors write through a temporary file: a file missing for a tick
        // is retried, not reported. A `use` that no longer resolves is.
        let watched = set.as_ref().map(|n| n.path()).unwrap_or_else(|| std::path::PathBuf::from(path));
        let new_source = match Source::load(&watched) {
            Ok(s) => s,
            Err(e) if e.starts_with("cannot read") && !e.contains(':') => continue,
            Err(e) => {
                rejected += 1;
                ui.rejected(vec![e]);
                continue;
            }
        };
        if new_source.text == source.text {
            continue;
        }
        source = new_source;
        let generation = stats.generation.load(Ordering::Relaxed);
        match planner.plan(&source.text, generation) {
            Ok(plan) => {
                let kind = plan.describe();
                if plan_tx.send(plan).is_err() {
                    break;
                }
                ui.saved(now_s(), kind);
                ui.song(&title(&source, &set), &source);
            }
            Err(err) => {
                rejected += 1;
                ui.rejected(source.error_lines(&err));
            }
        }
    }
    drop(inputs);
    drop(stream);
    // Put the terminal back before the summary is printed on it.
    drop(ui);

    let callbacks = stats.callbacks.load(Ordering::Relaxed);
    let frames = stats.frames.load(Ordering::Relaxed);
    let worst = stats.worst_ns.load(Ordering::Relaxed) as f64 / 1e6;
    let per_cb = if callbacks > 0 { frames as f64 / callbacks as f64 } else { 0.0 };
    let budget = per_cb / rate as f64 * 1e3;
    eprintln!(
        "played {:.1} s in {} callbacks of ~{:.0} frames ({:.2} ms each)",
        frames as f64 / rate as f64,
        callbacks,
        per_cb,
        budget
    );
    eprintln!(
        "worst callback {:.3} ms of {:.2} ms budget ({:.0}%), {} late",
        worst,
        budget,
        if budget > 0.0 { worst / budget * 100.0 } else { 0.0 },
        stats.late.load(Ordering::Relaxed)
    );
    let dropouts = STREAM_ERRORS.load(Ordering::Relaxed);
    if dropouts > 0 {
        eprintln!(
            "{} audio dropout{} (the device ran out of sound to play); `--buffer 1024` gives each callback more room",
            dropouts,
            if dropouts == 1 { "" } else { "s" }
        );
    }
    if watch {
        eprintln!("{} swaps, {} instant edits, {} rejected saves", swaps, fast, rejected);
    }
    if knob_moves > 0 || notes_played > 0 {
        eprintln!("{} knob moves, {} notes played", knob_moves, notes_played);
    }
    if dropped_in_place > 0 {
        eprintln!("{} retired engines were dropped on the audio thread (main thread fell behind)", dropped_in_place);
    }
    Ok(())
}

/// Apply a transform key to the selected track's `play` line in `file`.
/// Returns the file as it was, for undo, and what to tell the performer.
fn edit_play(
    file: &std::path::Path,
    selected: Vec<(String, String)>,
    op: crate::tui::edit::Op,
) -> Result<(String, String), String> {
    if selected.is_empty() {
        return Err("choose a track first with ↑ ↓".into());
    }
    let before = std::fs::read_to_string(file).map_err(|e| format!("cannot read {}: {}", file.display(), e))?;
    // Every selected track's line, in one save.
    let mut after = before.clone();
    let mut said = Vec::new();
    for (track, play) in selected {
        let mut clause =
            crate::tui::edit::Clause::parse(&play).ok_or_else(|| format!("cannot read `play {}`", play))?;
        clause.apply(op);
        after = crate::tui::edit::rewrite(&after, &track, &clause)?;
        said.push(format!("{}: play {}", track, clause.text()));
    }
    std::fs::write(file, &after).map_err(|e| format!("cannot write {}: {}", file.display(), e))?;
    Ok((before, said.join("  ·  ")))
}

/// The keys the song binds, for the plain screen.
fn print_keyboard(planner: &LivePlanner) {
    let (bindings, bars) = planner.keyboard();
    if bindings.is_empty() {
        return;
    }
    let said: Vec<String> = bindings
        .iter()
        .map(|b| {
            let what = match &b.action {
                KeyAction::Perform(s) => format!("scene {}", s),
                KeyAction::Voice(tatum_core::dsl::ast::VoiceMove::Next) => "next voice".into(),
                KeyAction::Voice(tatum_core::dsl::ast::VoiceMove::Prev) => "voice before".into(),
                KeyAction::Voice(tatum_core::dsl::ast::VoiceMove::To(p)) => format!("voice {}", p),
                KeyAction::Next => "next".into(),
                KeyAction::Prev => "prev".into(),
                KeyAction::Step(n) => format!("step {}", n),
            };
            format!("{} {}", b.key, what)
        })
        .collect();
    eprintln!(
        "  keys: {}  (a scene comes in on the next {})",
        said.join(", "),
        if bars == 1 { "bar".to_string() } else { format!("{} bars", bars) }
    );
}

/// Where the session's messages go: lines on stderr, or the screen.
enum Ui {
    Plain(Status),
    Screen(Box<crate::tui::Screen>),
}

impl Ui {
    fn is_screen(&self) -> bool {
        matches!(self, Ui::Screen(_))
    }

    fn say(&mut self, t: f32, line: &str, tone: crate::tui::Tone) {
        match self {
            Ui::Plain(status) => {
                status.close();
                eprintln!("[{:7.2}s] {}", t, line);
            }
            Ui::Screen(screen) => screen.say(line, tone),
        }
    }

    fn readings(&mut self, readings: Vec<String>) {
        match self {
            Ui::Plain(status) => status.show(&readings.join("   ")),
            Ui::Screen(screen) => {
                for r in readings {
                    screen.knob(r);
                }
            }
        }
    }

    /// A step waiting for its phrase line. The screen shows it in the steps.
    fn waiting(&mut self, line: &str) {
        if let Ui::Plain(status) = self {
            status.show(line);
        }
    }

    fn saved(&mut self, t: f32, kind: &str) {
        match self {
            Ui::Plain(status) => {
                status.close();
                eprintln!("[{:7.2}s] saved: {}", t, kind);
            }
            Ui::Screen(screen) => {
                screen.clear_error();
                screen.say(format!("saved: {}", kind), crate::tui::Tone::Good);
            }
        }
    }

    fn rejected(&mut self, lines: Vec<String>) {
        match self {
            Ui::Plain(status) => {
                status.close();
                for l in lines {
                    eprintln!("{}", l);
                }
                eprintln!("  (still playing the last good version)");
            }
            Ui::Screen(screen) => screen.error(lines),
        }
    }

    fn song(&mut self, title: &str, source: &Source) {
        if let Ui::Screen(screen) = self {
            if let Some(info) = crate::tui::SongInfo::from_source(title, &source.text) {
                screen.set_song(info);
            }
        }
    }
}

impl Drop for Ui {
    fn drop(&mut self) {
        if let Ui::Plain(status) = self {
            status.close();
        }
    }
}

fn verb(watch: bool) -> &'static str {
    if watch {
        "watching"
    } else {
        "playing"
    }
}

/// The line that shows what the knob being turned reads. On a terminal it is
/// rewritten in place, so turning a knob does not scroll everything else off
/// the screen; any other message closes it first and it redraws on the next
/// move. Off a terminal every reading is a line of its own.
struct Status {
    open: bool,
    terminal: bool,
}

impl Status {
    fn new() -> Self {
        Self { open: false, terminal: std::io::stderr().is_terminal() }
    }

    fn show(&mut self, text: &str) {
        if self.terminal {
            eprint!("\r\x1b[K  {}", text);
            self.open = true;
        } else {
            eprintln!("  {}", text);
        }
    }

    fn close(&mut self) {
        if self.open {
            eprintln!();
            self.open = false;
        }
    }
}
