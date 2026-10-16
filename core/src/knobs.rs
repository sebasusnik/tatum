//! What a track offers a knob: its level, pan and sends, its module's
//! parameters, and the named nodes of its chain -- the things a `midi`
//! line can name for it, the ones worth a hand on stage first. A screen
//! lists these for a track; a knob turned on one of them goes through
//! `LivePlanner::try_knob`.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::dsl::ast::{MidiSource, Song};
use crate::params::{self, ModuleKind};

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
                if crate::midi::node_kinds_for(p).contains(&node.kind.as_str()) {
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
