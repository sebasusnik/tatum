//! Checking `midi { }` and `auto` lines against the song, so a knob or a
//! sweep on something that does not exist is an error at compile time.

use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;

use crate::dsl::ast::*;
use crate::dsl::error::CompileError;
use crate::params::{self, ModuleKind};

use super::module::unknown_param_message;
use super::notes::drum_note;

/// Master-chain parameters that `auto master <param>` can move.
pub const MASTER_AUTO_PARAMS: &[&str] =
    &["tilt", "eq_low", "eq_mid", "eq_high", "drive", "gain", "cutoff", "comp_threshold"];

/// Node kinds that carry a given master automation parameter.
pub fn master_auto_node_kinds(param: &str) -> &'static [&'static str] {
    match param {
        "tilt" => &["tilt"],
        "eq_low" | "eq_mid" | "eq_high" => &["eq"],
        "drive" => &["saturate", "drive"],
        "gain" => &["gain"],
        "cutoff" => &["lowpass", "highpass", "bandpass", "ladder"],
        "comp_threshold" => &["compressor"],
        _ => &[],
    }
}

/// What a knob can be put on, for error messages.
const MIDI_TARGETS: &str =
    "<module> <param>, <track> level, <track> pan, <track> <node> wet, reverb_mix, delay_mix or reverb_freeze";

/// Validate `midi { }` against the song. A knob, a keyboard or a pad on
/// something that does not exist is an error here rather than a control that
/// silently does nothing on stage.
pub(super) fn validate_midi(song: &Song) -> Vec<CompileError> {
    let mut errors = Vec::new();
    // The module a track plays, as written: its name and its kind.
    let module_of = |track: &str| -> Option<(&str, Option<&str>)> {
        let t = song.tracks.iter().find(|t| t.name == track)?;
        let kind = song.module_defs.iter().find(|m| m.name == t.using_instrument).map(|m| m.module_type.as_str());
        Some((t.using_instrument.as_str(), kind))
    };
    for map in &song.midi {
        let words: Vec<&str> = map.target.split('.').collect();
        let source = match map.source {
            MidiSource::Cc(cc) => format!("cc {}", cc),
            MidiSource::Keys => String::from("keys"),
            MidiSource::Pad(n) => format!("pad {}", n),
        };
        let err = |msg: String| CompileError::at(map.line, format!("midi: {} > {}: {}", source, words.join(" "), msg));
        match map.source {
            MidiSource::Keys => match words.as_slice() {
                [track] => match module_of(track) {
                    None => errors.push(err(format!("no track named '{}'", track))),
                    Some((_, Some("beats"))) => errors
                        .push(err(format!("'{}' plays drums, which the pads play: `pad 36 > {} kick`", track, track))),
                    Some(_) => {}
                },
                _ => errors.push(err(String::from("the keys play one track: `keys > solo`"))),
            },
            MidiSource::Pad(_) => match words.as_slice() {
                [track, drum] => match module_of(track) {
                    None => errors.push(err(format!("no track named '{}'", track))),
                    Some((_, Some("beats"))) if drum_note(drum).is_some() => {}
                    Some((_, Some("beats"))) => errors.push(err(format!(
                        "'{}' is not a drum (kick, snare, clap, hat, openhat, tom, tom2, tom3, crash)",
                        drum
                    ))),
                    Some((module, _)) => errors.push(err(format!(
                        "track '{}' plays '{}', which is not a drum kit; a pad hits a `beats` module",
                        track, module
                    ))),
                },
                _ => errors.push(err(String::from("a pad hits one drum of a track: `pad 36 > kick kick`"))),
            },
            MidiSource::Cc(_) => match words.as_slice() {
                ["reverb_mix"] | ["delay_mix"] | ["reverb_freeze"] => {}
                ["master", _] => errors.push(err(String::from(
                    "the master chain cannot be put on a knob yet; `auto master` can sweep it",
                ))),
                [name, "level"] | [name, "pan"] => {
                    let is_track = song.tracks.iter().any(|t| &t.name == name);
                    let is_module = song.module_defs.iter().any(|m| &m.name == name)
                        || song.instruments.iter().any(|i| &i.name == name);
                    if !is_track && !is_module {
                        errors.push(err(format!("no track or module named '{}'", name)));
                    }
                }
                [track, node, "wet"] => match song.tracks.iter().find(|t| &t.name == track) {
                    None => errors.push(err(format!("no track named '{}'", track))),
                    Some(t) if t.routing.iter().any(|n| n.label.as_deref() == Some(node)) => {}
                    Some(t) => {
                        let named: Vec<&str> = t.routing.iter().filter_map(|n| n.label.as_deref()).collect();
                        errors.push(err(format!(
                            "track '{}' has no node named '{}'{}",
                            track,
                            node,
                            if named.is_empty() {
                                String::from("; name one with `> effect(...) as <name>`")
                            } else {
                                format!(" (it has {})", named.join(", "))
                            }
                        )));
                    }
                },
                [module, param] => match song.module_defs.iter().find(|m| &m.name == module) {
                    Some(m) => {
                        if let Some(kind) = ModuleKind::from_str(&m.module_type) {
                            if params::lookup(kind, param).is_none() {
                                errors.push(err(unknown_param_message(kind, module, param)));
                            }
                        }
                    }
                    None if song.instruments.iter().any(|i| &i.name == module) => {
                        errors.push(err(String::from("graph instruments have no named params (only level and pan)")))
                    }
                    None => errors.push(err(format!("no module named '{}'", module))),
                },
                _ => errors.push(err(format!("a knob moves {}", MIDI_TARGETS))),
            },
        }
        if let (MidiSource::Cc(_), Some(range)) = (map.source, &map.range) {
            if let Err(msg) = crate::midi::knob_span(song, &map.target, range) {
                errors.push(err(msg));
            }
        }
    }
    errors
}

/// Validate `auto <target> ...` lanes against tracks, instruments and the param registry.
pub(super) fn validate_automations(song: &Song) -> Vec<CompileError> {
    let mut errors = Vec::new();
    for scene in &song.scenes {
        for auto in &scene.automations {
            let target = auto.target.as_str();
            if target == "reverb_mix" || target == "delay_mix" || target == "reverb_freeze" {
                continue;
            }
            let (name, param) = match target.find('.') {
                Some(i) => (&target[..i], &target[i + 1..]),
                None => {
                    errors.push(CompileError::new(format!(
                        "scene '{}': automation target '{}' must be reverb_mix, delay_mix, <track> level or <module> <param>",
                        scene.name, target
                    )));
                    continue;
                }
            };
            if name == "master" {
                let needed = master_auto_node_kinds(param);
                if needed.is_empty() {
                    errors.push(CompileError::new(format!(
                        "scene '{}': automation — master has no parameter '{}' (expected {})",
                        scene.name,
                        param,
                        MASTER_AUTO_PARAMS.join(", ")
                    )));
                } else if !song
                    .master
                    .as_ref()
                    .is_some_and(|m| m.chain.iter().any(|n| needed.contains(&n.kind.as_str())))
                {
                    errors.push(CompileError::new(format!(
                        "scene '{}': automation — `auto master {}` needs a {} node in the master chain",
                        scene.name,
                        param,
                        needed.join(" or ")
                    )));
                }
                continue;
            }
            // `<track>.<node> wet`: sweeping an effect in or out over a scene.
            // Checked here so a misspelled node name is an error at compile
            // time rather than an automation that silently does nothing.
            if let Some(node) = param.strip_suffix(".wet") {
                // A scene track inherits the top-level chain when it does not
                // write one, so an empty scene routing is not "no chain".
                let routing = scene
                    .tracks
                    .iter()
                    .find(|t| t.name == name && !t.routing.is_empty())
                    .or_else(|| song.tracks.iter().find(|t| t.name == name))
                    .map(|t| &t.routing);
                match routing {
                    Some(r) if r.iter().any(|n| n.label.as_deref() == Some(node)) => continue,
                    Some(_) => {
                        let named: Vec<&str> =
                            routing.into_iter().flatten().filter_map(|n| n.label.as_deref()).collect();
                        errors.push(CompileError::new(format!(
                            "scene '{}': automation target '{}' — track '{}' has no node named '{}'{}",
                            scene.name,
                            target,
                            name,
                            node,
                            if named.is_empty() {
                                String::from("; name one with `> effect(...) as <name>`")
                            } else {
                                format!(" (it has {})", named.join(", "))
                            }
                        )));
                        continue;
                    }
                    None => {
                        errors.push(CompileError::new(format!(
                            "scene '{}': automation target '{}' — no track named '{}'",
                            scene.name, target, name
                        )));
                        continue;
                    }
                }
            }
            let is_track = scene.tracks.iter().chain(song.tracks.iter()).any(|t| t.name == name);
            let module = song.module_defs.iter().find(|m| m.name == name);
            let is_graph = song.instruments.iter().any(|i| i.name == name);
            if param == "level" && (is_track || module.is_some() || is_graph) {
                continue;
            }
            match module {
                Some(m) => {
                    if let Some(kind) = ModuleKind::from_str(&m.module_type) {
                        if params::lookup(kind, param).is_none() {
                            errors.push(CompileError::new(format!(
                                "scene '{}': automation — {}",
                                scene.name,
                                unknown_param_message(kind, name, param)
                            )));
                        }
                    }
                }
                None if is_graph => errors.push(CompileError::new(format!(
                    "scene '{}': automation target '{}' — graph instruments have no named params (only level)",
                    scene.name, target
                ))),
                None => errors.push(CompileError::new(format!(
                    "scene '{}': automation target '{}' — no module or track named '{}'",
                    scene.name, target, name
                ))),
            }
        }
    }
    errors
}
