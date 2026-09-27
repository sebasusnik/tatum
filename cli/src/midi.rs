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
    let [status, a, b, ..] = *message else { return None };
    let channel = (status & 0x0F) + 1;
    let (a, b) = (a & 0x7F, b & 0x7F);
    match status & 0xF0 {
        0xB0 => Some(Event::Cc(a, b)),
        0x90 => Some(Event::Note { channel, note: a, velocity: b }),
        0x80 => Some(Event::Note { channel, note: a, velocity: 0 }),
        0xE0 => Some(Event::Bend((b as u16) << 7 | a as u16)),
        _ => None,
    }
}

/// Whether a port is a DAW-control port, where notes are buttons.
fn speaks_mackie(name: &str) -> bool {
    let upper = name.to_uppercase();
    ["MCU", "HUI", "DAW"].iter().any(|tag| upper.split(|c: char| !c.is_ascii_alphanumeric()).any(|w| w == *tag))
}
