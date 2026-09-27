//! MIDI controls: what a `midi { }` block maps, resolved against one engine.
//!
//! Three kinds. A knob (`cc 74 > acid cutoff`) turns a 7-bit value into the
//! same [`FastOp`]s a text edit produces, and says what it now reads in the
//! target's own units; the registry is what makes that short, since every
//! parameter already says whether it spans 0..1, left to right, a gain or a
//! list of choices, and what real quantity it stands for. The keys
//! (`keys > solo`) play a track's instrument. A pad (`pad 36 > kick kick`)
//! hits one drum of a track.
//!
//! No I/O here: whatever reads the controller hands over numbers.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::dsl::ast::{MidiSource, Param, RangeEnd, Song};
use crate::live::FastOp;
use crate::params::{self, ModuleKind, ParamSpec, Range};

/// One thing a knob moves, resolved to engine indices.
#[derive(Debug, Clone, Copy)]
pub enum Move {
    Module { instrument: usize, spec: &'static ParamSpec },
    TrackLevel { track: usize },
    TrackPan { track: usize },
    NodeWet { track: usize, node: usize },
    ReverbMix,
    DelayMix,
    ReverbFreeze,
}

/// The names one engine answers to, which is all resolution needs. The
/// engine gives every track that names a module its own copy of it, so
/// `instruments` can carry the same name more than once.
pub struct Names<'a> {
    pub instruments: &'a [String],
    pub tracks: &'a [String],
    /// Which instrument each track plays, `usize::MAX` for none.
    pub track_instruments: &'a [usize],
    /// The `as` names of each track's insert nodes, in chain order.
    pub track_nodes: &'a [Vec<Option<String>>],
}

/// A controller and everything it moves in one engine.
#[derive(Debug, Clone)]
pub struct Knob {
    pub cc: u8,
    /// The target as the file writes it, dotted: `acid.cutoff`.
    pub target: String,
    pub moves: Vec<Move>,
    /// Where the bottom and the top of the travel land, as the engine reads
    /// the target; from `cc 74 > acid cutoff 200hz..4khz`. `None`: all of it.
    pub span: Option<(f32, f32)>,
}

impl Knob {
    /// The operations that put this knob at `value`.
    pub fn ops(&self, value: u8) -> impl Iterator<Item = FastOp> + '_ {
        self.moves.iter().map(move |m| op(m, scaled(m, value, self.span)))
    }

    /// What the knob reads at `value`: `acid cutoff 1.2khz`, `pad level -6.0 dB`.
    pub fn reading(&self, value: u8) -> String {
        let label = self.target.replace('.', " ");
        match self.moves.first() {
            Some(m) => format!("{} {}", label, reading(m, scaled(m, value, self.span))),
            None => label,
        }
    }
}

/// A track the keyboard plays.
#[derive(Debug, Clone, Copy)]
pub struct Keys {
    pub track: usize,
    pub instrument: usize,
    /// One note at a time. The `bass` module is: it glides from note to note,
    /// and it releases on any note-off, so the keyboard has to decide which
    /// key it is actually playing.
    pub mono: bool,
}

/// One pad: the drum channel note it answers to, and what it hits.
#[derive(Debug, Clone, Copy)]
pub struct Pad {
    pub note: u8,
    pub track: usize,
    /// The note the `beats` module plays that drum on.
    pub drum: u8,
}

/// Everything a `midi` block maps, resolved against one engine.
#[derive(Debug, Clone, Default)]
pub struct Controls {
    pub knobs: Vec<Knob>,
    pub keys: Vec<Keys>,
    pub pads: Vec<Pad>,
}

/// Every `midi { }` mapping in `song`, resolved against one engine. A knob
/// that resolves to nothing is kept with no moves rather than dropped, so a
/// knob on a track that only exists in another scene does not vanish; keys
/// and pads that do not resolve are left out.
pub fn resolve_all(song: &Song, names: &Names) -> Controls {
    let mut controls = Controls::default();
    let track = |name: &str| names.tracks.iter().position(|t| t == name);
    for m in &song.midi {
        match m.source {
            MidiSource::Cc(cc) => {
                let span = m.range.as_ref().and_then(|r| knob_span(song, &m.target, r).ok());
                controls.knobs.push(Knob { cc, target: m.target.clone(), moves: resolve(song, names, &m.target), span })
            }
            MidiSource::Keys => {
                let Some(t) = track(&m.target) else { continue };
                let Some(&instrument) = names.track_instruments.get(t).filter(|&&i| i != usize::MAX) else { continue };
                let module = song.tracks.iter().find(|d| d.name == m.target).map(|d| d.using_instrument.as_str());
                let mono = song
                    .module_defs
                    .iter()
                    .find(|d| Some(d.name.as_str()) == module)
                    .is_some_and(|d| d.module_type == "bass");
                controls.keys.push(Keys { track: t, instrument, mono });
            }
            MidiSource::Pad(note) => {
                let Some((t, drum)) = m.target.split_once('.') else { continue };
                let (Some(t), Some(drum)) = (track(t), crate::dsl::compiler::drum_note(drum)) else { continue };
                controls.pads.push(Pad { note, track: t, drum });
            }
        }
    }
    controls
}

/// What `target` moves in the engine `names` describes. Resolves the way
/// `auto` does: `level` finds a track by name first and otherwise every track
/// that plays the module, and a module parameter reaches every copy.
pub fn resolve(song: &Song, names: &Names, target: &str) -> Vec<Move> {
    let words: Vec<&str> = target.split('.').collect();
    let mut out = Vec::new();
    let track = |name: &str| names.tracks.iter().position(|t| t == name);
    match words.as_slice() {
        ["reverb_mix"] => out.push(Move::ReverbMix),
        ["delay_mix"] => out.push(Move::DelayMix),
        ["reverb_freeze"] => out.push(Move::ReverbFreeze),
        [name, which @ ("level" | "pan")] => {
            let tracks: Vec<usize> = match track(name) {
                Some(t) => Vec::from([t]),
                None => (0..names.tracks.len())
                    .filter(|&t| {
                        names
                            .track_instruments
                            .get(t)
                            .and_then(|&i| names.instruments.get(i))
                            .is_some_and(|n| n == name)
                    })
                    .collect(),
            };
            for t in tracks {
                out.push(if *which == "level" { Move::TrackLevel { track: t } } else { Move::TrackPan { track: t } });
            }
        }
        [t, node, "wet"] => {
            if let Some(t) = track(t) {
                let found =
                    names.track_nodes.get(t).and_then(|nodes| nodes.iter().position(|l| l.as_deref() == Some(*node)));
                if let Some(n) = found {
                    out.push(Move::NodeWet { track: t, node: n });
                }
            }
        }
        [module, param] => {
            let spec = song
                .module_defs
                .iter()
                .find(|m| &m.name == module)
                .and_then(|def| ModuleKind::from_str(&def.module_type))
                .and_then(|kind| params::lookup(kind, param));
            if let Some(spec) = spec {
                for (i, n) in names.instruments.iter().enumerate() {
                    if n == module {
                        out.push(Move::Module { instrument: i, spec });
                    }
                }
            }
        }
        _ => {}
    }
    out
}

/// The value `target` has in the text, where the text gives it one. A knob
/// forgets what it was turned to when this changes: the text was edited on
/// purpose, and the edit should be what plays.
pub fn text_value(song: &Song, target: &str) -> Option<f32> {
    let words: Vec<&str> = target.split('.').collect();
    match words.as_slice() {
        [t, "level"] => song.tracks.iter().find(|d| &d.name == t)?.level,
        [t, "pan"] => song.tracks.iter().find(|d| &d.name == t)?.pan,
        [t, node, "wet"] => {
            let def = song.tracks.iter().find(|d| &d.name == t)?;
            let n = def.routing.iter().find(|n| n.label.as_deref() == Some(*node))?;
            n.params.iter().find_map(|p| match p {
                Param::Named(k, v) if k == "wet" => Some(*v),
                _ => None,
            })
        }
        [module, param] => song
            .module_defs
            .iter()
            .find(|m| &m.name == module)?
            .params
            .iter()
            .find(|p| &p.name == param)
            .map(|p| p.value),
        _ => None,
    }
}

/// Where a knob's range, as written, lands on `target` as the engine reads
/// it. The ends are in the units the text uses for that target: a module
/// parameter takes its own (`200hz`, `40ms`, `60%`) or a plain number as its
/// line in the module would; a level takes a gain (`0.8`), `-6db` or `%`; a
/// pan, a wet and a mix take a number or `%`. The error is for `tatum check`.
pub fn knob_span(song: &Song, target: &str, range: &(RangeEnd, RangeEnd)) -> Result<(f32, f32), String> {
    let words: Vec<&str> = target.split('.').collect();
    let spec = match words.as_slice() {
        [module, param] if !matches!(*param, "level" | "pan") => song
            .module_defs
            .iter()
            .find(|m| &m.name == module)
            .and_then(|def| ModuleKind::from_str(&def.module_type))
            .and_then(|kind| params::lookup(kind, param)),
        _ => None,
    };
    let end = |e: &RangeEnd| -> Result<f32, String> {
        let unit = e.unit.as_deref();
        match (words.as_slice(), spec) {
            (["reverb_freeze"], _) => Err(String::from("reverb_freeze is on or off; it takes no range")),
            (_, Some(spec)) => {
                if let Range::Choice(_) = spec.range {
                    return Err(format!(
                        "'{}' is a choice; a knob steps through all of them, with no range",
                        spec.name
                    ));
                }
                let v = match unit {
                    Some(u) => spec.value_from_quantity(e.value, u)?,
                    None => e.value,
                };
                if spec.range.contains(v) {
                    Ok(v)
                } else {
                    Err(format!("{} is outside what '{}' takes", e.value, spec.name))
                }
            }
            ([_, "level"], _) => {
                let gain = match unit {
                    None => e.value,
                    Some("db") => crate::math::pow(10.0, e.value / 20.0),
                    Some("%") => e.value / 100.0,
                    Some(u) => return Err(format!("a level takes a gain, dB or %, not '{}'", u)),
                };
                if (0.0..=crate::song_engine::MAX_TRACK_LEVEL).contains(&gain) {
                    Ok(gain)
                } else {
                    Err(format!("a level runs from 0 to {} (+12 dB)", crate::song_engine::MAX_TRACK_LEVEL))
                }
            }
            (w, _) => {
                let (lo, hi) = if matches!(w, [_, "pan"]) { (-1.0, 1.0) } else { (0.0, 1.0) };
                let v = match unit {
                    None => e.value,
                    Some("%") => e.value / 100.0,
                    Some(u) => return Err(format!("this takes a plain number or %, not '{}'", u)),
                };
                if (lo..=hi).contains(&v) {
                    Ok(v)
                } else {
                    Err(format!("{} is outside {}..{}", e.value, lo, hi))
                }
            }
        }
    };
    Ok((end(&range.0)?, end(&range.1)?))
}

/// Where a 7-bit controller value lands on a target, as the engine reads it.
fn scaled(m: &Move, value: u8, span: Option<(f32, f32)>) -> f32 {
    let x = value.min(127) as f32 / 127.0;
    // A range written with the knob: straight from one end to the other,
    // in the values the engine reads, so a cutoff still sweeps along its
    // own curve. Choices and the freeze switch have none.
    if let Some((lo, hi)) = span {
        let continuous = match m {
            Move::Module { spec, .. } => !matches!(spec.range, Range::Choice(_)),
            Move::ReverbFreeze => false,
            _ => true,
        };
        if continuous {
            return lo + (hi - lo) * x;
        }
    }
    match m {
        Move::Module { spec, .. } => match spec.range {
            Range::Unit => x,
            Range::Bipolar => x * 2.0 - 1.0,
            Range::Gain { max } => x * max,
            // Evenly spaced across the travel, so every choice gets the same
            // stretch of the knob.
            Range::Choice(names) => {
                let n = names.len();
                let idx = crate::math::floor(x * (n.max(1) - 1) as f32 + 0.5) as usize;
                params::choice_value(idx.min(n.saturating_sub(1)), n)
            }
        },
        // A fader at the top is `level 1.0`, not the +12 dB ceiling a level
        // can be written up to: the top of the travel is where it sounds as
        // loud as the file would have it at full.
        Move::TrackLevel { .. } => x,
        Move::TrackPan { .. } => x * 2.0 - 1.0,
        Move::NodeWet { .. } | Move::ReverbMix | Move::DelayMix => x,
        // Past half is on, which is also what a pad sending 0 and 127 means.
        Move::ReverbFreeze => {
            if value >= 64 {
                1.0
            } else {
                0.0
            }
        }
    }
}

fn op(m: &Move, v: f32) -> FastOp {
    match *m {
        Move::Module { instrument, spec } => FastOp::ModuleParam { instrument, id: spec.id, value: v },
        Move::TrackLevel { track } => FastOp::TrackLevel { track, level: v },
        Move::TrackPan { track } => FastOp::TrackPan { track, pan: v },
        Move::NodeWet { track, node } => FastOp::NodeWet { track, node, wet: v },
        Move::ReverbMix => FastOp::ReverbMix(v),
        Move::DelayMix => FastOp::DelayMix(v),
        Move::ReverbFreeze => FastOp::ReverbFreeze(v >= 0.5),
    }
}

/// A value in the target's own terms: `1.2khz`, `-6.0 dB`, `unison`, `40% L`.
fn reading(m: &Move, v: f32) -> String {
    match m {
        Move::Module { spec, .. } => match (spec.range, spec.curve.unit()) {
            (Range::Choice(_), _) => spec.choice_name(v).map(String::from).unwrap_or_default(),
            (Range::Gain { .. }, _) => decibels(v),
            (_, Some(unit)) => params::write_amount(spec.curve.to_real(v), unit),
            (Range::Bipolar, None) => format!("{:.2}", v),
            (Range::Unit, None) => percent(v),
        },
        Move::TrackLevel { .. } => decibels(v),
        Move::TrackPan { .. } => {
            let amount = crate::math::floor(crate::math::abs(v) * 100.0 + 0.5) as i32;
            if amount == 0 {
                String::from("center")
            } else {
                format!("{}% {}", amount, if v < 0.0 { "L" } else { "R" })
            }
        }
        Move::NodeWet { .. } | Move::ReverbMix | Move::DelayMix => percent(v),
        Move::ReverbFreeze => String::from(if v >= 0.5 { "frozen" } else { "off" }),
    }
}

fn percent(v: f32) -> String {
    format!("{}%", crate::math::floor(v * 100.0 + 0.5) as i32)
}

fn decibels(gain: f32) -> String {
    if gain <= 0.0001 {
        String::from("off")
    } else {
        format!("{:+.1} dB", 20.0 * crate::math::log10(gain))
    }
}
