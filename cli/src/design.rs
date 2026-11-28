//! `tatum design <sound.synth>`: a sound designer in the browser, on this
//! machine. The page turns the knobs; this process is the engine behind it.
//!
//! A sound is a file with one `instrument` graph or one `module` (beats,
//! bass, fm, keys) in it, the way `sounds/` keeps them, and a comment on top
//! that says what it is. The page shows a knob for every argument of every
//! node of a graph, and for every parameter of a module, from the same tables
//! `tatum params` prints and the compiler checks against. Nodes are added,
//! removed and wired from the page too. Each turn is sent here: the sound is
//! written out again, compiled, and what the loop plays -- a hit, four on the
//! floor, a held note or a chord, on a note the page picks; a lane or a
//! groove on a drum kit -- is rendered with nothing after it (no master
//! chain, no loudness) and sent back, with one hit's pitch and level over
//! time. Save writes the file, and only a sound that compiles; a file with
//! more in it than the one sound and its comment is not opened, because Save
//! would not keep the rest. The text stays the sound: a song plays it with `use "sounds/kicks/deep.synth"`, and
//! anyone can read and diff it.
//!
//! The `sounds/` library around the file (or `--library <dir>`) is listed on
//! the page; a sound in it can be opened, and the one open duplicated to a
//! new name. Nothing is written outside the library and the file the
//! designer was started on.
//!
//! A record can be loaded next to it: its hits are averaged on the beat grid
//! (`tatum_debug::hit`), and the page draws and plays the two side by side.
//!
//! It serves 127.0.0.1 only, and nothing it is sent leaves the machine.

mod audition;
mod library;
mod sound;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process;
use std::time::{Duration, Instant};

use tatum_core::nodes;
use tatum_core::params::{self, Curve, Range};
use tatum_core::song_engine::SongEngine;
use tatum_core::SAMPLE_RATE;
use tatum_debug::hit;

use audition::Audition;
use library::Library;
use sound::{apply, number, Body, Sound};

const PAGE: &str = include_str!("design.html");

/// What a new sound starts as: a deep kick, tuned and held.
const NEW_SOUND: &str = "# A deep kick.
instrument kick {
  kick(F#1, body=70ms, hold=100ms, release=200ms) as k
  k > out
}
";

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

/// The sound as the page builds its knobs from it: a graph's nodes with
/// every argument their kind takes, or a module's parameters, each with its
/// range, its unit, and what the file says; and what the loop plays.
fn sound_json(state: &State) -> String {
    let sound = &state.sound;
    let body = match &sound.body {
        Body::Graph(g) => {
            let mut nodes_json = Vec::new();
            for n in &g.nodes {
                let Some(def) = nodes::lookup(&n.kind) else { continue };
                let mut args = Vec::new();
                let mut seen: Vec<&str> = Vec::new();
                // `wet=` is on every node that can sit in a chain.
                let wet = nodes::Arg { name: nodes::WET, min: 0.0, max: 1.0, default: 1.0, doc: "dry/wet; 0 bypasses" };
                let extra: &[nodes::Arg] = if def.in_chains { std::slice::from_ref(&wet) } else { &[] };
                for a in def.positional.iter().chain(def.named.iter()).chain(extra.iter()) {
                    if seen.contains(&a.name) {
                        continue;
                    }
                    seen.push(a.name);
                    args.push(format!(
                        "{{\"name\":{},\"min\":{},\"max\":{},\"default\":{},\"unit\":{},\"doc\":{},\"value\":{}}}",
                        json_str(a.name),
                        json_num(a.min),
                        json_num(a.max),
                        json_num(a.default),
                        json_str(sound::arg_unit(a)),
                        json_str(a.doc),
                        n.args.get(a.name).map_or("null".into(), |v| json_num(*v))
                    ));
                }
                nodes_json.push(format!(
                    "{{\"kind\":{},\"alias\":{},\"doc\":{},\"words\":{},\"word_choices\":{},\"args\":[{}]}}",
                    json_str(&n.kind),
                    json_str(&n.alias),
                    json_str(def.doc),
                    json_strs(n.words.iter().map(String::as_str)),
                    json_strs(sound::word_choices(&n.kind).iter().copied()),
                    args.join(",")
                ));
            }
            let wires: Vec<String> =
                g.connections.iter().map(|(a, b)| format!("[{},{}]", json_str(a), json_str(b))).collect();
            format!(
                "\"gain\":{},\"nodes\":[{}],\"connections\":[{}]",
                g.gain.map_or("null".into(), json_num),
                nodes_json.join(","),
                wires.join(",")
            )
        }
        Body::Module(m) => {
            let mut ps = Vec::new();
            for spec in params::specs(m.kind) {
                let set = m.params.iter().find(|p| p.name == spec.name);
                let (choices, unit) = match spec.range {
                    Range::Choice(names) => (json_strs(names.iter().copied()), ""),
                    Range::Gain { .. } => ("null".into(), "db"),
                    _ => ("null".into(), spec.curve.unit().map_or("", |u| u.suffix())),
                };
                let curve = match spec.curve {
                    Curve::None => "null".into(),
                    Curve::Exp { scale, base, .. } => format!("{{\"exp\":[{},{}]}}", json_num(scale), json_num(base)),
                    Curve::Lin { min, max, .. } => format!("{{\"lin\":[{},{}]}}", json_num(min), json_num(max)),
                };
                ps.push(format!(
                    "{{\"name\":{},\"choices\":{choices},\"min\":{},\"max\":{},\"default\":{},\"unit\":{},\"curve\":{curve},\"doc\":{},\"value\":{},\"written\":{}}}",
                    json_str(spec.name),
                    json_num(spec.range.min()),
                    json_num(spec.range.max()),
                    json_num(spec.default),
                    json_str(unit),
                    json_str(spec.doc),
                    set.map_or("null".into(), |p| json_num(p.value)),
                    set.map_or("null".into(), |p| json_str(&sound::module_value(m.kind, p)))
                ));
            }
            let envs: Vec<String> = if m.kind == params::ModuleKind::Fm {
                (0..4)
                    .map(|op| {
                        m.envelopes.iter().find(|e| e.op_index == op).map_or("null".into(), |e| {
                            format!("[{},{},{},{}]", json_num(e.a), json_num(e.d), json_num(e.s), json_num(e.r))
                        })
                    })
                    .collect()
            } else {
                Vec::new()
            };
            format!(
                "\"module\":{},\"params\":[{}],\"envelopes\":[{}],\"env_time\":[{},{}],\"env_default\":[{}]",
                json_str(m.kind.as_str()),
                ps.join(","),
                envs.join(","),
                json_num(sound::ENV_TIME.0),
                json_num(sound::ENV_TIME.1),
                sound::ENV_DEFAULT.iter().map(|v| json_num(*v)).collect::<Vec<_>>().join(",")
            )
        }
    };
    let a = &state.audition;
    let spec = state.library.as_ref().and_then(|l| l.spec_of(&state.path));
    format!(
        "{{\"path\":{},\"library_spec\":{},\"type\":{},\"name\":{},\"description\":{},\"text\":{},{body},\"patterns\":{},\"audition\":{{\"pattern\":{},\"note\":{},\"bpm\":{},\"lane\":{}}},\"lanes\":{}}}",
        json_str(&state.path.display().to_string()),
        spec.map_or("null".into(), |s| json_str(&s)),
        json_str(sound.kind()),
        json_str(&sound.name),
        json_str(&sound.description),
        json_str(&sound.text()),
        json_strs(audition::patterns_for(sound).iter().map(|p| p.as_str())),
        json_str(a.pattern_for(sound).as_str()),
        json_str(&a.note),
        json_num(a.bpm),
        json_str(&a.lane),
        json_strs(audition::LANES.iter().copied())
    )
}

fn json_strs<'a>(items: impl Iterator<Item = &'a str>) -> String {
    format!("[{}]", items.map(json_str).collect::<Vec<_>>().join(","))
}

/// Every node a graph can be built from, for the page's "add a node": the
/// ones an instrument's line can be written with.
fn kinds_json() -> String {
    let items: Vec<String> = sound::graph_kinds()
        .into_iter()
        .map(|n| {
            format!(
                "{{\"name\":{},\"category\":{},\"doc\":{}}}",
                json_str(n.name),
                json_str(n.category.as_str()),
                json_str(n.doc)
            )
        })
        .collect();
    format!("[{}]", items.join(","))
}

/// The library as the page lists it.
fn library_json(state: &State) -> String {
    let Some(lib) = &state.library else {
        return fail("no sounds/ library found from the file's directory upward; start with --library <dir>");
    };
    let current = lib.spec_of(&state.path);
    let items: Vec<String> = lib
        .list()
        .iter()
        .map(|e| format!("{{\"type\":{},\"name\":{}}}", json_str(&e.kind), json_str(&e.name)))
        .collect();
    format!(
        "{{\"ok\":true,\"root\":{},\"current\":{},\"sounds\":[{}]}}",
        json_str(&lib.root.display().to_string()),
        current.map_or("null".into(), |c| json_str(&c)),
        items.join(",")
    )
}

/// A stereo bar as the page loops it: a 16-bit WAV, base64, at the same
/// peak as a hit.
fn bar_json(l: &[f32], r: &[f32]) -> String {
    let peak = l.iter().chain(r.iter()).fold(0.0f32, |m, v| m.max(v.abs())).max(1e-9);
    let nl: Vec<f32> = l.iter().map(|v| v / peak * 0.7).collect();
    let nr: Vec<f32> = r.iter().map(|v| v / peak * 0.7).collect();
    let wav = tatum_core::wav::encode_stereo_16(&nl, &nr, SAMPLE_RATE as u32);
    format!("{{\"audio\":{},\"ms\":{}}}", json_str(&base64(&wav)), json_num(l.len() as f32 * 1000.0 / SAMPLE_RATE))
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
        "{{\"audio\":{},\"pitch\":{},\"level\":{},\"ms\":{},\"click\":{}}}",
        json_str(&base64(&wav)),
        json_pairs(&c.pitch),
        json_pairs(&c.level),
        json_num(y.len() as f32 * 1000.0 / SAMPLE_RATE),
        // The click by the measure `tatum analyze` uses, read on this one
        // hit: yours and a record's averaged hit compare. What `analyze`
        // prints for the record need not be this: it reads every hit apart
        // and takes the median, which keeps what the average thins out.
        json_num(tatum_debug::fit::click_db(y))
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
    /// The file Save writes: the one the designer was started on, or one
    /// opened or made in the library.
    path: PathBuf,
    sound: Sound,
    /// What the loop plays.
    audition: Audition,
    /// The record's averaged hit, once one is loaded: what Fit fits to.
    reference: Option<Vec<f32>>,
    library: Option<Library>,
}

/// What a sound that compiles and plays nothing is told: a graph with
/// nothing wired to `out`, mostly; a module with its level at nothing.
fn silent(sound: &Sound) -> &'static str {
    match &sound.body {
        Body::Graph(_) => "it makes no sound: is anything wired to out?",
        Body::Module(_) if audition::is_drums(sound) => "it makes no sound on any lane: is its level at 0?",
        Body::Module(_) => "it makes no sound: is its level at 0?",
    }
}

/// The same for what the loop plays: on a drum kit, the one lane.
fn silent_hit(state: &State) -> String {
    if audition::is_drums(&state.sound) {
        format!("it makes no sound on the {} lane: is its level at 0?", state.audition.lane)
    } else {
        silent(&state.sound).to_string()
    }
}

fn fail(e: &str) -> String {
    format!("{{\"ok\":false,\"error\":{}}}", json_str(e))
}

/// The sound rendered as the loop plays it, with its text and its knobs;
/// when it does not compile, the error, with the text and knobs as they are.
fn render_reply(state: &State) -> String {
    let rendered = state
        .audition
        .hit(&state.sound)
        .and_then(|h| if h.iter().any(|v| *v != 0.0) { Ok(h) } else { Err(silent_hit(state)) })
        .and_then(|h| Ok((h, state.audition.bar(&state.sound)?)));
    let text = json_str(&state.sound.text());
    let sound = sound_json(state);
    match rendered {
        Ok((y, (l, r))) => format!(
            "{{\"ok\":true,\"text\":{text},\"hit\":{},\"bar\":{},\"sound\":{sound}}}",
            hit_json(&y),
            bar_json(&l, &r)
        ),
        Err(e) => format!("{{\"ok\":false,\"error\":{},\"text\":{text},\"sound\":{sound}}}", json_str(&e)),
    }
}

/// The sound's text, when it compiles, reads back as itself and makes a
/// sound: what a file has to do before it is written. A drum kit makes one
/// when any of its lanes does, whichever the loop is playing.
fn compiles(state: &State) -> Result<String, String> {
    let text = state.sound.text();
    let again = Sound::parse(&text)?;
    if !again.same_as(&state.sound) {
        return Err("the text does not read back as the same sound".into());
    }
    let pattern = state.audition.pattern_for(&state.sound);
    SongEngine::from_source(&state.audition.source(&state.sound, pattern, state.audition.bpm))?;
    if !state.audition.sounds(&state.sound)? {
        return Err(silent(&state.sound).into());
    }
    Ok(text)
}

/// Save: the file written, only when the sound compiles.
fn save_reply(state: &State) -> String {
    let text = match compiles(state) {
        Ok(t) => t,
        Err(e) => return fail(&format!("not saved: {e}")),
    };
    let result = (|| {
        if let Some(dir) = state.path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(&state.path, &text).map_err(|e| e.to_string())
    })();
    match result {
        Ok(()) => {
            eprintln!("saved {}", state.path.display());
            format!("{{\"ok\":true,\"path\":{}}}", json_str(&state.path.display().to_string()))
        }
        Err(e) => fail(&e),
    }
}

/// Open `type/name` from the library: the designer switched to it, as long
/// as it reads as a sound.
fn open_reply(state: &mut State, spec: &str) -> String {
    let Some(lib) = &state.library else { return fail("there is no library to open from") };
    let path = match lib.existing(spec) {
        Ok(p) => p,
        Err(e) => return fail(&e),
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => return fail(&format!("cannot read '{}': {e}", spec.trim())),
    };
    match Sound::parse(&text) {
        Ok(s) => {
            state.sound = s;
            state.path = path;
            render_reply(state)
        }
        Err(e) => fail(&format!("'{}' is not a sound the designer can open:\n{e}", spec.trim())),
    }
}

/// The sound open, written to `type/name` in the library under a name of
/// its own, and opened there. Never over a file that is there, and only a
/// sound that compiles.
fn duplicate_reply(state: &mut State, spec: &str) -> String {
    let Some(lib) = &state.library else { return fail("there is no library to duplicate into") };
    let name = match library::split(spec) {
        Ok((_, name)) => name,
        Err(e) => return fail(&e),
    };
    // What a song plays it by: the file's name, as a name the language takes.
    let mut ident = name.replace('-', "_");
    if !ident.starts_with(|c: char| c.is_ascii_lowercase()) {
        ident = format!("s_{ident}");
    }
    let mut next = state.sound.clone();
    if let Err(e) = apply(&mut next, &format!("name={ident}")) {
        return fail(&e);
    }
    let before = std::mem::replace(&mut state.sound, next);
    let text = match compiles(state) {
        Ok(t) => t,
        Err(e) => {
            state.sound = before;
            return fail(&format!("not duplicated: {e}"));
        }
    };
    match lib.create(spec, &text) {
        Ok(path) => {
            eprintln!("created {}", path.display());
            state.path = path;
            render_reply(state)
        }
        Err(e) => {
            state.sound = before;
            fail(&e)
        }
    }
}

/// A record's hits, averaged: `from`, `to` (seconds), `bpm` (else measured)
/// and `ms` (how much of each hit) from the query, as `tatum kick fit`
/// takes them. The average is kept for Fit.
fn reference_reply(state: &mut State, query: &BTreeMap<String, String>, body: &[u8]) -> String {
    let get = |k: &str| query.get(k).and_then(|v| v.parse::<f32>().ok()).filter(|v| v.is_finite() && *v >= 0.0);
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
    let Some(reference) = state.reference.clone() else {
        return fail("load a record first: Fit fits the kick to its averaged hit");
    };
    let Body::Graph(g) = &mut state.sound.body else {
        return fail("Fit sets a kick() node's knobs, and this sound is a module");
    };
    let Some(node) = g.nodes.iter().position(|n| n.kind == "kick") else {
        return fail("there is no kick() node to fit; Fit sets the kick node's knobs");
    };
    let target = tatum_debug::fit::Target::new(reference);
    let fitted = match crate::kick::fit_kick(&target, 0.6) {
        Ok(f) => f,
        Err(e) => return fail(&e),
    };
    let k = &fitted.args;
    let args = &mut g.nodes[node].args;
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
    let sound = sound_json(state);
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

/// Connections read at once; past it, a new one is told to come back.
const MAX_CONNECTIONS: usize = 16;

/// One of the connections being read: counted while it lives, and given
/// back when it is dropped, so a thread that panics does not keep its place
/// and, after enough of them, stop the server from taking any.
struct Slot(std::sync::Arc<std::sync::atomic::AtomicUsize>);

impl Slot {
    fn take(open: &std::sync::Arc<std::sync::atomic::AtomicUsize>) -> Slot {
        open.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Slot(open.clone())
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// The longest body taken on `/api/reference`: a WAV of a long extended mix.
const MAX_REFERENCE: usize = 400 << 20;
/// The longest body taken anywhere else: a sound's text, a line of knobs.
const MAX_BODY: usize = 1 << 20;

/// How long a body may be, by what is asked: only a record is big, and a
/// GET has none.
fn body_cap(method: &str, path: &str) -> usize {
    match (method, path) {
        ("POST", "/api/reference") => MAX_REFERENCE,
        ("POST", _) => MAX_BODY,
        _ => 0,
    }
}

/// How long a request has to send its head, from the moment it is taken.
const HEAD_TIME: Duration = Duration::from_secs(10);
/// How long it then has for its body: a record, and anything else.
const REFERENCE_TIME: Duration = Duration::from_secs(120);
const BODY_TIME: Duration = Duration::from_secs(30);

/// A stream a request is read from against a clock: `allow` says how long
/// what is left of it may take, all told.
trait Hurried: Read {
    fn allow(&mut self, _time: Duration) {}
}

/// A connection with a deadline for the whole request, not for each read: a
/// client that drips a byte now and then is let go like one that stops.
struct Timed<'a> {
    stream: &'a TcpStream,
    until: Instant,
}

impl Read for Timed<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let left = self.until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
        self.stream.set_read_timeout(Some(left))?;
        self.stream.read(buf)
    }
}

impl Hurried for Timed<'_> {
    fn allow(&mut self, time: Duration) {
        self.until = Instant::now() + time;
    }
}

/// A request, read in two steps: its head, which says who sends it and how
/// much; and its body, only when the head is the designer's own page asking
/// for something that takes one that long. A request from anywhere else is
/// refused before a byte of its body is taken.
fn read_request(stream: &mut impl Hurried, port: u16) -> Result<Request, Refused> {
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
    let (path, q) = target.split_once('?').unwrap_or((&target, ""));
    let query = q.split('&').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let mut req = Request { method, path: path.to_string(), query, headers, body: Vec::new() };
    same_origin(&req, port).map_err(|why| Some(("403 Forbidden", why)))?;
    let cap = body_cap(&req.method, &req.path);
    if length > cap {
        return Err(Some(("413 Content Too Large", format!("{length} bytes; at most {cap} here"))));
    }
    stream.allow(if cap > MAX_BODY { REFERENCE_TIME } else { BODY_TIME });
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
    req.body = body;
    Ok(req)
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
///
/// Asked of the head alone, before the body is read.
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
    let body = String::from_utf8_lossy(&req.body).to_string();
    let reply = match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/") => return ("200 OK".into(), "text/html; charset=utf-8", PAGE.as_bytes().to_vec()),
        ("GET", "/api/sound") => sound_json(state),
        ("GET", "/api/kinds") => kinds_json(),
        ("GET", "/api/library") => library_json(state),
        // Knobs, and a graph's nodes and wires: see `sound::apply`.
        ("POST", "/api/set") => {
            let mut next = state.sound.clone();
            match apply(&mut next, &body) {
                Ok(()) => {
                    state.sound = next;
                    render_reply(state)
                }
                Err(e) => fail(&e),
            }
        }
        // The whole file, as typed.
        ("POST", "/api/text") => match Sound::parse(&body) {
            Ok(s) => {
                state.sound = s;
                render_reply(state)
            }
            Err(e) => fail(&e),
        },
        // `pattern=`, `note=`, `bpm=`, `lane=`: what the loop plays.
        ("POST", "/api/audition") => match state.audition.apply(&body, &state.sound) {
            Ok(()) => render_reply(state),
            Err(e) => fail(&e),
        },
        ("POST", "/api/note") => match state.audition.apply(&format!("note={}", body.trim()), &state.sound) {
            Ok(()) => render_reply(state),
            Err(e) => fail(&e),
        },
        ("POST", "/api/save") => save_reply(state),
        ("POST", "/api/open") => open_reply(state, &body),
        ("POST", "/api/duplicate") => duplicate_reply(state, &body),
        ("POST", "/api/reference") => reference_reply(state, &req.query, &req.body),
        ("POST", "/api/fit") => fit_reply(state),
        _ => return ("404 Not Found".into(), "text/plain", b"not here".to_vec()),
    };
    ("200 OK".into(), "application/json", reply.into_bytes())
}

fn usage() -> ! {
    eprintln!("Usage: tatum design <sound.synth> [--library <dir>] [--port N] [--no-open]");
    eprintln!("  Opens a designer for the sound in the browser. The file is one `instrument`");
    eprintln!("  or one `module` and nothing else; if it does not exist it starts as a deep");
    eprintln!("  kick, and Save creates it. The sounds/ library around it (or --library) is");
    eprintln!("  listed beside it.");
    process::exit(1);
}

pub fn cmd(args: &[String]) {
    let mut path = None;
    let mut library_dir = None;
    let mut port: u16 = 7350;
    let mut open = true;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--port" => {
                i += 1;
                port = args.get(i).and_then(|p| p.parse().ok()).unwrap_or_else(|| usage());
            }
            "--library" => {
                i += 1;
                library_dir = Some(PathBuf::from(args.get(i).unwrap_or_else(|| usage())));
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
    let library = Library::find(&path, library_dir.as_deref());
    if library.is_none() && library_dir.is_some() {
        eprintln!("--library: not a directory");
        process::exit(1);
    }
    let state = State { path, sound, audition: Audition::default(), reference: None, library };
    if let Err(e) = state.audition.hit(&state.sound) {
        eprintln!("{}: {e}", state.path.display());
        process::exit(1);
    }
    let listener = TcpListener::bind(("127.0.0.1", port)).unwrap_or_else(|e| {
        eprintln!("cannot listen on 127.0.0.1:{port}: {e} (another `tatum design`? try --port)");
        process::exit(1);
    });
    let url = format!("http://127.0.0.1:{port}/");
    eprintln!("designing {} at {url}  (Ctrl-C to stop)", state.path.display());
    if let Some(lib) = &state.library {
        eprintln!("library: {}", lib.root.display());
    }
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
    // Each connection is read on its own thread, and only a request read
    // whole takes the sound: a browser opens sockets ahead of time and may
    // leave one idle, and a turn of a knob must not wait behind it until it
    // times out. The requests themselves still go one at a time.
    let state = std::sync::Arc::new(std::sync::Mutex::new(state));
    let open = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        use std::sync::atomic::Ordering;
        if open.load(Ordering::SeqCst) >= MAX_CONNECTIONS {
            let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
            respond(&mut stream, "503 Service Unavailable", "text/plain", b"too many connections at once");
            continue;
        }
        let slot = Slot::take(&open);
        let state = state.clone();
        std::thread::spawn(move || {
            // Given back however the thread ends, a panic included.
            let _slot = slot;
            // A client that stops sending, or sends a byte now and then, is
            // let go: the whole request has a deadline.
            let _ = stream.set_write_timeout(Some(Duration::from_secs(30)));
            let read = read_request(&mut Timed { stream: &stream, until: Instant::now() + HEAD_TIME }, port);
            match read {
                Ok(req) => {
                    let (status, kind, body) = {
                        let mut state = state.lock().unwrap_or_else(|p| p.into_inner());
                        handle(&mut state, req)
                    };
                    respond(&mut stream, &status, kind, &body);
                }
                Err(Some((status, why))) => {
                    if status.starts_with("403") {
                        eprintln!("refused: {why}");
                    }
                    respond(&mut stream, status, "text/plain", why.as_bytes());
                }
                Err(None) => {}
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    use sound::note_name;

    fn state_of(text: &str, path: &Path, library: Option<Library>) -> State {
        State {
            path: path.to_path_buf(),
            sound: Sound::parse(text).expect("parses"),
            audition: Audition::default(),
            reference: None,
            library,
        }
    }

    fn post(state: &mut State, path: &str, body: &str) -> String {
        let req = Request {
            method: "POST".into(),
            path: path.into(),
            query: BTreeMap::new(),
            headers: BTreeMap::new(),
            body: body.as_bytes().to_vec(),
        };
        String::from_utf8(handle(state, req).2).expect("utf-8")
    }

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tatum-design-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("sounds")).expect("mkdir");
        d
    }

    #[test]
    fn a_sound_written_out_reads_back_the_same() {
        let text = "# A test.\ninstrument boom {\n  gain 0.9\n  pitch_osc sine(300, 50, 0.999) as body\n  body > perc(0.001, 0.3) as amp\n  noise() as n\n  n > highpass(3000hz, 0.5) > amp\n  amp > out\n}\n";
        let s = Sound::parse(text).expect("parses");
        let again = Sound::parse(&s.text()).expect("reads back");
        assert_eq!(s.text(), again.text());
        assert!(again.same_as(&s));
        assert_eq!(s.description, "A test.");
        let Body::Graph(g) = &s.body else { panic!("a graph") };
        assert_eq!(g.gain, Some(0.9));
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
        let y = Audition::default().hit(&s).expect("renders");
        // hold 100 + release 200: about 300 ms, cut where it is 70 dB down.
        let ms = y.len() as f32 * 1000.0 / SAMPLE_RATE;
        assert!((250.0..400.0).contains(&ms), "{ms} ms");
        let c = hit::curves(&y);
        let tail = c.pitch.iter().filter(|p| (120.0..200.0).contains(&p.0)).map(|p| p.1).sum::<f32>()
            / c.pitch.iter().filter(|p| (120.0..200.0).contains(&p.0)).count() as f32;
        assert!((tail - 46.25).abs() < 0.5, "F#1, got {tail}");
    }

    impl Hurried for std::io::Cursor<Vec<u8>> {}

    fn request(raw: &str) -> Result<Request, Refused> {
        read_request(&mut std::io::Cursor::new(raw.as_bytes().to_vec()), 7350)
    }

    fn status_of(raw: &str) -> &'static str {
        match request(raw) {
            Ok(_) => "ok",
            Err(Some((s, _))) => s,
            Err(None) => "dropped",
        }
    }

    /// A POST's head as the page sends it, up to the blank line.
    fn page_post(path: &str, length: usize) -> String {
        format!("POST {path} HTTP/1.1\r\nHost: 127.0.0.1:7350\r\nX-Tatum-Design: 1\r\nContent-Length: {length}\r\n\r\n")
    }

    #[test]
    fn a_body_too_long_short_or_unsaid_is_answered_not_dropped() {
        assert!(status_of(&page_post("/api/set", MAX_BODY + 1)).starts_with("413"));
        let short = format!("{}k.body=80", page_post("/api/set", 50));
        assert!(status_of(&short).starts_with("400"));
        let nonsense = "POST /api/set HTTP/1.1\r\nContent-Length: lots\r\n\r\n";
        assert!(status_of(nonsense).starts_with("400"));
        let head = format!("GET / HTTP/1.1\r\nX: {}\r\n", "a".repeat(70 << 10));
        assert!(status_of(&head).starts_with("431"));
        assert_eq!(status_of("\r\n\r\n"), "400 Bad Request");
        assert_eq!(status_of(""), "dropped");
        let fine = "POST /api/set?x=1 HTTP/1.1\r\nHost: 127.0.0.1:7350\r\nX-Tatum-Design: 1\r\nContent-Length: 9\r\n\r\nk.body=80";
        let r = request(fine).ok().expect("reads");
        assert_eq!(r.body, b"k.body=80");
        assert_eq!(r.headers.get("host").map(String::as_str), Some("127.0.0.1:7350"));
    }

    #[test]
    fn a_body_is_only_as_long_as_its_path_needs() {
        // A record is big; nothing else is, and a GET has no body at all.
        assert_eq!(body_cap("POST", "/api/reference"), MAX_REFERENCE);
        for path in ["/api/set", "/api/text", "/api/save", "/api/open", "/api/duplicate", "/api/audition", "/nowhere"] {
            assert_eq!(body_cap("POST", path), MAX_BODY, "{path}");
            assert!(status_of(&page_post(path, MAX_BODY + 1)).starts_with("413"), "{path}");
        }
        assert!(status_of(&page_post("/api/reference", MAX_REFERENCE + 1)).starts_with("413"));
        // Said to be within the cap and then not sent: 400, having read what there was.
        assert!(status_of(&page_post("/api/reference", MAX_BODY + 1)).starts_with("400"));
        let get = "GET /api/sound HTTP/1.1\r\nHost: 127.0.0.1:7350\r\nContent-Length: 10\r\n\r\n0123456789";
        assert!(status_of(get).starts_with("413"));
    }

    /// A sender that counts what is taken from it: a head, then as much
    /// body as anyone will read.
    struct Endless {
        head: std::io::Cursor<Vec<u8>>,
        taken: usize,
    }

    impl Read for Endless {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = match self.head.read(buf)? {
                0 => {
                    buf.fill(b'x');
                    buf.len()
                }
                n => n,
            };
            self.taken += n;
            Ok(n)
        }
    }

    impl Hurried for Endless {}

    #[test]
    fn a_request_from_elsewhere_is_refused_before_its_body_is_read() {
        let refused = |head: String| {
            let mut from = Endless { head: std::io::Cursor::new(head.clone().into_bytes()), taken: 0 };
            let status = match read_request(&mut from, 7350) {
                Err(Some((s, _))) => s,
                _ => "taken",
            };
            assert!(status.starts_with("403"), "{status}: {head}");
            assert!(from.taken <= head.len() + 8192, "{} bytes read of a request refused: {head}", from.taken);
        };
        let length = 300 << 20;
        // Another site's page: its Origin, no header; the body never read.
        refused(format!(
            "POST /api/save HTTP/1.1\r\nHost: 127.0.0.1:7350\r\nOrigin: https://evil.example\r\nContent-Length: {length}\r\n\r\n"
        ));
        // The one path that takes a long body takes it from the page only.
        refused(format!("POST /api/reference HTTP/1.1\r\nHost: 127.0.0.1:7350\r\nContent-Length: {length}\r\n\r\n"));
        refused(format!(
            "POST /api/reference HTTP/1.1\r\nHost: evil.example:7350\r\nX-Tatum-Design: 1\r\nContent-Length: {length}\r\n\r\n"
        ));
        refused(format!("POST /api/reference HTTP/1.1\r\nX-Tatum-Design: 1\r\nContent-Length: {length}\r\n\r\n"));
    }

    #[test]
    fn a_request_that_drips_is_let_go_at_its_deadline() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("listens");
        let port = listener.local_addr().expect("an address").port();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let done = stop.clone();
        // A byte every 50 ms: each read comes back long before any timeout
        // of its own, and the head never ends.
        let sender = std::thread::spawn(move || {
            let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connects");
            for b in b"GET / HTTP/1.1\r\nX-Slow: ".iter().chain(std::iter::repeat(&b'a')) {
                if done.load(std::sync::atomic::Ordering::SeqCst) || s.write_all(&[*b]).is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        let (stream, _) = listener.accept().expect("accepts");
        let start = Instant::now();
        let mut timed = Timed { stream: &stream, until: start + Duration::from_millis(400) };
        assert!(matches!(read_request(&mut timed, port), Err(None)), "let go, with no one to answer");
        let took = start.elapsed();
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(took >= Duration::from_millis(350), "before its time: {took:?}");
        assert!(took < Duration::from_secs(5), "held for {took:?}");
        // The body gets its own time once the head is the page's.
        timed.allow(Duration::from_secs(60));
        assert!(timed.until > Instant::now() + Duration::from_secs(50));
        drop(stream);
        sender.join().expect("the sender ends");
    }

    #[test]
    fn only_the_designers_own_page_is_answered() {
        let status = |head: &str| status_of(&format!("{head}\r\n\r\n"));
        let refused = |head: &str| status(head).starts_with("403");
        let page =
            "POST /api/save HTTP/1.1\r\nHost: 127.0.0.1:7350\r\nOrigin: http://127.0.0.1:7350\r\nX-Tatum-Design: 1";
        assert_eq!(status(page), "ok");
        assert_eq!(status("GET / HTTP/1.1\r\nHost: localhost:7350"), "ok");
        // A site on the web posting to the designer: its own Origin, and no header.
        let csrf = "POST /api/save HTTP/1.1\r\nHost: 127.0.0.1:7350\r\nOrigin: https://evil.example";
        assert!(refused(csrf));
        // A form posted with no Origin at all still lacks the header.
        assert!(refused("POST /api/save HTTP/1.1\r\nHost: 127.0.0.1:7350"));
        // The library's writes are POSTs like Save, and refused the same way.
        for path in ["/api/duplicate", "/api/open", "/api/audition"] {
            let csrf = format!("POST {path} HTTP/1.1\r\nHost: 127.0.0.1:7350\r\nOrigin: https://evil.example");
            assert!(refused(&csrf), "{path}");
        }
        // DNS rebinding: the page's own name pointed at 127.0.0.1.
        assert!(refused("GET /api/library HTTP/1.1\r\nHost: evil.example:7350"));
        // Another port is another server.
        assert!(refused("GET / HTTP/1.1\r\nHost: 127.0.0.1:9999"));
        assert!(refused("GET / HTTP/1.1"));
    }

    #[test]
    fn nan_and_infinity_are_not_numbers_here() {
        let mut s = Sound::parse(NEW_SOUND).expect("parses");
        for bad in ["k.body=NaN", "k.body=inf", "k.body=-infinity", "gain=nan", "gain=loud", "k.body=", "k.body"] {
            assert!(apply(&mut s, bad).is_err(), "{bad} was taken");
        }
        assert!(!s.text().to_lowercase().contains("nan"), "{}", s.text());
        apply(&mut s, "gain=0.8").expect("a gain");
        assert!(s.text().contains("gain 0.8"));
        apply(&mut s, "gain=").expect("no gain");
        assert!(!s.text().contains("gain"));
        let mut m = Sound::parse("module bass b { }\n").expect("parses");
        for bad in ["cutoff=NaN", "cutoff=inf", "env0.a=nan"] {
            assert!(apply(&mut m, bad).is_err(), "{bad} was taken");
        }
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
        let mut state = state_of(NEW_SOUND, Path::new("/nonexistent/k.synth"), None);
        assert!(fit_reply(&mut state).contains("load a record first"));
        let mut q = BTreeMap::new();
        q.insert("bpm".to_string(), "125".to_string());
        let r = reference_reply(&mut state, &q, &wav);
        assert!(r.starts_with("{\"ok\":true"), "{}", &r[..r.len().min(200)]);
        let r = fit_reply(&mut state);
        assert!(r.starts_with("{\"ok\":true"), "{}", &r[..r.len().min(300)]);
        let text = state.sound.text();
        assert!(text.contains("kick(A1,"), "{text}");
        let Body::Graph(g) = &state.sound.body else { panic!("a graph") };
        assert!((g.nodes[0].args["body"] - 75.0).abs() < 8.0, "{text}");
        // No kick node, or a module: said, not guessed.
        state.sound = Sound::parse("instrument x {\n  osc sine(110) as w\n  w > out\n}\n").expect("parses");
        assert!(fit_reply(&mut state).contains("no kick() node"));
        state.sound = Sound::parse("module bass b { }\n").expect("parses");
        assert!(fit_reply(&mut state).contains("is a module"));
    }

    #[test]
    fn a_bad_reference_query_is_not_taken_for_a_number() {
        let mut state = state_of(NEW_SOUND, Path::new("/nonexistent/k.synth"), None);
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
    fn a_connection_that_panics_gives_its_place_back() {
        let open = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        for _ in 0..MAX_CONNECTIONS + 4 {
            let slot = Slot::take(&open);
            let t = std::thread::spawn(move || {
                let _slot = slot;
                panic!("a request that went wrong");
            });
            assert!(t.join().is_err());
        }
        assert_eq!(open.load(std::sync::atomic::Ordering::SeqCst), 0);
        let held = Slot::take(&open);
        assert_eq!(open.load(std::sync::atomic::Ordering::SeqCst), 1);
        drop(held);
        assert_eq!(open.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn base64_is_base64() {
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
    }

    #[test]
    fn a_sound_that_does_not_compile_is_never_saved() {
        let d = temp("nosave");
        let path = d.join("sounds/kicks/k.synth");
        let mut state = state_of(NEW_SOUND, &path, Library::find(&path, None));
        // Reads, does not compile: a wire to a node that is not there.
        let r = post(&mut state, "/api/text", "instrument k {\n  kick(A1) as k\n  k > nowhere\n}\n");
        assert!(r.starts_with("{\"ok\":false"), "{}", &r[..r.len().min(200)]);
        assert!(r.contains("\"sound\":"), "the knobs stay with the error");
        let r = post(&mut state, "/api/save", "");
        assert!(r.contains("not saved"), "{r}");
        assert!(!path.exists(), "nothing written");
        // The same through the page's own buttons: the only wire to out taken away.
        let mut state = state_of(NEW_SOUND, &path, None);
        let r = post(&mut state, "/api/set", "+node lowpass f\n+wire k f\n+wire f zz");
        assert!(r.contains("no node named 'zz'"), "{r}");
        assert!(!state.sound.text().contains("lowpass"), "a refused change changes nothing");
        // Compiles, plays nothing: nothing wired to out.
        let mut state = state_of(NEW_SOUND, &path, None);
        let r = post(&mut state, "/api/set", "-wire k out");
        assert!(r.contains("makes no sound"), "{}", &r[..r.len().min(200)]);
        assert!(post(&mut state, "/api/save", "").contains("not saved: it makes no sound"));
        assert!(!path.exists());
        let mut state = state_of("module bass b { }\n", &path, None);
        state.sound = Sound::parse("module bass b { }\n").expect("parses");
        if let Body::Module(m) = &mut state.sound.body {
            // A value no file could hold, the way a bug would put it there.
            m.params.push(sound::ModuleParam { name: "cutoff".into(), value: 7.0, style: sound::Style::Bare });
        }
        assert!(post(&mut state, "/api/save", "").contains("not saved"));
        assert!(!path.exists());
        // Fixed, it saves.
        assert!(post(&mut state, "/api/set", "cutoff=0.4").starts_with("{\"ok\":true"));
        assert!(post(&mut state, "/api/save", "").starts_with("{\"ok\":true"));
        assert!(std::fs::read_to_string(&path).unwrap().contains("module bass b {"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_library_is_listed_opened_and_duplicated_only_inside_itself() {
        let d = temp("lib");
        std::fs::create_dir_all(d.join("sounds/bass")).unwrap();
        std::fs::write(d.join("sounds/bass/acid.synth"), "# Acid.\nmodule bass acid {\n  cutoff 800hz\n}\n").unwrap();
        std::fs::write(d.join("sounds/bass/broken.synth"), "module bass broken { cutof 1 }\n").unwrap();
        std::fs::write(d.join("outside.synth"), "module keys outside { }\n").unwrap();
        let path = d.join("sounds/kicks/deep.synth");
        let mut state = state_of(NEW_SOUND, &path, Library::find(&path, None));
        let get = |state: &mut State, p: &str| {
            let req = Request {
                method: "GET".into(),
                path: p.into(),
                query: BTreeMap::new(),
                headers: BTreeMap::new(),
                body: Vec::new(),
            };
            String::from_utf8(handle(state, req).2).unwrap()
        };
        let list = get(&mut state, "/api/library");
        assert!(list.contains("{\"type\":\"bass\",\"name\":\"acid\"}"), "{list}");
        // Opened: the designer is on it, a module with its knobs.
        let r = post(&mut state, "/api/open", "bass/acid");
        assert!(r.starts_with("{\"ok\":true") && r.contains("\"module\":\"bass\""), "{}", &r[..200]);
        assert!(state.path.ends_with("sounds/bass/acid.synth"));
        for bad in ["../outside", "/etc/passwd", "bass/../../outside", "bass/nope", "bass/broken", "bass", ""] {
            let r = post(&mut state, "/api/open", bad);
            assert!(r.starts_with("{\"ok\":false"), "{bad}: {r}");
            assert!(state.path.ends_with("sounds/bass/acid.synth"), "{bad} moved the designer");
        }
        // Duplicated: a new file under a name of its own, opened there.
        post(&mut state, "/api/set", "resonance=0.7");
        let r = post(&mut state, "/api/duplicate", "bass/acid-2");
        assert!(r.starts_with("{\"ok\":true"), "{}", &r[..r.len().min(200)]);
        let new = d.join("sounds/bass/acid-2.synth");
        let text = std::fs::read_to_string(&new).expect("written");
        assert!(text.contains("module bass acid_2 {") && text.contains("resonance 0.7"), "{text}");
        assert!(state.path.ends_with("sounds/bass/acid-2.synth"));
        for bad in ["bass/acid", "bass/acid-2", "../x", "/tmp/x", "bass/../x", "bass/x.synth", "Bass/x", "bass/a b"] {
            let r = post(&mut state, "/api/duplicate", bad);
            assert!(r.starts_with("{\"ok\":false"), "{bad}: {r}");
        }
        assert_eq!(std::fs::read_to_string(d.join("sounds/bass/acid.synth")).unwrap().lines().count(), 4);
        assert!(!d.join("x.synth").exists() && !d.join("sounds/x.synth").exists());
        // A sound that does not compile is not duplicated either.
        post(&mut state, "/api/text", "instrument k {\n  kick(A1) as k\n  k > nowhere\n}\n");
        assert!(post(&mut state, "/api/duplicate", "kicks/bad").contains("not duplicated"));
        assert!(!d.join("sounds/kicks/bad.synth").exists());
        // Without a library, nothing to list or write.
        let mut alone = state_of(NEW_SOUND, Path::new("/nonexistent/k.synth"), None);
        assert!(get(&mut alone, "/api/library").starts_with("{\"ok\":false"));
        assert!(post(&mut alone, "/api/duplicate", "kicks/x").starts_with("{\"ok\":false"));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The library's one-node pieces in the designer, as the page drives
    /// it: each opens with its knobs, is heard as a single hit, takes a turn
    /// of its main knob, and saves a file a song then plays.
    #[test]
    fn the_node_pieces_open_turn_and_save_in_the_designer() {
        let d = temp("pieces");
        for (kind, name, plays_as, knob, written) in [
            ("hats", "tick", "tick", "h.decay=300", "decay=300ms"),
            ("snares", "crack", "crack", "s.wires=260", "wires=260ms"),
            ("claps", "room", "room", "c.tail=500", "tail=500ms"),
            ("toms", "low", "low_tom", "t.release=450", "release=450ms"),
        ] {
            let text = std::fs::read_to_string(format!("../sounds/{kind}/{name}.synth")).expect("reads");
            let path = d.join(format!("sounds/{kind}/{name}.synth"));
            let mut state = state_of(&text, &path, Library::find(&path, None));
            let req = Request {
                method: "GET".into(),
                path: "/api/sound".into(),
                query: BTreeMap::new(),
                headers: BTreeMap::new(),
                body: Vec::new(),
            };
            let page = String::from_utf8(handle(&mut state, req).2).expect("utf-8");
            assert!(page.contains(&format!("\"{plays_as}\"")), "{name}: {}", &page[..page.len().min(300)]);
            let before = state.audition.hit(&state.sound).expect("a hit");
            assert!(before.iter().any(|v| v.abs() > 0.01), "{name} is heard");
            let r = post(&mut state, "/api/set", knob);
            assert!(r.starts_with("{\"ok\":true"), "{name}: {}", &r[..r.len().min(200)]);
            let after = state.audition.hit(&state.sound).expect("a hit");
            assert!(after.len() > before.len(), "{name}: {knob} made it longer ({} to {})", before.len(), after.len());
            assert!(post(&mut state, "/api/save", "").starts_with("{\"ok\":true"), "{name} saves");
            let saved = std::fs::read_to_string(&path).expect("saved");
            assert!(saved.contains(written), "{name}: {saved}");
            let song = format!("{saved}pattern p {{ C3 - - -  C3 - - - }}\ntrack t {{ play p using {plays_as} }}\n");
            let mut engine = SongEngine::from_source(&song).unwrap_or_else(|e| panic!("{name}: {e}"));
            let (l, _) = engine.render(1);
            assert!(l.iter().any(|v| v.abs() > 0.01), "{name}: the saved sound plays");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_song_in_the_library_is_not_opened_and_saved_as_its_one_instrument() {
        let d = temp("song");
        let song = "# A song.\ntempo 130\ninstrument song {\n  # the body\n  kick(A1) as k\n  k > out\n}\npattern demo { C3 - - -  C3 - - -  C3 - - -  C3 - - - }\ntrack main { play demo using song }\n";
        let noted = "# A kick.\ninstrument noted {\n  kick(A1) as k  # tuned by ear\n  k > out\n}\n";
        std::fs::create_dir_all(d.join("sounds/kicks")).unwrap();
        std::fs::write(d.join("sounds/kicks/song.synth"), song).unwrap();
        std::fs::write(d.join("sounds/kicks/noted.synth"), noted).unwrap();
        let path = d.join("sounds/kicks/deep.synth");
        let mut state = state_of(NEW_SOUND, &path, Library::find(&path, None));
        for (spec, why) in
            [("kicks/song", "more in it than a sound"), ("kicks/noted", "a comment after the ones on top")]
        {
            let r = post(&mut state, "/api/open", spec);
            assert!(r.starts_with("{\"ok\":false") && r.contains(why), "{spec}: {r}");
            assert!(state.path.ends_with("sounds/kicks/deep.synth"), "{spec} moved the designer");
            // Save goes to the sound that is open, never over the file refused.
            assert!(post(&mut state, "/api/save", "").starts_with("{\"ok\":true"));
        }
        assert_eq!(std::fs::read_to_string(d.join("sounds/kicks/song.synth")).unwrap(), song);
        assert_eq!(std::fs::read_to_string(d.join("sounds/kicks/noted.synth")).unwrap(), noted);
        // Typed into the page, the same: refused, the sound open kept.
        let r = post(&mut state, "/api/text", song);
        assert!(r.starts_with("{\"ok\":false") && r.contains("more in it than a sound"), "{r}");
        assert_eq!(state.sound.name, "kick");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_kit_is_saved_when_any_lane_sounds_whichever_one_is_playing() {
        let d = temp("kit");
        let path = d.join("sounds/drums/hats.synth");
        // A kit of hats and claps: the kick, the lane the loop starts on, is off.
        let mut state = state_of("module beats hats {\n  kick_level 0\n}\n", &path, None);
        assert_eq!(state.audition.lane, "kick");
        let r = post(&mut state, "/api/audition", "lane=kick");
        assert!(r.contains("no sound on the kick lane"), "{}", &r[..r.len().min(200)]);
        assert!(post(&mut state, "/api/save", "").starts_with("{\"ok\":true"), "the hats still play");
        assert!(std::fs::read_to_string(&path).unwrap().contains("kick_level 0"));
        // The whole kit off: nothing to save, and said of the kit, not of a wire.
        let mut state = state_of("module beats hats { }\n", &d.join("sounds/drums/off.synth"), None);
        let r = post(&mut state, "/api/set", "level=0");
        assert!(r.contains("\"sound\":"), "{}", &r[..r.len().min(300)]);
        post(&mut state, "/api/audition", "lane=crash");
        let r = post(&mut state, "/api/save", "");
        assert!(r.contains("not saved: it makes no sound on any lane"), "{r}");
        assert!(!d.join("sounds/drums/off.synth").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_designed_sound_is_played_by_a_song_that_uses_it() {
        let d = temp("use");
        let sounds = [
            (
                "kicks",
                "thud",
                "instrument thud {\n  kick(F#1, body=70ms, hold=100ms) as k\n  k > out\n}\n",
                "C3 - - -  C3 - - -  C3 - - -  C3 - - -",
            ),
            ("bass", "rubber", "module bass rubber { }\n", "A1 - A1 -  - - A1 -  A1 - - A1  - A1 - -"),
            ("fm", "bell", "module fm bell { algorithm two_op level 2.0 }\n", "A3 - - -  - - E4 -  - - - -  C4 - - -"),
            ("keys", "pad", "module keys pad { voice_mode poly attack 20ms }\n", "[A3 C4 E4] ..*15"),
            ("drums", "kit", "module beats kit { }\n", ""),
        ];
        for (kind, name, text, pattern) in sounds {
            let path = d.join(format!("sounds/{kind}/{name}.synth"));
            let mut state = state_of(text, &path, Library::find(&path, None));
            // Designed: a knob turned, a pattern chosen, saved.
            let knob = match state.sound.kind() {
                "graph" => "k.release=250",
                "beats" => "kick_decay=0.4",
                "fm" => "env0.d=0.3",
                _ => "attack=0.3",
            };
            let r = post(&mut state, "/api/set", &format!("description=Made in the designer.\n{knob}"));
            assert!(r.starts_with("{\"ok\":true"), "{kind}: {}", &r[..r.len().min(200)]);
            assert!(post(&mut state, "/api/save", "").starts_with("{\"ok\":true"), "{kind}");
            // A song beside the library that uses it.
            let pattern = if pattern.is_empty() {
                "pattern p {\n  kick: X - - -  X - - -  X - - -  X - - -\n  hat: - - x -  - - x -  - - x -  - - x -\n}"
                    .to_string()
            } else {
                format!("pattern p {{ {pattern} }}")
            };
            let song = format!(
                "use \"sounds/{kind}/{name}.synth\"\ntempo 124\n{pattern}\ntrack t {{ play p using {name} }}\n"
            );
            let song_path = d.join(format!("song_{name}.synth"));
            std::fs::write(&song_path, song).unwrap();
            let src = crate::include::Source::load(&song_path).expect("resolves the use");
            let mut engine = SongEngine::from_source(&src.text).unwrap_or_else(|e| panic!("{kind}: {e}"));
            let (l, r) = engine.render(2);
            let rms = (l.iter().chain(r.iter()).map(|v| v * v).sum::<f32>() / (2 * l.len()) as f32).sqrt();
            assert!(rms > 1e-3, "{kind}: the song is silent ({rms})");
        }
        let _ = std::fs::remove_dir_all(&d);
    }
}
