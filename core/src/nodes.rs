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

impl Arg {
    /// Resolve a number written with a unit against this argument. Node
    /// arguments are already in real units, so a suffix is a scale plus a check
    /// that the author meant the unit the argument actually uses.
    ///
    /// `makeup` is why this exists: it is a linear gain, and all fourteen uses
    /// in the corpus wrote it as if it were dB. `makeup=6db` now says so.
    pub fn value_from_quantity(&self, value: f32, suffix: &str) -> Result<f32, String> {
        let doc = self.doc;
        let is_hz = doc.starts_with("Hz");
        let is_ms = doc.starts_with("ms");
        let is_db = doc.starts_with("dB");
        let is_linear_gain = doc.starts_with("linear gain");
        let resolved = match suffix {
            "hz" if is_hz => Some(value),
            "khz" if is_hz => Some(value * 1000.0),
            "ms" if is_ms => Some(value),
            "s" | "sec" if is_ms => Some(value * 1000.0),
            "db" if is_db => Some(value),
            "db" if is_linear_gain => Some(crate::math::pow(10.0, value / 20.0)),
            _ => None,
        };
        match resolved {
            Some(v) => Ok(v),
            None => {
                let accepts = if is_hz {
                    "hz or khz"
                } else if is_ms {
                    "ms or s"
                } else if is_db || is_linear_gain {
                    "db"
                } else {
                    "no unit"
                };
                Err(format!("'{}' takes {} ({}), not '{}'", self.name, accepts, self.doc, suffix))
            }
        }
    }
}

/// Look up one argument of a node by name, or by position among the positionals.
pub fn arg_at(kind: &str, name: Option<&str>, index: usize) -> Option<&'static Arg> {
    let def = lookup(kind)?;
    match name {
        Some(n) => def.named.iter().chain(def.positional.iter()).find(|a| a.name == n),
        None => def.positional.get(index),
    }
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
    // The curve is Q = 0.5 + resonance * 19.5, so this number climbs fast:
    // 0.01 is flat, 0.1 is already a 2.5 dB bump, 0.3 is a clear whistle. The
    // default used to be 0.5, which is a Q of 10 -- a resonant filter for
    // anyone who just wrote `lowpass(2000)`.
    arg!("resonance", 0.0, 1.0, 0.01, "0.01 is flat, 0.1 a bump, 0.3 a whistle, 1 self-oscillation (Q = 0.5 + n*19.5)"),
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
        named: &[arg!("ratio", 1.0, 20.0, 3.0, "n:1"), arg!("attack", 0.0, 500.0, 20.0, "ms"), arg!("release", 1.0, 2000.0, 100.0, "ms"), arg!("makeup", 0.0, 4.0, 1.0, "linear gain after compression, written in dB: makeup=6db, up to +12 dB")],
        waveform: false, rhythm: false, in_chains: true, doc: "Feed-forward compressor." },
    NodeDefSpec { name: "capture", aliases: &[], category: Category::Effect,
        positional: &[arg!("bars", 0.25, 16.0, 2.0, "bars of audio to record")],
        named: &[
            arg!("start", 0.0, 512.0, 0.0, "bar the recording starts on"),
            arg!("speed", 0.05, 4.0, 1.0, "playback rate; 0.5 is half speed and an octave down"),
            arg!("reverse", 0.0, 1.0, 0.0, "1 plays the window backwards"),
            arg!("mix", 0.0, 1.0, 1.0, "wet amount against the live signal"),
        ],
        waveform: false, rhythm: false, in_chains: true,
        doc: "Record a window of this chain and loop it back, stretched or reversed. Passes the signal through until the window is full." },
    NodeDefSpec { name: "limiter", aliases: &[], category: Category::Effect,
        positional: &[arg!("threshold", 0.1, 1.0, 0.95, "output ceiling")], named: &[],
        waveform: false, rhythm: false, in_chains: true, doc: "Lookahead peak limiter, for one track or bus; on the master it is left out, the engine limits every song itself." },
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
    NodeDefSpec { name: "autowah", aliases: &["envfilter"], category: Category::Effect,
        positional: &[arg!("sens", 0.0, 1.0, 0.7, "how far the envelope pushes the sweep")],
        named: &[arg!("base", 20.0, 8000.0, 300.0, "Hz the filter rests at"),
                 arg!("range", 0.0, 12000.0, 2200.0, "Hz the envelope sweeps it through"),
                 arg!("peak", 0.0, 1.0, 0.75, "resonance; near 1 the filter rings on every attack"),
                 arg!("attack", 0.0, 500.0, 8.0, "ms the follower takes to rise"),
                 arg!("release", 1.0, 4000.0, 200.0, "ms it takes to fall"),
                 arg!("down", 0.0, 1.0, 0.0, "1 sweeps downward instead of up"),
                 arg!("wobble", 0.0, 6000.0, 0.0, "Hz the cutoff rings back and forth by, dying with the note"),
                 arg!("wobble_hz", 0.1, 30.0, 5.0, "how fast it rings")],
        waveform: true, rhythm: false, in_chains: true,
        doc: "Envelope filter, the auto-wah: a resonant filter the signal sweeps with its own envelope. `autowah(0.8, base=250hz, range=2500hz, peak=0.8)`. Add `lowpass`, `bandpass` or `highpass` as a word to pick the mode." },
    NodeDefSpec { name: "panenv", aliases: &[], category: Category::Effect,
        positional: &[arg!("depth", 0.0, 1.0, 0.6, "0 = still, 1 = full left/right")],
        named: &[arg!("attack", 0.0, 500.0, 5.0, "ms the follower takes to rise"), arg!("release", 1.0, 4000.0, 300.0, "ms it takes to fall")],
        waveform: false, rhythm: false, in_chains: true,
        doc: "Pans by the signal's own envelope, not an LFO: loud goes one way, the decay walks back. `panenv(0.7, release=400)`." },
    NodeDefSpec { name: "phaser", aliases: &[], category: Category::Effect,
        positional: &[arg!("mix", 0.0, 1.0, 0.5, "wet amount")],
        named: &[arg!("bars", 0.0, 64.0, 0.0, "sweep cycle in bars (tempo-synced)"), arg!("hz", 0.0, 10.0, 0.3, "free sweep rate when bars is 0"), arg!("stages", 2.0, 12.0, 6.0, "allpass stages; more = deeper notches"), arg!("feedback", 0.0, 0.9, 0.4, "resonance of the notches"), arg!("depth", 0.0, 1.0, 1.0, "sweep range")],
        waveform: false, rhythm: false, in_chains: true, doc: "Swept allpass phaser, the liquid pad effect: `phaser(0.5, bars=4)`. Right channel runs a quarter cycle behind." },
    NodeDefSpec { name: "vowel", aliases: &[], category: Category::Effect,
        positional: &[],
        named: &[arg!("bars", 0.0, 64.0, 0.0, "morph cycle in bars (tempo-synced)"), arg!("hz", 0.0, 10.0, 0.25, "free morph rate when bars is 0"), arg!("mix", 0.0, 1.0, 1.0, "wet amount")],
        waveform: true, rhythm: false, in_chains: true, doc: "Formant filter: `vowel(a)` holds a vowel, `vowel(a, o, bars=2)` morphs between two. Words: a e i o u." },
    NodeDefSpec { name: "reverb", aliases: &[], category: Category::Effect,
        positional: &[arg!("size", 0.0, 1.0, 0.5, "room size / decay")], named: &[],
        waveform: false, rhythm: false, in_chains: true, doc: "Plate reverb, wet only; use it on a bus." },
];

/// The universal chain-node option: dry/wet, where 0 bypasses.
pub const WET: &str = "wet";

pub fn lookup(kind: &str) -> Option<&'static NodeDefSpec> {
    NODES.iter().find(|n| n.name == kind || n.aliases.contains(&kind))
}

pub fn suggest(kind: &str) -> Option<&'static str> {
    let mut best: Option<(usize, &'static str)> = None;
    for n in NODES {
        let d = crate::params::levenshtein(kind, n.name);
        if best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, n.name));
        }
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
        if let Some(s) = suggest(kind) {
            msg.push_str(&format!(". Did you mean '{}'?", s));
        }
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
                            errors.push(format!(
                                "{}: {} = {} is out of range ({}..{})",
                                spec.name, arg.name, v, arg.min, arg.max
                            ));
                        }
                    }
                    None => errors.push(format!(
                        "{}: too many positional arguments ({} takes {})",
                        spec.name,
                        spec.name,
                        spec.positional.len()
                    )),
                }
                positional_idx += 1;
            }
            // `wet` is universal on anything that can sit in a chain: 0 bypasses
            // the node and skips its processing, 1 is the node alone, and the
            // values between blend it against the dry signal. It is the lever a
            // live set uses to switch an effect in and out without a swap, so it
            // has to exist on every node rather than the few that declare a
            // `mix` of their own -- and it is a different thing from those, which
            // are the node's internal wet amount.
            Param::Named(name, v) if name == WET && spec.in_chains => {
                if *v < 0.0 || *v > 1.0 {
                    errors.push(format!("{}: wet = {} is out of range (0..1)", spec.name, v));
                }
            }
            Param::Named(name, v) => match spec.named.iter().find(|a| a.name == name) {
                Some(arg) => {
                    if *v < arg.min || *v > arg.max {
                        errors.push(format!(
                            "{}: {} = {} is out of range ({}..{})",
                            spec.name, arg.name, v, arg.min, arg.max
                        ));
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
                    errors
                        .push(format!("{}: unexpected word '{}' (arguments are numbers or name=value)", spec.name, w));
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
    out.push('\n');
    out.push_str("Two options apply to every node that can sit in a chain, on top of the ones\n");
    out.push_str("listed below:\n\n");
    out.push_str("- `wet=` 0..1 (default 1) — dry/wet. At 0 the node is **bypassed**: it is not\n");
    out.push_str("  processed at all, so a chain of effects that are switched off costs nothing.\n");
    out.push_str("  At 1 it replaces the signal, and in between it is blended against the dry.\n");
    out.push_str("  Changing only `wet=` applies inside the bar with no hot-swap, so it is how a\n");
    out.push_str("  live set switches an effect in and out without restarting the voices.\n");
    out.push_str("  It is a different thing from the `mix` that `phaser` and `vowel` declare,\n");
    out.push_str("  which is those nodes' own internal wet amount.\n");
    out.push_str("- `as <name>` — name the node, after the closing parenthesis:\n");
    out.push_str("  `> autowah(0.9, base=450hz) as wah`. A name is what `auto <track>.<name> wet`\n");
    out.push_str("  points at, and it survives someone inserting another node earlier in the\n");
    out.push_str("  chain, which a position does not. Two nodes in one chain cannot share a name.\n\n");
    out.push_str("| node | where | arguments | options | description |\n");
    out.push_str("|------|-------|-----------|---------|-------------|\n");
    for n in NODES {
        let mut args: Vec<String> =
            n.positional.iter().map(|a| format!("`{}` {}..{} (default {})", a.name, a.min, a.max, a.default)).collect();
        if n.waveform {
            args.push(String::from("waveform word"));
        }
        if n.rhythm {
            args.push(String::from("division like `1/8`"));
        }
        let opts: Vec<String> = n
            .named
            .iter()
            .map(|a| format!("`{}=` {}..{} (default {}), {}", a.name, a.min, a.max, a.default, a.doc))
            .collect();
        let name = if n.aliases.is_empty() {
            format!("`{}`", n.name)
        } else {
            format!("`{}` / `{}`", n.name, n.aliases.join("` / `"))
        };
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            name,
            if n.in_chains { "chain, graph" } else { "graph" },
            if args.is_empty() { String::from("none") } else { args.join("; ") },
            if opts.is_empty() { String::from("none") } else { opts.join("; ") },
            n.doc
        ));
    }
    out.push('\n');
    out
}

/// JSON reference of every node.
pub fn json() -> String {
    let arg_json = |a: &Arg| {
        format!(
            "{{\"name\": \"{}\", \"min\": {}, \"max\": {}, \"default\": {}, \"doc\": \"{}\"}}",
            a.name,
            a.min,
            a.max,
            a.default,
            a.doc.replace('"', "\\\"")
        )
    };
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
