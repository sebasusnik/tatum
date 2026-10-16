//! The live session held whole, driven by messages: a planner, a player,
//! the files a song is made of, and the loop of `tatum compose` when one
//! is open. This is what a shell with no threads and no filesystem -- the
//! browser's AudioWorklet -- runs: every operation the terminal's live
//! session does is one JSON message in and one JSON answer out, and the
//! audio comes from [`Live::process`]. The CLI keeps its own threads and
//! channels; this is the single-threaded reading of the same planner.
//!
//! A message is `{"op": "...", ...}`. An answer is `{"ok": true, ...}` or
//! `{"ok": false, "error": "..."}`. The operations are the planner's, by
//! their names; what each takes and gives is in the match in [`Live::run`].

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::compose::{self, Action, Library, Session, GROOVE_ROLE};
use crate::dsl::ast::VoiceMove;
use crate::include::{dir_of, normalize, Files, Source};
use crate::json::{self, Obj, Value};
use crate::live::{Applied, LivePlanner, LivePlayer, Plan};
use crate::midi::{decode, Message, DRUM_CHANNEL};
use crate::song_engine::DslError;

/// The files handed over so far, by normalized path.
struct Map<'a>(&'a [(String, String)]);

impl Files for Map<'_> {
    fn read(&self, path: &str) -> Option<String> {
        self.0.iter().find(|(p, _)| p == path).map(|(_, t)| t.clone())
    }
}

pub struct Live {
    planner: LivePlanner,
    player: LivePlayer,
    files: Vec<(String, String)>,
    /// The file playing, once one was loaded.
    root: Option<String>,
    compose: Option<Session>,
    /// The compose row the controller's knobs are on.
    focus: usize,
}

impl Default for Live {
    fn default() -> Self {
        Self::new()
    }
}

impl Live {
    pub fn new() -> Self {
        Self {
            planner: LivePlanner::new(),
            player: LivePlayer::new(),
            files: Vec::new(),
            root: None,
            compose: None,
            focus: 1,
        }
    }

    pub fn player(&self) -> &LivePlayer {
        &self.player
    }

    pub fn player_mut(&mut self) -> &mut LivePlayer {
        &mut self.player
    }

    pub fn planner(&self) -> &LivePlanner {
        &self.planner
    }

    /// Render the next block. On the bar a swap lands, the loop's knobs
    /// are held on the new engine again, and what the player retired is
    /// freed here: there is one thread, so this is where it has to be.
    pub fn process(&mut self, out_l: &mut [f32], out_r: &mut [f32]) -> Option<usize> {
        let swapped = self.player.process(out_l, out_r);
        if swapped.is_some() {
            if let Some(session) = &self.compose {
                let generation = self.player.generation();
                for plan in session.reapply(&mut self.planner, generation) {
                    self.player.apply(plan);
                }
            }
        }
        while self.player.take_retired().is_some() {}
        swapped
    }

    /// One message in, one answer out.
    pub fn command(&mut self, text: &str) -> String {
        match self.run(text) {
            Ok(answer) => answer,
            Err(e) => Obj::new().bool("ok", false).str("error", &e).finish(),
        }
    }

    fn run(&mut self, text: &str) -> Result<String, String> {
        let msg = Value::parse(text)?;
        let op = msg.get("op").and_then(Value::str).ok_or("a message has an \"op\"")?;
        let generation = self.player.generation();
        match op {
            // `{"files": {"session.synth": "...", "grooves/dub.synth": "..."}}`:
            // files handed over, added to or replacing what was.
            "files" => {
                let files = msg.get("files").and_then(Value::obj).ok_or("files takes an object of path: text")?;
                for (path, text) in files {
                    let text = text.str().ok_or("a file's text is a string")?;
                    self.put_file(path, text);
                }
                Ok(ok().int("files", self.files.len() as i64).finish())
            }
            "forget" => {
                let path = normalize(msg.get("path").and_then(Value::str).ok_or("forget takes a path")?);
                self.files.retain(|(p, _)| *p != path);
                Ok(ok().finish())
            }
            // `{"root": "session.synth"}`: the song to play, its `use` lines
            // resolved over the files. A text that does not compile is
            // reported and the last good one keeps playing.
            "load" => {
                let root = msg.get("root").and_then(Value::str).ok_or("load takes a root")?;
                self.root = Some(normalize(root));
                self.load()
            }
            "reload" => self.load(),
            // `{"source": "..."}`: a whole text, no `use` lines, as the first
            // browser shell sent it. What `load` does for a file.
            "text" => {
                let source = msg.get("source").and_then(Value::str).ok_or("text takes a source")?;
                match self.planner.plan(source, generation) {
                    Ok(plan) => {
                        let kind = plan.describe();
                        let applied = self.player.apply(plan);
                        Ok(ok().str("plan", kind).str("applied", applied_word(&applied)).finish())
                    }
                    Err(err) => Ok(Obj::new().bool("ok", false).raw("errors", &errors_json(&err)).finish()),
                }
            }
            "start" => {
                self.player.start();
                Ok(ok().finish())
            }
            "stop" => {
                self.player.stop();
                Ok(ok().finish())
            }
            // `{"bytes": [176, 74, 100]}`: one message from a controller.
            "midi" => {
                let bytes: Vec<u8> = msg
                    .get("bytes")
                    .and_then(Value::arr)
                    .ok_or("midi takes bytes")?
                    .iter()
                    .filter_map(|b| b.int().map(|n| n as u8))
                    .collect();
                let Some(message) = decode(&bytes) else { return Ok(ok().strs("readings", &[]).finish()) };
                let readings = self.midi(message, generation);
                Ok(ok().strs("readings", &readings).finish())
            }
            "knob" => {
                let cc = int(&msg, "cc")? as u8;
                let value = int(&msg, "value")? as u8;
                let turn = self.planner.knob(cc, value, generation);
                self.send(turn.plans);
                Ok(ok().strs("readings", &turn.readings).finish())
            }
            "try_knob" => {
                let target = msg.get("target").and_then(Value::str).ok_or("try_knob takes a target")?.to_string();
                let value = int(&msg, "value")? as u8;
                let turn = self.planner.try_knob(&target, value, generation);
                self.send(turn.plans);
                Ok(ok().strs("readings", &turn.readings).finish())
            }
            // Where the text leaves a target and what it reads there.
            "probe_knob" => {
                let target = msg.get("target").and_then(Value::str).ok_or("probe_knob takes a target")?;
                match self.planner.probe_knob(target) {
                    Some(knob) => {
                        let rest = self.planner.song().and_then(|s| knob.rest_position(s));
                        let reading = rest.map(|r| knob.reading(r));
                        let mut out = ok().bool("found", true);
                        out = match rest {
                            Some(r) => out.int("rest", r as i64),
                            None => out.raw("rest", "null"),
                        };
                        Ok(out.opt_str("reading", reading.as_deref()).finish())
                    }
                    None => Ok(ok().bool("found", false).finish()),
                }
            }
            "mute" => {
                let track = msg.get("track").and_then(Value::str).ok_or("mute takes a track")?.to_string();
                let plans = self.planner.toggle_mute(&track, generation);
                self.send(plans);
                Ok(ok().finish())
            }
            "solo" => {
                let track = msg.get("track").and_then(Value::str).ok_or("solo takes a track")?.to_string();
                let plans = self.planner.toggle_solo(&track, generation);
                self.send(plans);
                Ok(ok().strs("soloed", self.planner.soloed()).finish())
            }
            "reset_knobs" => {
                let plans = self.planner.reset_knobs(generation);
                self.send(plans);
                Ok(ok().finish())
            }
            // A performance scene, with its values, now.
            "scene" => {
                let name = msg.get("name").and_then(Value::str).ok_or("scene takes a name")?.to_string();
                let plans =
                    self.planner.enter_scene(&name, generation).ok_or_else(|| format!("no scene named {}", name))?;
                self.send(plans);
                if let Some(values) = self.planner.scene_values(&name, None, generation) {
                    self.send(values);
                }
                Ok(ok().str("scene", &self.planner.describe_scene(&name)).finish())
            }
            "scenes" => Ok(ok().strs("scenes", &self.planner.scene_names()).finish()),
            "pages" => {
                let pages: Vec<String> = self
                    .planner
                    .pages()
                    .iter()
                    .map(|(name, on, alive)| {
                        Obj::new().str("name", name).bool("on", *on).bool("alive", *alive).finish()
                    })
                    .collect();
                Ok(ok().raw("pages", &json::array(&pages)).finish())
            }
            // `{"to": "next" | "prev" | "<page>"}`: the knobs onto a page.
            "voice" => {
                let to = msg.get("to").and_then(Value::str).ok_or("voice takes a page, or next or prev")?;
                let mv = match to {
                    "next" => VoiceMove::Next,
                    "prev" => VoiceMove::Prev,
                    name => VoiceMove::To(name.to_string()),
                };
                let said = self.planner.voice(&mv, generation).ok_or("no knob pages in this song")?;
                Ok(ok().str("said", &said).finish())
            }
            "position" => {
                let (bar, step, tempo) = match self.player.engine() {
                    Some(e) => (e.current_bar(), e.global_step(), e.tempo()),
                    None => (0, 0, 0.0),
                };
                Ok(ok()
                    .int("bar", bar as i64)
                    .int("step", step as i64)
                    .num("tempo", tempo as f64)
                    .int("generation", generation as i64)
                    .bool("running", self.player.running())
                    .int("swaps", self.player.swaps() as i64)
                    .finish())
            }
            "song" => self.song(),
            // The loop: the library is every `grooves/*.synth` and
            // `voices/<role>/*.synth` beside the session file among the
            // files handed over; the session is its text, or new.
            "compose_open" => {
                let root = normalize(msg.get("root").and_then(Value::str).ok_or("compose_open takes a root")?);
                let knob_ccs: Vec<u8> = match msg.get("knob_ccs").and_then(Value::arr) {
                    Some(ccs) => ccs.iter().filter_map(|c| c.int().map(|n| n as u8)).collect(),
                    None => compose::DEFAULT_KNOB_CCS.to_vec(),
                };
                let library = self.library(&root)?;
                let session = match Map(&self.files).read(&root) {
                    Some(text) => Session::from_text(library, &root, knob_ccs, &text),
                    None => {
                        let s = Session::new(library, &root, knob_ccs);
                        self.put_file(&root, &s.text());
                        s
                    }
                };
                let text = session.text();
                self.compose = Some(session);
                self.root = Some(root);
                self.focus = 1;
                let loaded = self.load()?;
                let view = self.view_json();
                Ok(ok().raw("loaded", &loaded).raw("view", &view).str("session", &text).finish())
            }
            // `{"action": "choose", "row": 1, "by": 1}`, `turn` with `knob`
            // and `value`, `mute`, `reset`, `compare`, `save` with `name`.
            "compose" => {
                let action = match msg.get("action").and_then(Value::str).ok_or("compose takes an action")? {
                    "choose" => Action::Choose { row: int(&msg, "row")? as usize, by: int(&msg, "by")? as i32 },
                    "turn" => Action::Turn {
                        row: int(&msg, "row")? as usize,
                        knob: int(&msg, "knob")? as usize,
                        value: int(&msg, "value")? as u8,
                    },
                    "mute" => Action::Mute(int(&msg, "row")? as usize),
                    "reset" => Action::Reset(int(&msg, "row")? as usize),
                    "compare" => Action::Compare,
                    "save" => Action::Save {
                        row: int(&msg, "row")? as usize,
                        name: msg.get("name").and_then(Value::str).ok_or("save takes a name")?.to_string(),
                    },
                    other => return Err(format!("no compose action called {}", other)),
                };
                let session = self.compose.as_mut().ok_or("no loop open: compose_open first")?;
                let outcome = session.act(action, &mut self.planner, generation)?;
                self.send(outcome.plans);
                let mut reload = false;
                let mut writes = Vec::new();
                for (path, text) in &outcome.writes {
                    self.put_file(path, text);
                    reload |= self.root.as_deref() == Some(normalize(path).as_str());
                    writes.push(Obj::new().str("path", path).str("text", text).finish());
                }
                let loaded = if reload { self.load()? } else { String::from("null") };
                let view = self.view_json();
                Ok(ok()
                    .str("said", &outcome.said)
                    .raw("writes", &json::array(&writes))
                    .raw("loaded", &loaded)
                    .raw("view", &view)
                    .finish())
            }
            // The rows and the knobs of the row chosen; `focus` chooses it,
            // for the controller's knobs too.
            "compose_view" => {
                if let Some(focus) = msg.get("focus").and_then(Value::int) {
                    self.focus = focus.max(0) as usize;
                }
                self.compose.as_ref().ok_or("no loop open: compose_open first")?;
                Ok(ok().raw("view", &self.view_json()).finish())
            }
            other => Err(format!("no operation called {}", other)),
        }
    }

    fn put_file(&mut self, path: &str, text: &str) {
        let path = normalize(path);
        match self.files.iter_mut().find(|(p, _)| *p == path) {
            Some(entry) => entry.1 = text.to_string(),
            None => self.files.push((path, text.to_string())),
        }
    }

    /// Plan the root over the files and hand the plan to the player.
    fn load(&mut self) -> Result<String, String> {
        let root = self.root.clone().ok_or("nothing loaded yet: load takes a root")?;
        let source = Source::load(&root, &Map(&self.files))?;
        let generation = self.player.generation();
        match self.planner.plan(&source.text, generation) {
            Ok(plan) => {
                let kind = plan.describe();
                let applied = self.player.apply(plan);
                Ok(ok().str("plan", kind).str("applied", applied_word(&applied)).strs("files", &source.files).finish())
            }
            Err(err) => Ok(Obj::new()
                .bool("ok", false)
                .raw("errors", &errors_json(&err))
                .strs("lines", &source.error_lines(&err))
                .finish()),
        }
    }

    fn send(&mut self, plans: Vec<Plan>) {
        for plan in plans {
            self.player.apply(plan);
        }
    }

    /// A controller's message, as the terminal's live loop reads it: on
    /// the compose screen a knob the loop maps turns the chosen row's macro.
    fn midi(&mut self, message: Message, generation: u64) -> Vec<String> {
        match message {
            Message::Cc(cc, value) => {
                let macro_on = self.compose.as_ref().and_then(|s| s.knob_index(cc));
                if let (Some(session), Some(knob)) = (self.compose.as_mut(), macro_on) {
                    let action = Action::Turn { row: self.focus, knob, value };
                    return match session.act(action, &mut self.planner, generation) {
                        Ok(outcome) => {
                            self.send(outcome.plans);
                            vec![outcome.said]
                        }
                        Err(e) => vec![e],
                    };
                }
                let turn = self.planner.knob(cc, value, generation);
                self.send(turn.plans);
                turn.readings
            }
            Message::Note { channel, note, velocity } => {
                let pad = channel == DRUM_CHANNEL;
                let plans = if pad {
                    self.planner.pad(note, velocity, generation)
                } else {
                    self.planner.key(note, velocity, generation)
                };
                match plans {
                    Some(plans) => {
                        self.send(plans);
                        Vec::new()
                    }
                    None if velocity > 0 => vec![format!("{} {} (not mapped)", if pad { "pad" } else { "key" }, note)],
                    None => Vec::new(),
                }
            }
            Message::Bend(value) => {
                let plans = self.planner.bend(value, generation);
                self.send(plans);
                vec![self.planner.bend_reading()]
            }
            Message::Pressure { channel, note, value } => {
                let plans = self.planner.pressure(channel == DRUM_CHANNEL, note, value);
                self.send(plans);
                Vec::new()
            }
            Message::Other { .. } => Vec::new(),
        }
    }

    /// The song as a screen needs it: its tracks with what each offers a
    /// knob, the knobs the text maps, its scenes.
    fn song(&self) -> Result<String, String> {
        let ast = self.planner.song().ok_or("nothing loaded yet")?;
        let engine = self.player.engine();
        let tracks: Vec<String> = ast
            .tracks
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let targets: Vec<String> = crate::knobs::targets(ast, &t.name)
                    .iter()
                    .map(|k| Obj::new().str("words", &k.words).str("target", &k.dotted()).str("doc", &k.doc).finish())
                    .collect();
                let mut o = Obj::new().str("name", &t.name).str("instrument", &t.using_instrument).str("play", &t.play);
                if let Some(e) = engine {
                    if i < e.track_count() {
                        o = o
                            .num("level", e.track_level(i) as f64)
                            .num("pan", e.track_pan(i) as f64)
                            .bool("muted", e.track_muted(i));
                    }
                }
                o.raw("knobs", &json::array(&targets)).finish()
            })
            .collect();
        let bindings: Vec<String> = crate::knobs::bindings(ast)
            .iter()
            .map(|(cc, target)| Obj::new().int("cc", *cc as i64).str("target", target).finish())
            .collect();
        let patterns: Vec<String> = ast.patterns.iter().map(|p| p.name.clone()).collect();
        Ok(ok()
            .num("tempo", ast.globals.tempo as f64)
            .raw("tracks", &json::array(&tracks))
            .raw("bindings", &json::array(&bindings))
            .strs("patterns", &patterns)
            .strs("scenes", &self.planner.scene_names())
            .finish())
    }

    /// The library beside `root` among the files: `grooves/*.synth` and
    /// `voices/<role>/*.synth`, relative to the session's directory.
    fn library(&self, root: &str) -> Result<Library, String> {
        let dir = dir_of(root);
        let prefix = if dir.is_empty() { String::new() } else { format!("{}/", dir) };
        let mut entries: Vec<(String, &str, &str)> = Vec::new();
        for (path, text) in &self.files {
            let Some(rel) = path.strip_prefix(&prefix) else { continue };
            if !rel.ends_with(".synth") {
                continue;
            }
            let parts: Vec<&str> = rel.split('/').collect();
            match parts.as_slice() {
                ["grooves", _] => entries.push((GROOVE_ROLE.to_string(), path, text)),
                ["voices", role, _] => entries.push((role.to_string(), path, text)),
                _ => {}
            }
        }
        entries.sort_by(|a, b| a.1.cmp(b.1));
        let (library, _skipped) = Library::from_files(entries.iter().map(|(r, p, t)| (r.as_str(), *p, *t)))?;
        Ok(library)
    }

    fn view_json(&self) -> String {
        let Some(session) = &self.compose else { return String::from("null") };
        let view = session.view(&self.planner, self.focus);
        let rows: Vec<String> = view
            .rows
            .iter()
            .map(|r| {
                let o = Obj::new()
                    .str("role", &r.role)
                    .opt_str("name", r.name.as_deref())
                    .str("kind", &r.kind)
                    .str("doc", &r.doc)
                    .str("track", &r.track);
                let o = match r.index {
                    Some(i) => o.int("index", i as i64),
                    None => o.raw("index", "null"),
                };
                o.int("count", r.count as i64).bool("held", r.held).finish()
            })
            .collect();
        let macros: Vec<String> = view
            .macros
            .iter()
            .map(|m| {
                Obj::new()
                    .str("label", &m.label)
                    .int("position", m.position as i64)
                    .str("reading", &m.reading)
                    .bool("held", m.held)
                    .finish()
            })
            .collect();
        let ccs: Vec<String> = view.knob_ccs.iter().map(|c| c.to_string()).collect();
        Obj::new()
            .int("focus", self.focus as i64)
            .raw("rows", &json::array(&rows))
            .raw("macros", &json::array(&macros))
            .bool("comparing", view.comparing)
            .raw("knob_ccs", &json::array(&ccs))
            .finish()
    }
}

fn ok() -> Obj {
    Obj::new().bool("ok", true)
}

fn int(msg: &Value, key: &str) -> Result<i64, String> {
    msg.get(key).and_then(Value::int).ok_or_else(|| format!("{} takes a number", key))
}

fn applied_word(applied: &Applied) -> &'static str {
    match applied {
        Applied::Unchanged => "unchanged",
        Applied::Fast => "fast",
        Applied::Loaded => "loaded",
        Applied::Queued => "queued",
        Applied::Stale => "stale",
        Applied::Control => "control",
    }
}

/// `[{"line": 3, "col": 0, "msg": "..."}]`, as `tatum_check` reports them.
fn errors_json(err: &DslError) -> String {
    let items: Vec<String> = match err {
        DslError::Parse(errs) => errs
            .iter()
            .map(|e| Obj::new().int("line", e.line as i64).int("col", e.col as i64).str("msg", &e.message).finish())
            .collect(),
        DslError::Compile(errs) => errs
            .iter()
            .map(|e| Obj::new().int("line", e.line as i64).int("col", 0).str("msg", &e.message).finish())
            .collect(),
    };
    json::array(&items)
}

/// Keeps the engine's boxes off the stack of whoever embeds this.
pub fn boxed() -> Box<Live> {
    Box::new(Live::new())
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::BLOCK_SIZE;
    use std::path::Path;

    /// The repo's `compose/` directory as the browser would hand it over.
    fn files_message() -> String {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../compose");
        let mut members = Vec::new();
        for sub in ["grooves", "voices/bass", "voices/lead", "voices/pad", "voices/perc"] {
            let mut names: Vec<_> = std::fs::read_dir(root.join(sub)).unwrap().flatten().map(|e| e.path()).collect();
            names.sort();
            for p in names {
                let text = std::fs::read_to_string(&p).unwrap();
                let rel = format!("{}/{}", sub, p.file_name().unwrap().to_string_lossy());
                members.push(format!("\"{}\":\"{}\"", json::escape(&rel), json::escape(&text)));
            }
        }
        format!("{{\"op\":\"files\",\"files\":{{{}}}}}", members.join(","))
    }

    fn render(live: &mut Live, blocks: usize) -> Vec<f32> {
        let (mut l, mut r) = ([0.0f32; BLOCK_SIZE], [0.0f32; BLOCK_SIZE]);
        let mut out = Vec::new();
        for _ in 0..blocks {
            live.process(&mut l, &mut r);
            out.extend_from_slice(&l);
        }
        out
    }

    #[test]
    fn a_message_that_is_not_one_is_refused_not_crashed() {
        let mut live = Live::new();
        let bad = Value::parse(&live.command("{\"op\":\"nope\"}")).unwrap();
        assert_eq!(bad.get("ok").and_then(Value::bool), Some(false));
        assert!(bad.get("error").unwrap().str().unwrap().contains("nope"));
        let bad = Value::parse(&live.command("not json")).unwrap();
        assert_eq!(bad.get("ok").and_then(Value::bool), Some(false));
        let bad = Value::parse(&live.command("{\"op\":\"reload\"}")).unwrap();
        assert!(bad.get("error").unwrap().str().unwrap().contains("nothing loaded"));
    }

    #[test]
    fn a_song_loads_over_its_use_lines_and_plays_the_same_samples_as_the_text_whole() {
        let mut live = Live::new();
        live.command(&files_message());
        let session = "use \"grooves/dub.synth\"\nuse \"voices/bass/sub_dub.synth\"\n";
        let msg =
            Obj::new().str("op", "files").raw("files", &Obj::new().str("session.synth", session).finish()).finish();
        live.command(&msg);
        let loaded = Value::parse(&live.command("{\"op\":\"load\",\"root\":\"session.synth\"}")).unwrap();
        assert_eq!(loaded.get("ok").and_then(Value::bool), Some(true), "{:?}", loaded);
        assert_eq!(loaded.get("applied").and_then(Value::str), Some("loaded"));
        live.command("{\"op\":\"start\"}");
        let heard = render(&mut live, 200);

        // The same text resolved on disk, played by a player of its own.
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../compose");
        let disk = std::fs::read_to_string(root.join("grooves/dub.synth")).unwrap()
            + &std::fs::read_to_string(root.join("voices/bass/sub_dub.synth")).unwrap();
        let mut planner = LivePlanner::new();
        let mut player = LivePlayer::new();
        player.apply(planner.plan(&disk, player.generation()).unwrap());
        player.start();
        let (mut l, mut r) = ([0.0f32; BLOCK_SIZE], [0.0f32; BLOCK_SIZE]);
        let mut expected = Vec::new();
        for _ in 0..200 {
            player.process(&mut l, &mut r);
            expected.extend_from_slice(&l);
        }
        assert_eq!(heard, expected, "the files map and the disk play different samples");
        assert!(heard.iter().any(|s| s.abs() > 0.01), "silence");

        let position = Value::parse(&live.command("{\"op\":\"position\"}")).unwrap();
        assert_eq!(position.get("running").and_then(Value::bool), Some(true));
        assert!(position.get("tempo").and_then(Value::num) > Some(121.0));
        let song = Value::parse(&live.command("{\"op\":\"song\"}")).unwrap();
        let tracks = song.get("tracks").unwrap().arr().unwrap();
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[1].get("name").and_then(Value::str), Some("bass"));
        let knobs = tracks[1].get("knobs").unwrap().arr().unwrap();
        assert_eq!(knobs[0].get("target").and_then(Value::str), Some("sub_dub.cutoff"));
    }

    #[test]
    fn a_save_that_does_not_compile_is_reported_and_the_last_good_one_plays_on() {
        let mut live = Live::new();
        live.command(&files_message());
        let good = Obj::new()
            .str("op", "files")
            .raw("files", &Obj::new().str("s.synth", "use \"grooves/dub.synth\"\n").finish())
            .finish();
        live.command(&good);
        assert!(live.command("{\"op\":\"load\",\"root\":\"s.synth\"}").contains("\"ok\":true"));
        let bad = Obj::new()
            .str("op", "files")
            .raw("files", &Obj::new().str("s.synth", "use \"grooves/dub.synth\"\ntempo nope\n").finish())
            .finish();
        live.command(&bad);
        let rejected = Value::parse(&live.command("{\"op\":\"reload\"}")).unwrap();
        assert_eq!(rejected.get("ok").and_then(Value::bool), Some(false));
        let lines = rejected.get("lines").unwrap().arr().unwrap();
        assert!(lines.iter().any(|l| l.str().unwrap().contains("s.synth:2")), "{:?}", lines);
        assert!(live.player.engine().is_some(), "the last good engine is gone");
        let missing = Obj::new()
            .str("op", "files")
            .raw("files", &Obj::new().str("s.synth", "use \"grooves/nope.synth\"\n").finish())
            .finish();
        live.command(&missing);
        let err = Value::parse(&live.command("{\"op\":\"reload\"}")).unwrap();
        assert!(err.get("error").unwrap().str().unwrap().contains("nope.synth"));
    }

    #[test]
    fn the_loop_opens_chooses_turns_and_saves_through_messages() {
        let mut live = Live::new();
        live.command(&files_message());
        let opened = Value::parse(&live.command("{\"op\":\"compose_open\",\"root\":\"session.synth\"}")).unwrap();
        assert_eq!(opened.get("ok").and_then(Value::bool), Some(true), "{:?}", opened);
        let view = opened.get("view").unwrap();
        let rows = view.get("rows").unwrap().arr().unwrap();
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[1].get("name").and_then(Value::str), Some("acid"));
        assert!(live.files.iter().any(|(p, _)| p == "session.synth"), "a new session file was written");
        live.command("{\"op\":\"start\"}");

        // The lead's next voice: the session file changes and reloads.
        let chosen =
            Value::parse(&live.command("{\"op\":\"compose\",\"action\":\"choose\",\"row\":2,\"by\":1}")).unwrap();
        assert!(chosen.get("said").unwrap().str().unwrap().starts_with("lead: bell"));
        assert_eq!(chosen.get("loaded").unwrap().get("plan").and_then(Value::str), Some("swap"));
        let session_text = live.files.iter().find(|(p, _)| p == "session.synth").map(|(_, t)| t.clone()).unwrap();
        assert!(session_text.contains("use \"voices/lead/bell.synth\""), "{session_text}");

        // The controller's first knob turns the chosen row's first macro.
        live.command("{\"op\":\"compose_view\",\"focus\":1}");
        let turned = Value::parse(&live.command("{\"op\":\"midi\",\"bytes\":[176,74,127]}")).unwrap();
        let readings = turned.get("readings").unwrap().arr().unwrap();
        assert!(readings[0].str().unwrap().starts_with("bass cutoff"), "{:?}", readings);
        let view = Value::parse(&live.command("{\"op\":\"compose_view\"}")).unwrap();
        let macros = view.get("view").unwrap().get("macros").unwrap().arr().unwrap();
        assert_eq!(macros[0].get("position").and_then(Value::int), Some(127));
        assert_eq!(macros[0].get("held").and_then(Value::bool), Some(true));

        // Saved: the new voice is a file to write, and the session plays it.
        let saved =
            Value::parse(&live.command("{\"op\":\"compose\",\"action\":\"save\",\"row\":1,\"name\":\"acid_open\"}"))
                .unwrap();
        assert_eq!(saved.get("ok").and_then(Value::bool), Some(true), "{:?}", saved);
        let writes = saved.get("writes").unwrap().arr().unwrap();
        assert_eq!(writes[0].get("path").and_then(Value::str), Some("voices/bass/acid_open.synth"));
        assert!(writes[0].get("text").unwrap().str().unwrap().contains("cutoff 20000hz"));
        assert_eq!(saved.get("loaded").unwrap().get("plan").and_then(Value::str), Some("swap"));
        let probe = Value::parse(&live.command("{\"op\":\"probe_knob\",\"target\":\"bass.cutoff\"}")).unwrap();
        assert_eq!(
            probe.get("rest").and_then(Value::int),
            Some(127),
            "the saved text leaves the knob where it was turned"
        );
    }
}
