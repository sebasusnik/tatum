//! A sound as the designer edits it: one `instrument` graph or one `module`,
//! read from its file, turned by the page, and written back in the units
//! the file uses.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use tatum_core::dsl::ast::{InstrumentDef, ModuleDef, OpEnvelopeDef, Param, Song};
use tatum_core::nodes;
use tatum_core::params::{self, ModuleKind, ParamSpec, Range};

/// A node of a graph as the page edits it: its arguments by name.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub kind: String,
    pub alias: String,
    /// The waveform, vowel or mode words, in the order written.
    pub words: Vec<String>,
    /// A delay's `1/8`.
    pub rhythm: Option<(u8, u8)>,
    /// Every argument written in the file, positional ones by their name.
    pub args: BTreeMap<String, f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Graph {
    pub gain: Option<f32>,
    pub nodes: Vec<Node>,
    pub connections: Vec<(String, String)>,
}

/// How a module parameter is written in its file, so it is written back the
/// same way: `cutoff 800hz` stays in Hz, `kick_level 1.0` stays a number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// A plain number: the knob position.
    Bare,
    /// In the parameter's unit: `800hz`, `20ms`, `-12st`, `1.5x`.
    Unit,
    Percent,
    Db,
    /// A choice by its name: `waveform half_sine`.
    Name,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModuleParam {
    pub name: String,
    /// The knob position, as the module takes it.
    pub value: f32,
    pub style: Style,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Module {
    pub kind: ModuleKind,
    /// In the order written; a parameter set from the page goes at the end.
    pub params: Vec<ModuleParam>,
    /// `opN_envelope a d s r`, in seconds, fm only: they override the
    /// operator's own `opN_attack` .. `opN_release`.
    pub envelopes: Vec<OpEnvelopeDef>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    Graph(Graph),
    Module(Module),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sound {
    pub description: String,
    pub name: String,
    pub body: Body,
}

/// An operator envelope's stages, as the page names them.
pub const ENV_STAGES: [&str; 4] = ["a", "d", "s", "r"];
/// The span an operator envelope's times are turned over, in seconds.
pub const ENV_TIME: (f32, f32) = (0.0005, 8.0);
/// What an operator envelope starts as when one of its knobs is first
/// turned: the operator's own defaults, in seconds.
pub const ENV_DEFAULT: [f32; 4] = [0.01, 0.1, 0.7, 0.3];

impl Sound {
    /// The sound in `text`: its comment and its one instrument or module,
    /// and nothing else. `text()` writes those two back and no more, so a
    /// file with anything besides -- a song's tempo, patterns and tracks, a
    /// comment inside the definition or after it -- is refused here rather
    /// than opened and then saved without them.
    pub fn parse(text: &str) -> Result<Sound, String> {
        let description = text
            .lines()
            .take_while(|l| l.trim_start().starts_with('#') || l.trim().is_empty())
            .filter_map(|l| l.trim_start().strip_prefix('#'))
            .map(|l| l.trim())
            .collect::<Vec<_>>()
            .join(" ");
        let song = tatum_core::dsl::parse(text).map_err(|errs| {
            errs.iter().map(|e| format!("line {}: {}", e.line, e.message)).collect::<Vec<_>>().join("\n")
        })?;
        if let ([_], []) | ([], [_]) = (song.instruments.as_slice(), song.module_defs.as_slice()) {
            if let Some(more) = besides(&song) {
                return Err(format!(
                    "there is more in it than a sound ({more}): the designer opens a file that is one `instrument` or one `module` and nothing else, and Save would not keep the rest"
                ));
            }
            if let Some(line) = comment_inside(text) {
                return Err(format!(
                    "line {line}: a comment after the ones on top; the designer keeps the comment above the sound and no other, and Save would drop this one. Move it to the top"
                ));
            }
        }
        match (song.instruments.as_slice(), song.module_defs.as_slice()) {
            ([inst], []) => Ok(Sound { description, name: inst.name.clone(), body: Body::Graph(parse_graph(inst)?) }),
            ([], [m]) => Ok(Sound { description, name: m.name.clone(), body: Body::Module(parse_module(m, text)?) }),
            ([], []) => Err("there is no `instrument` or `module` in it; a sound is one of them".into()),
            _ => Err("there is more than one `instrument` or `module` in it; a sound is one of them".into()),
        }
    }

    /// The sound as a file: its comment, then the instrument or module, one
    /// line a node, connection or parameter, every value in its unit.
    pub fn text(&self) -> String {
        let mut o = String::new();
        for line in wrap(&self.description, 76) {
            let _ = writeln!(o, "# {line}");
        }
        match &self.body {
            Body::Graph(g) => {
                let _ = writeln!(o, "instrument {} {{", self.name);
                if let Some(v) = g.gain {
                    let _ = writeln!(o, "  gain {}", number(v));
                }
                for n in &g.nodes {
                    let _ = writeln!(o, "  {}", node_text(n));
                }
                for (a, b) in &g.connections {
                    let _ = writeln!(o, "  {a} > {b}");
                }
            }
            Body::Module(m) => {
                let _ = writeln!(o, "module {} {} {{", m.kind.as_str(), self.name);
                for p in &m.params {
                    let _ = writeln!(o, "  {} {}", p.name, module_value(m.kind, p));
                }
                for e in &m.envelopes {
                    let _ = writeln!(
                        o,
                        "  op{}_envelope {} {} {} {}",
                        e.op_index,
                        number(e.a),
                        number(e.d),
                        number(e.s),
                        number(e.r)
                    );
                }
            }
        }
        o.push_str("}\n");
        o
    }

    /// What `play ... using` names it by, and what the page shows it as:
    /// `graph`, or the module's kind.
    pub fn kind(&self) -> &'static str {
        match &self.body {
            Body::Graph(_) => "graph",
            Body::Module(m) => m.kind.as_str(),
        }
    }

    /// Whether two reads of a sound are the same sound: equal, with every
    /// number allowed the last digit a unit written in text cannot carry.
    /// How a value is written is not what it is: a gain at 0 is `0`, never
    /// `-infdb`, whatever the others are written in.
    pub fn same_as(&self, other: &Sound) -> bool {
        let close = |a: f32, b: f32| (a - b).abs() <= 1e-5 * a.abs().max(b.abs()).max(1.0);
        if self.description != other.description || self.name != other.name {
            return false;
        }
        match (&self.body, &other.body) {
            (Body::Graph(a), Body::Graph(b)) => {
                a.gain.zip(b.gain).map_or(a.gain.is_none() && b.gain.is_none(), |(x, y)| close(x, y))
                    && a.connections == b.connections
                    && a.nodes.len() == b.nodes.len()
                    && a.nodes.iter().zip(&b.nodes).all(|(x, y)| {
                        x.kind == y.kind
                            && x.alias == y.alias
                            && x.words == y.words
                            && x.rhythm == y.rhythm
                            && x.args.len() == y.args.len()
                            && x.args.iter().zip(&y.args).all(|((k1, v1), (k2, v2))| k1 == k2 && close(*v1, *v2))
                    })
            }
            (Body::Module(a), Body::Module(b)) => {
                a.kind == b.kind
                    && a.envelopes == b.envelopes
                    && a.params.len() == b.params.len()
                    && a.params.iter().zip(&b.params).all(|(x, y)| x.name == y.name && close(x.value, y.value))
            }
            _ => false,
        }
    }
}

/// What a file with one sound in it has besides the sound, when it has
/// anything: the first few things, by name.
fn besides(song: &Song) -> Option<String> {
    let mut rest = song.clone();
    rest.instruments.clear();
    rest.module_defs.clear();
    // What a file with nothing in it reads as: the settings nobody wrote.
    let empty = tatum_core::dsl::parse("").ok()?;
    if rest == empty {
        return None;
    }
    let mut found: Vec<&str> = Vec::new();
    for (there, what) in [
        (rest.globals != empty.globals, "settings such as `tempo`"),
        (!rest.patterns.is_empty(), "patterns"),
        (!rest.tracks.is_empty(), "tracks"),
        (!rest.scenes.is_empty() || !rest.arrangement.is_empty(), "scenes"),
        (!rest.buses.is_empty() || !rest.bus_chains.is_empty() || rest.master.is_some(), "buses or a master"),
    ] {
        if there {
            found.push(what);
        }
    }
    Some(if found.is_empty() { "other blocks of a song".into() } else { found.join(", ") })
}

/// The line of the first comment after the ones on top of the file, when
/// there is one. A `#` right after a letter is a sharp (`F#1`), as the
/// reader takes it; any other starts a comment.
fn comment_inside(text: &str) -> Option<usize> {
    let top = text.lines().take_while(|l| l.trim_start().starts_with('#') || l.trim().is_empty()).count();
    text.lines().enumerate().skip(top).find_map(|(i, l)| {
        let b = l.as_bytes();
        (0..b.len()).any(|k| b[k] == b'#' && (k == 0 || !b[k - 1].is_ascii_alphabetic())).then_some(i + 1)
    })
}

fn parse_graph(inst: &InstrumentDef) -> Result<Graph, String> {
    // Nodes with no name get one the page can show and the text can keep:
    // their kind and their place, or the next number no node has, so it is
    // never a name the file gave another node.
    let written: Vec<&str> = inst.nodes.iter().filter_map(|n| n.alias.as_deref()).collect();
    let mut given: Vec<String> = Vec::new();
    let mut fresh = |kind: &str, i: usize| {
        let name = (i..)
            .map(|k| format!("{}{k}", kind.replace('_', "")))
            .find(|a| !written.contains(&a.as_str()) && !given.contains(a))
            .unwrap_or_default();
        given.push(name.clone());
        name
    };
    let mut renames: Vec<(String, String)> = Vec::new();
    let mut nodes_out = Vec::new();
    for (i, n) in inst.nodes.iter().enumerate() {
        let alias = match n.alias.as_deref() {
            Some(a) if !a.starts_with("_anon_") => a.to_string(),
            Some(a) => {
                let fresh = fresh(&n.kind, i);
                renames.push((a.to_string(), fresh.clone()));
                fresh
            }
            None => fresh(&n.kind, i),
        };
        let def = nodes::lookup(&n.kind).ok_or_else(|| format!("unknown node '{}'", n.kind))?;
        let mut args = BTreeMap::new();
        let mut words = Vec::new();
        let mut rhythm = None;
        let mut positional = 0;
        for p in &n.params {
            match p {
                Param::Float(v) => {
                    if let Some(a) = def.positional.get(positional) {
                        args.insert(a.name.to_string(), *v);
                    }
                    positional += 1;
                }
                Param::Expr(e) => {
                    if let Some(a) = def.positional.get(positional) {
                        args.insert(a.name.to_string(), e.eval());
                    }
                    positional += 1;
                }
                Param::Named(k, v) => {
                    args.insert(k.clone(), *v);
                }
                Param::Waveform(w) => words.push(w.clone()),
                Param::RhythmDiv(a, b) => rhythm = Some((*a, *b)),
            }
        }
        nodes_out.push(Node { kind: n.kind.clone(), alias, words, rhythm, args });
    }
    let rename = |s: &str| renames.iter().find(|r| r.0 == s).map_or(s.to_string(), |r| r.1.clone());
    let connections = inst.connections.iter().map(|c| (rename(&c.from), rename(&c.to))).collect();
    Ok(Graph { gain: inst.gain, nodes: nodes_out, connections })
}

fn parse_module(m: &ModuleDef, text: &str) -> Result<Module, String> {
    let kind = ModuleKind::from_str(&m.module_type)
        .ok_or_else(|| format!("unknown module type '{}' (bass, fm, keys or beats)", m.module_type))?;
    let lines: Vec<&str> = text.lines().collect();
    let mut params_out: Vec<ModuleParam> = Vec::new();
    for p in &m.params {
        let spec = params::lookup(kind, &p.name)
            .ok_or_else(|| format!("line {}: {} has no parameter '{}'", p.line, kind.as_str(), p.name))?;
        let style = if p.bare {
            Style::Bare
        } else {
            written_style(lines.get(p.line.wrapping_sub(1)).copied().unwrap_or(""), &p.name, spec)
        };
        let param = ModuleParam { name: p.name.clone(), value: p.value, style };
        // Written twice, the last one is the one that plays.
        match params_out.iter_mut().find(|q| q.name == p.name) {
            Some(q) => *q = param,
            None => params_out.push(param),
        }
    }
    let mut envelopes: Vec<OpEnvelopeDef> = Vec::new();
    for e in &m.op_envelopes {
        envelopes.retain(|x| x.op_index != e.op_index);
        envelopes.push(e.clone());
    }
    Ok(Module { kind, params: params_out, envelopes })
}

/// How `name` is written on its line: the word after it, by its ending.
fn written_style(line: &str, name: &str, spec: &ParamSpec) -> Style {
    let mut words = line.split(|c: char| c.is_whitespace() || c == '{' || c == '}');
    while let Some(w) = words.next() {
        if w != name {
            continue;
        }
        let Some(v) = words.find(|w| !w.is_empty()) else { break };
        let v = v.to_ascii_lowercase();
        return if v.ends_with('%') {
            Style::Percent
        } else if v.ends_with("db") {
            Style::Db
        } else if v.starts_with(|c: char| c.is_ascii_alphabetic()) {
            Style::Name
        } else if spec.curve.unit().is_some() {
            Style::Unit
        } else {
            Style::Bare
        };
    }
    default_style(spec)
}

/// How a parameter the page sets for the first time is written: in its unit
/// when it has one, by name when it is a choice, in dB when it is a gain.
pub fn default_style(spec: &ParamSpec) -> Style {
    if matches!(spec.range, Range::Choice(_)) {
        Style::Name
    } else if spec.curve.unit().is_some() {
        Style::Unit
    } else if matches!(spec.range, Range::Gain { .. }) {
        Style::Db
    } else {
        Style::Bare
    }
}

/// The shortest of `candidates(decimals)` that reads back as `target`, or the
/// nearest when none does exactly.
fn shortest(target: f32, real: f64, back: impl Fn(f32) -> Option<f32>) -> String {
    let mut best = (f32::MAX, number(real as f32));
    for decimals in 0..=7 {
        let mut text = format!("{real:.decimals$}");
        if text.contains('.') {
            text = text.trim_end_matches('0').trim_end_matches('.').to_string();
        }
        if text == "-0" {
            text = "0".into();
        }
        let miss = text.parse::<f32>().ok().and_then(&back).map_or(f32::MAX, |k| (k - target).abs());
        if miss < best.0 {
            best = (miss, text);
        }
        if best.0 == 0.0 {
            break;
        }
    }
    best.1
}

/// A module parameter's value as its line writes it.
pub fn module_value(kind: ModuleKind, p: &ModuleParam) -> String {
    let Some(spec) = params::lookup(kind, &p.name) else { return number(p.value) };
    let v = p.value;
    let style = match (p.style, spec.range) {
        // A choice written by name has to land on one of the names.
        (Style::Name, Range::Choice(_)) => Style::Name,
        (Style::Name, _) => default_style(spec),
        (Style::Unit, _) if spec.curve.unit().is_none() => Style::Bare,
        (Style::Db, Range::Gain { .. }) if v <= 0.0 => Style::Bare,
        (Style::Db, r) if !matches!(r, Range::Gain { .. }) => Style::Bare,
        (Style::Percent, Range::Choice(_)) => Style::Bare,
        (s, _) => s,
    };
    match style {
        Style::Name => match spec.choice_name(v) {
            Some(n) if spec.value_from_name(n) == Some(v) => n.to_string(),
            _ => number(v),
        },
        Style::Unit => spec.write_in_units(v).unwrap_or_else(|| number(v)),
        Style::Percent => {
            format!("{}%", shortest(v, v as f64 * 100.0, |x| spec.value_from_quantity(x, "%").ok()))
        }
        Style::Db => {
            let db = 20.0 * (v as f64).log10();
            format!("{}db", shortest(v, db, |x| spec.value_from_quantity(x, "db").ok()))
        }
        Style::Bare => number(v),
    }
}

pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.len() + 1 + word.len() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// A number as short as it can be written and still be the same knob: the
/// shortest text that reads back as the same f32 (173.4, not 173.39999),
/// held to five decimals.
pub fn number(v: f32) -> String {
    let short = format!("{v}");
    let s = match short.split_once('.') {
        Some((_, frac)) if frac.len() > 5 => {
            let s = format!("{v:.5}");
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        }
        _ => short,
    };
    if s == "-0" {
        "0".into()
    } else {
        s
    }
}

/// The note `hz` is, when it is one to within a hundredth of a cent: what
/// `kick(F#1)` was written as comes back as F#1, not 46.24931hz.
pub fn note_name(hz: f32) -> Option<String> {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    let midi = (69.0 + 12.0 * (hz / 440.0).log2()).round();
    if !(0.0..=127.0).contains(&midi) {
        return None;
    }
    let exact = tatum_core::math::midi_to_freq(midi as u8);
    ((hz / exact).log2().abs() * 1200.0 < 0.01)
        .then(|| format!("{}{}", NAMES[midi as usize % 12], midi as i32 / 12 - 1))
}

/// The unit a node argument is written in: `hz`, `ms`, `db`, or none.
pub fn arg_unit(arg: &nodes::Arg) -> &'static str {
    if arg.doc.starts_with("Hz") {
        "hz"
    } else if arg.doc.starts_with("ms") {
        "ms"
    } else if arg.doc.starts_with("dB") {
        "db"
    } else {
        ""
    }
}

/// A value in the unit its argument is written in.
fn with_unit(arg: &nodes::Arg, v: f32) -> String {
    match arg_unit(arg) {
        "hz" => note_name(v).unwrap_or_else(|| format!("{}hz", number(v))),
        "ms" => format!("{}ms", number(v)),
        "db" => format!("{}db", number(v)),
        _ => number(v),
    }
}

/// The nodes whose waveform word goes before the parentheses.
fn word_first(kind: &str) -> bool {
    matches!(kind, "osc" | "fixosc" | "pitch_osc")
}

/// The words a node takes, for the page to offer: empty when it takes none.
pub fn word_choices(kind: &str) -> &'static [&'static str] {
    match nodes::lookup(kind).map(|d| d.name) {
        Some("osc" | "fixosc" | "pitch_osc") => &["sine", "saw", "square", "triangle", "pulse"],
        Some("vowel") => &["a", "e", "i", "o", "u"],
        Some("autowah") => &["lowpass", "bandpass", "highpass"],
        _ => &[],
    }
}

/// The words a node added from the page starts with: the ones it cannot do
/// without.
pub fn default_words(kind: &str) -> Vec<String> {
    match nodes::lookup(kind).map(|d| d.name) {
        Some("osc" | "fixosc" | "pitch_osc") => vec!["sine".into()],
        Some("vowel") => vec!["a".into()],
        _ => Vec::new(),
    }
}

/// The node kinds a graph line can be written with: every one in the table
/// that reads back as a node on a line of its own (`mix` is a name, and some
/// effects only sit in chains).
pub fn graph_kinds() -> Vec<&'static nodes::NodeDefSpec> {
    nodes::NODES
        .iter()
        .filter(|d| {
            let n = Node {
                kind: d.name.into(),
                alias: "probe".into(),
                words: default_words(d.name),
                rhythm: None,
                args: BTreeMap::new(),
            };
            let text = format!("instrument t {{\n  {}\n  probe > out\n}}\n", node_text(&n));
            tatum_core::dsl::parse(&text).is_ok_and(|s| s.instruments.first().is_some_and(|i| i.nodes.len() == 1))
        })
        .collect()
}

pub fn node_text(n: &Node) -> String {
    let def = nodes::lookup(&n.kind);
    let mut parts: Vec<String> = Vec::new();
    let first = word_first(&n.kind);
    if let Some((a, b)) = n.rhythm {
        parts.push(format!("{a}/{b}"));
    }
    if !first {
        parts.extend(n.words.iter().cloned());
    }
    let mut written: Vec<&str> = Vec::new();
    if let Some(def) = def {
        // Positionals in order, up to the last one written; one left out
        // before it takes its default.
        let last = def.positional.iter().rposition(|a| n.args.contains_key(a.name));
        if let Some(last) = last {
            for a in &def.positional[..=last] {
                parts.push(with_unit(a, n.args.get(a.name).copied().unwrap_or(a.default)));
                written.push(a.name);
            }
        }
        for a in def.named {
            // A name that is also positional was written as one, above.
            if def.positional.iter().any(|p| p.name == a.name) {
                written.push(a.name);
                continue;
            }
            if let Some(&v) = n.args.get(a.name) {
                parts.push(format!("{}={}", a.name, with_unit(a, v)));
            }
            written.push(a.name);
        }
    }
    // Anything else the file said, `wet=` among them, as it said it.
    for (k, v) in &n.args {
        if !written.contains(&k.as_str()) {
            parts.push(format!("{k}={}", number(*v)));
        }
    }
    let word = if first { n.words.iter().map(|w| format!(" {w}")).collect::<String>() } else { String::new() };
    format!("{}{}({}) as {}", n.kind, word, parts.join(", "), n.alias)
}

// ── What the page asks for ──

/// A name the file can hold: a letter or `_` first, then letters, digits
/// and `_`, lower case.
pub fn is_ident(s: &str) -> bool {
    let mut c = s.chars();
    c.next().is_some_and(|f| f.is_ascii_lowercase() || f == '_')
        && c.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
        && s.len() <= 40
}

/// A number as a knob takes it: `NaN` and `inf` parse as floats, and would
/// be written into the file as `NaNms`.
pub fn finite(v: &str) -> Option<f32> {
    v.parse::<f32>().ok().filter(|v| v.is_finite())
}

/// Apply the page's changes, one a line:
/// - `description=...`, `name=...`, and on a graph `gain=` (empty unsets);
/// - on a graph: `alias.arg=value` sets a knob, `-alias.arg` unsets it,
///   `word alias=saw` sets a node's words, `+node kind alias` adds a node,
///   `-node alias` removes one and its connections, `+wire a b` and
///   `-wire a b` connect and disconnect `a > b`;
/// - on a module: `param=value` sets a knob (a number, or a choice by name),
///   `-param` unsets it, `envN.a=seconds` (a, d, s, r) sets an fm
///   operator's envelope and `-envN` removes it.
///
/// Values past a range are held to it. Anything else is an error naming the
/// line, and a change that would not read back as the same sound is refused.
pub fn apply(sound: &mut Sound, body: &str) -> Result<(), String> {
    for line in body.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(rest) = line.strip_prefix("description=") {
            sound.description = rest.replace(['\r', '\n'], " ");
            continue;
        }
        if let Some(rest) = line.strip_prefix("name=") {
            let rest = rest.trim();
            if !is_ident(rest) {
                return Err(format!("'{rest}' is not a name: a lower-case letter, then letters, digits and _"));
            }
            sound.name = rest.to_string();
            continue;
        }
        match &mut sound.body {
            Body::Graph(g) => apply_graph(g, line)?,
            Body::Module(m) => apply_module(m, line)?,
        }
    }
    // A name that is a word of the language, or anything else that would
    // not read back, is caught here rather than written into the file.
    let again = Sound::parse(&sound.text()).map_err(|e| format!("that would not read back: {e}"))?;
    if !again.same_as(sound) {
        return Err("that would not read back as the same sound".into());
    }
    Ok(())
}

fn apply_graph(g: &mut Graph, line: &str) -> Result<(), String> {
    if let Some(rest) = line.strip_prefix("gain=") {
        g.gain = match rest.trim() {
            "" => None,
            v => Some(finite(v).ok_or_else(|| format!("'{line}': '{v}' is not a number"))?.clamp(0.0, 10.0)),
        };
        return Ok(());
    }
    if let Some(rest) = line.strip_prefix("+node ") {
        let mut w = rest.split_whitespace();
        let (Some(kind), alias, None) = (w.next(), w.next(), w.next()) else {
            return Err(format!("'{line}': expected +node kind [alias]"));
        };
        let def = nodes::lookup(kind).ok_or_else(|| {
            let hint = nodes::suggest(kind).map(|s| format!(" Did you mean '{s}'?")).unwrap_or_default();
            format!("'{line}': there is no node '{kind}'.{hint}")
        })?;
        if !graph_kinds().iter().any(|k| k.name == def.name) {
            return Err(format!("'{kind}' cannot be written as a node of an instrument; it sits in a track's chain"));
        }
        let alias = match alias {
            Some(a) => a.to_string(),
            None => {
                (1..).map(|i| format!("{}{i}", def.name.replace('_', ""))).find(|a| !taken(g, a)).unwrap_or_default()
            }
        };
        if !is_ident(&alias) || matches!(alias.as_str(), "out" | "mix" | "in") || !wires_by(&alias) {
            return Err(format!(
                "'{alias}' cannot name a node: a lower-case name that is not a word of the language (out, mix, o, x, tempo...)"
            ));
        }
        if taken(g, &alias) {
            return Err(format!("there is already a node named '{alias}'"));
        }
        g.nodes.push(Node {
            kind: def.name.to_string(),
            alias,
            words: default_words(def.name),
            rhythm: None,
            args: BTreeMap::new(),
        });
        return Ok(());
    }
    if let Some(alias) = line.strip_prefix("-node ") {
        let alias = alias.trim();
        let before = g.nodes.len();
        g.nodes.retain(|n| n.alias != alias);
        if g.nodes.len() == before {
            return Err(format!("'{line}': no node named '{alias}'"));
        }
        g.connections.retain(|(a, b)| a != alias && b != alias);
        return Ok(());
    }
    for (prefix, add) in [("+wire ", true), ("-wire ", false)] {
        let Some(rest) = line.strip_prefix(prefix) else { continue };
        let mut w = rest.split_whitespace();
        let (Some(a), Some(b), None) = (w.next(), w.next(), w.next()) else {
            return Err(format!("'{line}': expected {}wire from to", &prefix[..1]));
        };
        let pair = (a.to_string(), b.to_string());
        if add {
            if !(a == "mix" || taken(g, a)) {
                return Err(format!("'{line}': no node named '{a}' to connect from"));
            }
            if !(b == "out" || b == "mix" || taken(g, b)) {
                return Err(format!("'{line}': no node named '{b}' to connect to"));
            }
            if a == b {
                return Err(format!("'{line}': a node cannot take its own output"));
            }
            if g.connections.contains(&pair) {
                return Err(format!("'{a} > {b}' is already there"));
            }
            g.connections.push(pair);
        } else {
            let before = g.connections.len();
            g.connections.retain(|c| *c != pair);
            if g.connections.len() == before {
                return Err(format!("there is no '{a} > {b}' to take out"));
            }
        }
        return Ok(());
    }
    if let Some(rest) = line.strip_prefix("word ") {
        let (alias, words) = rest.split_once('=').ok_or_else(|| format!("'{line}': expected word alias=w"))?;
        let node = g
            .nodes
            .iter_mut()
            .find(|n| n.alias == alias.trim())
            .ok_or_else(|| format!("'{line}': no node named '{}'", alias.trim()))?;
        let choices = word_choices(&node.kind);
        let words: Vec<String> = words.split_whitespace().map(String::from).collect();
        if words.is_empty() && !default_words(&node.kind).is_empty() {
            return Err(format!("{} needs a word: {}", node.kind, choices.join(" | ")));
        }
        let most = if node.kind == "vowel" { 2 } else { 1 };
        if words.len() > most {
            return Err(format!("{} takes at most {most} word(s)", node.kind));
        }
        if let Some(w) = words.iter().find(|w| !choices.contains(&w.as_str())) {
            return Err(format!("{} has no word '{w}' ({})", node.kind, choices.join(" | ")));
        }
        node.words = words;
        return Ok(());
    }
    let (key, value) = match line.strip_prefix('-') {
        Some(k) => (k, None),
        None => {
            let (k, v) = line.split_once('=').ok_or_else(|| format!("'{line}': expected alias.arg=value"))?;
            let v = finite(v.trim()).ok_or_else(|| format!("'{line}': '{v}' is not a number"))?;
            (k, Some(v))
        }
    };
    let (alias, arg) = key.split_once('.').ok_or_else(|| format!("'{line}': expected alias.arg"))?;
    let node =
        g.nodes.iter_mut().find(|n| n.alias == alias).ok_or_else(|| format!("'{line}': no node named '{alias}'"))?;
    let def = nodes::lookup(&node.kind).ok_or_else(|| format!("unknown node '{}'", node.kind))?;
    let (name, min, max) = match def.positional.iter().chain(def.named.iter()).find(|a| a.name == arg) {
        Some(a) => (a.name, a.min, a.max),
        None if arg == nodes::WET && def.in_chains => (nodes::WET, 0.0, 1.0),
        None => return Err(format!("'{line}': {} has no argument '{arg}'", node.kind)),
    };
    match value {
        Some(v) => {
            node.args.insert(name.to_string(), v.clamp(min, max));
        }
        None => {
            node.args.remove(name);
        }
    }
    Ok(())
}

/// Whether `alias` reads back as a node's name at both ends of a wire: `o`
/// and `x` are drum hits to the reader, `tempo` a word of its own.
fn wires_by(alias: &str) -> bool {
    let text = format!("instrument t {{\n  noise() as {alias}\n  {alias} > out\n}}\n");
    tatum_core::dsl::parse(&text).is_ok_and(|s| {
        s.instruments.first().is_some_and(|i| {
            i.nodes.len() == 1
                && i.nodes[0].alias.as_deref() == Some(alias)
                && i.connections.len() == 1
                && i.connections[0].from == alias
        })
    })
}

fn taken(g: &Graph, alias: &str) -> bool {
    g.nodes.iter().any(|n| n.alias == alias)
}

fn apply_module(m: &mut Module, line: &str) -> Result<(), String> {
    if let Some(rest) = line.strip_prefix("-env") {
        let op: usize = rest.trim().parse().map_err(|_| format!("'{line}': expected -envN"))?;
        m.envelopes.retain(|e| e.op_index != op);
        return Ok(());
    }
    if let Some(rest) = line.strip_prefix("env") {
        if m.kind != ModuleKind::Fm {
            return Err(format!("'{line}': only an fm module has operator envelopes"));
        }
        let (key, v) = rest.split_once('=').ok_or_else(|| format!("'{line}': expected envN.a=seconds"))?;
        let (op, stage) = key.split_once('.').ok_or_else(|| format!("'{line}': expected envN.a=seconds"))?;
        let op: usize = op.parse().ok().filter(|o| *o < 4).ok_or_else(|| format!("'{line}': operators are 0..3"))?;
        let stage =
            ENV_STAGES.iter().position(|s| *s == stage).ok_or_else(|| format!("'{line}': a stage is a, d, s or r"))?;
        let v = finite(v.trim()).ok_or_else(|| format!("'{line}': '{}' is not a number", v.trim()))?;
        let v = if stage == 2 { v.clamp(0.0, 1.0) } else { v.clamp(ENV_TIME.0, ENV_TIME.1) };
        let i = match m.envelopes.iter().position(|e| e.op_index == op) {
            Some(i) => i,
            None => {
                let [a, d, s, r] = ENV_DEFAULT;
                m.envelopes.push(OpEnvelopeDef { op_index: op, a, d, s, r });
                m.envelopes.sort_by_key(|e| e.op_index);
                m.envelopes.iter().position(|e| e.op_index == op).unwrap_or(0)
            }
        };
        let e = &mut m.envelopes[i];
        *[&mut e.a, &mut e.d, &mut e.s, &mut e.r][stage] = v;
        return Ok(());
    }
    let (name, value) = match line.strip_prefix('-') {
        Some(k) => (k.trim(), None),
        None => {
            let (k, v) = line.split_once('=').ok_or_else(|| format!("'{line}': expected param=value"))?;
            (k.trim(), Some(v.trim()))
        }
    };
    let spec = params::lookup(m.kind, name).ok_or_else(|| {
        let hint = params::suggest(m.kind, name).map(|s| format!(" Did you mean '{s}'?")).unwrap_or_default();
        format!("'{line}': {} has no parameter '{name}'.{hint}", m.kind.as_str())
    })?;
    let Some(value) = value else {
        m.params.retain(|p| p.name != name);
        return Ok(());
    };
    let (v, style) = match (spec.range, finite(value)) {
        (Range::Choice(names), _) if names.contains(&value) => {
            (spec.value_from_name(value).unwrap_or(0.0), Style::Name)
        }
        (Range::Choice(names), Some(v)) => {
            // A number on a choice lands on the nearest choice.
            let i = (v.clamp(0.0, 1.0) * (names.len() - 1) as f32).round() as usize;
            (params::choice_value(i, names.len()), Style::Name)
        }
        (Range::Choice(names), None) => {
            return Err(format!("{name} has no option '{value}' ({})", names.join(" | ")));
        }
        (r, Some(v)) => (v.clamp(r.min(), r.max()), default_style(spec)),
        (_, None) => return Err(format!("'{line}': '{value}' is not a number")),
    };
    match m.params.iter_mut().find(|p| p.name == name) {
        // It keeps the way the file writes it.
        Some(p) => p.value = v,
        None => m.params.push(ModuleParam { name: name.to_string(), value: v, style }),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASS: &str = "# An acid line.\nmodule bass acid {\n  cutoff 800hz\n  resonance 80%\n  attack 0.2\n  osc1_wave square\n  lfo_sync bars_4\n  osc2_pitch -12st\n  vel_env 0.5\n}\n";

    fn round_trips(text: &str) -> Sound {
        let s = Sound::parse(text).expect("parses");
        let again = Sound::parse(&s.text()).expect("reads back");
        assert!(again.same_as(&s), "{}\n---\n{}", s.text(), again.text());
        assert_eq!(again.text(), s.text(), "the text is a fixed point");
        s
    }

    #[test]
    fn a_module_reads_and_writes_in_the_units_its_file_uses() {
        let s = round_trips(BASS);
        let t = s.text();
        assert_eq!(s.kind(), "bass");
        assert_eq!(s.description, "An acid line.");
        for line in ["module bass acid {", "cutoff 800hz", "resonance 80%", "attack 0.2", "osc1_wave square"] {
            assert!(t.contains(line), "{line}: {t}");
        }
        assert!(t.contains("lfo_sync bars_4") && t.contains("osc2_pitch -12st"), "{t}");
    }

    #[test]
    fn every_module_kind_and_every_style_round_trips() {
        round_trips("module beats kit {\n  kick_level 1.0\n  kick_decay 55%\n  kick_pitch 0.78x\n  snare_pan -0.15\n  hihat_level -3db\n  stutter_drum hihat\n}\n");
        round_trips("module keys pad { voice_mode poly cutoff 2khz attack 300ms level 0.5 }\n");
        let fm = round_trips("module fm bell {\n  algorithm two_op\n  level 2.0\n  mod_index 45%\n  op1_ratio 1.41x\n  op0_envelope 0.001 0.28 0.0 0.12\n  op1_envelope 0.001 0.07 0.05 0.05\n}\n");
        let Body::Module(m) = &fm.body else { panic!() };
        assert_eq!(m.envelopes.len(), 2);
        assert!(fm.text().contains("op1_envelope 0.001 0.07 0.05 0.05"), "{}", fm.text());
    }

    #[test]
    fn a_module_knob_keeps_its_style_and_a_new_one_takes_its_unit() {
        let mut s = Sound::parse(BASS).expect("parses");
        apply(&mut s, "cutoff=0.5\nresonance=0.5\nattack=0.3\ndecay=0.6\nlfo_target=pitch\nosc1_wave=1\n")
            .expect("applies");
        let t = s.text();
        assert!(t.contains("resonance 50%"), "{t}");
        assert!(t.contains("attack 0.3"), "a bare number stays bare: {t}");
        assert!(t.lines().any(|l| l.trim().starts_with("cutoff ") && l.ends_with("hz")), "{t}");
        assert!(t.lines().any(|l| l.trim().starts_with("decay ") && l.ends_with("ms")), "{t}");
        assert!(t.contains("lfo_target pitch") && t.contains("osc1_wave square"), "{t}");
        // Past the range: held to it. Not a number, or no such knob: refused.
        apply(&mut s, "resonance=7").expect("held");
        assert!(s.text().contains("resonance 100%"), "{}", s.text());
        for bad in ["resonance=NaN", "cutof=0.3", "lfo_target=nowhere", "resonance", "kick_level=1"] {
            assert!(apply(&mut s, bad).is_err(), "{bad}");
        }
        apply(&mut s, "-resonance").expect("unsets");
        assert!(!s.text().contains("resonance"), "{}", s.text());
        round_trips(&s.text());
    }

    #[test]
    fn a_gain_written_new_is_in_db_and_reads_back() {
        let mut s = Sound::parse("module keys k { voice_mode poly }\n").expect("parses");
        apply(&mut s, "level=0.5").expect("applies");
        assert!(s.text().contains("level -6.0206db") || s.text().contains("level -6.02"), "{}", s.text());
        round_trips(&s.text());
        apply(&mut s, "level=0").expect("silence");
        assert!(s.text().contains("level 0\n"), "{}", s.text());
    }

    #[test]
    fn fm_envelopes_are_knobs_too() {
        let mut s = Sound::parse("module fm f { algorithm two_op }\n").expect("parses");
        apply(&mut s, "env1.d=0.2").expect("applies");
        assert!(s.text().contains("op1_envelope 0.01 0.2 0.7 0.3"), "{}", s.text());
        apply(&mut s, "env1.s=3\nenv1.a=-1").expect("held");
        assert!(s.text().contains("op1_envelope 0.0005 0.2 1 0.3"), "{}", s.text());
        assert!(apply(&mut s, "env4.a=0.1").is_err());
        apply(&mut s, "-env1").expect("removes");
        assert!(!s.text().contains("envelope"), "{}", s.text());
        let mut b = Sound::parse(BASS).expect("parses");
        assert!(apply(&mut b, "env0.a=0.1").is_err(), "only fm");
    }

    #[test]
    fn nodes_are_added_wired_and_removed() {
        let mut s = Sound::parse("instrument k {\n  kick(A1) as k\n  k > out\n}\n").expect("parses");
        apply(&mut s, "+node lowpass f\n-wire k out\n+wire k f\n+wire f out").expect("applies");
        let t = s.text();
        assert!(t.contains("lowpass() as f") && t.contains("k > f") && t.contains("f > out"), "{t}");
        assert!(!t.contains("k > out"), "{t}");
        apply(&mut s, "f.cutoff=2000").expect("a knob on the new node");
        assert!(s.text().contains("lowpass(2000hz) as f"), "{}", s.text());
        apply(&mut s, "+node osc").expect("an alias of its own");
        assert!(s.text().contains("osc sine() as osc1"), "{}", s.text());
        apply(&mut s, "word osc1=saw").expect("a word");
        assert!(s.text().contains("osc saw() as osc1"), "{}", s.text());
        for bad in [
            "+node nope x",
            "+node phaser p",
            "+node mix m",
            "+node lowpass f",
            "+node lowpass out",
            "+node lowpass Big",
            "+node lowpass tempo",
            "+node lowpass o",
            "+node lowpass x",
            "+wire f f",
            "+wire zz out",
            "+wire k f",
            "-wire k out",
            "-node zz",
            "word osc1=wobble",
            "word f=saw",
        ] {
            let before = s.clone();
            assert!(apply(&mut s, bad).is_err(), "{bad} was taken");
            s = before;
        }
        apply(&mut s, "-node f").expect("removes");
        let t = s.text();
        assert!(!t.contains("lowpass") && !t.contains("> f") && !t.contains("f >"), "{t}");
        round_trips(&t);
    }

    #[test]
    fn a_graph_keeps_what_it_does_not_show() {
        // A rhythm division, a wet= and an expression all come back.
        let s = round_trips("instrument v {\n  osc saw(55 * 2) as src\n  lowpass(800hz, 0.2, wet=0.5) as f\n  delay(1/8, 0.4) as d\n  src > f\n  f > d\n  d > out\n}\n");
        let t = s.text();
        assert!(t.contains("osc saw(110) as src"), "{t}");
        assert!(t.contains("lowpass(800hz, 0.2, wet=0.5) as f"), "{t}");
        assert!(t.contains("delay(1/8, 0.4) as d"), "{t}");
    }

    #[test]
    fn a_file_with_more_than_the_sound_is_not_a_sound() {
        // A song with one instrument in it: opened, Save would write the
        // instrument alone and the rest would be gone.
        let song = "# A song.\ntempo 130\ninstrument k {\n  kick(A1) as k\n  k > out\n}\npattern demo { C3 - - - }\ntrack main { play demo using k }\n";
        let e = Sound::parse(song).expect_err("a song is not a sound");
        assert!(e.contains("more in it than a sound"), "{e}");
        assert!(e.contains("tempo") && e.contains("patterns") && e.contains("tracks"), "{e}");
        for (more, what) in [
            ("tempo 130\n", "tempo"),
            ("swing 0.6\n", "settings"),
            ("pattern p { C3 - - - }\n", "patterns"),
            ("groove g { 0 10 0 10 }\n", "other blocks"),
        ] {
            for text in [format!("{more}module bass b {{ }}\n"), format!("module bass b {{ }}\n{more}")] {
                match Sound::parse(&text) {
                    Err(e) => assert!(e.contains("more in it than a sound") && e.contains(what), "{text}: {e}"),
                    // Not every one of these is a line the reader takes; one that is has to be refused.
                    Ok(_) => panic!("{text} was opened as a sound"),
                }
            }
        }
        // A comment anywhere but on top is not kept either.
        for text in [
            "# Top.\ninstrument k {\n  # the body\n  kick(A1) as k\n  k > out\n}\n",
            "instrument k {\n  kick(F#1) as k  # tuned by ear\n  k > out\n}\n",
            "module bass b {\n  cutoff 800hz #dark\n}\n",
            "module bass b { }\n# and a note after\n",
        ] {
            let e = Sound::parse(text).expect_err("a comment that would be lost");
            assert!(e.contains("a comment after the ones on top"), "{text}: {e}");
        }
        let e = Sound::parse("# Top.\ninstrument k {\n  # the body\n  kick(A1) as k\n  k > out\n}\n").unwrap_err();
        assert!(e.starts_with("line 3:"), "{e}");
        // A sharp is not a comment, and the comment on top may hold anything.
        let s = round_trips("# A kick on F#1 # twice.\n\n# More.\ninstrument k {\n  kick(F#1) as k\n  k > out\n}\n");
        assert_eq!(s.description, "A kick on F#1 # twice. More.");
    }

    #[test]
    fn a_node_with_no_name_never_takes_one_the_file_gave_another() {
        // The unnamed highpass is node 1; `highpass1` is the noise.
        let text = "instrument an {\n  noise() as highpass1\n  highpass1 > highpass(3000hz, 0.5) > perc(0.001, 0.3) as amp\n  amp > out\n}\n";
        let s = round_trips(text);
        let Body::Graph(g) = &s.body else { panic!("a graph") };
        let mut names: Vec<&str> = g.nodes.iter().map(|n| n.alias.as_str()).collect();
        assert_eq!(names.len(), 3, "{}", s.text());
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 3, "two nodes by one name: {}", s.text());
        assert!(g.connections.iter().all(|(a, b)| a != b), "{}", s.text());
        // And it plays: the same wires as the file, under the new name.
        let src = format!("{}pattern p {{ C3 - - - }}\ntrack t {{ play p using an }}\n", s.text());
        tatum_core::song_engine::SongEngine::from_source(&src).expect("compiles");
        // Two unnamed nodes of a kind, and a name already one ahead of them.
        round_trips("instrument an {\n  noise() as lowpass2\n  lowpass2 > lowpass(900hz) > lowpass(400hz) as lowpass3\n  lowpass3 > out\n}\n");
    }

    fn library_sounds() -> Vec<(std::path::PathBuf, String)> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../sounds");
        let mut found = Vec::new();
        for dir in std::fs::read_dir(&root).expect("sounds/").flatten() {
            let Ok(files) = std::fs::read_dir(dir.path()) else { continue };
            for f in files.flatten().map(|f| f.path()).filter(|f| f.extension().is_some_and(|e| e == "synth")) {
                let text = std::fs::read_to_string(&f).expect("reads");
                found.push((f, text));
            }
        }
        assert!(found.len() >= 5, "the sounds/ library was not found at {}", root.display());
        found
    }

    #[test]
    fn saving_a_library_sound_changes_how_it_is_written_and_nothing_else() {
        for (path, text) in library_sounds() {
            let at = path.display();
            let s = Sound::parse(&text).unwrap_or_else(|e| panic!("{at}: {e}"));
            let saved = s.text();
            let again = Sound::parse(&saved).unwrap_or_else(|e| panic!("{at}: {e}"));
            // Every value exactly, not to a tolerance; and a second Save is the first.
            assert_eq!(again, s, "{at}: Save changed the sound");
            assert_eq!(again.text(), saved, "{at}: a second Save changed the text");
            // Line for line the file's own, but for the comment's wrapping
            // and a number written another way.
            let body = |t: &str| t.lines().filter(|l| !l.starts_with('#')).map(String::from).collect::<Vec<_>>();
            let (before, after) = (body(&text), body(&saved));
            assert_eq!(before.len(), after.len(), "{at}: lines added or lost\n{saved}");
            for (b, a) in before.iter().zip(&after) {
                let name = |l: &str| {
                    l.split(|c: char| c.is_whitespace() || c == '(').find(|w| !w.is_empty()).map(String::from)
                };
                assert_eq!(name(b), name(a), "{at}: '{b}' became '{a}'");
            }
            let words = |t: &str| {
                t.lines()
                    .filter_map(|l| l.strip_prefix('#'))
                    .flat_map(str::split_whitespace)
                    .map(String::from)
                    .collect::<Vec<_>>()
            };
            assert_eq!(words(&text), words(&saved), "{at}: the comment's words");
        }
    }

    #[test]
    fn what_save_normalises_is_what_the_docs_say() {
        // A number is written the shortest way, a time in ms, a frequency in
        // hz or as its note; what is written in %, dB or by name stays so.
        let m = Sound::parse(
            "module keys pad {\n  level 1.0\n  release 1.2s\n  cutoff 2khz\n  resonance 25%\n  voice_mode poly\n}\n",
        )
        .expect("parses");
        assert_eq!(
            m.text(),
            "module keys pad {\n  level 1\n  release 1200ms\n  cutoff 2000hz\n  resonance 25%\n  voice_mode poly\n}\n"
        );
        let g = Sound::parse(
            "instrument k {\n  kick(46.2493hz, tone=3khz, body=0.07s) as k\n  lowpass(440) as f\n  k > f > out\n}\n",
        )
        .expect("parses");
        let t = g.text();
        assert!(t.contains("tone=3000hz") && t.contains("body=70ms"), "{t}");
        assert!(t.contains("lowpass(A4) as f"), "a frequency that is a note is written as the note: {t}");
        assert!(t.contains("  k > f\n  f > out\n"), "a chain is written a wire a line: {t}");
        // The comment is one paragraph, wrapped again.
        let c = Sound::parse("# One.\n#\n# Two.\nmodule bass b { cutoff 800hz }\n").expect("parses");
        assert_eq!(c.text(), "# One. Two.\nmodule bass b {\n  cutoff 800hz\n}\n");
    }

    #[test]
    fn a_sound_is_one_instrument_or_one_module() {
        assert!(Sound::parse("tempo 120\n").is_err());
        assert!(Sound::parse("module bass a { }\nmodule keys b { }\n").is_err());
        assert!(Sound::parse("module bass a { }\ninstrument b {\n  osc sine() as o\n  o > out\n}\n").is_err());
        assert!(Sound::parse("module bass a { cutof 0.3 }\n").is_err());
        let mut s = Sound::parse(BASS).expect("parses");
        apply(&mut s, "name=acid_two").expect("a name");
        assert!(s.text().contains("module bass acid_two {"));
        assert!(apply(&mut s, "name=Two words").is_err());
        assert!(apply(&mut s, "name=tempo").is_err(), "a word of the language");
    }
}
