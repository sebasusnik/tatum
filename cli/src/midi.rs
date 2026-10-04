//! MIDI input for `play` and `watch`: knobs and faders, keys, pads and the
//! pitch strip. Everything else a controller sends is ignored.
//!
//! Every input port is opened unless `--midi` names one, because a controller
//! usually shows up as more than one port and there is no telling from the
//! name which one carries the knobs. The exception is a port that speaks
//! Mackie Control (MCU, HUI or DAW in its name): there a note is a transport
//! button and pitch bend is a fader, so reading it would play the solo every
//! time someone pressed Play. `--midi` can still name one.

use std::sync::mpsc::Sender;

use midir::{Ignore, MidiInput, MidiInputConnection};

/// What arrives from a controller, reduced to what the session uses.
#[derive(Debug, Clone, Copy)]
pub enum Event {
    /// A knob or a fader: controller number and value.
    Cc(u8, u8),
    /// A key or a pad. Velocity 0 is a release, which is also how most
    /// controllers send one. `channel` counts from 1, so pads are 10.
    Note { channel: u8, note: u8, velocity: u8 },
    /// The pitch strip, 14 bits with 8192 at rest.
    Bend(u16),
    /// Aftertouch: how hard a held key or pad is pressed, 0..127. `note`
    /// is the key for polyphonic aftertouch, `None` for the channel's.
    Pressure { channel: u8, note: Option<u8>, value: u8 },
    /// Anything else on a channel (a program change, a mode message): only
    /// shown, so the monitor can say a button sent something.
    Other { bytes: [u8; 3], len: u8 },
}

/// The channel pads send on, by General MIDI convention.
pub const DRUM_CHANNEL: u8 = 10;

/// Open connections. Dropping this closes them.
pub struct Inputs {
    _connections: Vec<MidiInputConnection<()>>,
    pub names: Vec<String>,
}

pub fn list() {
    match MidiInput::new("tatum") {
        Ok(input) => {
            let ports = input.ports();
            if ports.is_empty() {
                println!("no MIDI inputs");
            }
            for port in &ports {
                println!("{}", input.port_name(port).unwrap_or_else(|_| "?".into()));
            }
        }
        Err(e) => eprintln!("error: cannot reach MIDI: {}", e),
    }
}

/// Connect to every input whose name contains `wanted`, or to every input
/// when it is `None`, and send what arrives down `tx`. No inputs at all is
/// not an error: the song plays and the controls wait.
pub fn open(wanted: Option<&str>, tx: &Sender<Event>) -> Result<Inputs, String> {
    let probe = MidiInput::new("tatum").map_err(|e| format!("cannot reach MIDI: {}", e))?;
    let ports = probe.ports();
    let needle = wanted.map(|w| w.to_lowercase());
    let mut connections = Vec::new();
    let mut names = Vec::new();
    let mut all = Vec::new();
    for port in &ports {
        let name = probe.port_name(port).unwrap_or_else(|_| "?".into());
        all.push(name.clone());
        match &needle {
            Some(n) if !name.to_lowercase().contains(n) => continue,
            None if speaks_mackie(&name) => continue,
            _ => {}
        }
        // One client per connection: `connect` consumes it.
        let mut input = MidiInput::new("tatum").map_err(|e| format!("cannot reach MIDI: {}", e))?;
        input.ignore(Ignore::All);
        let tx = tx.clone();
        let conn = input
            .connect(
                port,
                "tatum-in",
                move |_stamp, message, _| {
                    if let Some(event) = decode(message) {
                        let _ = tx.send(event);
                    }
                },
                (),
            )
            .map_err(|e| format!("cannot open MIDI input '{}': {}", name, e))?;
        connections.push(conn);
        names.push(name);
    }
    if let (Some(w), true) = (wanted, names.is_empty()) {
        return Err(if all.is_empty() {
            format!("no MIDI input matching '{}': there are no MIDI inputs", w)
        } else {
            format!("no MIDI input matching '{}'. Available: {}", w, all.join(", "))
        });
    }
    Ok(Inputs { _connections: connections, names })
}

/// One MIDI message as an [`Event`], on any channel.
fn decode(message: &[u8]) -> Option<Event> {
    if let [status, value] = *message {
        return match status & 0xF0 {
            0xD0 => Some(Event::Pressure { channel: (status & 0x0F) + 1, note: None, value: value & 0x7F }),
            0x80..=0xE0 => Some(Event::Other { bytes: [status, value, 0], len: 2 }),
            _ => None,
        };
    }
    let [status, a, b, ..] = *message else { return None };
    let channel = (status & 0x0F) + 1;
    let (a, b) = (a & 0x7F, b & 0x7F);
    match status & 0xF0 {
        0xB0 => Some(Event::Cc(a, b)),
        0x90 => Some(Event::Note { channel, note: a, velocity: b }),
        0x80 => Some(Event::Note { channel, note: a, velocity: 0 }),
        0xE0 => Some(Event::Bend((b as u16) << 7 | a as u16)),
        0xA0 => Some(Event::Pressure { channel, note: Some(a), value: b }),
        0xC0 | 0xD0 => Some(Event::Other { bytes: [status, a, 0], len: 2 }),
        _ => None,
    }
}

/// Whether a port is a DAW-control port, where notes are buttons.
fn speaks_mackie(name: &str) -> bool {
    let upper = name.to_uppercase();
    ["MCU", "HUI", "DAW"].iter().any(|tag| upper.split(|c: char| !c.is_ascii_alphanumeric()).any(|w| w == *tag))
}

/// `tatum midi monitor [<song or set>] [--midi <name>]`: every message the
/// controller sends, one line each, with what the song maps it to. The way to
/// find out which numbers a controller really sends before writing them into
/// a `midi` block, and to check a block against the hardware.
pub fn cmd(args: &[String]) {
    match args.first().map(String::as_str) {
        Some("monitor") => {}
        Some("list") => {
            list();
            return;
        }
        _ => {
            eprintln!("usage: tatum midi monitor [<song.synth> | <set dir> [--step N]] [--midi <name>]");
            eprintln!("       tatum midi list");
            std::process::exit(1);
        }
    }
    let mut wanted: Option<String> = None;
    let mut song_path: Option<String> = None;
    let mut step: usize = 1;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--midi" => {
                i += 1;
                wanted = args.get(i).cloned();
            }
            "--step" => {
                i += 1;
                step = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(1);
            }
            other => song_path = Some(other.to_string()),
        }
        i += 1;
    }
    let song = match song_path.as_deref().map(|p| load_song(p, step)).transpose() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {}", e);
            std::process::exit(1);
        }
    };
    if let Err(e) = monitor(wanted.as_deref(), song.as_ref().map(|(name, s, text)| (name.as_str(), s, text.as_str()))) {
        eprintln!("error: {}", e);
        std::process::exit(1);
    }
}

/// A song, or step `step` of a set (from 1), parsed, with its name.
fn load_song(path: &str, step: usize) -> Result<(String, tatum_core::dsl::ast::Song, String), String> {
    let p = std::path::Path::new(path);
    let (name, file) = if p.is_dir() {
        let steps = crate::set::load(p, 32)?;
        let s = steps.get(step.clamp(1, steps.len().max(1)) - 1).ok_or("the set has no steps")?;
        (format!("{} step {}", path, s.name()), s.path.clone())
    } else {
        (path.to_string(), p.to_path_buf())
    };
    let source = crate::include::Source::load(&file)?;
    let song = tatum_core::dsl::parse(&source.text).map_err(|errs| {
        let lines: Vec<String> = errs.iter().map(|e| format!("line {}: {}", e.line, e.message)).collect();
        format!("{}: {}", file.display(), lines.join("; "))
    })?;
    Ok((name, song, source.text))
}

/// `C2` for 36: the names the text writes notes with.
pub fn note_name(n: u8) -> String {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    format!("{}{}", NAMES[(n % 12) as usize], (n / 12) as i32 - 1)
}

/// What `song` does with one decoded message, as the `midi` block and the
/// zones say: `→ ...`, or nothing when it maps nothing.
/// `planner`, when there is one, knows the scene and the zones' register.
pub fn meaning(
    song: &tatum_core::dsl::ast::Song,
    planner: Option<&tatum_core::live::LivePlanner>,
    event: &Event,
) -> Option<String> {
    use tatum_core::dsl::ast::{MidiSource, ZoneKind};
    let line = |src: MidiSource| -> Vec<String> {
        song.midi.iter().filter(|m| m.source == src).map(|m| m.target.replace('.', " ")).collect()
    };
    let out = match *event {
        Event::Cc(cc, _) => {
            let t = line(MidiSource::Cc(cc));
            // Scenes that put this controller on something of their own.
            let scenes: Vec<&str> = song
                .perform
                .scenes
                .iter()
                .filter(|sc| sc.knobs.iter().any(|k| k.map.source == MidiSource::Cc(cc)))
                .map(|sc| sc.name.as_str())
                .collect();
            match (t.is_empty(), scenes.is_empty()) {
                (true, true) => return Some(String::from("not mapped")),
                (true, false) => format!("in scenes {}", scenes.join(", ")),
                (false, true) => t.join(" + "),
                (false, false) => format!("{} (scenes {} move their own)", t.join(" + "), scenes.join(", ")),
            }
        }
        Event::Note { channel: DRUM_CHANNEL, note, .. } => {
            let t = line(MidiSource::Pad(note));
            if t.is_empty() {
                return Some(format!("pad {} not mapped", note));
            }
            format!("pad: {}", t.join(" + "))
        }
        Event::Note { note, .. } => {
            let t = line(MidiSource::Key(note));
            if !t.is_empty() {
                format!("trigger key: {}", t.join(" + "))
            } else {
                match song.perform.zones.iter().find(|z| (z.low..=z.high).contains(&note)) {
                    Some(z) if z.kind == ZoneKind::Triggers => {
                        format!("trigger zone {}..{}: key {} not mapped", z.low, z.high, note)
                    }
                    Some(z) => match planner.map(|p| p.zone_note(note)) {
                        Some(Some((_, track, out, roll))) => format!(
                            "{} zone {}..{}: plays {} on {}{}",
                            z.kind.word(),
                            z.low,
                            z.high,
                            note_name(out),
                            track,
                            if roll { " (roll)" } else { "" }
                        ),
                        Some(None) => format!("{} zone {}..{}: plays nothing here", z.kind.word(), z.low, z.high),
                        None => {
                            let lock = song.perform.lock.unwrap_or(tatum_core::perform::scale::Lock::Snap);
                            let scale = song
                                .globals
                                .scale
                                .as_ref()
                                .and_then(|d| tatum_core::perform::scale::Scale::named(&d.root, &d.kind));
                            let plays = match scale.map(|s| s.lock(lock, note)) {
                                None => note_name(note),
                                Some(Some(n)) => note_name(n),
                                Some(None) => String::from("nothing (a black key under lock white)"),
                            };
                            let track = z
                                .play
                                .as_ref()
                                .map(|p| format!(" on {}{}", p.track, if p.roll { " (roll)" } else { "" }))
                                .unwrap_or_default();
                            format!("{} zone {}..{}: plays {}{}", z.kind.word(), z.low, z.high, plays, track)
                        }
                    },
                    None => {
                        let keys = line(MidiSource::Keys);
                        if keys.is_empty() {
                            return Some(String::from("outside every zone"));
                        }
                        format!("keys > {}", keys.join(", "))
                    }
                }
            }
        }
        Event::Bend(_) => String::from("pitch strip: bends the zones as the scene says"),
        Event::Pressure { .. } => String::from("aftertouch: read by the pads in stage 3"),
        Event::Other { .. } => String::from("not used"),
    };
    Some(out)
}

pub fn describe(event: &Event) -> String {
    match *event {
        Event::Cc(cc, v) => format!("cc {:>3}  = {:>3}", cc, v),
        Event::Note { channel, note, velocity } => {
            let what = if channel == DRUM_CHANNEL { "pad " } else { "note" };
            if velocity > 0 {
                format!("{} {:>3} {:<4} on  vel {:>3}", what, note, note_name(note), velocity)
            } else {
                format!("{} {:>3} {:<4} off", what, note, note_name(note))
            }
        }
        Event::Bend(v) => format!("bend    {:>5}  ({:+})", v, v as i32 - 8192),
        Event::Pressure { note: Some(n), value, channel } => {
            let what = if channel == DRUM_CHANNEL { "pad" } else { "key" };
            format!("aftertouch {} {:>3} = {:>3}", what, n, value)
        }
        Event::Other { bytes, len } => {
            let kind = match bytes[0] & 0xF0 {
                0xC0 => format!("program change {:>3}", bytes[1]),
                _ => String::from("other"),
            };
            let hex: Vec<String> = bytes[..len as usize].iter().map(|b| format!("{:02X}", b)).collect();
            format!("{} [{}]", kind, hex.join(" "))
        }
        Event::Pressure { note: None, value, channel } => {
            let what = if channel == DRUM_CHANNEL { "pads" } else { "keys" };
            format!("aftertouch {} (channel) = {:>3}", what, value)
        }
    }
}

fn monitor(wanted: Option<&str>, song: Option<(&str, &tatum_core::dsl::ast::Song, &str)>) -> Result<(), String> {
    // A planner over the song, never played, so a key reads as the session
    // would play it: in the first scene, in the zone's register.
    let planner = song.and_then(|(_, _, text)| {
        let mut p = tatum_core::live::LivePlanner::new();
        p.set_output_gain(1.0);
        p.plan(text, 0).ok().map(|_| p)
    });
    let song = song.map(|(name, s, _)| (name, s));
    let probe = MidiInput::new("tatum").map_err(|e| format!("cannot reach MIDI: {}", e))?;
    let needle = wanted.map(|w| w.to_lowercase());
    let (tx, rx) = std::sync::mpsc::channel::<(String, Vec<u8>)>();
    let mut connections = Vec::new();
    let mut names = Vec::new();
    for port in &probe.ports() {
        let name = probe.port_name(port).unwrap_or_else(|_| "?".into());
        if needle.as_ref().is_some_and(|n| !name.to_lowercase().contains(n)) {
            continue;
        }
        let mut input = MidiInput::new("tatum").map_err(|e| format!("cannot reach MIDI: {}", e))?;
        // Clock and active sensing would bury everything else.
        input.ignore(Ignore::TimeAndActiveSense);
        let tx = tx.clone();
        let label = name.clone();
        let conn = input
            .connect(
                port,
                "tatum-monitor",
                move |_stamp, message, _| {
                    let _ = tx.send((label.clone(), message.to_vec()));
                },
                (),
            )
            .map_err(|e| format!("cannot open MIDI input '{}': {}", name, e))?;
        connections.push(conn);
        names.push(name);
    }
    if names.is_empty() {
        return Err(String::from("no MIDI input to watch (tatum midi list shows them)"));
    }
    eprintln!("watching {}", names.join(", "));
    if let Some((name, _)) = song {
        eprintln!("mapping from {}", name);
    }
    eprintln!("every message, one per line; Ctrl-C stops\n");
    let started = std::time::Instant::now();
    for (port, bytes) in rx {
        let t = started.elapsed().as_secs_f32();
        let hex: Vec<String> = bytes.iter().take(8).map(|b| format!("{:02X}", b)).collect();
        let more = if bytes.len() > 8 { format!(" … ({} bytes)", bytes.len()) } else { String::new() };
        let channel = bytes
            .first()
            .filter(|&&b| b < 0xF0)
            .map(|b| format!("ch {:>2}", (b & 0x0F) + 1))
            .unwrap_or_else(|| String::from("     "));
        match decode(&bytes) {
            Some(ev) => {
                let what = song
                    .and_then(|(_, s)| meaning(s, planner.as_ref(), &ev))
                    .map(|m| format!("  → {}", m))
                    .unwrap_or_default();
                println!(
                    "{:>8.3}s  {:<28} {}  {:<34} [{}{}]{}",
                    t,
                    port,
                    channel,
                    describe(&ev),
                    hex.join(" "),
                    more,
                    what
                );
            }
            None => {
                let kind = match bytes.first().map(|b| b & 0xF0) {
                    Some(0xC0) => "program change",
                    Some(0xF0) => "system / sysex",
                    _ => "other",
                };
                println!("{:>8.3}s  {:<28} {}  {:<34} [{}{}]", t, port, channel, kind, hex.join(" "), more);
            }
        }
    }
    drop(connections);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_message_the_session_reads_decodes() {
        assert!(matches!(decode(&[0xB0, 74, 64]), Some(Event::Cc(74, 64))));
        assert!(matches!(decode(&[0x99, 36, 100]), Some(Event::Note { channel: 10, note: 36, velocity: 100 })));
        assert!(matches!(decode(&[0x80, 60, 40]), Some(Event::Note { channel: 1, note: 60, velocity: 0 })));
        assert!(matches!(decode(&[0xE0, 0, 64]), Some(Event::Bend(8192))));
        // Aftertouch: the channel's is two bytes long, a key's three.
        assert!(matches!(decode(&[0xD9, 90]), Some(Event::Pressure { channel: 10, note: None, value: 90 })));
        assert!(matches!(decode(&[0xA0, 61, 30]), Some(Event::Pressure { channel: 1, note: Some(61), value: 30 })));
        assert!(
            matches!(decode(&[0xC0, 5]), Some(Event::Other { bytes: [0xC0, 5, 0], len: 2 })),
            "a program change is only shown by the monitor"
        );
    }

    #[test]
    fn the_monitor_says_what_the_song_does_with_a_message() {
        let src =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../examples/keylab_psy_e_frigio.synth"))
                .unwrap();
        let song = tatum_core::dsl::parse(&src).unwrap();
        let say = |e: Event| meaning(&song, None, &e).unwrap();
        assert_eq!(say(Event::Note { channel: 1, note: 36, velocity: 100 }), "trigger key: play ruido");
        assert_eq!(say(Event::Note { channel: 1, note: 40, velocity: 100 }), "trigger zone 36..47: key 40 not mapped");
        assert_eq!(
            say(Event::Note { channel: 1, note: 54, velocity: 100 }),
            "bass zone 48..59: plays F3 on bass (roll)"
        );
        assert_eq!(say(Event::Note { channel: 1, note: 64, velocity: 100 }), "lead zone 60..84: plays E4 on pluck");
        assert_eq!(say(Event::Note { channel: 1, note: 100, velocity: 100 }), "outside every zone");
        assert_eq!(say(Event::Cc(7, 3)), "not mapped");
        assert_eq!(note_name(36), "C2");
        assert_eq!(note_name(61), "C#4");
    }
}
