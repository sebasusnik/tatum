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
    &["tilt", "eq_low", "eq_mid", "eq_high", "drive", "gain", "cutoff", "comp_threshold", "position"];

/// Node kinds that carry a given master automation parameter.
pub fn master_auto_node_kinds(param: &str) -> &'static [&'static str] {
    match param {
        "tilt" => &["tilt"],
        "eq_low" | "eq_mid" | "eq_high" => &["eq"],
        "drive" => &["saturate", "drive"],
        "gain" => &["gain"],
        "cutoff" => &["lowpass", "highpass", "bandpass", "ladder"],
        "comp_threshold" => &["compressor"],
        "position" => &["vowel"],
        _ => &[],
    }
}

/// What a knob can be put on, for error messages.
const MIDI_TARGETS: &str = "<module> <param>, <track> level, <track> pan, <track> delay_send, <track> reverb_send, \
<track> <node> wet, <track> <node> <param>, master <param>, master <node> <param>, reverb_mix, delay_mix, \
reverb_freeze or tempo";

/// Zones, scenes and the computer's keys against the song: a scene that
/// plays a track the rig does not have is caught by `tatum check`, not on
/// stage.
pub(super) fn validate_perform(song: &Song) -> Vec<CompileError> {
    use crate::perform::scale;
    let p = &song.perform;
    let mut errors = Vec::new();
    let kind_of = |track: &str| -> Option<Option<&str>> {
        let t = song.tracks.iter().find(|t| t.name == track)?;
        Some(song.module_defs.iter().find(|m| m.name == t.using_instrument).map(|m| m.module_type.as_str()))
    };
    let check_play = |errors: &mut Vec<CompileError>, line: usize, head: &str, play: &ZonePlay| {
        match kind_of(&play.track) {
            None => errors.push(CompileError::at(line, format!("{}: no track named '{}'", head, play.track))),
            Some(Some("beats")) => errors.push(CompileError::at(
                line,
                format!("{}: '{}' plays drums; a key zone plays notes", head, play.track),
            )),
            Some(_) => {}
        }
        if let Some(k) = &play.kick {
            match kind_of(k) {
                None => errors.push(CompileError::at(line, format!("{}: kick={}: no track named '{}'", head, k, k))),
                Some(Some("beats")) => {}
                Some(_) => errors.push(CompileError::at(
                    line,
                    format!("{}: kick={}: '{}' is not a drum kit; the roll strikes a `beats` track's kick", head, k, k),
                )),
            }
        }
    };
    for (i, z) in p.zones.iter().enumerate() {
        for o in &p.zones[..i] {
            if z.low <= o.high && o.low <= z.high {
                errors.push(CompileError::at(
                    z.line,
                    format!(
                        "zone {} {}..{} overlaps zone {} {}..{}",
                        z.kind.word(),
                        z.low,
                        z.high,
                        o.kind.word(),
                        o.low,
                        o.high
                    ),
                ));
            }
        }
        match (&z.play, z.kind) {
            (Some(_), ZoneKind::Triggers) => errors.push(CompileError::at(
                z.line,
                String::from("zone triggers: its keys are `key` lines, like `key 36 > hold riser`; it plays no track"),
            )),
            (Some(play), _) => check_play(&mut errors, z.line, &format!("zone {}", z.kind.word()), play),
            (None, _) => {}
        }
    }
    for sc in &p.scenes {
        let head = format!("perform {}", sc.name);
        if let Some(def) = &sc.scale {
            if scale::Scale::named(&def.root, &def.kind).is_none() {
                let known: Vec<&str> = scale::names().collect();
                errors.push(CompileError::at(
                    sc.line,
                    format!(
                        "{}: scale {} {}: a root like E or F#, and one of {}",
                        head,
                        def.root,
                        def.kind,
                        known.join(", ")
                    ),
                ));
            }
        }
        for (play, kind) in [(&sc.bass, ZoneKind::Bass), (&sc.lead, ZoneKind::Lead)] {
            if let Some(play) = play {
                check_play(&mut errors, sc.line, &format!("{}: {}", head, kind.word()), play);
                if !p.zones.iter().any(|z| z.kind == kind) {
                    errors.push(CompileError::at(
                        sc.line,
                        format!(
                            "{}: there is no {} zone; add one to `midi`, like `zone {} 48..59`",
                            head,
                            kind.word(),
                            kind.word()
                        ),
                    ));
                }
            }
        }
        let maps: Vec<MidiMapDef> = sc
            .sets
            .iter()
            .map(|set| MidiMapDef {
                source: MidiSource::Cc(0),
                target: set.target.clone(),
                range: Some(alloc::vec![set.value.clone(), set.value.clone()]),
                quantize: None,
                line: set.line,
            })
            .collect();
        errors.extend(validate_maps(song, &maps, |_, words| format!("{}: set {}", head, words)));
        let maps: Vec<MidiMapDef> = sc.knobs.iter().map(|k| k.map.clone()).collect();
        errors.extend(validate_maps(song, &maps, |source, words| {
            let what = if source == "cc 1" { String::from("wheel") } else { String::from(source) };
            format!("{}: {} > {}", head, what, words)
        }));
        for k in &sc.knobs {
            if let Hands::Playing(z) = k.hands {
                if !p.zones.iter().any(|o| o.kind == z) {
                    errors.push(CompileError::at(
                        k.map.line,
                        format!("{}: {} >: there is no {} zone to play", head, z.word(), z.word()),
                    ));
                }
            }
        }
        for b in &sc.bends {
            if !p.zones.iter().any(|z| z.kind == b.zone) {
                errors.push(CompileError::at(
                    b.line,
                    format!("{}: bend {}: there is no {} zone", head, b.zone.word(), b.zone.word()),
                ));
            }
        }
    }
    for b in &p.bends {
        if !p.zones.iter().any(|z| z.kind == b.zone) {
            errors
                .push(CompileError::at(b.line, format!("bend {}: there is no {} zone", b.zone.word(), b.zone.word())));
        }
    }
    // A page reaches across the tracks of a set, and a rig may lack what it
    // names (an `fm` bass has no cutoff): a line that names nothing here is
    // left out, not refused. One that does name something is checked whole.
    for page in &p.pages {
        let head = format!("page {}", page.name);
        for line in &page.knobs {
            let bare = MidiMapDef { range: None, ..line.clone() };
            if validate_maps(song, core::slice::from_ref(&bare), |_, _| String::new()).is_empty() {
                errors.extend(validate_maps(song, core::slice::from_ref(line), |source, words| {
                    format!("{}: {} > {}", head, source, words)
                }));
            }
        }
    }
    for b in &p.keyboard {
        if let KeyAction::Voice(VoiceMove::To(name)) = &b.action {
            if !p.pages.iter().any(|pg| &pg.name == name) {
                errors.push(CompileError::at(
                    b.line,
                    format!("keyboard: {} > voice {}: no page named '{}'", b.key, name, name),
                ));
            }
        }
        if b.key == "q" {
            errors.push(CompileError::at(b.line, String::from("keyboard: q quits; bind another key")));
        }
        if let KeyAction::Perform(name) = &b.action {
            if !p.scenes.iter().any(|s| &s.name == name) {
                let known: Vec<&str> = p.scenes.iter().map(|s| s.name.as_str()).collect();
                errors.push(CompileError::at(
                    b.line,
                    if known.is_empty() {
                        format!(
                            "keyboard: {} > perform {}: no scene; write one with `perform {} {{ ... }}`",
                            b.key, name, name
                        )
                    } else {
                        format!(
                            "keyboard: {} > perform {}: no such scene (there are {})",
                            b.key,
                            name,
                            known.join(", ")
                        )
                    },
                ));
            }
        }
    }
    errors
}

/// The `scale` line names a root and a scale the table knows: an unknown one
/// used to play as C major without a word.
pub(super) fn validate_scale(song: &Song) -> Vec<CompileError> {
    use crate::perform::scale;
    let Some(def) = &song.globals.scale else { return Vec::new() };
    let mut errors = Vec::new();
    if scale::root_pc(&def.root).is_none() {
        errors.push(CompileError::new(format!("scale: '{}' is not a root; write one like E, F# or Bb", def.root)));
    }
    if scale::intervals(&def.kind).is_none() {
        let known: Vec<&str> = scale::names().collect();
        errors.push(CompileError::new(format!("scale: '{}' is not a scale; one of {}", def.kind, known.join(", "))));
    }
    errors
}

/// Validate `midi { }` against the song. A knob, a keyboard or a pad on
/// something that does not exist is an error here rather than a control that
/// silently does nothing on stage.
pub(super) fn validate_midi(song: &Song) -> Vec<CompileError> {
    validate_maps(song, &song.midi, |source, words| format!("midi: {} > {}", source, words))
}

/// `maps` against `song`, each error headed by what `head` makes of the
/// line's source and target.
fn validate_maps(song: &Song, maps: &[MidiMapDef], head: impl Fn(&str, &str) -> String) -> Vec<CompileError> {
    let mut errors = Vec::new();
    // The module a track plays, as written: its name and its kind.
    let module_of = |track: &str| -> Option<(&str, Option<&str>)> {
        let t = song.tracks.iter().find(|t| t.name == track)?;
        let kind = song.module_defs.iter().find(|m| m.name == t.using_instrument).map(|m| m.module_type.as_str());
        Some((t.using_instrument.as_str(), kind))
    };
    for map in maps {
        let words: Vec<&str> = map.target.split('.').collect();
        let source = match map.source {
            MidiSource::Cc(cc) => format!("cc {}", cc),
            MidiSource::Keys => String::from("keys"),
            MidiSource::Pad(n) => format!("pad {}", n),
            MidiSource::Key(n) => format!("key {}", n),
        };
        let err = |msg: String| CompileError::at(map.line, format!("{}: {}", head(&source, &words.join(" ")), msg));
        if let (MidiSource::Key(n), Some(z)) =
            (map.source, song.perform.zones.iter().find(|z| z.kind == ZoneKind::Triggers))
        {
            if !(z.low..=z.high).contains(&n) {
                errors.push(err(format!("{} is outside the trigger zone, {}..{}", n, z.low, z.high)));
            }
        }
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
            MidiSource::Key(_) if matches!(words.first(), Some(&("next" | "prev" | "step"))) => errors.push(err(
                String::from("the set moves from the computer's keys, not the controller: `keyboard { space > next }`"),
            )),
            MidiSource::Pad(_) | MidiSource::Key(_) => match words.as_slice() {
                ["freeze"] | ["next"] | ["prev"] => {}
                ["step", n] => {
                    if !n.parse::<usize>().is_ok_and(|n| n >= 1) {
                        errors.push(err(String::from("a set's steps count from 1: `pad 50 > step 3`")));
                    }
                }
                ["step"] => errors.push(err(String::from("which step: `pad 50 > step 3`"))),
                ["repeat", d] => {
                    if crate::midi::repeat_division(d).is_none() {
                        errors.push(err(format!("a repeat is 1/4, 1/8, 1/16 or 1/32 of a bar, got '{}'", d)));
                    }
                }
                ["repeat"] => errors.push(err(String::from("how long a repeat: `pad 38 > repeat 1/16`"))),
                [fx, rest @ ..] if crate::midi::out_fx(fx, rest).is_some() => {}
                ["gate", d] => errors.push(err(format!("a gate chops in 1/4, 1/8, 1/16 or 1/32, got '{}'", d))),
                ["play", track, rest @ ..] if rest.len() <= 1 => match module_of(track) {
                    None => errors.push(err(format!("no track named '{}'", track))),
                    Some((_, kind)) => {
                        if crate::midi::play_note(song, track, rest.first().copied()).is_none() {
                            errors.push(err(match kind {
                                Some("beats") => format!(
                                    "'{}' is not a drum (kick, snare, clap, hat, openhat, tom, tom2, tom3, crash)",
                                    rest[0]
                                ),
                                _ => format!("'{}' is not a note: write one like C2 or F#3", rest[0]),
                            }));
                        }
                    }
                },
                ["mute" | "toggle" | "throw" | "hold", track] => {
                    if module_of(track).is_none() {
                        errors.push(err(format!("no track named '{}'", track)));
                    }
                }
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
                _ => errors.push(err(String::from(
                    "a pad hits a drum (`pad 36 > kick kick`), or does one of: mute <track>, toggle <track>, \
throw <track>, hold <track>, play <track> [<note>], repeat <1/4|1/8|1/16|1/32>, freeze, \
tapestop, gate [<1/8|1/16|1/32>], crush, scream, cut, sweep, next, prev, step <n>",
                ))),
            },
            MidiSource::Cc(_) => match words.as_slice() {
                ["reverb_mix"] | ["delay_mix"] | ["reverb_freeze"] | ["tempo"] => {}
                // The knobs' voice: a knob picks the page, an encoder steps.
                ["voice"] | ["voice", "step"] => {}
                [track, "delay_send" | "reverb_send"] => {
                    if !song.tracks.iter().any(|t| &t.name == track) {
                        errors.push(err(format!("no track named '{}'", track)));
                    }
                }
                ["master", param] => {
                    if !MASTER_AUTO_PARAMS.contains(param) {
                        errors
                            .push(err(format!("a knob on the master moves one of {}", MASTER_AUTO_PARAMS.join(", "))));
                    } else if crate::midi::master_node(song, None, param).is_none() {
                        errors.push(err(format!(
                            "the master chain has no {} node",
                            master_auto_node_kinds(param).join(" or ")
                        )));
                    }
                }
                ["master", label, param] => {
                    if !MASTER_AUTO_PARAMS.contains(param) {
                        errors.push(err(format!("a knob on a node moves one of {}", MASTER_AUTO_PARAMS.join(", "))));
                    } else if crate::midi::master_node(song, Some(label), param).is_none() {
                        errors.push(err(format!(
                            "the master chain has no node named '{}' that takes '{}'; name one with `> lowpass(...) as {}`",
                            label, param, label
                        )));
                    }
                }
                [name, "level"] | [name, "pan"] => {
                    let is_track = song.tracks.iter().any(|t| &t.name == name);
                    let is_module = song.module_defs.iter().any(|m| &m.name == name)
                        || song.instruments.iter().any(|i| &i.name == name);
                    if !is_track && !is_module {
                        errors.push(err(format!("no track or module named '{}'", name)));
                    }
                }
                [track, node, param] if *param != "wet" => match song.tracks.iter().find(|t| &t.name == track) {
                    None => errors.push(err(format!("no track named '{}'", track))),
                    Some(_) if !MASTER_AUTO_PARAMS.contains(param) => errors
                        .push(err(format!("a knob on a node moves wet or one of {}", MASTER_AUTO_PARAMS.join(", ")))),
                    Some(t) => match t.routing.iter().find(|n| n.label.as_deref() == Some(node)) {
                        None => errors.push(err(format!(
                            "track '{}' has no node named '{}'; name one with `> effect(...) as {}`",
                            track, node, node
                        ))),
                        Some(n) if master_auto_node_kinds(param).contains(&n.kind.as_str()) => {}
                        Some(n) => errors.push(err(format!("'{}' is a {}, which has no '{}'", node, n.kind, param))),
                    },
                },
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
                [module, param] => match crate::midi::module_named(song, module) {
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
    // Each lane with who owns it, for the message, and the tracks a scene
    // redefines, which a `<track>.<node> wet` looks at first. A top-level
    // lane has only the song's own tracks.
    let lanes = song
        .scenes
        .iter()
        .flat_map(|sc| sc.automations.iter().map(move |a| (format!("scene '{}': ", sc.name), &sc.tracks[..], a)))
        .chain(song.automations.iter().map(|a| (String::new(), &[][..], a)));
    for (owner, scene_tracks, auto) in lanes {
        let target = auto.target.as_str();
        if target == "reverb_mix" || target == "delay_mix" || target == "reverb_freeze" {
            continue;
        }
        let (name, param) = match target.find('.') {
            Some(i) => (&target[..i], &target[i + 1..]),
            None => {
                errors.push(CompileError::new(format!(
                    "{}automation target '{}' must be reverb_mix, delay_mix, <track> level or <module> <param>",
                    owner, target
                )));
                continue;
            }
        };
        // `master <node> <param>`: one named node of the master chain, the
        // way a knob names it, for a chain with two filters on it.
        if let Some((label, p)) = param.split_once('.').filter(|_| name == "master") {
            if !MASTER_AUTO_PARAMS.contains(&p) {
                errors.push(CompileError::new(format!(
                    "{}automation — a master node moves one of {}",
                    owner,
                    MASTER_AUTO_PARAMS.join(", ")
                )));
            } else if crate::midi::master_node(song, Some(label), p).is_none() {
                errors.push(CompileError::new(format!(
                    "{}automation — the master chain has no node named '{}' that takes '{}'; name one with `> lowpass(...) as {}`",
                    owner, label, p, label
                )));
            }
            continue;
        }
        if name == "master" {
            let needed = master_auto_node_kinds(param);
            if needed.is_empty() {
                errors.push(CompileError::new(format!(
                    "{}automation — master has no parameter '{}' (expected {})",
                    owner,
                    param,
                    MASTER_AUTO_PARAMS.join(", ")
                )));
            } else if !song.master.as_ref().is_some_and(|m| m.chain.iter().any(|n| needed.contains(&n.kind.as_str()))) {
                errors.push(CompileError::new(format!(
                    "{}automation — `auto master {}` needs a {} node in the master chain",
                    owner,
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
            let routing = scene_tracks
                .iter()
                .find(|t| t.name == name && !t.routing.is_empty())
                .or_else(|| song.tracks.iter().find(|t| t.name == name))
                .map(|t| &t.routing);
            match routing {
                Some(r) if r.iter().any(|n| n.label.as_deref() == Some(node)) => continue,
                Some(_) => {
                    let named: Vec<&str> = routing.into_iter().flatten().filter_map(|n| n.label.as_deref()).collect();
                    errors.push(CompileError::new(format!(
                        "{}automation target '{}' — track '{}' has no node named '{}'{}",
                        owner,
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
                        "{}automation target '{}' — no track named '{}'",
                        owner, target, name
                    )));
                    continue;
                }
            }
        }
        let is_track = scene_tracks.iter().chain(song.tracks.iter()).any(|t| t.name == name);
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
                            "{}automation — {}",
                            owner,
                            unknown_param_message(kind, name, param)
                        )));
                    }
                }
            }
            None if is_graph => errors.push(CompileError::new(format!(
                "{}automation target '{}' — graph instruments have no named params (only level)",
                owner, target
            ))),
            None => errors.push(CompileError::new(format!(
                "{}automation target '{}' — no module or track named '{}'",
                owner, target, name
            ))),
        }
    }
    errors
}
