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
use crate::dsl::ast::{PatternDef, Step, VoiceMove};
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
            // `tracks` is where each track is in the pattern it plays, the
            // step sounding now: a track's count starts over when a new
            // text takes over, so the song's step is not where a pattern is.
            "position" => {
                let (bar, step, tempo, tracks) = match self.player.engine() {
                    Some(e) => {
                        let mut tracks = Obj::new();
                        for i in 0..e.track_count() {
                            tracks = tracks.int(e.track_name(i), e.track_step(i) as i64);
                        }
                        (e.current_bar(), e.global_step(), e.tempo(), tracks.finish())
                    }
                    None => (0, 0, 0.0, String::from("{}")),
                };
                Ok(ok()
                    .int("bar", bar as i64)
                    .int("step", step as i64)
                    .raw("tracks", &tracks)
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
                    "solo" => Action::Solo(int(&msg, "row")? as usize),
                    // `{"action": "mix", "row": 1, "what": "pan", "value": 80}`
                    "mix" => Action::Mix {
                        row: int(&msg, "row")? as usize,
                        what: msg.get("what").and_then(Value::str).ok_or("mix takes what")?.to_string(),
                        value: int(&msg, "value")?.clamp(0, 127) as u8,
                    },
                    "reset" => Action::Reset(int(&msg, "row")? as usize),
                    "compare" => Action::Compare,
                    "save" => Action::Save {
                        row: int(&msg, "row")? as usize,
                        name: msg.get("name").and_then(Value::str).ok_or("save takes a name")?.to_string(),
                    },
                    // `{"action": "scale", "root": "A", "kind": "minor"}`;
                    // without a root, the groove's scale again.
                    "scale" => Action::Scale(msg.get("root").and_then(Value::str).map(|root| {
                        (root.to_string(), msg.get("kind").and_then(Value::str).unwrap_or("minor").to_string())
                    })),
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
            // The pattern a track plays, as a grid: one lane, or one per
            // drum, every step as its token and what it is.
            "pattern_get" => {
                let track = msg.get("track").and_then(Value::str).ok_or("pattern_get takes a track")?.to_string();
                Ok(ok().raw("pattern", &self.pattern_json(&track)?).finish())
            }
            // `{"track", "lane", "step", "token"}`, or `{"track", "edits":
            // [{"lane", "step", "token"}, ...]}`: steps of the track's
            // pattern rewritten, in the file that defines the pattern, and
            // the song reloaded once. A token the parser refuses, or a file
            // that does not compile with them, is refused and nothing
            // changes: the edits are taken whole or not at all.
            "pattern_set" => {
                let track = msg.get("track").and_then(Value::str).ok_or("pattern_set takes a track")?.to_string();
                let edit = |m: &Value| -> Result<(usize, usize, String), String> {
                    let token = m.get("token").and_then(Value::str).ok_or("pattern_set takes a token")?;
                    Ok((int(m, "lane")? as usize, int(m, "step")? as usize, token.to_string()))
                };
                let edits = match msg.get("edits").and_then(Value::arr) {
                    Some(list) => list.iter().map(edit).collect::<Result<Vec<_>, _>>()?,
                    None => vec![edit(&msg)?],
                };
                if edits.is_empty() {
                    return Err(String::from("pattern_set takes at least one edit"));
                }
                let (writes, loaded) = self.pattern_set(&track, &edits)?;
                let writes: Vec<String> =
                    writes.iter().map(|(p, t)| Obj::new().str("path", p).str("text", t).finish()).collect();
                Ok(ok()
                    .raw("writes", &json::array(&writes))
                    .raw("loaded", &loaded)
                    .raw("pattern", &self.pattern_json(&track)?)
                    .finish())
            }
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

    /// The pattern `track` plays first, from the latest text.
    fn pattern_of(&self, track: &str) -> Result<PatternDef, String> {
        let ast = self.planner.song().ok_or("nothing loaded yet")?;
        let def =
            ast.tracks.iter().rev().find(|t| t.name == track).ok_or_else(|| format!("no track called {}", track))?;
        ast.patterns
            .iter()
            .rev()
            .find(|p| p.name == def.play)
            .cloned()
            .ok_or_else(|| format!("{} plays {}, which is not a pattern", track, def.play))
    }

    /// The pattern as a grid, with the song's scale: its root's pitch class
    /// and its seven intervals, so a piano roll can lay out a row per degree
    /// and every note carries the MIDI note it sounds.
    fn pattern_json(&self, track: &str) -> Result<String, String> {
        let p = self.pattern_of(track)?;
        let song = self.planner.song().ok_or("nothing loaded yet")?;
        let scale = crate::dsl::compiler::scale_context(song);
        let (lanes, length): (Vec<String>, usize) = if p.lane_labels.is_empty() {
            let steps: Vec<&Step> = p.rows.iter().flatten().collect();
            (vec![lane_json(None, &steps, scale)], steps.len())
        } else {
            let lanes: Vec<String> = p
                .lane_labels
                .iter()
                .zip(&p.rows)
                .map(|(label, row)| lane_json(Some(label), &row.iter().collect::<Vec<_>>(), scale))
                .collect();
            (lanes, p.rows.iter().map(Vec::len).max().unwrap_or(0))
        };
        let intervals: Vec<String> = scale.0.iter().map(|i| i.to_string()).collect();
        let name = song.globals.scale.as_ref().map(|s| format!("{} {}", s.root, s.kind));
        Ok(Obj::new()
            .str("track", track)
            .str("name", &p.name)
            .bool("drums", !p.lane_labels.is_empty())
            .int("steps_per_bar", 16)
            .int("length", length as i64)
            .raw(
                "scale",
                &Obj::new()
                    .int("root", scale.1 as i64)
                    .raw("intervals", &json::array(&intervals))
                    .opt_str("name", name.as_deref())
                    .finish(),
            )
            .raw("lanes", &json::array(&lanes))
            .finish())
    }

    /// Steps of `track`'s pattern set to their tokens, in the file that
    /// defines the pattern; the song reloaded over it once. Returns the
    /// file to write and the reload's answer.
    fn pattern_set(
        &mut self,
        track: &str,
        edits: &[(usize, usize, String)],
    ) -> Result<(Vec<(String, String)>, String), String> {
        let mut p = self.pattern_of(track)?;
        let drums = !p.lane_labels.is_empty();
        for (lane, step, token) in edits {
            let (lane, step) = (*lane, *step);
            // The token through the parser itself, as the one step of a
            // pattern of its own: every form the language has, with its
            // own errors.
            let probe = if drums {
                format!("pattern _t {{ {}: {} }}\n", p.lane_labels.get(lane).ok_or("no such lane")?, token)
            } else {
                format!("pattern _t {{ {} }}\n", token)
            };
            let parsed = crate::dsl::parse(&probe)
                .map_err(|e| e.first().map(|e| e.message.clone()).unwrap_or_else(|| String::from("not a step")))?;
            let steps: Vec<Step> =
                parsed.patterns.first().map(|t| t.rows.iter().flatten().cloned().collect()).unwrap_or_default();
            if steps.len() != 1 {
                return Err(format!("`{}` is {} steps; one at a time", token, steps.len()));
            }
            let new = steps.into_iter().next().unwrap_or(Step::Rest);
            if drums {
                let row = p.rows.get_mut(lane).ok_or("no such lane")?;
                *row.get_mut(step).ok_or("no such step")? = new;
            } else {
                let mut at = step;
                let mut placed = false;
                for row in &mut p.rows {
                    if at < row.len() {
                        row[at] = new.clone();
                        placed = true;
                        break;
                    }
                    at -= row.len();
                }
                if !placed {
                    return Err(String::from("no such step"));
                }
            }
        }
        // The file that defines the pattern: the last one in `use` order
        // that has its block, since a later definition wins.
        let root = self.root.clone().ok_or("nothing loaded yet")?;
        let source = Source::load(&root, &Map(&self.files))?;
        let mut found = None;
        for path in source.files.iter().rev() {
            let Some(text) = Map(&self.files).read(path) else { continue };
            if let Some((start, end)) = compose::edit::block(&text, "pattern", &p.name) {
                found = Some((path.clone(), text, start, end));
                break;
            }
        }
        let (path, before, start, end) = found.ok_or_else(|| format!("no file defines pattern {}", p.name))?;
        let after = format!("{}{}{}", &before[..start], crate::dsl::write::pattern_body(&p), &before[end..]);
        self.put_file(&path, &after);
        let loaded = self.load()?;
        if Value::parse(&loaded).ok().and_then(|v| v.get("ok").and_then(Value::bool)) != Some(true) {
            self.put_file(&path, &before);
            let _ = self.load();
            let why = Value::parse(&loaded)
                .ok()
                .and_then(|v| {
                    v.get("lines")
                        .and_then(Value::arr)
                        .map(|l| l.iter().filter_map(Value::str).map(String::from).collect::<Vec<_>>())
                })
                .unwrap_or_default();
            return Err(format!("that does not compile: {}", why.join(" · ")));
        }
        Ok((vec![(path, after)], loaded))
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
                o.int("count", r.count as i64)
                    .bool("held", r.held)
                    .bool("muted", self.planner.is_muted(&r.track))
                    .bool("soloed", self.planner.soloed().contains(&r.track))
                    .finish()
            })
            .collect();
        let knob = |m: &compose::MacroRow| {
            Obj::new()
                .str("label", &m.label)
                .int("position", m.position as i64)
                .str("reading", &m.reading)
                .bool("held", m.held)
                .finish()
        };
        let macros: Vec<String> = view.macros.iter().map(knob).collect();
        // A channel per row, level, pan and sends; `[]` for a row with nothing.
        let mix: Vec<String> =
            view.mix.iter().map(|channel| json::array(&channel.iter().map(knob).collect::<Vec<_>>())).collect();
        let ccs: Vec<String> = view.knob_ccs.iter().map(|c| c.to_string()).collect();
        // The song's key as the text has it, whether the session set it
        // over the groove's, and every scale the language knows.
        let scale = self.planner.song().and_then(|s| s.globals.scale.clone());
        let key = Obj::new()
            .opt_str("root", scale.as_ref().map(|s| s.root.as_str()))
            .opt_str("kind", scale.as_ref().map(|s| s.kind.as_str()))
            .bool("own", session.own_scale())
            .finish();
        let kinds: Vec<String> = crate::perform::scale::SCALES.iter().map(|(name, _)| json::string(name)).collect();
        Obj::new()
            .int("focus", self.focus as i64)
            .raw("rows", &json::array(&rows))
            .raw("macros", &json::array(&macros))
            .raw("mix", &json::array(&mix))
            .bool("comparing", view.comparing)
            .raw("knob_ccs", &json::array(&ccs))
            .raw("key", &key)
            .raw("scales", &json::array(&kinds))
            .finish()
    }
}

fn ok() -> Obj {
    Obj::new().bool("ok", true)
}

/// One lane of a pattern: every step as its token, and what it is for a
/// grid to draw it -- its kind, its note and the MIDI notes it sounds, its
/// velocity.
fn lane_json(label: Option<&str>, steps: &[&Step], scale: ([u8; 7], u8)) -> String {
    use crate::dsl::write::{note_ref_text, step_text};
    let midi = |n: &crate::dsl::ast::NoteRef| crate::dsl::compiler::resolve_note(n, &scale.0, scale.1).to_string();
    let items: Vec<String> = steps
        .iter()
        .map(|s| {
            let o = Obj::new().str("token", &step_text(s));
            match s {
                Step::Rest => o.str("kind", "rest"),
                Step::Tie => o.str("kind", "tie"),
                Step::Note(n) => {
                    let o = o
                        .str("kind", "note")
                        .str("note", &note_ref_text(&n.note))
                        .raw("midi", &json::array(&[midi(&n.note)]))
                        .bool("slide", n.slide);
                    match n.velocity {
                        Some(v) => o.num("velocity", v as f64),
                        None => o.raw("velocity", "null"),
                    }
                }
                Step::Chord(c) => {
                    let notes: Vec<String> = c.notes.iter().map(|n| note_ref_text(&n.note)).collect();
                    let midis: Vec<String> = c.notes.iter().map(|n| midi(&n.note)).collect();
                    let o = o.str("kind", "chord").str("note", &notes.join(" ")).raw("midi", &json::array(&midis));
                    match c.velocity {
                        Some(v) => o.num("velocity", v as f64),
                        None => o.raw("velocity", "null"),
                    }
                }
                Step::DrumHit(d) => o.str("kind", "hit").num("velocity", d.velocity as f64).int("roll", d.roll as i64),
                Step::Subdiv(notes) => {
                    let names: Vec<String> = notes.iter().map(|n| note_ref_text(&n.note)).collect();
                    let midis: Vec<String> = notes.iter().map(|n| midi(&n.note)).collect();
                    o.str("kind", "sub").str("note", &names.join(" ")).raw("midi", &json::array(&midis))
                }
                Step::DrumSub(_) => o.str("kind", "drumsub"),
            }
            .finish()
        })
        .collect();
    Obj::new().opt_str("label", label).raw("steps", &json::array(&items)).finish()
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

    #[test]
    fn a_pattern_reads_as_a_grid_and_a_step_set_rewrites_its_file() {
        let mut live = Live::new();
        live.command(&files_message());
        live.command("{\"op\":\"compose_open\",\"root\":\"session.synth\"}");
        let got = Value::parse(&live.command("{\"op\":\"pattern_get\",\"track\":\"bass\"}")).unwrap();
        let pattern = got.get("pattern").unwrap();
        assert_eq!(pattern.get("name").and_then(Value::str), Some("acid_line"));
        assert_eq!(pattern.get("drums").and_then(Value::bool), Some(false));
        assert_eq!(pattern.get("length").and_then(Value::int), Some(32));
        let steps = pattern.get("lanes").unwrap().arr().unwrap()[0].get("steps").unwrap().arr().unwrap();
        assert_eq!(steps[0].get("token").and_then(Value::str), Some("1.2:0.9"));
        assert_eq!(steps[0].get("kind").and_then(Value::str), Some("note"));
        assert_eq!(steps[2].get("slide").and_then(Value::bool), Some(true));

        // Step 1 of the bass line becomes a slide up; the voice file carries it.
        let set = Value::parse(
            &live.command("{\"op\":\"pattern_set\",\"track\":\"bass\",\"lane\":0,\"step\":1,\"token\":\"~5.2:0.7\"}"),
        )
        .unwrap();
        assert_eq!(set.get("ok").and_then(Value::bool), Some(true), "{:?}", set);
        let writes = set.get("writes").unwrap().arr().unwrap();
        assert_eq!(writes[0].get("path").and_then(Value::str), Some("voices/bass/acid.synth"));
        let text = writes[0].get("text").unwrap().str().unwrap();
        assert!(text.contains("1.2:0.9 ~5.2:0.7 ~2.2:0.8 1.2:0.6"), "{text}");
        assert!(text.contains("module bass acid {"), "the rest of the file is kept:\n{text}");
        let steps =
            set.get("pattern").unwrap().get("lanes").unwrap().arr().unwrap()[0].get("steps").unwrap().arr().unwrap();
        assert_eq!(steps[1].get("token").and_then(Value::str), Some("~5.2:0.7"));

        // The drums: a lane and a step; what is not one step is refused.
        let drums = Value::parse(&live.command("{\"op\":\"pattern_get\",\"track\":\"drums\"}")).unwrap();
        let lanes = drums.get("pattern").unwrap().get("lanes").unwrap().arr().unwrap();
        assert_eq!(lanes[0].get("label").and_then(Value::str), Some("kick"));
        let set = Value::parse(
            &live.command("{\"op\":\"pattern_set\",\"track\":\"drums\",\"lane\":0,\"step\":2,\"token\":\"o\"}"),
        )
        .unwrap();
        assert_eq!(set.get("ok").and_then(Value::bool), Some(true), "{:?}", set);
        assert!(writes_text(&set).contains("kick:"));
        let bad = Value::parse(
            &live.command("{\"op\":\"pattern_set\",\"track\":\"drums\",\"lane\":0,\"step\":2,\"token\":\"- -\"}"),
        )
        .unwrap();
        assert_eq!(bad.get("ok").and_then(Value::bool), Some(false));
        let bad = Value::parse(
            &live.command("{\"op\":\"pattern_set\",\"track\":\"bass\",\"lane\":0,\"step\":0,\"token\":\"nope!\"}"),
        )
        .unwrap();
        assert_eq!(bad.get("ok").and_then(Value::bool), Some(false), "{:?}", bad);
    }

    #[test]
    fn the_mixer_holds_a_channel_a_save_writes_it_and_solo_toggles() {
        let mut live = Live::new();
        live.command(&files_message());
        let opened = Value::parse(&live.command("{\"op\":\"compose_open\",\"root\":\"session.synth\"}")).unwrap();
        let mix = opened.get("view").unwrap().get("mix").unwrap().arr().unwrap();
        assert_eq!(mix[1].arr().unwrap().len(), 4, "level, pan and the two sends");
        assert_eq!(mix[1].arr().unwrap()[1].get("label").and_then(Value::str), Some("pan"));

        let panned = Value::parse(
            &live.command("{\"op\":\"compose\",\"action\":\"mix\",\"row\":1,\"what\":\"pan\",\"value\":100}"),
        )
        .unwrap();
        assert_eq!(panned.get("ok").and_then(Value::bool), Some(true), "{:?}", panned);
        let channel = panned.get("view").unwrap().get("mix").unwrap().arr().unwrap()[1].arr().unwrap().to_vec();
        assert_eq!(channel[1].get("position").and_then(Value::int), Some(100));
        assert_eq!(channel[1].get("held").and_then(Value::bool), Some(true));
        live.command("{\"op\":\"compose\",\"action\":\"mix\",\"row\":1,\"what\":\"level\",\"value\":60}");

        // The row's save writes the channel into the voice's track.
        let saved =
            Value::parse(&live.command("{\"op\":\"compose\",\"action\":\"save\",\"row\":1,\"name\":\"bass_mixed\"}"))
                .unwrap();
        assert_eq!(saved.get("ok").and_then(Value::bool), Some(true), "{:?}", saved);
        let voice = saved
            .get("writes")
            .unwrap()
            .arr()
            .unwrap()
            .iter()
            .find(|w| w.get("path").and_then(Value::str).is_some_and(|p| p.ends_with("bass_mixed.synth")))
            .map(|w| w.get("text").unwrap().str().unwrap().to_string())
            .unwrap();
        assert!(voice.contains("pan "), "{voice}");
        assert!(voice.contains("level "), "{voice}");

        let bad = live.command("{\"op\":\"compose\",\"action\":\"mix\",\"row\":1,\"what\":\"cutoff\",\"value\":10}");
        assert!(bad.contains("\"ok\":false"), "{bad}");

        let solo = Value::parse(&live.command("{\"op\":\"compose\",\"action\":\"solo\",\"row\":1}")).unwrap();
        let soloed = |v: &Value| {
            v.get("view").unwrap().get("rows").unwrap().arr().unwrap()[1].get("soloed").and_then(Value::bool)
        };
        assert_eq!(soloed(&solo), Some(true));
        let back = Value::parse(&live.command("{\"op\":\"compose\",\"action\":\"solo\",\"row\":1}")).unwrap();
        assert_eq!(soloed(&back), Some(false));
    }

    #[test]
    fn the_session_sets_the_songs_key_over_the_grooves_and_gives_it_back() {
        let mut live = Live::new();
        live.command(&files_message());
        let opened = Value::parse(&live.command("{\"op\":\"compose_open\",\"root\":\"session.synth\"}")).unwrap();
        let key = |v: &Value| {
            let k = v.get("view").unwrap().get("key").unwrap();
            (
                k.get("root").and_then(Value::str).unwrap_or("").to_string(),
                k.get("kind").and_then(Value::str).unwrap_or("").to_string(),
                k.get("own").and_then(Value::bool).unwrap(),
            )
        };
        let (groove_root, groove_kind, own) = key(&opened);
        assert!(!own);
        assert!(opened.get("view").unwrap().get("scales").unwrap().arr().unwrap().len() > 7);
        let first_midi = |live: &mut Live| {
            let got = Value::parse(&live.command("{\"op\":\"pattern_get\",\"track\":\"bass\"}")).unwrap();
            got.get("pattern").unwrap().get("lanes").unwrap().arr().unwrap()[0].get("steps").unwrap().arr().unwrap()[0]
                .get("midi")
                .unwrap()
                .arr()
                .unwrap()[0]
                .int()
                .unwrap()
        };
        let before = first_midi(&mut live);

        // A minor: the session carries the line, the degrees follow it.
        let set =
            Value::parse(&live.command("{\"op\":\"compose\",\"action\":\"scale\",\"root\":\"A\",\"kind\":\"minor\"}"))
                .unwrap();
        assert_eq!(set.get("ok").and_then(Value::bool), Some(true), "{:?}", set);
        assert!(writes_text(&set).contains("\nscale A minor\n"), "{}", writes_text(&set));
        assert_eq!(set.get("loaded").unwrap().get("ok").and_then(Value::bool), Some(true));
        assert_eq!(key(&set), (String::from("A"), String::from("minor"), true));
        let after = first_midi(&mut live);
        assert_eq!((after - 9).rem_euclid(12), 0, "the bass line's first degree is A now");
        assert_ne!(before, after);

        // Again, and the session still has one scale line, not two.
        let set =
            Value::parse(&live.command("{\"op\":\"compose\",\"action\":\"scale\",\"root\":\"D\",\"kind\":\"dorian\"}"))
                .unwrap();
        assert_eq!(writes_text(&set).matches("scale ").count(), 1, "{}", writes_text(&set));

        // No root: the groove's own scale back.
        let back = Value::parse(&live.command("{\"op\":\"compose\",\"action\":\"scale\"}")).unwrap();
        assert!(!writes_text(&back).contains("scale "), "{}", writes_text(&back));
        assert_eq!(key(&back), (groove_root, groove_kind, false));
        assert_eq!(first_midi(&mut live), before);

        let bad =
            Value::parse(&live.command("{\"op\":\"compose\",\"action\":\"scale\",\"root\":\"H\",\"kind\":\"minor\"}"))
                .unwrap();
        assert_eq!(bad.get("ok").and_then(Value::bool), Some(false));
    }

    #[test]
    fn a_pattern_carries_its_scale_and_takes_edits_whole_or_not_at_all() {
        let mut live = Live::new();
        live.command(&files_message());
        live.command("{\"op\":\"compose_open\",\"root\":\"session.synth\"}");
        let got = Value::parse(&live.command("{\"op\":\"pattern_get\",\"track\":\"bass\"}")).unwrap();
        let pattern = got.get("pattern").unwrap();
        let scale = pattern.get("scale").unwrap();
        assert_eq!(scale.get("intervals").unwrap().arr().unwrap().len(), 7);
        let root = scale.get("root").and_then(Value::int).unwrap();
        let steps = pattern.get("lanes").unwrap().arr().unwrap()[0].get("steps").unwrap().arr().unwrap();
        // `1.2` is the root two octaves up from octave 0's: (2 + 1) * 12 + root.
        let first = steps[0].get("midi").unwrap().arr().unwrap()[0].int().unwrap();
        assert_eq!(first, 36 + root);

        // A note four steps long, written in one reload.
        let set = Value::parse(&live.command(
            "{\"op\":\"pattern_set\",\"track\":\"bass\",\"edits\":[\
             {\"lane\":0,\"step\":0,\"token\":\"3.2\"},{\"lane\":0,\"step\":1,\"token\":\"..\"},\
             {\"lane\":0,\"step\":2,\"token\":\"..\"},{\"lane\":0,\"step\":3,\"token\":\"..\"}]}",
        ))
        .unwrap();
        assert_eq!(set.get("ok").and_then(Value::bool), Some(true), "{:?}", set);
        assert!(writes_text(&set).contains("3.2 .. .. .."), "{}", writes_text(&set));

        // One bad token and none of the edits land.
        let before = Value::parse(&live.command("{\"op\":\"pattern_get\",\"track\":\"bass\"}")).unwrap();
        let bad = Value::parse(&live.command(
            "{\"op\":\"pattern_set\",\"track\":\"bass\",\"edits\":[\
             {\"lane\":0,\"step\":0,\"token\":\"5.2\"},{\"lane\":0,\"step\":1,\"token\":\"nope!\"}]}",
        ))
        .unwrap();
        assert_eq!(bad.get("ok").and_then(Value::bool), Some(false));
        let after = Value::parse(&live.command("{\"op\":\"pattern_get\",\"track\":\"bass\"}")).unwrap();
        assert_eq!(before, after);
    }

    /// Render until the song's position satisfies `until`, 128 samples at a time.
    fn render_until(live: &mut Live, until: impl Fn(&Value) -> bool) -> Value {
        let (mut l, mut r) = ([0.0f32; 128], [0.0f32; 128]);
        for _ in 0..200_000 {
            live.process(&mut l, &mut r);
            let pos = Value::parse(&live.command("{\"op\":\"position\"}")).unwrap();
            if until(&pos) {
                return pos;
            }
        }
        panic!("the song never got there");
    }

    #[test]
    fn the_position_says_where_each_track_is_in_its_pattern_after_a_swap() {
        let mut live = Live::new();
        live.command(&files_message());
        live.command("{\"op\":\"compose_open\",\"root\":\"session.synth\"}");
        live.command("{\"op\":\"start\"}");
        let len = Value::parse(&live.command("{\"op\":\"pattern_get\",\"track\":\"bass\"}"))
            .unwrap()
            .get("pattern")
            .unwrap()
            .get("length")
            .and_then(Value::int)
            .unwrap();
        assert_eq!(len, 32, "a two-bar line, so a swap on an odd bar lands mid-pattern");
        let bar = |p: &Value| p.get("bar").and_then(Value::int).unwrap();
        let bass = |p: &Value| p.get("tracks").unwrap().get("bass").and_then(Value::int).unwrap();

        // Before any swap the track and the song agree.
        let pos = render_until(&mut live, |p| bar(p) == 0 && p.get("step").and_then(Value::int) == Some(5));
        assert_eq!(bass(&pos), 4, "the step sounding is the one before the song's next");

        // An edit in bar 2 swaps in on bar 3, and the bass line starts over there.
        render_until(&mut live, |p| bar(p) == 2);
        let set = live.command("{\"op\":\"pattern_set\",\"track\":\"bass\",\"lane\":0,\"step\":4,\"token\":\"5.2\"}");
        assert!(set.contains("\"ok\":true"), "{set}");
        let pos = render_until(&mut live, |p| bar(p) == 3 && p.get("step").and_then(Value::int) == Some(3 * 16 + 5));
        assert_eq!(bass(&pos), 4, "{:?}", pos);
        let song_step = pos.get("step").and_then(Value::int).unwrap();
        assert_ne!((song_step - 1) % len, bass(&pos), "the song's step is not where the pattern is");
    }

    fn writes_text(answer: &Value) -> String {
        answer.get("writes").unwrap().arr().unwrap()[0].get("text").unwrap().str().unwrap().to_string()
    }
}
