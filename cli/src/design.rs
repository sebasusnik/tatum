//! `tatum design <sound.synth>`: a sound designer in the browser, on this
//! machine. The page turns the knobs; this process is the engine behind it.
//!
//! A sound is a file with one `instrument` in it, the way `sounds/` keeps
//! them, and a comment on top that says what it is. The page shows a knob for
//! every argument of every node, from the same table `tatum params` prints and
//! the compiler checks against, so whatever node an instrument is built from
//! gets its knobs. Each turn is sent here: the instrument is written out
//! again, compiled, one hit is rendered with nothing after it -- no master
//! chain, no loudness -- and sent back with its pitch and level over time.
//! Save writes the file. The text stays the sound: a song plays it with
//! `use "sounds/kicks/deep.synth"`, and anyone can read and diff it.
//!
//! A record can be loaded next to it: its hits are averaged on the beat grid
//! (`tatum_debug::hit`), and the page draws and plays the two side by side.
//!
//! It serves 127.0.0.1 only, and nothing it is sent leaves the machine.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process;

use tatum_core::dsl::ast::{InstrumentDef, Param};
use tatum_core::nodes;
use tatum_core::song_engine::SongEngine;
use tatum_core::SAMPLE_RATE;
use tatum_debug::hit;

const PAGE: &str = include_str!("design.html");

/// What a new sound starts as: a deep kick, tuned and held.
const NEW_SOUND: &str = "# A deep kick.
instrument kick {
  kick(F#1, body=70ms, hold=100ms, release=200ms) as k
  k > out
}
";

/// A node of the sound as the page edits it: its arguments by name.
#[derive(Clone, Debug)]
struct Node {
    kind: String,
    alias: String,
    /// The waveform, vowel or mode word, when the node takes one.
    word: Option<String>,
    /// Every argument written in the file, positional ones by their name.
    args: BTreeMap<String, f32>,
}

#[derive(Clone, Debug)]
struct Sound {
    description: String,
    name: String,
    gain: Option<f32>,
    nodes: Vec<Node>,
    connections: Vec<(String, String)>,
}

impl Sound {
    /// The sound in `text`: its comment and its one instrument.
    fn parse(text: &str) -> Result<Sound, String> {
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
        let inst: &InstrumentDef = match song.instruments.as_slice() {
            [one] => one,
            [] => return Err("there is no `instrument` in it; a sound is one instrument".into()),
            _ => return Err("there is more than one `instrument` in it; a sound is one instrument".into()),
        };
        // Nodes with no name get one the page can show and the text can keep.
        let mut renames: Vec<(String, String)> = Vec::new();
        let mut nodes_out = Vec::new();
        for (i, n) in inst.nodes.iter().enumerate() {
            let alias = match n.alias.as_deref() {
                Some(a) if !a.starts_with("_anon_") => a.to_string(),
                Some(a) => {
                    let fresh = format!("{}{}", n.kind.replace('_', ""), i);
                    renames.push((a.to_string(), fresh.clone()));
                    fresh
                }
                None => format!("{}{}", n.kind.replace('_', ""), i),
            };
            let def = nodes::lookup(&n.kind).ok_or_else(|| format!("unknown node '{}'", n.kind))?;
            let mut args = BTreeMap::new();
            let mut word = None;
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
                    Param::Waveform(w) => word = Some(w.clone()),
                    Param::RhythmDiv(..) => {
                        return Err(format!("{}: a rhythm division is not something the designer can turn yet", n.kind))
                    }
                }
            }
            nodes_out.push(Node { kind: n.kind.clone(), alias, word, args });
        }
        let rename = |s: &str| renames.iter().find(|r| r.0 == s).map_or(s.to_string(), |r| r.1.clone());
        let connections = inst.connections.iter().map(|c| (rename(&c.from), rename(&c.to))).collect();
        Ok(Sound { description, name: inst.name.clone(), gain: inst.gain, nodes: nodes_out, connections })
    }

    /// The sound as a file: its comment, then the instrument, one node a line
    /// and one connection a line, every value in its unit.
    fn text(&self) -> String {
        let mut o = String::new();
        for line in wrap(&self.description, 76) {
            let _ = writeln!(o, "# {line}");
        }
        let _ = writeln!(o, "instrument {} {{", self.name);
        if let Some(g) = self.gain {
            let _ = writeln!(o, "  gain {}", number(g));
        }
        for n in &self.nodes {
            let _ = writeln!(o, "  {}", node_text(n));
        }
        for (a, b) in &self.connections {
            let _ = writeln!(o, "  {a} > {b}");
        }
        o.push_str("}\n");
        o
    }
}

fn wrap(text: &str, width: usize) -> Vec<String> {
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
fn number(v: f32) -> String {
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
fn note_name(hz: f32) -> Option<String> {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    let midi = (69.0 + 12.0 * (hz / 440.0).log2()).round();
    if !(0.0..=127.0).contains(&midi) {
        return None;
    }
    let exact = tatum_core::math::midi_to_freq(midi as u8);
    ((hz / exact).log2().abs() * 1200.0 < 0.01)
        .then(|| format!("{}{}", NAMES[midi as usize % 12], midi as i32 / 12 - 1))
}

/// A value in the unit its argument is written in.
fn with_unit(arg: &nodes::Arg, v: f32) -> String {
    let doc = arg.doc;
    if doc.starts_with("Hz") {
        note_name(v).unwrap_or_else(|| format!("{}hz", number(v)))
    } else if doc.starts_with("ms") {
        format!("{}ms", number(v))
    } else if doc.starts_with("dB") {
        format!("{}db", number(v))
    } else {
        number(v)
    }
}

fn node_text(n: &Node) -> String {
    let def = nodes::lookup(&n.kind);
    let mut parts: Vec<String> = Vec::new();
    // A waveform goes before the parentheses on the oscillators, inside them
    // on every other node that takes a word.
    let word_first = matches!(n.kind.as_str(), "osc" | "fixosc" | "pitch_osc");
    if let (Some(w), false) = (&n.word, word_first) {
        parts.push(w.clone());
    }
    if let Some(def) = def {
        // Positionals in order, up to the last one written; one left out
        // before it takes its default.
        let last = def.positional.iter().rposition(|a| n.args.contains_key(a.name));
        if let Some(last) = last {
            for a in &def.positional[..=last] {
                parts.push(with_unit(a, n.args.get(a.name).copied().unwrap_or(a.default)));
            }
        }
        for a in def.named {
            // A name that is also positional was written as one, above.
            if def.positional.iter().any(|p| p.name == a.name) {
                continue;
            }
            if let Some(&v) = n.args.get(a.name) {
                parts.push(format!("{}={}", a.name, with_unit(a, v)));
            }
        }
    }
    let word = match (&n.word, word_first) {
        (Some(w), true) => format!(" {w}"),
        _ => String::new(),
    };
    format!("{}{}({}) as {}", n.kind, word, parts.join(", "), n.alias)
}

// ── Rendering ──

/// One hit of the sound, alone: the instrument's own output, before any
/// insert, the master or the engine's loudness. Up to two seconds, cut where
/// it is 70 dB down.
fn render_hit(sound: &Sound, note: &str) -> Result<Vec<f32>, String> {
    let src = format!(
        "tempo 60\n{}\npattern one {{ {note} - - -  - - - -  - - - -  - - - - }}\ntrack t {{ play one using {} }}\n",
        sound.text(),
        sound.name
    );
    let mut engine = SongEngine::from_source(&src)?;
    engine.set_taps(true);
    let mut out = Vec::new();
    engine.render_each(1, |e, _, _| {
        if let Some(t) = e.taps() {
            out.extend_from_slice(&t.dry[0].l[..t.len]);
        }
    });
    out.truncate((2.0 * SAMPLE_RATE) as usize);
    let start = out.iter().position(|v| *v != 0.0).unwrap_or(0);
    let mut y = out.split_off(start);
    let peak = y.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    if let Some(last) = y.iter().rposition(|v| v.abs() > peak * 3.2e-4) {
        y.truncate((last + (0.01 * SAMPLE_RATE) as usize).min(y.len()));
    }
    Ok(y)
}

// ── JSON, by hand: the shapes are small and fixed ──

fn json_str(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(o, "\\u{:04x}", c as u32);
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn json_num(v: f32) -> String {
    if v.is_finite() {
        number(v)
    } else {
        "null".into()
    }
}

fn json_pairs(v: &[(f32, f32)]) -> String {
    let items: Vec<String> = v.iter().map(|(a, b)| format!("[{},{}]", json_num(*a), json_num(*b))).collect();
    format!("[{}]", items.join(","))
}

/// The sound as the page builds its knobs from it: each node with every
/// argument its kind takes, its range, its unit, and what the file says.
fn sound_json(sound: &Sound, path: &Path) -> String {
    let mut nodes_json = Vec::new();
    for n in &sound.nodes {
        let Some(def) = nodes::lookup(&n.kind) else { continue };
        let mut args = Vec::new();
        let mut seen: Vec<&str> = Vec::new();
        for a in def.positional.iter().chain(def.named.iter()) {
            if seen.contains(&a.name) {
                continue;
            }
            seen.push(a.name);
            let unit = if a.doc.starts_with("Hz") {
                "hz"
            } else if a.doc.starts_with("ms") {
                "ms"
            } else if a.doc.starts_with("dB") {
                "db"
            } else {
                ""
            };
            args.push(format!(
                "{{\"name\":{},\"min\":{},\"max\":{},\"default\":{},\"unit\":{},\"doc\":{},\"value\":{}}}",
                json_str(a.name),
                json_num(a.min),
                json_num(a.max),
                json_num(a.default),
                json_str(unit),
                json_str(a.doc),
                n.args.get(a.name).map_or("null".into(), |v| json_num(*v))
            ));
        }
        nodes_json.push(format!(
            "{{\"kind\":{},\"alias\":{},\"doc\":{},\"args\":[{}]}}",
            json_str(&n.kind),
            json_str(&n.alias),
            json_str(def.doc),
            args.join(",")
        ));
    }
    format!(
        "{{\"path\":{},\"name\":{},\"description\":{},\"text\":{},\"nodes\":[{}]}}",
        json_str(&path.display().to_string()),
        json_str(&sound.name),
        json_str(&sound.description),
        json_str(&sound.text()),
        nodes_json.join(",")
    )
}

/// A hit as the page plays and draws it: a mono 16-bit WAV, base64, and its
/// curves.
fn hit_json(y: &[f32]) -> String {
    let c = hit::curves(y);
    let peak = y.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-9);
    // Played at the same peak as anything it is compared with.
    let norm: Vec<f32> = y.iter().map(|v| v / peak * 0.7).collect();
    let wav = tatum_core::wav::encode_stereo_16(&norm, &norm, SAMPLE_RATE as u32);
    format!(
        "{{\"audio\":{},\"pitch\":{},\"level\":{},\"ms\":{}}}",
        json_str(&base64(&wav)),
        json_pairs(&c.pitch),
        json_pairs(&c.level),
        json_num(y.len() as f32 * 1000.0 / SAMPLE_RATE)
    )
}

fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut o = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        o.push(T[(n >> 18) as usize & 63] as char);
        o.push(T[(n >> 12) as usize & 63] as char);
        o.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        o.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    o
}

// ── The page's requests ──

struct State {
    path: PathBuf,
    sound: Sound,
    /// The note the hit is played on, for instruments that follow it.
    note: String,
    /// The record's averaged hit, once one is loaded: what Fit fits to.
    reference: Option<Vec<f32>>,
}

/// `alias.arg=value` lines set a knob, `-alias.arg` unsets one (back to its
/// default), `description=` and `gain=` set those; anything else is an error
/// naming the line.
fn apply(sound: &mut Sound, body: &str) -> Result<(), String> {
    for line in body.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(rest) = line.strip_prefix("description=") {
            sound.description = rest.to_string();
            continue;
        }
        if let Some(rest) = line.strip_prefix("gain=") {
            sound.gain = match rest.trim() {
                "" => None,
                v => Some(finite(v).ok_or_else(|| format!("'{line}': '{v}' is not a number"))?),
            };
            continue;
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
        let node = sound
            .nodes
            .iter_mut()
            .find(|n| n.alias == alias)
            .ok_or_else(|| format!("'{line}': no node named '{alias}'"))?;
        let def = nodes::lookup(&node.kind).ok_or_else(|| format!("unknown node '{}'", node.kind))?;
        let a = def
            .positional
            .iter()
            .chain(def.named.iter())
            .find(|a| a.name == arg)
            .ok_or_else(|| format!("'{line}': {} has no argument '{arg}'", node.kind))?;
        match value {
            Some(v) => {
                node.args.insert(a.name.to_string(), v.clamp(a.min, a.max));
            }
            None => {
                node.args.remove(a.name);
            }
        }
    }
    Ok(())
}

/// A number as a knob takes it: `NaN` and `inf` parse as floats, and would
/// be written into the file as `NaNms`.
fn finite(v: &str) -> Option<f32> {
    v.parse::<f32>().ok().filter(|v| v.is_finite())
}

fn render_reply(state: &State) -> String {
    match render_hit(&state.sound, &state.note) {
        Ok(y) => format!("{{\"ok\":true,\"text\":{},\"hit\":{}}}", json_str(&state.sound.text()), hit_json(&y)),
        Err(e) => format!("{{\"ok\":false,\"error\":{},\"text\":{}}}", json_str(&e), json_str(&state.sound.text())),
    }
}

/// A record's hits, averaged: `from`, `to` (seconds), `bpm` (else measured)
/// and `ms` (how much of each hit) from the query, as `tatum kick fit`
/// takes them. The average is kept for Fit.
fn reference_reply(state: &mut State, query: &BTreeMap<String, String>, body: &[u8]) -> String {
    let get = |k: &str| query.get(k).and_then(|v| v.parse::<f32>().ok()).filter(|v| v.is_finite() && *v >= 0.0);
    let fail = |e: &str| format!("{{\"ok\":false,\"error\":{}}}", json_str(e));
    let audio = match tatum_debug::wav::decode(body) {
        Ok(a) => a,
        Err(e) => return fail(&format!("not a WAV it can read: {e}")),
    };
    let req = crate::kick::FitRequest {
        from: get("from"),
        to: get("to"),
        bpm: get("bpm").filter(|b| *b > 0.0),
        ms: get("ms").unwrap_or(230.0).clamp(20.0, 2000.0),
        effort: 1.0,
        hit: None,
    };
    match crate::kick::reference_hit(&audio, &req) {
        Ok((y, bpm)) => {
            let reply = format!("{{\"ok\":true,\"bpm\":{},\"hit\":{}}}", json_num(bpm), hit_json(&y));
            state.reference = Some(y);
            reply
        }
        Err(e) => fail(&e),
    }
}

/// The first `kick` node of the sound fitted to the loaded record: its
/// knobs set to what `tatum kick fit` finds, the hit rendered again.
fn fit_reply(state: &mut State) -> String {
    let fail = |e: &str| format!("{{\"ok\":false,\"error\":{}}}", json_str(e));
    let Some(reference) = state.reference.clone() else {
        return fail("load a record first: Fit fits the kick to its averaged hit");
    };
    let Some(node) = state.sound.nodes.iter().position(|n| n.kind == "kick") else {
        return fail("there is no kick() node to fit; Fit sets the kick node's knobs");
    };
    let target = tatum_debug::fit::Target::new(reference);
    let fitted = match crate::kick::fit_kick(&target, 0.6) {
        Ok(f) => f,
        Err(e) => return fail(&e),
    };
    let k = &fitted.args;
    let args = &mut state.sound.nodes[node].args;
    for (name, v) in [
        ("note", k.note),
        ("start", k.start),
        ("knee", k.knee),
        ("snap", k.snap),
        ("body", k.body),
        ("hold", k.hold),
        ("sag", k.sag),
        ("release", k.release),
        ("phase", k.phase),
        ("click", k.click),
        ("tone", k.tone),
    ] {
        args.insert(name.to_string(), v);
    }
    if k.chirp > 0.0 {
        args.insert("chirp".into(), k.chirp);
        args.insert("chirp_time".into(), k.chirp_time);
    } else {
        args.remove("chirp");
        args.remove("chirp_time");
    }
    let s = fitted.score;
    let render = render_reply(state);
    let sound = sound_json(&state.sound, &state.path);
    format!(
        "{{\"ok\":true,\"sound\":{sound},\"render\":{render},\"fit\":{{\"line\":{},\"pitch_st\":{},\"attack_st\":{},\"level_db\":{},\"click_db\":{}}}}}",
        json_str(&k.line()),
        json_num(s.pitch_st),
        json_num(s.attack_st),
        json_num(s.level_db),
        json_num(s.click_db)
    )
}

// ── HTTP, the little of it a local page needs ──

struct Request {
    method: String,
    path: String,
    query: BTreeMap<String, String>,
    /// Header names in lower case.
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

/// Why a request was not read: the status to answer with, and why. `None`
/// when the connection went away and there is no one to answer.
type Refused = Option<(&'static str, String)>;

/// The longest body taken: a WAV of a long extended mix.
const MAX_BODY: usize = 400 << 20;

fn read_request(stream: &mut impl Read) -> Result<Request, Refused> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let head_end = loop {
        let n = stream.read(&mut chunk).map_err(|_| None)?;
        if n == 0 {
            return Err(None);
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        if buf.len() > 64 << 10 {
            return Err(Some(("431 Request Header Fields Too Large", "the head is over 64 kB".into())));
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.lines();
    let bad = || Some(("400 Bad Request", "not an HTTP request".to_string()));
    let mut first = lines.next().ok_or_else(bad)?.split_whitespace();
    let method = first.next().ok_or_else(bad)?.to_string();
    let target = first.next().ok_or_else(bad)?.to_string();
    let headers: BTreeMap<String, String> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    let length = match headers.get("content-length") {
        None => 0,
        Some(v) => v.parse::<usize>().map_err(|_| Some(("400 Bad Request", format!("Content-Length '{v}'"))))?,
    };
    if length > MAX_BODY {
        return Err(Some(("413 Content Too Large", format!("{length} bytes; at most {MAX_BODY}"))));
    }
    let mut body = buf[head_end..].to_vec();
    while body.len() < length {
        let n = stream.read(&mut chunk).map_err(|_| None)?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    if body.len() < length {
        return Err(Some(("400 Bad Request", format!("{} bytes of the {length} it said", body.len()))));
    }
    body.truncate(length);
    let (path, q) = target.split_once('?').unwrap_or((&target, ""));
    let query = q.split('&').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect();
    Ok(Request { method, path: path.to_string(), query, headers, body })
}

/// Whether the request comes from the page this process served, and not from
/// another site open in the same browser. The server only listens on
/// 127.0.0.1, but any page can send a request there: a form or a `fetch`
/// from a site on the web would otherwise rewrite and save the sound. So:
/// - `Host` is this server, by address or as localhost (a DNS name pointed
///   at 127.0.0.1 -- rebinding -- carries its own name);
/// - an `Origin`, when there is one, is this server too;
/// - every POST carries `X-Tatum-Design`, a header a cross-site request can
///   only send after a preflight this server never answers.
fn same_origin(req: &Request, port: u16) -> Result<(), String> {
    let ours = |h: &str| h == format!("127.0.0.1:{port}") || h == format!("localhost:{port}");
    match req.headers.get("host") {
        Some(h) if ours(h) => {}
        Some(h) => return Err(format!("Host '{h}' is not this server")),
        None => return Err("no Host".into()),
    }
    if let Some(o) = req.headers.get("origin") {
        let host = o.strip_prefix("http://").unwrap_or("");
        if !ours(host) {
            return Err(format!("Origin '{o}' is not this page"));
        }
    }
    if req.method != "GET" && !req.headers.contains_key("x-tatum-design") {
        return Err("a POST from somewhere other than the designer's page".into());
    }
    Ok(())
}

fn respond(stream: &mut TcpStream, status: &str, kind: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
}

fn handle(state: &mut State, req: Request) -> (String, &'static str, Vec<u8>) {
    let json = "application/json";
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/") => ("200 OK".into(), "text/html; charset=utf-8", PAGE.as_bytes().to_vec()),
        ("GET", "/api/sound") => ("200 OK".into(), json, sound_json(&state.sound, &state.path).into_bytes()),
        ("POST", "/api/set") => {
            let body = String::from_utf8_lossy(&req.body).to_string();
            let mut next = state.sound.clone();
            match apply(&mut next, &body) {
                Ok(()) => {
                    state.sound = next;
                    ("200 OK".into(), json, render_reply(state).into_bytes())
                }
                Err(e) => ("200 OK".into(), json, format!("{{\"ok\":false,\"error\":{}}}", json_str(&e)).into_bytes()),
            }
        }
        ("POST", "/api/text") => {
            // The whole file, as typed: for adding a node or wiring.
            let body = String::from_utf8_lossy(&req.body).to_string();
            match Sound::parse(&body) {
                Ok(s) => {
                    state.sound = s;
                    let reply = render_reply(state);
                    let sound = sound_json(&state.sound, &state.path);
                    ("200 OK".into(), json, format!("{{\"sound\":{sound},\"render\":{reply}}}").into_bytes())
                }
                Err(e) => ("200 OK".into(), json, format!("{{\"ok\":false,\"error\":{}}}", json_str(&e)).into_bytes()),
            }
        }
        ("POST", "/api/note") => {
            let note = String::from_utf8_lossy(&req.body).trim().to_string();
            if note.is_empty() || !tatum_core::dsl::compiler::notes::note_in_midi_range(&note) {
                return (
                    "200 OK".into(),
                    json,
                    format!("{{\"ok\":false,\"error\":{}}}", json_str("not a note")).into_bytes(),
                );
            }
            state.note = note;
            ("200 OK".into(), json, render_reply(state).into_bytes())
        }
        ("POST", "/api/save") => {
            let text = state.sound.text();
            let result = (|| {
                if let Some(dir) = state.path.parent().filter(|d| !d.as_os_str().is_empty()) {
                    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
                }
                std::fs::write(&state.path, &text).map_err(|e| e.to_string())
            })();
            let reply = match result {
                Ok(()) => {
                    eprintln!("saved {}", state.path.display());
                    format!("{{\"ok\":true,\"path\":{}}}", json_str(&state.path.display().to_string()))
                }
                Err(e) => format!("{{\"ok\":false,\"error\":{}}}", json_str(&e)),
            };
            ("200 OK".into(), json, reply.into_bytes())
        }
        ("POST", "/api/reference") => {
            ("200 OK".into(), json, reference_reply(state, &req.query, &req.body).into_bytes())
        }
        ("POST", "/api/fit") => ("200 OK".into(), json, fit_reply(state).into_bytes()),
        _ => ("404 Not Found".into(), "text/plain", b"not here".to_vec()),
    }
}

fn usage() -> ! {
    eprintln!("Usage: tatum design <sound.synth> [--port N] [--no-open]");
    eprintln!("  Opens a designer for the sound in the browser. The file is one `instrument`;");
    eprintln!("  if it does not exist it starts as a deep kick, and Save creates it.");
    process::exit(1);
}

pub fn cmd(args: &[String]) {
    let mut path = None;
    let mut port: u16 = 7350;
    let mut open = true;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--port" => {
                i += 1;
                port = args.get(i).and_then(|p| p.parse().ok()).unwrap_or_else(|| usage());
            }
            "--no-open" => open = false,
            a if a.starts_with('-') => usage(),
            a => path = Some(PathBuf::from(a)),
        }
        i += 1;
    }
    let path = path.unwrap_or_else(|| usage());
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| NEW_SOUND.to_string());
    let sound = Sound::parse(&text).unwrap_or_else(|e| {
        eprintln!("{}: {e}", path.display());
        process::exit(1);
    });
    let mut state = State { path, sound, note: "C3".into(), reference: None };
    if let Err(e) = render_hit(&state.sound, &state.note) {
        eprintln!("{}: {e}", state.path.display());
        process::exit(1);
    }
    let listener = TcpListener::bind(("127.0.0.1", port)).unwrap_or_else(|e| {
        eprintln!("cannot listen on 127.0.0.1:{port}: {e} (another `tatum design`? try --port)");
        process::exit(1);
    });
    let url = format!("http://127.0.0.1:{port}/");
    eprintln!("designing {} at {url}  (Ctrl-C to stop)", state.path.display());
    if open {
        let opener = if cfg!(target_os = "macos") {
            "open"
        } else if cfg!(windows) {
            "explorer"
        } else {
            "xdg-open"
        };
        let _ = process::Command::new(opener).arg(&url).spawn();
    }
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        // One request at a time: a client that stops sending must not hold
        // the page's next turn of a knob for ever.
        let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(30)));
        let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(30)));
        match read_request(&mut stream) {
            Ok(req) => match same_origin(&req, port) {
                Ok(()) => {
                    let (status, kind, body) = handle(&mut state, req);
                    respond(&mut stream, &status, kind, &body);
                }
                Err(why) => {
                    eprintln!("refused: {why}");
                    respond(&mut stream, "403 Forbidden", "text/plain", why.as_bytes());
                }
            },
            Err(Some((status, why))) => respond(&mut stream, status, "text/plain", why.as_bytes()),
            Err(None) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sound_written_out_reads_back_the_same() {
        let text = "# A test.\ninstrument boom {\n  gain 0.9\n  pitch_osc sine(300, 50, 0.999) as body\n  body > perc(0.001, 0.3) as amp\n  noise() as n\n  n > highpass(3000hz, 0.5) > amp\n  amp > out\n}\n";
        let s = Sound::parse(text).expect("parses");
        let again = Sound::parse(&s.text()).expect("reads back");
        assert_eq!(s.text(), again.text());
        assert_eq!(s.description, "A test.");
        assert_eq!(s.gain, Some(0.9));
        assert!(s.text().contains("pitch_osc sine(300hz, 50hz, 0.999) as body"), "{}", s.text());
        // 55 Hz is A1, 440 Hz is A4; 300 Hz is no note.
        assert_eq!(note_name(55.0).as_deref(), Some("A1"));
        assert_eq!(note_name(440.0).as_deref(), Some("A4"));
        assert_eq!(note_name(300.0), None);
    }

    #[test]
    fn a_knob_sets_the_argument_in_its_unit() {
        let mut s = Sound::parse(NEW_SOUND).expect("parses");
        apply(&mut s, "k.body=85\nk.sag=-3\n").expect("applies");
        let t = s.text();
        assert!(t.contains("body=85ms"), "{t}");
        assert!(t.contains("sag=-3db"), "{t}");
        assert!(t.contains("kick(F#1,"), "a note comes back as its name: {t}");
        apply(&mut s, "k.knee=173.4\nk.snap=1.97\n").expect("applies");
        assert!(s.text().contains("knee=173.4hz, snap=1.97ms"), "the shortest text: {}", s.text());
        apply(&mut s, "-k.sag").expect("unsets");
        assert!(!s.text().contains("sag="), "{}", s.text());
        assert!(apply(&mut s, "k.nope=1").is_err());
        assert!(apply(&mut s, "zz.body=1").is_err());
    }

    #[test]
    fn a_value_past_its_range_is_held_to_it() {
        let mut s = Sound::parse(NEW_SOUND).expect("parses");
        apply(&mut s, "k.sag=-100").expect("applies");
        assert!(s.text().contains("sag=-24db"), "{}", s.text());
    }

    #[test]
    fn the_hit_is_rendered_alone() {
        let s = Sound::parse(NEW_SOUND).expect("parses");
        let y = render_hit(&s, "C3").expect("renders");
        // hold 100 + release 200: about 300 ms, cut where it is 70 dB down.
        let ms = y.len() as f32 * 1000.0 / SAMPLE_RATE;
        assert!((250.0..400.0).contains(&ms), "{ms} ms");
        let c = hit::curves(&y);
        let tail = c.pitch.iter().filter(|p| (120.0..200.0).contains(&p.0)).map(|p| p.1).sum::<f32>()
            / c.pitch.iter().filter(|p| (120.0..200.0).contains(&p.0)).count() as f32;
        assert!((tail - 46.25).abs() < 0.5, "F#1, got {tail}");
    }

    fn request(raw: &str) -> Result<Request, Refused> {
        read_request(&mut std::io::Cursor::new(raw.as_bytes().to_vec()))
    }

    fn status_of(raw: &str) -> &'static str {
        match request(raw) {
            Ok(_) => "ok",
            Err(Some((s, _))) => s,
            Err(None) => "dropped",
        }
    }

    #[test]
    fn a_body_too_long_short_or_unsaid_is_answered_not_dropped() {
        let huge =
            format!("POST /api/set HTTP/1.1\r\nHost: 127.0.0.1:7350\r\nContent-Length: {}\r\n\r\n", MAX_BODY + 1);
        assert!(status_of(&huge).starts_with("413"));
        let short = "POST /api/set HTTP/1.1\r\nHost: 127.0.0.1:7350\r\nContent-Length: 50\r\n\r\nk.body=80";
        assert!(status_of(short).starts_with("400"));
        let nonsense = "POST /api/set HTTP/1.1\r\nContent-Length: lots\r\n\r\n";
        assert!(status_of(nonsense).starts_with("400"));
        let head = format!("GET / HTTP/1.1\r\nX: {}\r\n", "a".repeat(70 << 10));
        assert!(status_of(&head).starts_with("431"));
        assert_eq!(status_of("\r\n\r\n"), "400 Bad Request");
        assert_eq!(status_of(""), "dropped");
        let fine = "POST /api/set?x=1 HTTP/1.1\r\nHost: 127.0.0.1:7350\r\nContent-Length: 9\r\n\r\nk.body=80";
        let r = request(fine).ok().expect("reads");
        assert_eq!(r.body, b"k.body=80");
        assert_eq!(r.headers.get("host").map(String::as_str), Some("127.0.0.1:7350"));
    }

    #[test]
    fn only_the_designers_own_page_is_answered() {
        let req = |head: &str| request(&format!("{head}\r\n\r\n")).ok().expect("reads");
        let page =
            "POST /api/save HTTP/1.1\r\nHost: 127.0.0.1:7350\r\nOrigin: http://127.0.0.1:7350\r\nX-Tatum-Design: 1";
        assert!(same_origin(&req(page), 7350).is_ok());
        assert!(same_origin(&req("GET / HTTP/1.1\r\nHost: localhost:7350"), 7350).is_ok());
        // A site on the web posting to the designer: its own Origin, and no header.
        let csrf = "POST /api/save HTTP/1.1\r\nHost: 127.0.0.1:7350\r\nOrigin: https://evil.example";
        assert!(same_origin(&req(csrf), 7350).is_err());
        // A form posted with no Origin at all still lacks the header.
        assert!(same_origin(&req("POST /api/save HTTP/1.1\r\nHost: 127.0.0.1:7350"), 7350).is_err());
        // DNS rebinding: the page's own name pointed at 127.0.0.1.
        let rebound = "GET /api/sound HTTP/1.1\r\nHost: evil.example:7350";
        assert!(same_origin(&req(rebound), 7350).is_err());
        // Another port is another server.
        assert!(same_origin(&req("GET / HTTP/1.1\r\nHost: 127.0.0.1:9999"), 7350).is_err());
        assert!(same_origin(&req("GET / HTTP/1.1"), 7350).is_err());
    }

    #[test]
    fn nan_and_infinity_are_not_numbers_here() {
        let mut s = Sound::parse(NEW_SOUND).expect("parses");
        for bad in ["k.body=NaN", "k.body=inf", "k.body=-infinity", "gain=nan", "gain=loud", "k.body=", "k.body"] {
            assert!(apply(&mut s, bad).is_err(), "{bad} was taken");
        }
        assert!(!s.text().to_lowercase().contains("nan"), "{}", s.text());
        apply(&mut s, "gain=0.8").expect("a gain");
        assert_eq!(s.gain, Some(0.8));
        apply(&mut s, "gain=").expect("no gain");
        assert_eq!(s.gain, None);
    }

    /// A record made of a known kick at 125 BPM, as the page would send it.
    fn record_wav() -> Vec<u8> {
        let known = "kick(A1, start=900hz, knee=170hz, snap=3ms, body=75ms, hold=90ms, release=200ms, phase=20)";
        let one = crate::kick::engine_hit(known, 600.0).expect("renders");
        let beat = (60.0 / 125.0 * SAMPLE_RATE) as usize;
        let mut x = vec![0.0f32; 20 * SAMPLE_RATE as usize];
        let mut at = 500;
        while at + beat < x.len() {
            for (i, v) in one.iter().enumerate().take(beat) {
                x[at + i] += 0.8 * v;
            }
            at += beat;
        }
        tatum_core::wav::encode_stereo_16(&x, &x, SAMPLE_RATE as u32)
    }

    #[test]
    fn fit_sets_the_kick_knobs_to_the_record() {
        let wav = record_wav();
        let mut state = State {
            path: PathBuf::from("/nonexistent/k.synth"),
            sound: Sound::parse(NEW_SOUND).expect("parses"),
            note: "C3".into(),
            reference: None,
        };
        assert!(fit_reply(&mut state).contains("load a record first"));
        let mut q = BTreeMap::new();
        q.insert("bpm".to_string(), "125".to_string());
        let r = reference_reply(&mut state, &q, &wav);
        assert!(r.starts_with("{\"ok\":true"), "{}", &r[..r.len().min(200)]);
        let r = fit_reply(&mut state);
        assert!(r.starts_with("{\"ok\":true"), "{}", &r[..r.len().min(300)]);
        let text = state.sound.text();
        assert!(text.contains("kick(A1,"), "{text}");
        let body = state.sound.nodes[0].args["body"];
        assert!((body - 75.0).abs() < 8.0, "{text}");
        // No kick node: said, not guessed.
        state.sound = Sound::parse("instrument x {\n  osc sine(110) as w\n  w > out\n}\n").expect("parses");
        assert!(fit_reply(&mut state).contains("no kick() node"));
    }

    #[test]
    fn a_bad_reference_query_is_not_taken_for_a_number() {
        let mut state = State {
            path: PathBuf::from("/nonexistent/k.synth"),
            sound: Sound::parse(NEW_SOUND).expect("parses"),
            note: "C3".into(),
            reference: None,
        };
        // A WAV it can read, so the query is what is looked at: what is not
        // a number from 0 up is left out, and the reply is the one without it.
        let wav = record_wav();
        let mut q = BTreeMap::new();
        q.insert("bpm".to_string(), "125".to_string());
        let plain = reference_reply(&mut state, &q, &wav);
        assert!(plain.starts_with("{\"ok\":true"), "{}", &plain[..plain.len().min(200)]);
        for (k, bad) in [("from", "NaN"), ("from", "-5"), ("to", "inf"), ("to", "-1"), ("ms", "NaN"), ("ms", "-inf")] {
            let mut q = q.clone();
            q.insert(k.to_string(), bad.to_string());
            state.reference = None;
            let r = reference_reply(&mut state, &q, &wav);
            assert!(r == plain, "{k}={bad}: {}", &r[..r.len().min(200)]);
            assert!(state.reference.is_some(), "{k}={bad}");
        }
        // A tempo that is not one is measured instead, not taken.
        for bad in ["NaN", "inf", "-125", "0"] {
            q.insert("bpm".to_string(), bad.to_string());
            let r = reference_reply(&mut state, &q, &wav);
            let bpm = r.strip_prefix("{\"ok\":true,\"bpm\":").and_then(|r| r.split(',').next()?.parse::<f32>().ok());
            assert!(bpm.is_some_and(|b| (b - 125.0).abs() < 0.1), "bpm={bad}: {}", &r[..r.len().min(200)]);
        }
    }

    #[test]
    fn base64_is_base64() {
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
    }
}
