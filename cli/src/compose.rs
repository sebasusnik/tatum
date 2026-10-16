//! `tatum compose`: a loop to try voices in. A groove plays; each role --
//! bass, lead, pad, perc -- holds one voice from a library, swapped with a
//! key on the next bar line; a handful of knobs per voice turn the
//! parameters a hand reaches for first, from the keyboard or a controller;
//! a voice that sounds right is saved as a small `.synth` of its own, which
//! any song then gets with one `use` line.
//!
//! The session is a file of `use` lines this module rewrites. The live
//! session watches it like any other file, so choosing a voice is a save:
//! the swap lands on the bar line, and the file left behind is what a song
//! builds on. A knob turned is held on the engine, not written, until the
//! voice is saved: then its values go into the new file, in units.

use std::fs;
use std::path::{Path, PathBuf};

use tatum_core::live::{LivePlanner, Plan};
use tatum_core::params::{self, ModuleKind, Range};

use crate::tui::compose::{Action, MacroRow, Row, View};

/// The knobs of a KeyLab Essential mk3, left to right: what the eight
/// macros of the chosen voice sit on unless `--knobs` says otherwise.
pub const DEFAULT_KNOB_CCS: [u8; 8] = [74, 71, 76, 77, 93, 18, 19, 16];

/// Roles in the order the screen lists them; any other directory under
/// `voices/` follows, alphabetically.
const ROLE_ORDER: &[&str] = &["bass", "lead", "pad", "perc"];

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

/// A voice in the library: one file holding one module and one track named
/// by its role. A groove is the same with the role `drums`, plus the
/// globals, the sends and the master.
#[derive(Debug, Clone)]
pub struct Voice {
    pub role: String,
    pub name: String,
    pub path: PathBuf,
    pub kind: ModuleKind,
    pub module: String,
    pub track: String,
    /// The file's first comment line, for the screen.
    pub doc: String,
    /// The `as` names of its chain, for a wet knob each.
    pub nodes: Vec<String>,
}

impl Voice {
    /// Read one file: its module, its track and the names its chain gives.
    fn load(role: &str, path: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(path).map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
        let song = tatum_core::dsl::parse(&text)
            .map_err(|e| format!("{}: {}", path.display(), e.first().map(|e| e.to_string()).unwrap_or_default()))?;
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let def = song
            .tracks
            .iter()
            .find(|t| !t.using_instrument.is_empty())
            .ok_or_else(|| format!("{}: no track plays a module", path.display()))?;
        let module = song.module_defs.iter().find(|m| m.name == def.using_instrument).ok_or_else(|| {
            format!(
                "{}: track {} uses {}, which the file does not define",
                path.display(),
                def.name,
                def.using_instrument
            )
        })?;
        let kind = ModuleKind::from_str(&module.module_type)
            .ok_or_else(|| format!("{}: unknown module kind {}", path.display(), module.module_type))?;
        let doc = text
            .lines()
            .map(str::trim)
            .find(|l| l.starts_with('#'))
            .map(|l| l.trim_start_matches('#').trim().to_string())
            .unwrap_or_default();
        Ok(Self {
            role: role.to_string(),
            name,
            path: path.to_path_buf(),
            kind,
            module: module.name.clone(),
            track: def.name.clone(),
            doc,
            nodes: def.routing.iter().filter_map(|n| n.label.clone()).collect(),
        })
    }

    /// What the voice offers a knob, likeliest first: `bass cutoff`,
    /// `bass level`, `bass drive wet`. Words, as a `midi` line writes them.
    fn macros(&self) -> Vec<(String, String)> {
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

/// Every groove and voice under a directory: `grooves/*.synth` and
/// `voices/<role>/*.synth`.
pub struct Library {
    pub root: PathBuf,
    pub grooves: Vec<Voice>,
    pub voices: Vec<Voice>,
    pub roles: Vec<String>,
}

impl Library {
    pub fn scan(root: &Path) -> Result<Self, String> {
        let grooves = Self::files(&root.join("grooves"))?
            .iter()
            .filter_map(|p| Voice::load("drums", p).map_err(|e| eprintln!("skipped: {}", e)).ok())
            .collect::<Vec<_>>();
        if grooves.is_empty() {
            return Err(format!(
                "no grooves in {}: a groove is a .synth with a drums track",
                root.join("grooves").display()
            ));
        }
        let voices_dir = root.join("voices");
        let mut roles: Vec<String> = fs::read_dir(&voices_dir)
            .map_err(|e| format!("cannot read {}: {}", voices_dir.display(), e))?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        roles.sort_by_key(|r| (ROLE_ORDER.iter().position(|o| o == r).unwrap_or(ROLE_ORDER.len()), r.clone()));
        let mut voices = Vec::new();
        for role in &roles {
            for p in Self::files(&voices_dir.join(role))? {
                match Voice::load(role, &p) {
                    Ok(v) => voices.push(v),
                    Err(e) => eprintln!("skipped: {}", e),
                }
            }
        }
        Ok(Self { root: root.to_path_buf(), grooves, voices, roles })
    }

    fn files(dir: &Path) -> Result<Vec<PathBuf>, String> {
        let mut out: Vec<PathBuf> = fs::read_dir(dir)
            .map_err(|e| format!("cannot read {}: {}", dir.display(), e))?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "synth"))
            .collect();
        out.sort();
        Ok(out)
    }

    /// The voices of one role, in file order.
    pub fn of(&self, role: &str) -> Vec<&Voice> {
        self.voices.iter().filter(|v| v.role == role).collect()
    }
}

/// One role of the loop: which voice it holds, and the knobs turned on it
/// since, by target (`bass.cutoff`) and controller position.
#[derive(Debug, Clone)]
pub struct Slot {
    pub role: String,
    pub voice: Option<String>,
    pub held: Vec<(String, u8)>,
}

/// The loop: a groove, a voice per role, and the file that says so.
pub struct Session {
    pub library: Library,
    pub path: PathBuf,
    pub groove: usize,
    /// Knobs turned on the groove's drums, like a slot's.
    pub groove_held: Vec<(String, u8)>,
    pub slots: Vec<Slot>,
    /// Lines of the session file that are not its `use` lines, kept as they
    /// are: the file is the user's too.
    extra: Vec<String>,
    pub knob_ccs: Vec<u8>,
    /// `a`: the knobs turned are heard at the text's values instead, to
    /// compare.
    pub comparing: bool,
}

const HEADER: &str =
    "# tatum compose: the loop. The `use` lines are rewritten from the screen; every other line is kept.";

impl Session {
    /// Open `path` -- a session file, or a directory holding one as
    /// `session.synth` -- and the library beside it. A session that does
    /// not exist yet is written with the first groove and the first bass.
    pub fn open(path: &Path, knob_ccs: Vec<u8>) -> Result<Self, String> {
        let path = if path.is_dir() { path.join("session.synth") } else { path.to_path_buf() };
        let root = path.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
        let library = Library::scan(&root)?;
        let mut session = Self {
            slots: library.roles.iter().map(|r| Slot { role: r.clone(), voice: None, held: Vec::new() }).collect(),
            library,
            path: path.clone(),
            groove: 0,
            groove_held: Vec::new(),
            extra: Vec::new(),
            knob_ccs,
            comparing: false,
        };
        if path.exists() {
            session.read()?;
        } else {
            if let Some(slot) = session.slots.iter_mut().find(|s| s.role == "bass") {
                slot.voice = session.library.of("bass").first().map(|v| v.name.clone());
            }
            session.write()?;
        }
        Ok(session)
    }

    /// What the file says the loop is: its `use` lines, by where they point.
    fn read(&mut self) -> Result<(), String> {
        let text = fs::read_to_string(&self.path).map_err(|e| format!("cannot read {}: {}", self.path.display(), e))?;
        self.extra.clear();
        for line in text.lines() {
            if line.trim_start().starts_with("# tatum compose") {
                continue;
            }
            let Some(Ok(target)) = crate::include::use_target(line) else {
                self.extra.push(line.to_string());
                continue;
            };
            let parts: Vec<&str> = target.split('/').collect();
            let stem = |s: &str| s.trim_end_matches(".synth").to_string();
            match parts.as_slice() {
                ["grooves", name] => match self.library.grooves.iter().position(|g| g.name == stem(name)) {
                    Some(i) => self.groove = i,
                    None => self.extra.push(line.to_string()),
                },
                ["voices", role, name] => match self.slots.iter_mut().find(|s| s.role == *role) {
                    Some(slot) if self.library.of(role).iter().any(|v| v.name == stem(name)) => {
                        slot.voice = Some(stem(name));
                    }
                    _ => self.extra.push(line.to_string()),
                },
                _ => self.extra.push(line.to_string()),
            }
        }
        Ok(())
    }

    /// The session file as the loop is now: a save the watcher picks up.
    fn write(&self) -> Result<(), String> {
        let mut text = String::from(HEADER);
        text.push('\n');
        text.push_str(&format!("use \"grooves/{}.synth\"\n", self.library.grooves[self.groove].name));
        for slot in &self.slots {
            if let Some(v) = &slot.voice {
                text.push_str(&format!("use \"voices/{}/{}.synth\"\n", slot.role, v));
            }
        }
        for line in &self.extra {
            text.push_str(line);
            text.push('\n');
        }
        fs::write(&self.path, text).map_err(|e| format!("cannot write {}: {}", self.path.display(), e))
    }

    /// Row 0 is the groove; the slots follow.
    fn voice_at(&self, row: usize) -> Option<&Voice> {
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

    /// What the screen draws: every row, and the knobs of the row chosen,
    /// where they sit -- turned, or where the text leaves them.
    pub fn view(&self, planner: &LivePlanner, focus: usize) -> View {
        let groove = &self.library.grooves[self.groove];
        let rows = std::iter::once(Row {
            role: "drums".into(),
            name: Some(groove.name.clone()),
            kind: "groove".into(),
            doc: groove.doc.clone(),
            track: groove.track.clone(),
            index: Some(self.groove + 1),
            count: self.library.grooves.len(),
            held: !self.groove_held.is_empty(),
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
            }
        }))
        .collect();
        let macros = match self.voice_at(focus) {
            Some(voice) => {
                let held = self.held_at(focus);
                voice
                    .macros()
                    .into_iter()
                    .map(|(label, words)| {
                        let dotted = words.replace(' ', ".");
                        let turned = held.iter().find(|(t, _)| *t == dotted).map(|(_, v)| *v);
                        let knob = planner.probe_knob(&dotted);
                        let rest = knob.as_ref().and_then(|k| planner.song().and_then(|s| k.rest_position(s)));
                        let position = turned.or(rest).unwrap_or(0);
                        // `bass cutoff 1.2khz` without its words: the row names them.
                        let reading = knob
                            .as_ref()
                            .map(|k| k.reading(position))
                            .map(|r| r.strip_prefix(&words).map(|v| v.trim_start().to_string()).unwrap_or(r))
                            .unwrap_or_else(|| "—".into());
                        MacroRow { label, position, reading, held: turned.is_some() }
                    })
                    .collect()
            }
            None => Vec::new(),
        };
        View { rows, macros, comparing: self.comparing, knob_ccs: self.knob_ccs.clone() }
    }

    /// Do what a key or a knob asked. Choosing a voice writes the file,
    /// which the watcher turns into a swap; turning a knob holds a value on
    /// the engine, returned as plans to send.
    pub fn act(
        &mut self,
        action: Action,
        planner: &mut LivePlanner,
        generation: u64,
    ) -> Result<(Vec<Plan>, String), String> {
        match action {
            Action::Choose { row, by } => {
                if row == 0 {
                    let n = self.library.grooves.len();
                    self.groove = (self.groove as i64 + by as i64).rem_euclid(n as i64) as usize;
                    self.groove_held.clear();
                    self.write()?;
                    let g = &self.library.grooves[self.groove];
                    return Ok((Vec::new(), format!("groove {}: {} · on the next bar", g.name, g.doc)));
                }
                let slot = self.slots.get(row - 1).ok_or("no such row")?;
                let names: Vec<String> = self.library.of(&slot.role).iter().map(|v| v.name.clone()).collect();
                if names.is_empty() {
                    return Err(format!("no voices in voices/{}", slot.role));
                }
                // The choices run: nothing, then every voice, around.
                let at = slot.voice.as_ref().and_then(|v| names.iter().position(|n| n == v)).map_or(0, |i| i + 1);
                let n = names.len() + 1;
                let next = (at as i64 + by as i64).rem_euclid(n as i64) as usize;
                let slot = &mut self.slots[row - 1];
                slot.voice = if next == 0 { None } else { Some(names[next - 1].clone()) };
                slot.held.clear();
                self.write()?;
                let said = match self.voice_at(row) {
                    Some(v) => format!("{}: {} · {} · on the next bar", v.role, v.name, v.doc),
                    None => format!("{}: nothing · on the next bar", self.slots[row - 1].role),
                };
                Ok((Vec::new(), said))
            }
            Action::Turn { row, knob, value } => {
                let voice = self.voice_at(row).ok_or("nothing to turn")?;
                let (_, words) = voice.macros().into_iter().nth(knob).ok_or("no such knob")?;
                let dotted = words.replace(' ', ".");
                let held = self.held_at_mut(row).ok_or("no such row")?;
                match held.iter_mut().find(|(t, _)| *t == dotted) {
                    Some(h) => h.1 = value,
                    None => held.push((dotted.clone(), value)),
                }
                self.comparing = false;
                let turn = planner.try_knob(&dotted, value, generation);
                let said = turn.readings.first().cloned().unwrap_or_else(|| format!("{}: moves nothing", words));
                Ok((turn.plans, said))
            }
            Action::Mute(row) => {
                let voice = self.voice_at(row).ok_or("nothing to mute")?;
                let track = voice.track.clone();
                Ok((planner.toggle_mute(&track, generation), format!("{} muted or back", track)))
            }
            Action::Reset(row) => {
                let held = self.held_at_mut(row).ok_or("no such row")?;
                let targets: Vec<String> = held.drain(..).map(|(t, _)| t).collect();
                if targets.is_empty() {
                    return Ok((Vec::new(), "nothing turned here".into()));
                }
                let plans = self.rest_plans(&targets, planner, generation);
                Ok((plans, format!("{} knobs back to the text", targets.len())))
            }
            Action::Compare => {
                let held: Vec<(String, u8)> = std::iter::once(&self.groove_held)
                    .chain(self.slots.iter().map(|s| &s.held))
                    .flatten()
                    .cloned()
                    .collect();
                if held.is_empty() {
                    return Ok((Vec::new(), "nothing turned to compare with".into()));
                }
                self.comparing = !self.comparing;
                let plans = if self.comparing {
                    let targets: Vec<String> = held.iter().map(|(t, _)| t.clone()).collect();
                    self.rest_plans(&targets, planner, generation)
                } else {
                    held.iter().flat_map(|(t, v)| planner.try_knob(t, *v, generation).plans).collect()
                };
                let said = if self.comparing { "A: as the text has it" } else { "B: as you turned it" };
                Ok((plans, said.into()))
            }
            Action::Save { row, name } => {
                let path = self.save_voice(row, &name, planner)?;
                Ok((Vec::new(), format!("saved {} · playing it on the next bar", path.display())))
            }
        }
    }

    /// Every target put back where the text leaves it.
    fn rest_plans(&self, targets: &[String], planner: &mut LivePlanner, generation: u64) -> Vec<Plan> {
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
    pub fn reapply(&self, planner: &mut LivePlanner, generation: u64) -> Vec<Plan> {
        if self.comparing {
            return Vec::new();
        }
        std::iter::once(&self.groove_held)
            .chain(self.slots.iter().map(|s| &s.held))
            .flatten()
            .flat_map(|(t, v)| planner.try_knob(t, *v, generation).plans)
            .collect()
    }

    /// The knobs a row is on, in `knob_ccs` order: controller `cc` turns
    /// knob `i` of the chosen row.
    pub fn knob_index(&self, cc: u8) -> Option<usize> {
        self.knob_ccs.iter().position(|&c| c == cc)
    }

    /// Write the voice of `row` as it sounds now -- its file with every
    /// knob turned written into it, in units -- as `voices/<role>/<name>.synth`,
    /// and switch the slot to it. The same name writes over the voice
    /// playing; another name is a new voice, and refuses to replace one.
    fn save_voice(&mut self, row: usize, name: &str, planner: &LivePlanner) -> Result<PathBuf, String> {
        let name = name.trim();
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            return Err("a voice's name is letters, digits, _ and -".into());
        }
        let voice = self.voice_at(row).ok_or("nothing to save here")?.clone();
        let dir = voice.path.parent().ok_or("the voice has no directory")?.to_path_buf();
        let out = dir.join(format!("{}.synth", name));
        if out != voice.path && out.exists() {
            return Err(format!("{} exists already: another name, or its own to write over it", out.display()));
        }
        let mut text =
            fs::read_to_string(&voice.path).map_err(|e| format!("cannot read {}: {}", voice.path.display(), e))?;
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
        if out != voice.path {
            text = text.replacen('\n', &format!("\n# saved from {} with tatum compose\n", voice.name), 1);
        }
        fs::write(&out, text).map_err(|e| format!("cannot write {}: {}", out.display(), e))?;
        self.library = Library::scan(&self.library.root)?;
        if row == 0 {
            self.groove = self.library.grooves.iter().position(|g| g.name == name).unwrap_or(0);
            self.groove_held.clear();
        } else {
            let slot = &mut self.slots[row - 1];
            slot.voice = Some(name.to_string());
            slot.held.clear();
        }
        self.comparing = false;
        self.write()?;
        Ok(out)
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

/// A module parameter's value as its line writes it: in units where it has
/// them, by name for a choice, a plain number otherwise.
fn write_param(spec: &params::ParamSpec, value: f32) -> String {
    if let Range::Choice(_) = spec.range {
        return spec.choice_name(value).map(String::from).unwrap_or_else(|| format!("{:.3}", value));
    }
    spec.write_in_units(value).unwrap_or_else(|| format!("{:.3}", value))
}

/// Edits to a voice's text that keep everything else as written.
mod edit {
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

const USAGE: &str = "usage: tatum compose [<dir> | <session.synth>] [--knobs 74,71,76,77,93,18,19,16] [--device <name>] [--rate <hz>] [--midi <name>] [--buffer <frames>] [--glass]";

pub fn cmd(args: &[String]) {
    let mut path: Option<String> = None;
    let mut rest: Vec<String> = Vec::new();
    let mut knob_ccs = DEFAULT_KNOB_CCS.to_vec();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--knobs" => {
                i += 1;
                let list = args
                    .get(i)
                    .and_then(|v| v.split(',').map(|n| n.trim().parse::<u8>().ok()).collect::<Option<Vec<u8>>>());
                match list {
                    Some(l) if !l.is_empty() && l.iter().all(|&c| c < 128) => knob_ccs = l,
                    _ => {
                        eprintln!("error: --knobs takes controller numbers separated by commas, like 74,71,76,77");
                        std::process::exit(1);
                    }
                }
            }
            "--device" | "-d" | "--rate" | "--midi" | "--buffer" => {
                rest.push(args[i].clone());
                i += 1;
                if let Some(v) = args.get(i) {
                    rest.push(v.clone());
                }
            }
            "--glass" | "--list-devices" | "--list-midi" => rest.push(args[i].clone()),
            "--help" | "-h" => {
                eprintln!("{}", USAGE);
                return;
            }
            other if other.starts_with('-') => {
                eprintln!("error: unknown flag '{}'\n{}", other, USAGE);
                std::process::exit(1);
            }
            other => path = Some(other.to_string()),
        }
        i += 1;
    }
    let path = path.unwrap_or_else(|| "compose".to_string());
    let session = match Session::open(Path::new(&path), knob_ccs) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {}", e);
            std::process::exit(1);
        }
    };
    let session_path = session.path.to_string_lossy().into_owned();
    crate::live::cmd_with(&session_path, &rest, Some(session));
}

#[cfg(test)]
mod tests {
    use super::edit::{block, rename_module, set_option, set_wet};
    use super::*;
    use crate::tui::compose::Action;

    /// The repo's own library, copied where a test can write.
    fn library_copy(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tatum-compose-{}-{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../compose");
        for sub in ["grooves", "voices/bass", "voices/lead", "voices/pad", "voices/perc"] {
            fs::create_dir_all(dir.join(sub)).unwrap();
            for f in fs::read_dir(src.join(sub)).unwrap().flatten() {
                fs::copy(f.path(), dir.join(sub).join(f.file_name())).unwrap();
            }
        }
        dir
    }

    fn planner_for(session: &Session) -> LivePlanner {
        let source = crate::include::Source::load(&session.path).unwrap();
        let mut planner = LivePlanner::new();
        planner.plan(&source.text, 0).unwrap();
        planner
    }

    #[test]
    fn a_new_session_starts_with_a_groove_and_a_bass_and_chooses_by_writing_the_file() {
        let dir = library_copy("new");
        let mut session = Session::open(&dir, DEFAULT_KNOB_CCS.to_vec()).unwrap();
        let text = fs::read_to_string(dir.join("session.synth")).unwrap();
        assert!(text.contains("use \"grooves/dark.synth\""), "{text}");
        assert!(text.contains("use \"voices/bass/acid.synth\""), "{text}");
        assert_eq!(session.slots.iter().map(|s| s.role.as_str()).collect::<Vec<_>>(), ["bass", "lead", "pad", "perc"]);

        let mut planner = planner_for(&session);
        // Row 2 is the lead: from nothing, the first of its library.
        let (plans, said) = session.act(Action::Choose { row: 2, by: 1 }, &mut planner, 0).unwrap();
        assert!(plans.is_empty());
        assert!(said.starts_with("lead: bell"), "{said}");
        let text = fs::read_to_string(dir.join("session.synth")).unwrap();
        assert!(text.contains("use \"voices/lead/bell.synth\""), "{text}");
        // Back one is nothing again; the line goes.
        session.act(Action::Choose { row: 2, by: -1 }, &mut planner, 0).unwrap();
        let text = fs::read_to_string(dir.join("session.synth")).unwrap();
        assert!(!text.contains("voices/lead/"), "{text}");
        // The groove row wraps around its library.
        session.act(Action::Choose { row: 0, by: -1 }, &mut planner, 0).unwrap();
        let text = fs::read_to_string(dir.join("session.synth")).unwrap();
        assert!(text.contains("use \"grooves/psy.synth\""), "{text}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_hand_written_session_keeps_its_other_lines() {
        let dir = library_copy("kept");
        fs::write(
            dir.join("session.synth"),
            "use \"grooves/dub.synth\"\nuse \"voices/pad/nebula.synth\"\ntempo 118\ntrack pad { level 0.1 }\n",
        )
        .unwrap();
        let mut session = Session::open(&dir, vec![1, 2]).unwrap();
        assert_eq!(session.library.grooves[session.groove].name, "dub");
        assert_eq!(session.slots[2].voice.as_deref(), Some("nebula"));
        let mut planner = planner_for(&session);
        session.act(Action::Choose { row: 1, by: 1 }, &mut planner, 0).unwrap();
        let text = fs::read_to_string(dir.join("session.synth")).unwrap();
        assert!(text.contains("use \"voices/bass/acid.synth\""), "{text}");
        assert!(text.contains("use \"voices/pad/nebula.synth\""), "{text}");
        assert!(text.ends_with("tempo 118\ntrack pad { level 0.1 }\n"), "{text}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_knob_turned_is_held_then_written_into_the_saved_voice() {
        let dir = library_copy("save");
        let mut session = Session::open(&dir, DEFAULT_KNOB_CCS.to_vec()).unwrap();
        let mut planner = planner_for(&session);
        // The bass's first knob is its cutoff; all the way up.
        let (plans, said) = session.act(Action::Turn { row: 1, knob: 0, value: 127 }, &mut planner, 0).unwrap();
        assert!(!plans.is_empty(), "nothing held");
        assert!(said.starts_with("bass cutoff"), "{said}");
        let view = session.view(&planner, 1);
        assert_eq!(view.macros[0].label, "cutoff");
        assert_eq!(view.macros[0].position, 127);
        assert!(view.macros[0].held);
        // The drive's wet, down to nothing.
        let drive = view.macros.iter().position(|m| m.label == "drive wet").unwrap();
        session.act(Action::Turn { row: 1, knob: drive, value: 0 }, &mut planner, 0).unwrap();

        session.act(Action::Save { row: 1, name: "acid_open".into() }, &mut planner, 0).unwrap();
        let saved = fs::read_to_string(dir.join("voices/bass/acid_open.synth")).unwrap();
        assert!(saved.contains("module bass acid_open {"), "{saved}");
        assert!(saved.contains("    using acid_open\n"), "{saved}");
        assert!(saved.contains("    cutoff 20000hz\n"), "{saved}");
        assert!(saved.contains("saturate(0.6, wet=0.00) as drive"), "{saved}");
        assert!(saved.contains("# saved from acid with tatum compose"), "{saved}");
        // The original is as it was; the session plays the new one, with
        // nothing held any more.
        let original = fs::read_to_string(dir.join("voices/bass/acid.synth")).unwrap();
        assert!(original.contains("    cutoff 260hz\n"));
        let text = fs::read_to_string(dir.join("session.synth")).unwrap();
        assert!(text.contains("use \"voices/bass/acid_open.synth\""), "{text}");
        assert!(session.slots[0].held.is_empty());
        // Another voice's name is refused.
        let Err(err) = session.act(Action::Save { row: 1, name: "roller".into() }, &mut planner, 0) else {
            panic!("a name another voice has was accepted");
        };
        assert!(err.contains("exists already"), "{err}");
        let _ = fs::remove_dir_all(&dir);
    }

    const VOICE: &str = "# a bass\nmodule bass low {\n    cutoff 110hz\n    cutoff_env 750hz\n    sustain 0.9\n}\npattern line { 1.1 - - - }\ntrack bass {\n    play line\n    using low\n    level 0.5\n    out > saturate(0.3) as drive > lowpass(2khz, 0.1, wet=1) as lp > master\n}\n";

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
}
