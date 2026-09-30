//! Putting a knob on something from the screen: what a track offers a knob,
//! and the `midi` line that keeps it, written where the song keeps its knobs.
//!
//! What a track offers is what a `midi` line can name for it: its level, pan
//! and sends, its module's parameters, and the named nodes of its chain. The
//! ones worth a hand on stage come first.

use std::fs;
use std::path::PathBuf;

use tatum_core::dsl::ast::{MidiSource, Song};
use tatum_core::params::{self, ModuleKind};

/// One thing a knob can be put on.
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    /// As a `midi` line writes it: `acid cutoff`, `bass lp wet`.
    pub words: String,
    /// What it does, for choosing.
    pub doc: String,
}

impl Target {
    /// As the song stores it: `acid.cutoff`.
    pub fn dotted(&self) -> String {
        self.words.replace(' ', ".")
    }
}

/// Module parameters a hand reaches for first, in this order.
const FIRST: &[&str] = &[
    "cutoff",
    "resonance",
    "cutoff_env",
    "decay",
    "release",
    "attack",
    "drive",
    "mod_index",
    "feedback",
    "detune",
    "glide",
    "lfo_rate",
    "lfo_depth",
    "chorus_mix",
];

/// Parameters a knob reaches on a node of a chain, the ones `auto` sweeps.
const NODE_PARAMS: &[&str] = &["cutoff", "drive", "tilt", "eq_low", "eq_mid", "eq_high", "gain", "comp_threshold"];

/// Everything a knob can be put on for `track`, the likeliest first.
pub fn targets(song: &Song, track: &str) -> Vec<Target> {
    let mut out = Vec::new();
    let Some(def) = song.tracks.iter().rev().find(|t| t.name == track && !t.using_instrument.is_empty()) else {
        return out;
    };
    let module = song.module_defs.iter().find(|m| m.name == def.using_instrument);
    let mut module_targets: Vec<(usize, Target)> = Vec::new();
    if let Some(m) = module {
        let kind = ModuleKind::from_str(&m.module_type);
        // `<name> level` and `<name> pan` are the track's own, below.
        for spec in kind.map(params::specs).unwrap_or(&[]).iter().filter(|s| !matches!(s.name, "level" | "pan")) {
            let rank = FIRST.iter().position(|f| *f == spec.name).unwrap_or(FIRST.len());
            module_targets
                .push((rank, Target { words: format!("{} {}", m.name, spec.name), doc: spec.doc.to_string() }));
        }
    }
    module_targets.sort_by_key(|(rank, _)| *rank);
    let (first, rest): (Vec<_>, Vec<_>) = module_targets.into_iter().partition(|(r, _)| *r < FIRST.len());
    out.extend(first.into_iter().map(|(_, t)| t));
    for (what, doc) in [
        ("level", "the track's level"),
        ("pan", "left to right"),
        ("delay_send", "how much goes to the delay"),
        ("reverb_send", "how much goes to the reverb"),
    ] {
        out.push(Target { words: format!("{} {}", track, what), doc: doc.into() });
    }
    // The named nodes of the chain: `out > lowpass(2khz) as lp > master`.
    for tdef in song.tracks.iter().filter(|t| t.name == track) {
        for node in &tdef.routing {
            let Some(label) = &node.label else { continue };
            for p in NODE_PARAMS {
                if tatum_core::midi::node_kinds_for(p).contains(&node.kind.as_str()) {
                    let words = format!("{} {} {}", track, label, p);
                    if !out.iter().any(|t| t.words == words) {
                        out.push(Target { words, doc: format!("{} on the {} in its chain", p, node.kind) });
                    }
                }
            }
            let words = format!("{} {} wet", track, label);
            if !out.iter().any(|t| t.words == words) {
                out.push(Target { words, doc: format!("the {} in and out", node.kind) });
            }
        }
    }
    out.extend(rest.into_iter().map(|(_, t)| t));
    out
}

/// Every controller the song maps, with the target it moves, dotted.
pub fn bindings(song: &Song) -> Vec<(u8, String)> {
    song.midi
        .iter()
        .filter_map(|m| match m.source {
            MidiSource::Cc(cc) => Some((cc, m.target.clone())),
            _ => None,
        })
        .collect()
}

/// What a knob change does to the files.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Change {
    /// The knob moves this and nothing else it moved before.
    Keep(u8),
    /// The knob moves this as well: a macro.
    Add(u8),
    /// No knob moves this any more.
    Free,
}

/// Write `change` for `target` into the files of a song, root first. The
/// line goes in the first `midi` block found -- in a set that is the rig's,
/// so the knob stays on through every step -- or, with none, in a new block
/// at the end of the first file `use`d, else the root. Returns each file
/// changed with what it held before, for undo, and what was done.
pub fn write(files: &[PathBuf], target: &Target, change: Change) -> Result<(Vec<(PathBuf, String)>, String), String> {
    let mut texts = Vec::new();
    for f in files {
        texts.push(fs::read_to_string(f).map_err(|e| format!("cannot read {}: {}", f.display(), e))?);
    }
    let dotted = target.dotted();
    let mut changed: Vec<usize> = Vec::new();
    // Out of every block first: the lines the change replaces.
    for (i, text) in texts.iter_mut().enumerate() {
        let edited = remove_lines(text, |cc, t| match change {
            Change::Keep(k) => cc == k || t == dotted,
            Change::Add(k) => cc == k && t == dotted,
            Change::Free => t == dotted,
        });
        if edited != *text {
            *text = edited;
            changed.push(i);
        }
    }
    let said = match change {
        Change::Keep(cc) | Change::Add(cc) => {
            let line = format!("    cc {} > {}", cc, target.words);
            let at = texts.iter().position(|t| midi_block(t).is_some());
            let i = match at {
                Some(i) => {
                    let (_, close) = midi_block(&texts[i]).expect("found above");
                    texts[i].insert_str(close, &format!("{}\n", line));
                    i
                }
                None => {
                    let i = if files.len() > 1 { 1 } else { 0 };
                    let t = &mut texts[i];
                    if !t.ends_with('\n') {
                        t.push('\n');
                    }
                    t.push_str(&format!("\nmidi {{\n{}\n}}\n", line));
                    i
                }
            };
            if !changed.contains(&i) {
                changed.push(i);
            }
            let name = files[i].file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
            let verb = if matches!(change, Change::Add(_)) { "also moves" } else { "moves" };
            format!("knob cc {} {} {} · in {}", cc, verb, target.words, name)
        }
        Change::Free => {
            if changed.is_empty() {
                return Err(format!("no knob moves {}", target.words));
            }
            format!("no knob moves {} now", target.words)
        }
    };
    let mut undo = Vec::new();
    for i in changed {
        let before = fs::read_to_string(&files[i]).unwrap_or_default();
        fs::write(&files[i], &texts[i]).map_err(|e| format!("cannot write {}: {}", files[i].display(), e))?;
        undo.push((files[i].clone(), before));
    }
    Ok((undo, said))
}

/// Where the `midi { }` block of `text` opens and where its closing brace
/// line starts, when it has one at the top level.
fn midi_block(text: &str) -> Option<(usize, usize)> {
    let mut at = 0;
    let mut open: Option<usize> = None;
    for line in text.split_inclusive('\n') {
        let t = line.trim();
        match open {
            None if t.starts_with("midi") && t.trim_start_matches("midi").trim() == "{" => open = Some(at),
            Some(o) if t == "}" && line.starts_with('}') => return Some((o, at)),
            _ => {}
        }
        at += line.len();
    }
    None
}

/// `text` without the `cc` lines of its `midi` block that `drop` picks, by
/// controller and dotted target.
fn remove_lines(text: &str, drop: impl Fn(u8, &str) -> bool) -> String {
    let Some((open, close)) = midi_block(text) else { return text.to_string() };
    let mut out = String::with_capacity(text.len());
    out.push_str(&text[..open]);
    for line in text[open..close].split_inclusive('\n') {
        if let Some((cc, target)) = cc_line(line) {
            if drop(cc, &target) {
                continue;
            }
        }
        out.push_str(line);
    }
    out.push_str(&text[close..]);
    out
}

/// `cc 74 > acid cutoff 200hz..4khz  # note` as (74, `acid.cutoff`).
fn cc_line(line: &str) -> Option<(u8, String)> {
    let code = line.split('#').next()?.trim();
    let rest = code.strip_prefix("cc")?.trim_start();
    let (num, target) = rest.split_once('>')?;
    let cc = num.trim().parse().ok()?;
    let words: Vec<&str> = target.split_whitespace().take_while(|w| !w.contains("..")).collect();
    Some((cc, words.join(".")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str, text: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tatum-knobs-{}-{}", std::process::id(), name));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        fs::write(&p, text).unwrap();
        p
    }

    fn acid() -> Target {
        Target { words: "acid cutoff".into(), doc: String::new() }
    }

    #[test]
    fn a_knob_kept_replaces_what_it_moved() {
        let f = tmp("keep.synth", "tempo 120\nmidi {\n    cc 74 > pad cutoff 1khz..4khz\n    cc 71 > pad level\n}\n");
        let (undo, said) = write(std::slice::from_ref(&f), &acid(), Change::Keep(74)).unwrap();
        let text = fs::read_to_string(&f).unwrap();
        assert_eq!(text, "tempo 120\nmidi {\n    cc 71 > pad level\n    cc 74 > acid cutoff\n}\n");
        assert!(said.contains("cc 74 moves acid cutoff"), "{said}");
        assert_eq!(undo.len(), 1);
    }

    #[test]
    fn a_knob_added_keeps_what_it_moved() {
        let f = tmp("add.synth", "midi {\n    cc 74 > pad cutoff\n}\n");
        write(std::slice::from_ref(&f), &acid(), Change::Add(74)).unwrap();
        assert_eq!(fs::read_to_string(&f).unwrap(), "midi {\n    cc 74 > pad cutoff\n    cc 74 > acid cutoff\n}\n");
    }

    #[test]
    fn a_target_moves_to_the_new_knob() {
        let f = tmp("move.synth", "midi {\n    cc 20 > acid cutoff 200hz..2khz # old\n}\n");
        write(std::slice::from_ref(&f), &acid(), Change::Keep(74)).unwrap();
        assert_eq!(fs::read_to_string(&f).unwrap(), "midi {\n    cc 74 > acid cutoff\n}\n");
    }

    #[test]
    fn with_no_block_the_rig_gets_one() {
        let step = tmp("step.synth", "use \"rig.synth\"\ntempo 130\n");
        let rig = tmp("rig.synth", "tempo 120");
        write(&[step.clone(), rig.clone()], &acid(), Change::Keep(74)).unwrap();
        assert_eq!(fs::read_to_string(&rig).unwrap(), "tempo 120\n\nmidi {\n    cc 74 > acid cutoff\n}\n");
        assert_eq!(fs::read_to_string(&step).unwrap(), "use \"rig.synth\"\ntempo 130\n");
    }

    #[test]
    fn freeing_takes_every_line_off() {
        let f = tmp(
            "free.synth",
            "midi {\n    cc 74 > acid cutoff\n    cc 75 > acid cutoff 1khz..2khz\n    cc 76 > pad level\n}\n",
        );
        write(std::slice::from_ref(&f), &acid(), Change::Free).unwrap();
        assert_eq!(fs::read_to_string(&f).unwrap(), "midi {\n    cc 76 > pad level\n}\n");
        assert!(write(std::slice::from_ref(&f), &acid(), Change::Free).is_err());
    }

    #[test]
    fn a_track_offers_its_module_first_then_itself() {
        let song = tatum_core::dsl::parse(
            "module bass acid { cutoff 500hz }\npattern p { 1 - - - }\n\
             track acid { play p using acid out > lowpass(2khz) as lp > master }\n",
        )
        .unwrap();
        let t: Vec<String> = targets(&song, "acid").into_iter().map(|t| t.words).collect();
        assert_eq!(t[0], "acid cutoff");
        assert_eq!(t[1], "acid resonance");
        assert!(t.contains(&"acid level".to_string()));
        assert!(t.contains(&"acid lp cutoff".to_string()));
        assert!(t.contains(&"acid lp wet".to_string()));
    }
}
