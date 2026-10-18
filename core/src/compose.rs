//! A loop to try voices in. A groove plays; each role -- bass, lead, pad,
//! perc -- holds one voice from a library, swapped on the next bar line; a
//! handful of knobs per voice turn the parameters a hand reaches for first;
//! a voice that sounds right is saved as a small `.synth` of its own, which
//! any song then gets with one `use` line.
//!
//! Everything here is text and the planner: what a voice file is, which
//! knobs it offers, what the session file says, what to write when a voice
//! is saved. It has no files of its own. Whoever drives it -- the terminal
//! screen, a browser -- reads the library in, hands each [`Action`] over,
//! and writes out what the [`Outcome`] says. The session file is a list of
//! `use` lines: choosing a voice rewrites it, the live session watches it
//! like any other save, and the swap lands on the bar line. A knob turned
//! is held on the engine, not written, until the voice is saved: then its
//! values go into the new file, in units.

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use crate::include::{dir_of, join, use_target};
use crate::live::{Generation, LivePlanner, Plan};
use crate::params::{self, ModuleKind, Range};

/// The knobs of a KeyLab Essential mk3, left to right: what the eight
/// macros of the chosen voice sit on unless the caller says otherwise.
pub const DEFAULT_KNOB_CCS: [u8; 8] = [74, 71, 76, 77, 93, 18, 19, 16];

/// Roles in the order the screen lists them; any other role follows,
/// alphabetically.
const ROLE_ORDER: &[&str] = &["bass", "lead", "pad", "perc"];

/// The role a groove's file plays: its drums.
pub const GROOVE_ROLE: &str = "drums";

const HEADER: &str =
    "# tatum compose: the loop. The `use` lines are rewritten from the screen; every other line is kept.";

/// The module parameters a hand reaches for first, per kind of module.
/// Each voice gets these as its first knobs, then its level and sends,
/// then the wet of every node its chain names.
fn first_params(kind: ModuleKind) -> &'static [&'static str] {
    match kind {
        ModuleKind::Bass => &["cutoff", "resonance", "cutoff_env", "decay", "glide"],
        ModuleKind::Keys => &["cutoff", "resonance", "attack", "release", "detune"],
        ModuleKind::Fm => &["mod_index", "feedback", "decay", "release", "lfo_depth"],
        ModuleKind::Beats => &["kick_decay", "kick_pitch", "kick_drive", "hihat_decay", "hihat_level"],
    }
}

/// The kind as the text writes it.
fn kind_name(kind: ModuleKind) -> &'static str {
    match kind {
        ModuleKind::Bass => "bass",
        ModuleKind::Keys => "keys",
        ModuleKind::Fm => "fm",
        ModuleKind::Beats => "beats",
    }
}

/// A voice in the library: one file holding one module and one track named
/// by its role. A groove is the same with the role `drums`, plus the
/// globals, the sends and the master.
#[derive(Debug, Clone)]
pub struct Voice {
    pub role: String,
    /// The file's stem: `sub_dub` of `voices/bass/sub_dub.synth`.
    pub name: String,
    pub path: String,
    pub text: String,
    pub kind: ModuleKind,
    pub module: String,
    pub track: String,
    /// The file's first comment line, for the screen.
    pub doc: String,
    /// The `as` names of its chain, for a wet knob each.
    pub nodes: Vec<String>,
    /// Every track of the file that plays a module, `track` first: a voice
    /// of layers mutes, solos and changes its pattern as one.
    pub tracks: Vec<String>,
    /// The patterns the file writes, the one its track plays first: the
    /// variations a row chooses from.
    pub patterns: Vec<String>,
}

impl Voice {
    /// One file read: its module, its track and the names its chain gives.
    pub fn parse(role: &str, path: &str, text: &str) -> Result<Self, String> {
        let song = crate::dsl::parse(text)
            .map_err(|e| format!("{}: {}", path, e.first().map(|e| e.to_string()).unwrap_or_default()))?;
        let name = crate::include::file_name(path).trim_end_matches(".synth").to_string();
        let def = song
            .tracks
            .iter()
            .find(|t| !t.using_instrument.is_empty())
            .ok_or_else(|| format!("{}: no track plays a module", path))?;
        let module = song.module_defs.iter().find(|m| m.name == def.using_instrument).ok_or_else(|| {
            format!("{}: track {} uses {}, which the file does not define", path, def.name, def.using_instrument)
        })?;
        let kind = ModuleKind::from_str(&module.module_type)
            .ok_or_else(|| format!("{}: unknown module kind {}", path, module.module_type))?;
        let doc = text
            .lines()
            .map(str::trim)
            .find(|l| l.starts_with('#'))
            .map(|l| l.trim_start_matches('#').trim().to_string())
            .unwrap_or_default();
        let tracks: Vec<String> = core::iter::once(def.name.clone())
            .chain(
                song.tracks
                    .iter()
                    .filter(|t| !t.using_instrument.is_empty() && t.name != def.name)
                    .map(|t| t.name.clone()),
            )
            .collect();
        let mut patterns = vec![def.play.clone()];
        patterns.extend(song.patterns.iter().map(|p| p.name.clone()).filter(|p| *p != def.play));
        Ok(Self {
            tracks,
            patterns,
            role: role.to_string(),
            name,
            path: path.to_string(),
            text: text.to_string(),
            kind,
            module: module.name.clone(),
            track: def.name.clone(),
            doc,
            nodes: def.routing.iter().filter_map(|n| n.label.clone()).collect(),
        })
    }

    /// What the voice offers a knob, likeliest first: `bass cutoff`,
    /// `bass level`, `bass drive wet`. Words, as a `midi` line writes them.
    pub fn macros(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = first_params(self.kind)
            .iter()
            .filter(|p| params::lookup(self.kind, p).is_some())
            .map(|p| (p.to_string(), format!("{} {}", self.track, p)))
            .collect();
        for what in ["level", "reverb_send", "delay_send"] {
            out.push((what.to_string(), format!("{} {}", self.track, what)));
        }
        for node in &self.nodes {
            out.push((format!("{} wet", node), format!("{} {} wet", self.track, node)));
        }
        out
    }
}

/// Every groove and voice there is: `grooves/*.synth` and
/// `voices/<role>/*.synth`, read by whoever has the files.
pub struct Library {
    pub grooves: Vec<Voice>,
    pub voices: Vec<Voice>,
    pub roles: Vec<String>,
}

impl Library {
    /// From files already read: `(role, path, text)` each, in the order to
    /// list them; `drums` are the grooves. A file that is not a voice is
    /// skipped and named in the second half of the result.
    pub fn from_files<'a>(
        files: impl IntoIterator<Item = (&'a str, &'a str, &'a str)>,
    ) -> Result<(Self, Vec<String>), String> {
        let mut grooves = Vec::new();
        let mut voices = Vec::new();
        let mut skipped = Vec::new();
        for (role, path, text) in files {
            match Voice::parse(role, path, text) {
                Ok(v) if role == GROOVE_ROLE => grooves.push(v),
                Ok(v) => voices.push(v),
                Err(e) => skipped.push(e),
            }
        }
        if grooves.is_empty() {
            return Err(String::from("no grooves: a groove is a .synth with a drums track"));
        }
        let mut roles: Vec<String> = Vec::new();
        for v in &voices {
            if !roles.contains(&v.role) {
                roles.push(v.role.clone());
            }
        }
        roles.sort_by_key(|r| (ROLE_ORDER.iter().position(|o| o == r).unwrap_or(ROLE_ORDER.len()), r.clone()));
        Ok((Self { grooves, voices, roles }, skipped))
    }

    /// The voices of one role, in the order they were given.
    pub fn of(&self, role: &str) -> Vec<&Voice> {
        self.voices.iter().filter(|v| v.role == role).collect()
    }

    /// A voice written: it replaces one of the same role and name, or joins.
    fn put(&mut self, voice: Voice) {
        let list = if voice.role == GROOVE_ROLE { &mut self.grooves } else { &mut self.voices };
        match list.iter().position(|v| v.role == voice.role && v.name == voice.name) {
            Some(i) => list[i] = voice,
            None => list.push(voice),
        }
    }
}

/// One row of the loop: the role it draws from, which voice it holds, the
/// pattern it plays when not the voice's own, and the knobs turned on it
/// since, by target (`acid.cutoff`) and controller position. A role can
/// have several rows: a sub and a reese together.
#[derive(Debug, Clone)]
pub struct Slot {
    pub role: String,
    pub voice: Option<String>,
    pub play: Option<String>,
    pub held: Vec<(String, u8)>,
}

impl Slot {
    fn empty(role: &str) -> Self {
        Self { role: role.to_string(), voice: None, play: None, held: Vec::new() }
    }
}

/// One row of the loop as a screen draws it: the groove, or a role and the
/// voice it holds.
pub struct Row {
    pub role: String,
    /// `None`: the role holds nothing.
    pub name: Option<String>,
    pub kind: String,
    pub doc: String,
    pub track: String,
    /// Which of the library's this is, from 1, and how many it offers.
    pub index: Option<usize>,
    pub count: usize,
    /// A knob was turned on it and not saved.
    pub held: bool,
    /// The pattern it plays, and every one its voice offers, its own first.
    pub play: String,
    pub patterns: Vec<String>,
}

/// One knob of the chosen row.
pub struct MacroRow {
    pub label: String,
    pub position: u8,
    pub reading: String,
    pub held: bool,
}

/// What a mixer has of every row, in this order.
pub const MIX: [&str; 4] = ["level", "pan", "reverb_send", "delay_send"];

/// What a screen draws: every row, and the knobs of the row chosen.
pub struct View {
    pub rows: Vec<Row>,
    pub macros: Vec<MacroRow>,
    /// Every row's channel, [`MIX`] in order; empty for a row with nothing.
    pub mix: Vec<Vec<MacroRow>>,
    /// Every parameter of the chosen row's module, by name.
    pub params: Vec<MacroRow>,
    pub comparing: bool,
    pub knob_ccs: Vec<u8>,
}

/// What a key or a knob asks of the session.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// The next (or previous) voice in the row's library, on the next bar.
    Choose {
        row: usize,
        by: i32,
    },
    /// Knob `knob` of `row` put at `value`, held on the engine.
    Turn {
        row: usize,
        knob: usize,
        value: u8,
    },
    Mute(usize),
    /// Every knob of the row back where the text has it.
    Reset(usize),
    /// Every knob turned heard at the text's value, then back: A/B.
    Compare,
    /// The row's voice written with its knobs as a voice called `name`.
    Save {
        row: usize,
        name: String,
    },
    /// A channel of the mixer: `what`, one of [`MIX`], of `row`'s track put
    /// at `value`, held like a knob and written by the row's save.
    Mix {
        row: usize,
        what: String,
        value: u8,
    },
    /// The row's track alone, or back with the rest.
    Solo(usize),
    /// The song in another key: `Some((root, kind))` writes `scale root
    /// kind` into the session, after the groove's, so it wins; `None` takes
    /// it out and the groove's scale is the song's again. Patterns written
    /// in degrees follow it; notes by name do not.
    Scale(Option<(String, String)>),
    /// The song at another tempo: `Some(bpm)` writes `tempo bpm` into the
    /// session, after the groove's, so it wins; `None` takes it out.
    Tempo(Option<f32>),
    /// A row more for `role`, holding the first of its voices the loop
    /// does not play yet.
    Add(String),
    /// The row out of the loop. Not the groove's.
    Remove(usize),
    /// The next (or previous) pattern the row's voice offers, on the next
    /// bar: its variations, its own first.
    Vary {
        row: usize,
        by: i32,
    },
    /// Any parameter of the row's module put at `value`, held like a knob
    /// and written by the row's save.
    Set {
        row: usize,
        param: String,
        value: u8,
    },
    /// A voice from nothing: a module of `kind` at its defaults, a pattern
    /// to start from, a track, written as `voices/<role>/<name>.synth`,
    /// and a row for it.
    New {
        role: String,
        kind: String,
        name: String,
    },
}

/// What an action came to: plans for the engine, a line for the log, and
/// files to write -- `(path, text)` -- which the caller writes; the session
/// file among them is what the watcher turns into a swap.
pub struct Outcome {
    pub plans: Vec<Plan>,
    pub said: String,
    pub writes: Vec<(String, String)>,
}

impl Outcome {
    fn said(said: impl Into<String>) -> Self {
        Self { plans: Vec::new(), said: said.into(), writes: Vec::new() }
    }
}

/// The loop: a groove, a voice per role, and the file that says so.
pub struct Session {
    pub library: Library,
    /// The session file, as the caller's `Files` names it.
    pub path: String,
    pub groove: usize,
    /// Knobs turned on the groove's drums, like a slot's.
    pub groove_held: Vec<(String, u8)>,
    /// The groove's pattern when not its own.
    pub groove_play: Option<String>,
    pub slots: Vec<Slot>,
    /// What the `use` lines put before `grooves/` and `voices/`: `../../`
    /// for a loop of a set, `sets/jungle/01-intro.synth`, over the library
    /// two directories up.
    base: String,
    /// Lines of the session file that are not its `use` lines, kept as they
    /// are: the file is the user's too.
    extra: Vec<String>,
    pub knob_ccs: Vec<u8>,
    /// `a`: the knobs turned are heard at the text's values instead, to
    /// compare.
    pub comparing: bool,
}

impl Session {
    /// A session that does not exist yet: the first groove and the first
    /// bass. Its file is [`Session::text`], to write.
    pub fn new(library: Library, path: &str, knob_ccs: Vec<u8>) -> Self {
        Self::new_in(library, path, dir_of(path), knob_ccs)
    }

    /// A new session at `path` over the library in `library_dir`, an
    /// ancestor of `path`'s directory.
    pub fn new_in(library: Library, path: &str, library_dir: &str, knob_ccs: Vec<u8>) -> Self {
        let mut session = Self::empty_in(library, path, library_dir, knob_ccs);
        session.fill_roles();
        if let Some(slot) = session.slots.iter_mut().find(|s| s.role == "bass") {
            slot.voice = session.library.of("bass").first().map(|v| v.name.clone());
        }
        session
    }

    fn empty_in(library: Library, path: &str, library_dir: &str, knob_ccs: Vec<u8>) -> Self {
        let dir = dir_of(path);
        let below = dir.strip_prefix(library_dir).unwrap_or(dir).trim_start_matches('/');
        let depth = below.split('/').filter(|p| !p.is_empty()).count();
        Self {
            slots: Vec::new(),
            library,
            path: path.to_string(),
            groove: 0,
            groove_held: Vec::new(),
            groove_play: None,
            base: "../".repeat(depth),
            extra: Vec::new(),
            knob_ccs,
            comparing: false,
        }
    }

    /// An empty row for every role that has none, so each can be chosen,
    /// after the rows that play: the loop keeps the order it was built in.
    fn fill_roles(&mut self) {
        for role in self.library.roles.clone() {
            if !self.slots.iter().any(|s| s.role == role) {
                self.slots.push(Slot::empty(&role));
            }
        }
    }

    pub fn from_text(library: Library, path: &str, knob_ccs: Vec<u8>, text: &str) -> Self {
        Self::from_text_in(library, path, dir_of(path), knob_ccs, text)
    }

    /// The session its file describes: a row per `use` line, by where it
    /// points, and the pattern a `track <name> { play <pattern> }` line
    /// gives it; a line that points at nothing in the library, or says
    /// anything else, is kept as it is.
    pub fn from_text_in(library: Library, path: &str, library_dir: &str, knob_ccs: Vec<u8>, text: &str) -> Self {
        let mut session = Self::empty_in(library, path, library_dir, knob_ccs);
        let prefix = if library_dir.is_empty() { String::new() } else { format!("{}/", library_dir) };
        let mut plays: Vec<(String, String, String)> = Vec::new();
        for line in text.lines() {
            if line.trim_start().starts_with("# tatum compose") {
                continue;
            }
            if let Some((track, pattern)) = play_line(line) {
                plays.push((track, pattern, line.to_string()));
                continue;
            }
            let Some(Ok(target)) = use_target(line) else {
                session.extra.push(line.to_string());
                continue;
            };
            let full = join(path, &target);
            let rel = full.strip_prefix(&prefix).unwrap_or(&full);
            let parts: Vec<&str> = rel.split('/').collect();
            let stem = |s: &str| s.trim_end_matches(".synth").to_string();
            match parts.as_slice() {
                ["grooves", name] => match session.library.grooves.iter().position(|g| g.name == stem(name)) {
                    Some(i) => session.groove = i,
                    None => session.extra.push(line.to_string()),
                },
                ["voices", role, name] if session.library.of(role).iter().any(|v| v.name == stem(name)) => {
                    session.slots.push(Slot { voice: Some(stem(name)), ..Slot::empty(role) });
                }
                _ => session.extra.push(line.to_string()),
            }
        }
        for (track, pattern, line) in plays {
            let groove = &session.library.grooves[session.groove];
            if groove.tracks.contains(&track) && groove.patterns.contains(&pattern) {
                session.groove_play = (pattern != groove.patterns[0]).then_some(pattern);
                continue;
            }
            let at = (0..session.slots.len()).find(|&i| {
                session.voice_at(i + 1).is_some_and(|v| v.tracks.contains(&track) && v.patterns.contains(&pattern))
            });
            match at {
                Some(i) => {
                    let own = session.voice_at(i + 1).map(|v| v.patterns[0].clone());
                    session.slots[i].play = (Some(&pattern) != own.as_ref()).then_some(pattern);
                }
                None => session.extra.push(line),
            }
        }
        session.fill_roles();
        session
    }

    /// The session file as the loop is now.
    pub fn text(&self) -> String {
        let mut text = String::from(HEADER);
        text.push('\n');
        let groove = &self.library.grooves[self.groove];
        text.push_str(&format!("use \"{}grooves/{}.synth\"\n", self.base, groove.name));
        for slot in &self.slots {
            if let Some(v) = &slot.voice {
                text.push_str(&format!("use \"{}voices/{}/{}.synth\"\n", self.base, slot.role, v));
            }
        }
        let mut play = |voice: &Voice, pattern: &Option<String>| {
            if let Some(p) = pattern {
                for t in &voice.tracks {
                    text.push_str(&format!("track {} {{ play {} }}\n", t, p));
                }
            }
        };
        play(groove, &self.groove_play);
        for (i, slot) in self.slots.iter().enumerate() {
            if let Some(v) = self.voice_at(i + 1) {
                play(v, &slot.play);
            }
        }
        for line in &self.extra {
            text.push_str(line);
            text.push('\n');
        }
        text
    }

    /// Row 0 is the groove; the slots follow.
    pub fn voice_at(&self, row: usize) -> Option<&Voice> {
        if row == 0 {
            return self.library.grooves.get(self.groove);
        }
        let slot = self.slots.get(row - 1)?;
        let name = slot.voice.as_deref()?;
        self.library.of(&slot.role).into_iter().find(|v| v.name == name)
    }

    fn held_at(&self, row: usize) -> &[(String, u8)] {
        if row == 0 {
            &self.groove_held
        } else {
            self.slots.get(row - 1).map(|s| s.held.as_slice()).unwrap_or(&[])
        }
    }

    fn held_at_mut(&mut self, row: usize) -> Option<&mut Vec<(String, u8)>> {
        if row == 0 {
            Some(&mut self.groove_held)
        } else {
            self.slots.get_mut(row - 1).map(|s| &mut s.held)
        }
    }

    fn all_held(&self) -> impl Iterator<Item = &(String, u8)> {
        core::iter::once(&self.groove_held).chain(self.slots.iter().map(|s| &s.held)).flatten()
    }

    /// What the screen draws: every row, and the knobs of the row chosen,
    /// where they sit -- turned, or where the text leaves them.
    pub fn view(&self, planner: &LivePlanner, focus: usize) -> View {
        let groove = &self.library.grooves[self.groove];
        let rows = core::iter::once(Row {
            role: GROOVE_ROLE.into(),
            name: Some(groove.name.clone()),
            kind: "groove".into(),
            doc: groove.doc.clone(),
            track: groove.track.clone(),
            index: Some(self.groove + 1),
            count: self.library.grooves.len(),
            held: !self.groove_held.is_empty(),
            play: self.groove_play.clone().unwrap_or_else(|| groove.patterns[0].clone()),
            patterns: groove.patterns.clone(),
        })
        .chain(self.slots.iter().map(|slot| {
            let of = self.library.of(&slot.role);
            let at = slot.voice.as_deref().and_then(|n| of.iter().position(|v| v.name == n));
            let voice = at.map(|i| of[i]);
            Row {
                role: slot.role.clone(),
                name: slot.voice.clone(),
                kind: voice.map(|v| kind_name(v.kind).to_string()).unwrap_or_default(),
                doc: voice.map(|v| v.doc.clone()).unwrap_or_default(),
                track: voice.map(|v| v.track.clone()).unwrap_or_default(),
                index: at.map(|i| i + 1),
                count: of.len(),
                held: !slot.held.is_empty(),
                play: slot.play.clone().or_else(|| voice.map(|v| v.patterns[0].clone())).unwrap_or_default(),
                patterns: voice.map(|v| v.patterns.clone()).unwrap_or_default(),
            }
        }))
        .collect();
        let macros = match self.voice_at(focus) {
            Some(voice) => {
                voice.macros().into_iter().map(|(label, words)| self.knob_row(planner, focus, label, &words)).collect()
            }
            None => Vec::new(),
        };
        // Every row's channel: level, pan and sends, for a mixer.
        let mix = (0..=self.slots.len())
            .map(|row| match self.voice_at(row) {
                Some(voice) => MIX
                    .iter()
                    .map(|what| self.knob_row(planner, row, what.to_string(), &format!("{} {}", voice.track, what)))
                    .collect(),
                None => Vec::new(),
            })
            .collect();
        // Every parameter of the chosen row's module, in the order the
        // language lists them: the whole instrument, not just its macros.
        let params = match self.voice_at(focus) {
            Some(voice) => params::specs(voice.kind)
                .iter()
                .map(|s| self.knob_row(planner, focus, s.name.to_string(), &format!("{} {}", voice.track, s.name)))
                .collect(),
            None => Vec::new(),
        };
        View { rows, macros, mix, params, comparing: self.comparing, knob_ccs: self.knob_ccs.clone() }
    }

    /// `words` (`bass cutoff`) held at `value` on the engine and kept as the
    /// row's, for a save to write.
    fn hold(
        &mut self,
        row: usize,
        words: &str,
        value: u8,
        planner: &mut LivePlanner,
        generation: Generation,
    ) -> Result<Outcome, String> {
        let dotted = words.replace(' ', ".");
        let held = self.held_at_mut(row).ok_or("no such row")?;
        match held.iter_mut().find(|(t, _)| *t == dotted) {
            Some(h) => h.1 = value,
            None => held.push((dotted.clone(), value)),
        }
        self.comparing = false;
        let turn = planner.try_knob(&dotted, value, generation);
        let said = turn.readings.first().cloned().unwrap_or_else(|| format!("{}: moves nothing", words));
        Ok(Outcome { plans: turn.plans, said, writes: Vec::new() })
    }

    /// One knob of a row: where it is (turned, or where the text has it)
    /// and what that reads in units.
    fn knob_row(&self, planner: &LivePlanner, row: usize, label: String, words: &str) -> MacroRow {
        let dotted = words.replace(' ', ".");
        let turned = self.held_at(row).iter().find(|(t, _)| *t == dotted).map(|(_, v)| *v);
        let knob = planner.probe_knob(&dotted);
        let rest = knob.as_ref().and_then(|k| planner.song().and_then(|s| k.rest_position(s)));
        let position = turned.or(rest).unwrap_or(0);
        // `bass cutoff 1.2khz` without its words: the row names them.
        let reading = knob
            .as_ref()
            .map(|k| k.reading(position))
            .map(|r| r.strip_prefix(words).map(|v| v.trim_start().to_string()).unwrap_or(r))
            .unwrap_or_else(|| "—".into());
        MacroRow { label, position, reading, held: turned.is_some() }
    }

    /// Do what a key or a knob asked. Choosing a voice rewrites the session
    /// file, which the watcher turns into a swap; turning a knob holds a
    /// value on the engine, returned as plans to send.
    pub fn act(
        &mut self,
        action: Action,
        planner: &mut LivePlanner,
        generation: Generation,
    ) -> Result<Outcome, String> {
        match action {
            Action::Choose { row, by } => {
                if row == 0 {
                    let n = self.library.grooves.len();
                    self.groove = (self.groove as i64 + by as i64).rem_euclid(n as i64) as usize;
                    self.groove_held.clear();
                    self.groove_play = None;
                    let g = &self.library.grooves[self.groove];
                    let said = format!("groove {}: {} · on the next bar", g.name, g.doc);
                    return Ok(Outcome { plans: Vec::new(), said, writes: vec![(self.path.clone(), self.text())] });
                }
                let slot = self.slots.get(row - 1).ok_or("no such row")?;
                // A voice another row plays is not offered twice: its
                // tracks would be defined twice.
                let names: Vec<String> = self
                    .library
                    .of(&slot.role)
                    .iter()
                    .map(|v| v.name.clone())
                    .filter(|n| self.slots.iter().enumerate().all(|(i, s)| i == row - 1 || s.voice.as_ref() != Some(n)))
                    .collect();
                if names.is_empty() {
                    return Err(format!("no other voices in voices/{}", slot.role));
                }
                // The choices run: nothing, then every voice, around.
                let at = slot.voice.as_ref().and_then(|v| names.iter().position(|n| n == v)).map_or(0, |i| i + 1);
                let n = names.len() + 1;
                let next = (at as i64 + by as i64).rem_euclid(n as i64) as usize;
                let slot = &mut self.slots[row - 1];
                slot.voice = if next == 0 { None } else { Some(names[next - 1].clone()) };
                slot.held.clear();
                slot.play = None;
                let said = match self.voice_at(row) {
                    Some(v) => format!("{}: {} · {} · on the next bar", v.role, v.name, v.doc),
                    None => format!("{}: nothing · on the next bar", self.slots[row - 1].role),
                };
                Ok(Outcome { plans: Vec::new(), said, writes: vec![(self.path.clone(), self.text())] })
            }
            Action::Turn { row, knob, value } => {
                let voice = self.voice_at(row).ok_or("nothing to turn")?;
                let (_, words) = voice.macros().into_iter().nth(knob).ok_or("no such knob")?;
                self.hold(row, &words, value, planner, generation)
            }
            Action::Mix { row, what, value } => {
                if !MIX.contains(&what.as_str()) {
                    return Err(format!("{} is not on a channel: {}", what, MIX.join(", ")));
                }
                let voice = self.voice_at(row).ok_or("nothing on that channel")?;
                let words = format!("{} {}", voice.track, what);
                self.hold(row, &words, value, planner, generation)
            }
            Action::Solo(row) => {
                let voice = self.voice_at(row).ok_or("nothing to solo")?;
                let (track, tracks) = (voice.track.clone(), voice.tracks.clone());
                let plans = planner.toggle_solo_group(&tracks, generation);
                let on = planner.soloed().contains(&track);
                Ok(Outcome {
                    plans,
                    said: format!("{} {}", track, if on { "alone" } else { "with the rest" }),
                    writes: Vec::new(),
                })
            }
            Action::Mute(row) => {
                let voice = self.voice_at(row).ok_or("nothing to mute")?;
                let (track, tracks) = (voice.track.clone(), voice.tracks.clone());
                // The layers follow the first: all out, or all back.
                let out = !planner.is_muted(&track);
                let turn: Vec<&String> = tracks.iter().filter(|t| planner.is_muted(t) != out).collect();
                let plans = turn.into_iter().flat_map(|t| planner.toggle_mute(t, generation)).collect();
                Ok(Outcome {
                    plans,
                    said: format!("{} {}", track, if out { "muted" } else { "back" }),
                    writes: Vec::new(),
                })
            }
            Action::Add(role) => {
                if !self.library.roles.contains(&role) {
                    return Err(format!("no voices in voices/{}", role));
                }
                let free = self
                    .library
                    .of(&role)
                    .iter()
                    .map(|v| v.name.clone())
                    .find(|n| self.slots.iter().all(|s| s.voice.as_ref() != Some(n)))
                    .ok_or_else(|| format!("every {} voice is in the loop", role))?;
                // Last of the rows that play, before the empty ones; the
                // role's empty row, if it had one, is this row now.
                self.slots.retain(|s| s.voice.is_some() || s.role != role);
                let at = self.slots.iter().position(|s| s.voice.is_none()).unwrap_or(self.slots.len());
                self.slots.insert(at, Slot { voice: Some(free.clone()), ..Slot::empty(&role) });
                let said = format!("{}: {} joins · on the next bar", role, free);
                Ok(Outcome { plans: Vec::new(), said, writes: vec![(self.path.clone(), self.text())] })
            }
            Action::Remove(row) => {
                if row == 0 || row > self.slots.len() {
                    return Err("the groove stays: choose another".into());
                }
                let slot = self.slots.remove(row - 1);
                self.fill_roles();
                let said = format!("{} out · on the next bar", slot.voice.as_deref().unwrap_or(&slot.role));
                Ok(Outcome { plans: Vec::new(), said, writes: vec![(self.path.clone(), self.text())] })
            }
            Action::Set { row, param, value } => {
                let voice = self.voice_at(row).ok_or("nothing to set")?;
                if params::lookup(voice.kind, &param).is_none() {
                    return Err(format!("{} has no parameter {}", kind_name(voice.kind), param));
                }
                let words = format!("{} {}", voice.track, param);
                self.hold(row, &words, value, planner, generation)
            }
            Action::New { role, kind, name } => {
                let kind = ModuleKind::from_str(&kind).ok_or_else(|| format!("no module kind {}", kind))?;
                if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    return Err("a voice's name is letters, digits and _".into());
                }
                if role.is_empty() || role == GROOVE_ROLE || !role.chars().all(|c| c.is_ascii_lowercase()) {
                    return Err(format!("{} is not a role for a voice", role));
                }
                if self.library.voices.iter().any(|v| v.name == name)
                    || self.library.grooves.iter().any(|g| g.name == name)
                {
                    return Err(format!("{} exists already", name));
                }
                let dir = dir_of(&self.library.grooves[0].path);
                let lib = dir.strip_suffix("grooves").unwrap_or(dir).trim_end_matches('/');
                let path = if lib.is_empty() {
                    format!("voices/{}/{}.synth", role, name)
                } else {
                    format!("{}/voices/{}/{}.synth", lib, role, name)
                };
                let text = new_voice_text(kind, &name);
                let voice = Voice::parse(&role, &path, &text)?;
                self.library.put(voice);
                if !self.library.roles.contains(&role) {
                    self.library.roles.push(role.clone());
                }
                self.slots.retain(|s| s.voice.is_some() || s.role != role);
                let at = self.slots.iter().position(|s| s.voice.is_none()).unwrap_or(self.slots.len());
                self.slots.insert(at, Slot { voice: Some(name.clone()), ..Slot::empty(&role) });
                let said = format!("{}: {}, a new {} · on the next bar", role, name, kind_name(kind));
                Ok(Outcome { plans: Vec::new(), said, writes: vec![(path, text), (self.path.clone(), self.text())] })
            }
            Action::Vary { row, by } => {
                let voice = self.voice_at(row).ok_or("nothing to vary")?;
                let (patterns, track) = (voice.patterns.clone(), voice.track.clone());
                let now = if row == 0 { &self.groove_play } else { &self.slots[row - 1].play };
                let at = now.as_ref().and_then(|p| patterns.iter().position(|q| q == p)).unwrap_or(0);
                let next = (at as i64 + by as i64).rem_euclid(patterns.len() as i64) as usize;
                let play = (next != 0).then(|| patterns[next].clone());
                if row == 0 {
                    self.groove_play = play;
                } else {
                    self.slots[row - 1].play = play;
                }
                let said = format!("{} plays {} · on the next bar", track, patterns[next]);
                Ok(Outcome { plans: Vec::new(), said, writes: vec![(self.path.clone(), self.text())] })
            }
            Action::Reset(row) => {
                let held = self.held_at_mut(row).ok_or("no such row")?;
                let targets: Vec<String> = held.drain(..).map(|(t, _)| t).collect();
                if targets.is_empty() {
                    return Ok(Outcome::said("nothing turned here"));
                }
                let plans = self.rest_plans(&targets, planner, generation);
                Ok(Outcome { plans, said: format!("{} knobs back to the text", targets.len()), writes: Vec::new() })
            }
            Action::Scale(to) => {
                if let Some((root, kind)) = &to {
                    if crate::perform::scale::Scale::named(root, kind).is_none() {
                        return Err(format!("no scale called {} {}", root, kind));
                    }
                }
                self.extra.retain(|line| !is_scale_line(line));
                let said = match &to {
                    Some((root, kind)) => {
                        self.extra.push(format!("scale {} {}", root, kind));
                        format!("the song in {} {}", root, kind)
                    }
                    None => String::from("the groove's own scale"),
                };
                Ok(Outcome { plans: Vec::new(), said, writes: vec![(self.path.clone(), self.text())] })
            }
            Action::Tempo(to) => {
                if let Some(bpm) = to {
                    if !(20.0..=400.0).contains(&bpm) {
                        return Err(format!("{} BPM: a tempo is 20 to 400", bpm));
                    }
                }
                self.extra.retain(|line| !is_tempo_line(line));
                let said = match to {
                    Some(bpm) => {
                        self.extra.push(format!("tempo {}", bpm));
                        format!("the song at {} BPM", bpm)
                    }
                    None => String::from("the groove's own tempo"),
                };
                Ok(Outcome { plans: Vec::new(), said, writes: vec![(self.path.clone(), self.text())] })
            }
            Action::Compare => {
                let held: Vec<(String, u8)> = self.all_held().cloned().collect();
                if held.is_empty() {
                    return Ok(Outcome::said("nothing turned to compare with"));
                }
                self.comparing = !self.comparing;
                let plans = if self.comparing {
                    let targets: Vec<String> = held.iter().map(|(t, _)| t.clone()).collect();
                    self.rest_plans(&targets, planner, generation)
                } else {
                    held.iter().flat_map(|(t, v)| planner.try_knob(t, *v, generation).plans).collect()
                };
                let said = if self.comparing { "A: as the text has it" } else { "B: as you turned it" };
                Ok(Outcome { plans, said: said.into(), writes: Vec::new() })
            }
            Action::Save { row, name } => {
                let (path, text) = self.save_voice(row, &name, planner)?;
                let said = format!("saved {} · playing it on the next bar", path);
                Ok(Outcome { plans: Vec::new(), said, writes: vec![(path, text), (self.path.clone(), self.text())] })
            }
        }
    }

    /// Every target put back where the text leaves it.
    fn rest_plans(&self, targets: &[String], planner: &mut LivePlanner, generation: Generation) -> Vec<Plan> {
        targets
            .iter()
            .filter_map(|t| {
                let knob = planner.probe_knob(t)?;
                let rest = knob.rest_position(planner.song()?)?;
                Some(planner.try_knob(t, rest, generation).plans)
            })
            .flatten()
            .collect()
    }

    /// After a swap the engine starts from the text again: every knob
    /// turned is held on it afresh.
    pub fn reapply(&self, planner: &mut LivePlanner, generation: Generation) -> Vec<Plan> {
        if self.comparing {
            return Vec::new();
        }
        self.all_held().flat_map(|(t, v)| planner.try_knob(t, *v, generation).plans).collect()
    }

    /// The knobs a row is on, in `knob_ccs` order: controller `cc` turns
    /// knob `i` of the chosen row.
    pub fn knob_index(&self, cc: u8) -> Option<usize> {
        self.knob_ccs.iter().position(|&c| c == cc)
    }

    /// The voice of `row` as it sounds now -- its file with every knob
    /// turned written into it, in units -- as `voices/<role>/<name>.synth`,
    /// put in the library and chosen for the slot. The same name writes
    /// over the voice playing; another name is a new voice, and refuses to
    /// replace one. Returns the file to write.
    fn save_voice(&mut self, row: usize, name: &str, planner: &LivePlanner) -> Result<(String, String), String> {
        let name = name.trim();
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            return Err("a voice's name is letters, digits, _ and -".into());
        }
        let voice = self.voice_at(row).ok_or("nothing to save here")?.clone();
        let dir = dir_of(&voice.path);
        let out = if dir.is_empty() { format!("{}.synth", name) } else { format!("{}/{}.synth", dir, name) };
        let taken = if row == 0 { self.library.grooves.iter() } else { self.library.voices.iter() }
            .any(|v| v.role == voice.role && v.name == name && v.name != voice.name);
        if taken {
            return Err(format!("{} exists already: another name, or its own to write over it", out));
        }
        let mut text = voice.text.clone();
        for (dotted, position) in self.held_at(row).to_vec() {
            let knob = planner.probe_knob(&dotted).ok_or_else(|| format!("{} names nothing now", dotted))?;
            let value = knob.value_at(position).ok_or_else(|| format!("{} has no value", dotted))?;
            let words: Vec<&str> = dotted.split('.').collect();
            text = match words.as_slice() {
                [_, what @ ("level" | "pan" | "delay_send" | "reverb_send")] => {
                    let block = edit::block(&text, "track", &voice.track).ok_or("the voice lost its track")?;
                    edit::set_option(&text, block, what, &format!("{:.3}", value))
                }
                [_, param] => {
                    let spec =
                        params::lookup(voice.kind, param).ok_or_else(|| format!("{} is not a parameter", param))?;
                    let block = edit::block(&text, "module", &voice.module).ok_or("the voice lost its module")?;
                    edit::set_option(&text, block, param, &write_param(spec, value))
                }
                [_, label, "wet"] => {
                    let block = edit::block(&text, "track", &voice.track).ok_or("the voice lost its track")?;
                    edit::set_wet(&text, block, label, &format!("{:.2}", value))?
                }
                _ => return Err(format!("{}: not something a voice file keeps", dotted)),
            };
        }
        // The module takes the voice's name, so two saved voices never
        // define the same module: a song can `use` both.
        if name != voice.module {
            text = edit::rename_module(&text, &voice.module, name);
        }
        // So does a track named as the voice: both play in one loop.
        if voice.track == voice.name && name != voice.name {
            text = edit::rename_track(&text, &voice.track, name);
        }
        if out != voice.path {
            text = text.replacen('\n', &format!("\n# saved from {} with tatum compose\n", voice.name), 1);
        }
        let saved = Voice::parse(&voice.role, &out, &text)?;
        self.library.put(saved);
        if row == 0 {
            self.groove = self.library.grooves.iter().position(|g| g.name == name).unwrap_or(0);
            self.groove_held.clear();
        } else {
            let slot = &mut self.slots[row - 1];
            slot.voice = Some(name.to_string());
            slot.held.clear();
        }
        self.comparing = false;
        Ok((out, text))
    }
}

/// A module parameter's value as its line writes it: in units where it has
/// them, by name for a choice, a plain number otherwise.
/// A session line that sets the song's scale: `scale A minor`.
fn is_scale_line(line: &str) -> bool {
    let mut words = line.split_whitespace();
    words.next() == Some("scale") && words.count() == 2
}

/// A voice from nothing: the module at its defaults, a pattern to start
/// from -- a root a beat, or a kick and a hat for drums -- and its track.
fn new_voice_text(kind: ModuleKind, name: &str) -> String {
    let kind_word = kind_name(kind);
    let pattern = match kind {
        ModuleKind::Beats => {
            String::from("    kick: X - - -  - - - -  X - - -  - - - -\n    hat:  - - x -  - - x -  - - x -  - - x -\n")
        }
        ModuleKind::Keys => String::from("    [1.3 3.3 5.3]:0.7 ..*15\n"),
        _ => String::from("    1.2:0.9 - - -  1.2:0.7 - - -  1.2:0.9 - - -  1.2:0.7 - - -\n"),
    };
    format!(
        "# A new {kind_word} voice, from its defaults\n\
         # A voice: one module, its patterns in degrees of whatever scale the groove\n\
         # sets, and a track named as the voice. `use` it from any song.\n\
         module {kind_word} {name} {{\n}}\n\n\
         pattern {name}_line {{\n{pattern}}}\n\n\
         track {name} {{\n    play {name}_line\n    using {name}\n    level 0.6\n    out > lowpass(18khz, 0.05) as lp > master\n}}\n"
    )
}

/// A session line that gives a track another pattern, as the session
/// writes it: `track acid { play acid_half }`.
fn play_line(line: &str) -> Option<(String, String)> {
    let code = line.split('#').next().unwrap_or("");
    match code.split_whitespace().collect::<Vec<_>>().as_slice() {
        ["track", track, "{", "play", pattern, "}"] => Some((track.to_string(), pattern.to_string())),
        _ => None,
    }
}

/// A session line that sets the song's tempo: `tempo 140`.
fn is_tempo_line(line: &str) -> bool {
    let mut words = line.split_whitespace();
    words.next() == Some("tempo") && words.count() == 1
}

impl Session {
    /// A library file rewritten from outside the loop -- a pattern drawn, a
    /// chain changed -- read again, so the macros it offers and the text a
    /// save starts from are the new ones. Not a voice: nothing to do.
    pub fn reread(&mut self, path: &str, text: &str) {
        let path = crate::include::normalize(path);
        let known = self.library.grooves.iter().chain(self.library.voices.iter()).find(|v| v.path == path);
        let Some(role) = known.map(|v| v.role.clone()) else { return };
        if let Ok(voice) = Voice::parse(&role, &path, text) {
            self.library.put(voice);
        }
    }

    /// Whether the session sets its own scale over the groove's.
    pub fn own_scale(&self) -> bool {
        self.extra.iter().any(|l| is_scale_line(l))
    }

    /// Whether the session sets its own tempo over the groove's.
    pub fn own_tempo(&self) -> bool {
        self.extra.iter().any(|l| is_tempo_line(l))
    }
}

fn write_param(spec: &params::ParamSpec, value: f32) -> String {
    if let Range::Choice(_) = spec.range {
        return spec.choice_name(value).map(String::from).unwrap_or_else(|| format!("{:.3}", value));
    }
    spec.write_in_units(value).unwrap_or_else(|| format!("{:.3}", value))
}

/// Edits to a voice's text that keep everything else as written.
pub mod edit {
    use alloc::string::{String, ToString};
    use alloc::vec::Vec;

    /// The inside of the first top-level `<what> <name> {` block, or of
    /// `module <kind> <name> {`: from after its `{` to its `}`.
    pub fn block(text: &str, what: &str, name: &str) -> Option<(usize, usize)> {
        let mut at = 0;
        for line in text.split_inclusive('\n') {
            let words: Vec<&str> = line.split('#').next().unwrap_or("").split_whitespace().collect();
            let named = match (what, words.as_slice()) {
                ("module", [w, _, n, ..]) => *w == what && *n == name,
                (_, [w, n, ..]) => *w == what && *n == name,
                _ => false,
            };
            if named {
                let open = at + line.find('{')?;
                let close = close_of(text, open)?;
                return Some((open + 1, close));
            }
            at += line.len();
        }
        None
    }

    /// The `}` matching the `{` at `open`, comments skipped.
    fn close_of(text: &str, open: usize) -> Option<usize> {
        let mut depth = 0usize;
        let mut in_comment = false;
        for (i, c) in text[open..].char_indices() {
            match c {
                '\n' => in_comment = false,
                _ if in_comment => {}
                '#' => in_comment = true,
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(open + i);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// `key` set to `value` inside the block: the word's value replaced
    /// where the block has it, else the line added before its `}`.
    pub fn set_option(text: &str, (start, end): (usize, usize), key: &str, value: &str) -> String {
        let body = &text[start..end];
        let mut at = 0;
        for line in body.split_inclusive('\n') {
            let code = line.split('#').next().unwrap_or("");
            let mut words = code.split_whitespace();
            // Only a line that begins with the word: `cutoff 110hz`, not the
            // `cutoff` an `auto` or a chain might name.
            if words.next() == Some(key) {
                if let Some(old) = words.next() {
                    let key_at = code.find(key).unwrap_or(0) + key.len();
                    let rel = code[key_at..].find(old).unwrap_or(0) + key_at;
                    let from = start + at + rel;
                    return format!("{}{}{}", &text[..from], value, &text[from + old.len()..]);
                }
            }
            at += line.len();
        }
        let one_line = !body.contains('\n');
        let insert = if one_line {
            format!(" {} {} ", key, value)
        } else {
            let indent = body
                .lines()
                .find(|l| !l.trim().is_empty())
                .map(|l| &l[..l.len() - l.trim_start().len()])
                .unwrap_or("    ");
            format!("{}{} {}\n", indent, key, value)
        };
        format!("{}{}{}", &text[..end], insert, &text[end..])
    }

    /// The block's routing (`out > a(..)\n    > b(..) > master`) put as
    /// `routing` writes it: the lines it took replaced, comments on them
    /// with them; added before the block's `}` where it has none. `routing`
    /// is given the indent of the line it lands on.
    pub fn set_routing(text: &str, (start, end): (usize, usize), routing: impl Fn(&str) -> String) -> String {
        let body = &text[start..end];
        let mut at = 0;
        let mut found: Option<(usize, usize, String)> = None;
        let mut open = false;
        for line in body.split_inclusive('\n') {
            let code = line.split('#').next().unwrap_or("").trim();
            let line_end = start + at + line.trim_end_matches('\n').len();
            match &mut found {
                None if code == "out" || code.starts_with("out ") || code.starts_with("out>") => {
                    let indent = &line[..line.len() - line.trim_start().len()];
                    found = Some((start + at + indent.len(), line_end, indent.to_string()));
                    open = code.ends_with('>');
                }
                Some(f) if open || code.starts_with('>') => {
                    f.1 = line_end;
                    open = code.ends_with('>');
                }
                Some(_) => break,
                None => {}
            }
            at += line.len();
        }
        match found {
            Some((from, to, indent)) => format!("{}{}{}", &text[..from], routing(&indent), &text[to..]),
            None => {
                let indent = body
                    .lines()
                    .find(|l| !l.trim().is_empty())
                    .map(|l| &l[..l.len() - l.trim_start().len()])
                    .unwrap_or("    ");
                format!("{}{}{}\n{}", &text[..end], indent, routing(indent), &text[end..])
            }
        }
    }

    /// `wet=` of the node the block names `as <label>`: set where it is
    /// written, added to the node's arguments where it is not.
    pub fn set_wet(text: &str, (start, end): (usize, usize), label: &str, value: &str) -> Result<String, String> {
        let body = &text[start..end];
        let marker = format!("as {}", label);
        let as_at = body
            .match_indices(&marker)
            .find(|(i, _)| body[i + marker.len()..].chars().next().is_none_or(|c| !c.is_alphanumeric() && c != '_'))
            .map(|(i, _)| i)
            .ok_or_else(|| format!("no node named {} in the chain", label))?;
        let close = body[..as_at].rfind(')').ok_or_else(|| format!("{} is not a node with arguments", label))?;
        let open = body[..close].rfind('(').ok_or_else(|| format!("{} is not a node with arguments", label))?;
        let args = &body[open + 1..close];
        let new_args = match args.find("wet=") {
            Some(w) => {
                let after = &args[w + 4..];
                let len = after.find(|c: char| c == ',' || c == ')' || c.is_whitespace()).unwrap_or(after.len());
                format!("{}wet={}{}", &args[..w], value, &after[len..])
            }
            None if args.trim().is_empty() => format!("wet={}", value),
            None => format!("{}, wet={}", args, value),
        };
        Ok(format!("{}{}{}", &text[..start + open + 1], new_args, &text[start + close..]))
    }

    /// The module called `old` called `new`: its definition and every
    /// `using old`.
    pub fn rename_module(text: &str, old: &str, new: &str) -> String {
        let mut out = String::with_capacity(text.len());
        for line in text.split_inclusive('\n') {
            let words: Vec<&str> = line.split_whitespace().collect();
            let is_def = matches!(words.as_slice(), ["module", _, n, ..] if *n == old);
            let mut line = line.to_string();
            if is_def {
                line = line.replacen(&format!(" {} ", old), &format!(" {} ", new), 1);
            }
            if line.contains("using ") {
                line = replace_word(&line, "using", old, new);
            }
            out.push_str(&line);
        }
        out
    }

    /// The track called `old` called `new`: the line that opens it.
    pub fn rename_track(text: &str, old: &str, new: &str) -> String {
        let mut out = String::with_capacity(text.len());
        for line in text.split_inclusive('\n') {
            let words: Vec<&str> = line.split_whitespace().collect();
            if matches!(words.as_slice(), ["track", n, ..] if *n == old) {
                out.push_str(&line.replacen(&format!("track {}", old), &format!("track {}", new), 1));
            } else {
                out.push_str(line);
            }
        }
        out
    }

    /// `<key> old` as `<key> new` where `old` is the whole word.
    fn replace_word(line: &str, key: &str, old: &str, new: &str) -> String {
        let mut out = String::new();
        let mut rest = line;
        while let Some(i) = rest.find(key) {
            let after = &rest[i + key.len()..];
            let trimmed = after.trim_start();
            let spaces = after.len() - trimmed.len();
            let word_len = trimmed.find(|c: char| c.is_whitespace() || c == '}').unwrap_or(trimmed.len());
            if spaces > 0 && &trimmed[..word_len] == old {
                out.push_str(&rest[..i + key.len() + spaces]);
                out.push_str(new);
                rest = &trimmed[word_len..];
            } else {
                out.push_str(&rest[..i + key.len()]);
                rest = after;
            }
        }
        out.push_str(rest);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::edit::{block, rename_module, set_option, set_wet};
    use super::*;
    use alloc::vec;

    const VOICE: &str = "# a bass\nmodule bass low {\n    cutoff 110hz\n    cutoff_env 750hz\n    sustain 0.9\n}\npattern line { 1.1 - - - }\ntrack bass {\n    play line\n    using low\n    level 0.5\n    out > saturate(0.3) as drive > lowpass(2khz, 0.1, wet=1) as lp > master\n}\n";

    const GROOVE: &str = "# a groove\ntempo 120\nscale F minor\nmodule beats kit { kick_level 1.0 }\npattern beat { kick: X - - - }\ntrack drums { play beat using kit out > master }\n";

    #[test]
    fn a_parameter_is_replaced_where_the_module_writes_it() {
        let b = block(VOICE, "module", "low").unwrap();
        let out = set_option(VOICE, b, "cutoff", "480hz");
        assert!(out.contains("    cutoff 480hz\n"), "{out}");
        assert!(out.contains("    cutoff_env 750hz\n"), "cutoff_env was touched:\n{out}");
    }

    #[test]
    fn a_parameter_the_module_leaves_out_is_added_before_its_brace() {
        let b = block(VOICE, "module", "low").unwrap();
        let out = set_option(VOICE, b, "resonance", "55%");
        assert!(out.contains("    sustain 0.9\n    resonance 55%\n}"), "{out}");
    }

    #[test]
    fn a_one_line_block_takes_the_word_inside_its_braces() {
        let text = "module bass low { cutoff 110hz sustain 0.9 }\n";
        let b = block(text, "module", "low").unwrap();
        assert_eq!(set_option(text, b, "cutoff", "200hz"), "module bass low { cutoff 200hz sustain 0.9 }\n");
        let b = block(text, "module", "low").unwrap();
        assert_eq!(set_option(text, b, "glide", "20%"), "module bass low { cutoff 110hz sustain 0.9  glide 20% }\n");
    }

    #[test]
    fn a_tracks_level_is_set_without_touching_the_chain() {
        let b = block(VOICE, "track", "bass").unwrap();
        let out = set_option(VOICE, b, "level", "0.620");
        assert!(out.contains("    level 0.620\n"), "{out}");
        assert!(out.contains("saturate(0.3) as drive"), "{out}");
    }

    #[test]
    fn a_nodes_wet_is_added_or_replaced() {
        let b = block(VOICE, "track", "bass").unwrap();
        let out = set_wet(VOICE, b, "drive", "0.50").unwrap();
        assert!(out.contains("saturate(0.3, wet=0.50) as drive"), "{out}");
        let b = block(&out, "track", "bass").unwrap();
        let out = set_wet(&out, b, "lp", "0.25").unwrap();
        assert!(out.contains("lowpass(2khz, 0.1, wet=0.25) as lp"), "{out}");
        let b = block(VOICE, "track", "bass").unwrap();
        assert!(set_wet(VOICE, b, "nope", "1").is_err());
    }

    #[test]
    fn renaming_the_module_follows_every_using() {
        let out = rename_module(VOICE, "low", "low_dark");
        assert!(out.contains("module bass low_dark {"), "{out}");
        assert!(out.contains("    using low_dark\n"), "{out}");
        assert!(!out.contains("using low\n"), "{out}");
    }

    fn library() -> Library {
        let (lib, skipped) = Library::from_files([
            ("drums", "grooves/dub.synth", GROOVE),
            ("bass", "voices/bass/low.synth", VOICE),
            ("lead", "voices/lead/bell.synth", "# a bell\nmodule fm bell { mod_index 40% }\npattern bl { 1.4 - - - }\ntrack lead { play bl using bell level 0.3 out > master }\n"),
            ("lead", "voices/lead/broken.synth", "tempo 120\n"),
        ])
        .unwrap();
        assert_eq!(skipped.len(), 1, "the file that is not a voice is skipped: {:?}", skipped);
        lib
    }

    struct Map(Vec<(String, String)>);

    impl crate::include::Files for Map {
        fn read(&self, path: &str) -> Option<String> {
            self.0.iter().find(|(p, _)| p == path).map(|(_, t)| t.clone())
        }
    }

    fn planner_for(session: &Session) -> LivePlanner {
        let mut files: Vec<(String, String)> = session
            .library
            .grooves
            .iter()
            .chain(session.library.voices.iter())
            .map(|v| (v.path.clone(), v.text.clone()))
            .collect();
        files.push((session.path.clone(), session.text()));
        let source = crate::include::Source::load(&session.path, &Map(files)).unwrap();
        let mut planner = LivePlanner::new();
        planner.plan(&source.text, 0).unwrap();
        planner
    }

    #[test]
    fn a_new_session_takes_the_first_groove_and_bass_and_choosing_rewrites_its_file() {
        let mut session = Session::new(library(), "session.synth", DEFAULT_KNOB_CCS.to_vec());
        assert_eq!(session.text(), format!("{}\nuse \"grooves/dub.synth\"\nuse \"voices/bass/low.synth\"\n", HEADER));
        let mut planner = planner_for(&session);
        let out = session.act(Action::Choose { row: 2, by: 1 }, &mut planner, 0).unwrap();
        assert!(out.plans.is_empty());
        assert!(out.said.starts_with("lead: bell"), "{}", out.said);
        assert_eq!(out.writes.len(), 1);
        assert!(out.writes[0].1.contains("use \"voices/lead/bell.synth\""), "{}", out.writes[0].1);
        // Back one is nothing again; the line goes.
        let out = session.act(Action::Choose { row: 2, by: -1 }, &mut planner, 0).unwrap();
        assert!(!out.writes[0].1.contains("voices/lead/"), "{}", out.writes[0].1);
    }

    #[test]
    fn a_hand_written_session_keeps_its_other_lines() {
        let text = "use \"grooves/dub.synth\"\nuse \"voices/lead/bell.synth\"\ntempo 118\ntrack pad { level 0.1 }\n";
        let session = Session::from_text(library(), "s.synth", vec![1, 2], text);
        // The rows the file plays first, in its order; an empty one for each
        // role it leaves out, after them.
        assert_eq!(session.slots[0].voice.as_deref(), Some("bell"));
        assert!(
            session.slots.iter().any(|s| s.role == "bass" && s.voice.is_none()),
            "the bass line is not in the file"
        );
        assert!(session.text().ends_with("tempo 118\ntrack pad { level 0.1 }\n"), "{}", session.text());
    }

    #[test]
    fn a_knob_turned_is_held_then_written_into_the_saved_voice() {
        let mut session = Session::new(library(), "session.synth", DEFAULT_KNOB_CCS.to_vec());
        let mut planner = planner_for(&session);
        // The bass's first knob is its cutoff; all the way up.
        let out = session.act(Action::Turn { row: 1, knob: 0, value: 127 }, &mut planner, 0).unwrap();
        assert!(!out.plans.is_empty(), "nothing held");
        assert!(out.said.starts_with("bass cutoff"), "{}", out.said);
        let view = session.view(&planner, 1);
        assert_eq!(view.macros[0].label, "cutoff");
        assert_eq!(view.macros[0].position, 127);
        assert!(view.macros[0].held);
        let drive = view.macros.iter().position(|m| m.label == "drive wet").unwrap();
        session.act(Action::Turn { row: 1, knob: drive, value: 0 }, &mut planner, 0).unwrap();

        let out = session.act(Action::Save { row: 1, name: "low_open".into() }, &mut planner, 0).unwrap();
        let (path, saved) = &out.writes[0];
        assert_eq!(path, "voices/bass/low_open.synth");
        assert!(saved.contains("module bass low_open {"), "{saved}");
        assert!(saved.contains("    using low_open\n"), "{saved}");
        assert!(saved.contains("    cutoff 20000hz\n"), "{saved}");
        assert!(saved.contains("saturate(0.3, wet=0.00) as drive"), "{saved}");
        assert!(saved.contains("# saved from low with tatum compose"), "{saved}");
        assert!(out.writes[1].1.contains("use \"voices/bass/low_open.synth\""), "{}", out.writes[1].1);
        assert!(session.slots[0].held.is_empty());
        assert_eq!(session.library.of("bass").len(), 2, "the saved voice joined the library");
        // Another voice's name is refused.
        let Err(err) = session.act(Action::Save { row: 1, name: "low".into() }, &mut planner, 0) else {
            panic!("a name another voice has was accepted");
        };
        assert!(err.contains("exists already"), "{err}");
    }
}
