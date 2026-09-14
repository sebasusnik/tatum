//! Node registry — every effect and graph node the DSL accepts in a chain
//! (`out > saturate(0.3) > master`, `master { in > eq(...) > out }`) or in an
//! `instrument { }` graph, with its arguments, ranges and defaults.
//!
//! The compiler validates node arguments against this table, so a misspelled
//! option or an out-of-range value is a compile error instead of a default
//! applied in silence. The CLI and MCP render it as documentation.

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;

use crate::dsl::ast::Param;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Source,
    Envelope,
    Filter,
    Effect,
    Util,
}

impl Category {
    pub fn as_str(self) -> &'static str {
        match self {
            Category::Source => "source",
            Category::Envelope => "envelope",
            Category::Filter => "filter",
            Category::Effect => "effect",
            Category::Util => "util",
        }
    }
}

/// One argument: positional (by order) or named (`key=value`).
#[derive(Debug, Clone, Copy)]
pub struct Arg {
    pub name: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub doc: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct NodeDefSpec {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub category: Category,
    pub positional: &'static [Arg],
    pub named: &'static [Arg],
    /// Accepts a waveform word (`saw`, `square`, ...) as an argument.
    pub waveform: bool,
    /// Accepts a rhythm division (`1/8`) as an argument.
    pub rhythm: bool,
    /// Usable in track / bus / master chains (not only inside `instrument` graphs).
    pub in_chains: bool,
    pub doc: &'static str,
}

macro_rules! arg {
    ($name:expr, $min:expr, $max:expr, $default:expr, $doc:expr) => {
        Arg { name: $name, min: $min, max: $max, default: $default, doc: $doc }
    };
}

const FILTER_ENV: &[Arg] = &[
    arg!("ea", 0.0, 10.0, 0.005, "envelope attack, seconds"),
    arg!("ed", 0.0, 10.0, 0.2, "envelope decay, seconds"),
    arg!("es", 0.0, 1.0, 0.0, "envelope sustain level"),
    arg!("er", 0.0, 10.0, 0.1, "envelope release, seconds"),
    arg!("edepth", -20000.0, 20000.0, 0.0, "envelope depth in Hz added to the cutoff"),
    arg!("lfo_bars", 0.0, 64.0, 0.0, "LFO cycle length in bars (tempo-synced); 0 = use lfo_hz"),
    arg!("lfo_hz", 0.0, 20.0, 0.0, "free LFO rate in Hz"),
    arg!("lfo_depth", 0.0, 20000.0, 0.0, "LFO sweep in Hz around the cutoff; 0 = off"),
];
const FILTER_POS: &[Arg] = &[
    arg!("cutoff", 20.0, 20000.0, 1000.0, "Hz"),
    arg!("resonance", 0.0, 1.0, 0.5, "0 = none, 1 = self-oscillation"),
];
const OSC_POS: &[Arg] = &[arg!("freq", 1.0, 20000.0, 440.0, "base frequency in Hz; notes retune it")];

pub const NODES: &[NodeDefSpec] = &[
    NodeDefSpec { name: "osc", aliases: &[], category: Category::Source, positional: OSC_POS,
        named: &[arg!("pitch", -48.0, 48.0, 0.0, "offset in semitones")], waveform: true, rhythm: false, in_chains: false,
        doc: "Oscillator that follows the played note. Waveform word: sine | saw | square | triangle. Has analog drift." },
    NodeDefSpec { name: "fixosc", aliases: &[], category: Category::Source, positional: OSC_POS, named: &[],
        waveform: true, rhythm: false, in_chains: false, doc: "Fixed-frequency oscillator (ignores the note), for metallic stacks." },
    NodeDefSpec { name: "pitch_osc", aliases: &[], category: Category::Source,
        positional: &[arg!("start_freq", 1.0, 20000.0, 300.0, "Hz at trigger"), arg!("end_freq", 1.0, 20000.0, 55.0, "Hz it decays to"), arg!("decay", 0.9, 0.99999, 0.995, "per-sample pitch decay factor")],
        named: &[arg!("decay", 0.9, 0.99999, 0.995, "per-sample pitch decay factor")], waveform: true, rhythm: false, in_chains: false,
        doc: "Pitch-sweeping oscillator for kicks and toms." },
    NodeDefSpec { name: "noise", aliases: &[], category: Category::Source, positional: &[], named: &[],
        waveform: false, rhythm: false, in_chains: false, doc: "White noise." },
    NodeDefSpec { name: "lfo", aliases: &[], category: Category::Source,
        positional: &[arg!("rate", 0.01, 50.0, 1.0, "Hz"), arg!("depth", 0.0, 1.0, 0.5, "amplitude")],
        named: &[], waveform: false, rhythm: false, in_chains: false, doc: "Low-frequency oscillator as a control signal." },
    NodeDefSpec { name: "adsr", aliases: &[], category: Category::Envelope,
        positional: &[arg!("attack", 0.0, 10.0, 0.01, "seconds"), arg!("decay", 0.0, 10.0, 0.1, "seconds"), arg!("sustain", 0.0, 1.0, 0.7, "level"), arg!("release", 0.0, 10.0, 0.3, "seconds")],
        named: &[], waveform: false, rhythm: false, in_chains: false, doc: "Amplitude envelope; the last envelope before `out` gates the voice." },
    NodeDefSpec { name: "perc", aliases: &[], category: Category::Envelope,
        positional: &[arg!("attack", 0.0, 10.0, 0.001, "seconds"), arg!("decay", 0.0, 10.0, 0.3, "seconds")],
        named: &[], waveform: false, rhythm: false, in_chains: false, doc: "Percussive envelope: attack, decay, no sustain." },
    NodeDefSpec { name: "lowpass", aliases: &[], category: Category::Filter, positional: FILTER_POS, named: FILTER_ENV,
        waveform: false, rhythm: false, in_chains: true, doc: "12 dB biquad lowpass with optional cutoff envelope." },
    NodeDefSpec { name: "highpass", aliases: &[], category: Category::Filter, positional: FILTER_POS, named: FILTER_ENV,
        waveform: false, rhythm: false, in_chains: true, doc: "12 dB biquad highpass; use it to keep pads out of the sub." },
    NodeDefSpec { name: "bandpass", aliases: &[], category: Category::Filter, positional: FILTER_POS, named: FILTER_ENV,
        waveform: false, rhythm: false, in_chains: true, doc: "Biquad bandpass." },
    NodeDefSpec { name: "ladder", aliases: &[], category: Category::Filter, positional: FILTER_POS, named: FILTER_ENV,
        waveform: false, rhythm: false, in_chains: true, doc: "Moog-style 4-pole ladder lowpass with drive." },
    NodeDefSpec { name: "mix", aliases: &[], category: Category::Util, positional: &[], named: &[],
        waveform: false, rhythm: false, in_chains: false, doc: "Sums every node routed into it." },
    NodeDefSpec { name: "gain", aliases: &[], category: Category::Util,
        positional: &[arg!("amount", 0.0, 10.0, 1.0, "linear multiplier")], named: &[],
        waveform: false, rhythm: false, in_chains: true, doc: "Linear gain." },
    NodeDefSpec { name: "saturate", aliases: &["drive"], category: Category::Effect,
        positional: &[arg!("drive", 0.0, 10.0, 1.0, "input gain into the tanh; 0.3 warm, 2 heavy")], named: &[],
        waveform: false, rhythm: false, in_chains: true, doc: "tanh soft clipping." },
    NodeDefSpec { name: "chorus", aliases: &[], category: Category::Effect,
        positional: &[arg!("mix", 0.0, 1.0, 0.3, "wet amount")], named: &[],
        waveform: false, rhythm: false, in_chains: true, doc: "Stereo chorus." },
    NodeDefSpec { name: "bitcrush", aliases: &[], category: Category::Effect,
        positional: &[arg!("bits", 1.0, 16.0, 8.0, "bit depth"), arg!("rate", 0.0, 1.0, 0.0, "sample-rate reduction, 0 = none")], named: &[],
        waveform: false, rhythm: false, in_chains: true, doc: "Bit and sample-rate reduction." },
    NodeDefSpec { name: "compressor", aliases: &[], category: Category::Effect,
        positional: &[arg!("threshold", -60.0, 0.0, -3.0, "dB")],
        named: &[arg!("ratio", 1.0, 20.0, 3.0, "n:1"), arg!("attack", 0.0, 500.0, 20.0, "ms"), arg!("release", 1.0, 2000.0, 100.0, "ms"), arg!("makeup", 0.0, 4.0, 1.0, "linear gain after compression; 2 = +6 dB")],
        waveform: false, rhythm: false, in_chains: true, doc: "Feed-forward compressor." },
    NodeDefSpec { name: "limiter", aliases: &[], category: Category::Effect,
        positional: &[arg!("threshold", 0.1, 1.0, 0.95, "output ceiling")], named: &[],
        waveform: false, rhythm: false, in_chains: true, doc: "Lookahead peak limiter; put it last on the master." },
    NodeDefSpec { name: "tilt", aliases: &[], category: Category::Effect,
        positional: &[arg!("amount", -1.0, 1.0, 0.0, "negative = darker, positive = brighter")], named: &[],
        waveform: false, rhythm: false, in_chains: true, doc: "One-knob tilt EQ around 1 kHz." },
    NodeDefSpec { name: "eq", aliases: &[], category: Category::Effect, positional: &[],
        named: &[arg!("low", -12.0, 12.0, 0.0, "dB shelf at 200 Hz"), arg!("mid", -12.0, 12.0, 0.0, "dB peak at 1 kHz"), arg!("high", -12.0, 12.0, 0.0, "dB shelf at 8 kHz")],
        waveform: false, rhythm: false, in_chains: true, doc: "Three-band EQ." },
    NodeDefSpec { name: "delay", aliases: &[], category: Category::Effect,
        positional: &[arg!("feedback", 0.0, 0.95, 0.3, "repeat level")], named: &[],
        waveform: false, rhythm: true, in_chains: true, doc: "Tempo-synced delay inside a chain: `delay(1/8, 0.4)`. Division defaults to 1/4." },
    NodeDefSpec { name: "autopan", aliases: &[], category: Category::Effect,
        positional: &[arg!("depth", 0.0, 1.0, 0.5, "0 = still, 1 = full left/right")],
        named: &[arg!("bars", 0.0, 64.0, 0.0, "cycle length in bars (tempo-synced)"), arg!("hz", 0.0, 20.0, 0.25, "free rate in Hz when bars is 0")],
        waveform: false, rhythm: false, in_chains: true, doc: "Slow stereo movement: `autopan(0.6, bars=4)`." },
    NodeDefSpec { name: "reverb", aliases: &[], category: Category::Effect,
        positional: &[arg!("size", 0.0, 1.0, 0.5, "room size / decay")], named: &[],
        waveform: false, rhythm: false, in_chains: true, doc: "Plate reverb, wet only; use it on a bus." },
];

pub fn lookup(kind: &str) -> Option<&'static NodeDefSpec> {
    NODES.iter().find(|n| n.name == kind || n.aliases.contains(&kind))
}

pub fn suggest(kind: &str) -> Option<&'static str> {
    let mut best: Option<(usize, &'static str)> = None;
    for n in NODES {
        let d = crate::params::levenshtein(kind, n.name);
        if best.map_or(true, |(bd, _)| d < bd) { best = Some((d, n.name)); }
    }
    best.filter(|(d, _)| *d <= (kind.len() / 3).max(2)).map(|(_, n)| n)
}

fn param_value(p: &Param) -> Option<f32> {
    match p {
        Param::Float(v) => Some(*v),
        Param::Expr(e) => Some(e.eval()),
        _ => None,
    }
}

/// Check a node's arguments. Returns every problem found, empty when valid.
pub fn validate(kind: &str, params: &[Param]) -> Vec<String> {
    let mut errors = Vec::new();
    let Some(spec) = lookup(kind) else {
        let mut msg = format!("unknown node '{}'", kind);
        if let Some(s) = suggest(kind) { msg.push_str(&format!(". Did you mean '{}'?", s)); }
        errors.push(msg);
        return errors;
    };

    let mut positional_idx = 0usize;
    for p in params {
        match p {
            Param::Float(_) | Param::Expr(_) => {
                let v = param_value(p).unwrap_or(0.0);
                match spec.positional.get(positional_idx) {
                    Some(arg) => {
                        if v < arg.min || v > arg.max {
                            errors.push(format!("{}: {} = {} is out of range ({}..{})", spec.name, arg.name, v, arg.min, arg.max));
                        }
                    }
                    None => errors.push(format!(
                        "{}: too many positional arguments ({} takes {})",
                        spec.name, spec.name, spec.positional.len()
                    )),
                }
                positional_idx += 1;
            }
            Param::Named(name, v) => match spec.named.iter().find(|a| a.name == name) {
                Some(arg) => {
                    if *v < arg.min || *v > arg.max {
                        errors.push(format!("{}: {} = {} is out of range ({}..{})", spec.name, arg.name, v, arg.min, arg.max));
                    }
                }
                None => {
                    let known: Vec<&str> = spec.named.iter().map(|a| a.name).collect();
                    errors.push(if known.is_empty() {
                        format!("{}: takes no named options, got '{}'", spec.name, name)
                    } else {
                        format!("{}: unknown option '{}' (expected {})", spec.name, name, known.join(", "))
                    });
                }
            },
            Param::Waveform(w) => {
                if !spec.waveform {
                    errors.push(format!("{}: unexpected word '{}' (arguments are numbers or name=value)", spec.name, w));
                }
            }
            Param::RhythmDiv(_, _) => {
                if !spec.rhythm {
                    errors.push(format!("{}: does not take a rhythm division", spec.name));
                }
            }
        }
    }
    errors
}

/// Markdown reference of every node.
pub fn markdown() -> String {
    let mut out = String::from("## effects and nodes\n\n");
    out.push_str("Used in chains (`out > saturate(0.3) > master`, buses, `master { in > ... > out }`) and,\n");
    out.push_str("for sources and envelopes, inside `instrument { }` graphs. Positional arguments in order,\n");
    out.push_str("then `name=value` options. Unknown names and out-of-range values are compile errors.\n\n");
    out.push_str("| node | where | arguments | options | description |\n");
    out.push_str("|------|-------|-----------|---------|-------------|\n");
    for n in NODES {
        let mut args: Vec<String> = n.positional.iter().map(|a| format!("`{}` {}..{} (default {})", a.name, a.min, a.max, a.default)).collect();
        if n.waveform { args.push(String::from("waveform word")); }
        if n.rhythm { args.push(String::from("division like `1/8`")); }
        let opts: Vec<String> = n.named.iter().map(|a| format!("`{}=` {}..{} (default {}), {}", a.name, a.min, a.max, a.default, a.doc)).collect();
        let name = if n.aliases.is_empty() { format!("`{}`", n.name) } else { format!("`{}` / `{}`", n.name, n.aliases.join("` / `")) };
        out.push_str(&format!("| {} | {} | {} | {} | {} |\n",
            name,
            if n.in_chains { "chain, graph" } else { "graph" },
            if args.is_empty() { String::from("none") } else { args.join("; ") },
            if opts.is_empty() { String::from("none") } else { opts.join("; ") },
            n.doc));
    }
    out.push('\n');
    out
}

/// JSON reference of every node.
pub fn json() -> String {
    let arg_json = |a: &Arg| format!(
        "{{\"name\": \"{}\", \"min\": {}, \"max\": {}, \"default\": {}, \"doc\": \"{}\"}}",
        a.name, a.min, a.max, a.default, a.doc.replace('"', "\\\"")
    );
    let mut out = String::from("[\n");
    for (i, n) in NODES.iter().enumerate() {
        let pos: Vec<String> = n.positional.iter().map(arg_json).collect();
        let named: Vec<String> = n.named.iter().map(arg_json).collect();
        let aliases: Vec<String> = n.aliases.iter().map(|a| format!("\"{}\"", a)).collect();
        out.push_str(&format!(
            "  {{\"name\": \"{}\", \"aliases\": [{}], \"category\": \"{}\", \"in_chains\": {}, \"waveform\": {}, \"rhythm\": {}, \"positional\": [{}], \"named\": [{}], \"doc\": \"{}\"}}{}\n",
            n.name, aliases.join(", "), n.category.as_str(), n.in_chains, n.waveform, n.rhythm,
            pos.join(", "), named.join(", "), n.doc.replace('"', "\\\""),
            if i + 1 < NODES.len() { "," } else { "" }
        ));
    }
    out.push_str("]\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::ast::Param;

    #[test]
    fn valid_and_invalid_arguments() {
        assert!(validate("compressor", &[Param::Float(-8.0), Param::Named("ratio".into(), 4.0)]).is_empty());
        let e = validate("compressor", &[Param::Float(-8.0), Param::Named("ration".into(), 4.0)]);
        assert!(e[0].contains("unknown option 'ration'"), "{:?}", e);
        let e = validate("saturate", &[Param::Float(0.3), Param::Float(1.0)]);
        assert!(e[0].contains("too many positional"), "{:?}", e);
        let e = validate("limiter", &[Param::Float(1.5)]);
        assert!(e[0].contains("out of range"), "{:?}", e);
        let e = validate("saturat", &[]);
        assert!(e[0].contains("Did you mean 'saturate'"), "{:?}", e);
        assert!(validate("drive", &[Param::Float(0.5)]).is_empty(), "alias");
        assert!(validate("delay", &[Param::RhythmDiv(1, 8), Param::Float(0.4)]).is_empty());
        let e = validate("reverb", &[Param::RhythmDiv(1, 8)]);
        assert!(e[0].contains("rhythm division"), "{:?}", e);
    }
}
