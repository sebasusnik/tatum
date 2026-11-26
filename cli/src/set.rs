//! `tatum set` — a live set as a sequence of states.
//!
//! A set is a directory of numbered `.synth` files. Each one is the whole rig
//! at a moment; the set is the walk from one to the next. The engine moves
//! between them with the same hot swap `tatum watch` uses, so a state that
//! kept a voice's name keeps that voice: nothing restarts at a transition.
//!
//! There is no manifest. How long a step lasts travels in the file, on a
//! comment the DSL already ignores:
//!
//!     # set: bars=48 phase=warm-up energy=2
//!     # set-note: brought the reese in under the pad
//!
//! A manifest beside the files is a second thing to keep in sync, and the
//! first time it drifts the set plays in the wrong order for reasons nobody
//! can see. The header cannot drift from the file it is in.

use std::fs;
use std::path::{Path, PathBuf};

use tatum_core::live::{LivePlanner, LivePlayer};
use tatum_core::song_engine::SongEngine;
use tatum_core::{BLOCK_SIZE, SAMPLE_RATE};

pub const DEFAULT_BARS: u32 = 32;

pub struct Step {
    pub path: PathBuf,
    pub src: String,
    pub bars: u32,
    pub phase: String,
    pub energy: Option<f32>,
    pub note: String,
    /// `blend=8`: coming into this step in `set play`, the step before keeps
    /// playing under it for this many bars, DJ style.
    pub blend: Option<f32>,
    /// `# cue: 9 B1 + K3, the grinder by hand`: what to play and on which bar
    /// of the step (from 1, fractions for beats: 8.75 is the last beat of
    /// bar 8). `set play --tui` shows the cue that comes next, so a set can
    /// carry its own practice sheet.
    pub cues: Vec<(f32, String)>,
    /// `perform=drop` in the header: the scene the keyboard comes into
    /// when this step lands, as if its key had been pressed.
    pub perform: Option<String>,
    /// `auto` or `auto=2` in the header: a bridge, a break, a build. In
    /// `set play` the step plays its bars this many times and then moves on
    /// by itself, where any other step loops until the performer moves; the
    /// performer can still keep it (Shift+L).
    pub auto: Option<u32>,
    /// `max=2` in the header: in `set play`, the most times the step plays
    /// its bars before it moves on by itself, so a step left alone does not
    /// loop for ever. `set render` plays it once, as written.
    pub max: Option<u32>,
}

impl Step {
    pub fn name(&self) -> String {
        self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }

    /// Bars the step plays when the set walks on its own: its header's bars,
    /// as many times as its `auto=` says.
    pub fn length(&self) -> usize {
        self.bars as usize * self.auto.unwrap_or(1) as usize
    }

    /// The passes it plays in `set play` before moving on by itself, if it
    /// does: its `auto=`, else its `max=`.
    pub fn passes(&self) -> Option<u32> {
        self.auto.or(self.max)
    }
}

/// `max=N` in the `# set:` header.
fn parse_max(src: &str) -> Option<u32> {
    src.lines()
        .filter_map(|l| l.trim().strip_prefix("# set:").or_else(|| l.trim().strip_prefix("#set:")))
        .flat_map(str::split_whitespace)
        .filter_map(|f| f.strip_prefix("max=")?.parse::<u32>().ok().filter(|n| *n > 0))
        .next_back()
}

/// `auto` or `auto=N` in the `# set:` header: the passes a step plays before
/// it moves on by itself. A bare `auto` is one.
fn parse_auto(src: &str) -> Option<u32> {
    src.lines()
        .filter_map(|l| l.trim().strip_prefix("# set:").or_else(|| l.trim().strip_prefix("#set:")))
        .flat_map(str::split_whitespace)
        .filter_map(|f| match f.split_once('=') {
            None if f == "auto" => Some(1),
            Some(("auto", n)) => n.parse::<u32>().ok().filter(|n| *n > 0),
            _ => None,
        })
        .next_back()
}

/// `# set: bars=48 phase=warm-up energy=2` and `# set-note: ...`, anywhere in
/// the file. Unknown keys are ignored rather than rejected: the header is a
/// note to the tooling, not part of the language, and a set written by hand
/// should not fail to play because someone added a word.
fn parse_header(src: &str, default_bars: u32) -> (u32, String, Option<f32>, String, Option<f32>) {
    let (mut bars, mut phase, mut energy, mut note) = (default_bars, String::new(), None, String::new());
    let mut blend = None;
    for line in src.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("# set-note:").or_else(|| t.strip_prefix("#set-note:")) {
            note = rest.trim().to_string();
            continue;
        }
        let Some(rest) = t.strip_prefix("# set:").or_else(|| t.strip_prefix("#set:")) else { continue };
        for field in rest.split_whitespace() {
            let Some((k, v)) = field.split_once('=') else { continue };
            match k {
                "bars" => {
                    if let Ok(n) = v.parse::<u32>() {
                        if n > 0 {
                            bars = n;
                        }
                    }
                }
                "phase" => phase = v.to_string(),
                "energy" => energy = v.parse::<f32>().ok(),
                "blend" => blend = v.parse::<f32>().ok().filter(|b| *b >= 0.0),
                _ => {}
            }
        }
    }
    (bars, phase, energy, note, blend)
}

/// `perform=<scene>` in the `# set:` header.
fn parse_perform(src: &str) -> Option<String> {
    src.lines()
        .filter_map(|l| l.trim().strip_prefix("# set:").or_else(|| l.trim().strip_prefix("#set:")))
        .flat_map(str::split_whitespace)
        .filter_map(|f| f.strip_prefix("perform="))
        .next_back()
        .map(String::from)
}

/// Every `# cue: <bar> <what>` in the file, in bar order. A line whose bar
/// does not read as a number is skipped, like an unknown header key.
fn parse_cues(src: &str) -> Vec<(f32, String)> {
    let mut cues: Vec<(f32, String)> = src
        .lines()
        .filter_map(|l| l.trim().strip_prefix("# cue:").or_else(|| l.trim().strip_prefix("#cue:")))
        .filter_map(|rest| {
            let (bar, what) = rest.trim().split_once(char::is_whitespace)?;
            let bar = bar.parse::<f32>().ok().filter(|b| *b >= 1.0)?;
            Some((bar, what.trim().to_string()))
        })
        .collect();
    cues.sort_by(|a, b| a.0.total_cmp(&b.0));
    cues
}

pub fn load(dir: &Path, default_bars: u32) -> Result<Vec<Step>, String> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|e| format!("{}: {}", dir.display(), e))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "synth"))
        // A leading underscore means "not a step". `use` resolves relative to
        // the file that writes it, so a candidate has to sit in the set's own
        // directory for the rig path to work -- and then it must not also be
        // read as part of the set.
        .filter(|p| !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('_')))
        .collect();
    files.sort();
    if files.is_empty() {
        return Err(format!("{}: no .synth files", dir.display()));
    }
    files
        .into_iter()
        .map(|path| {
            // Resolved through `use`, so a step can be the rig plus its overrides
            // rather than a copy of the whole rig. The header is read from the raw
            // file, not the expansion: `bars` belongs to the step, and a rig that
            // carried one would otherwise leak into every step that includes it.
            let raw = fs::read_to_string(&path).map_err(|e| format!("{}: {}", path.display(), e))?;
            let src = crate::include::Source::load(&path)?.text;
            let (bars, phase, energy, note, blend) = parse_header(&raw, default_bars);
            let cues = parse_cues(&raw);
            let perform = parse_perform(&raw);
            let auto = parse_auto(&raw);
            let max = parse_max(&raw);
            Ok(Step { path, src, bars, phase, energy, note, blend, cues, perform, auto, max })
        })
        .collect()
}

/// The output gain that brings the loudest step of the set to the engine's
/// target. Every step plays at it, so a quiet intro stays quiet and the peak
/// of the set is where one song would be, instead of each step levelled on
/// its own or all of them on the first.
pub fn set_gain(steps: &[Step]) -> Result<f32, String> {
    // Every step is rendered to be measured, and an hour-long set has a
    // hundred of them: one per core, or `set play` sits for a minute before
    // the first note.
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(steps.len().max(1));
    let chunk = steps.len().div_ceil(threads).max(1);
    let measured: Vec<Result<Option<f32>, String>> = std::thread::scope(|scope| {
        let workers: Vec<_> = steps
            .chunks(chunk)
            .map(|part| scope.spawn(move || part.iter().map(step_loudness).collect::<Vec<_>>()))
            .collect();
        workers.into_iter().flat_map(|w| w.join().expect("a measuring thread panicked")).collect()
    });
    let mut loudest: Option<f32> = None;
    for lufs in measured {
        if let Some(lufs) = lufs? {
            loudest = Some(loudest.map_or(lufs, |l: f32| l.max(lufs)));
        }
    }
    Ok(tatum_core::output::gain_for(loudest))
}

/// One step's loudness, measured as it plays: what a `hold` pad keeps out is
/// not heard.
fn step_loudness(step: &Step) -> Result<Option<f32>, String> {
    let mut ast = tatum_core::dsl::parse(&step.src)
        .map_err(|e| format!("{}: {}", step.name(), describe(&tatum_core::song_engine::DslError::Parse(e))))?;
    tatum_core::midi::at_rest(&mut ast);
    let compiled = tatum_core::dsl::compiler::compile(&ast)
        .map_err(|e| format!("{}: {}", step.name(), describe(&tatum_core::song_engine::DslError::Compile(e))))?;
    Ok(SongEngine::loudness(&compiled))
}

pub struct Rendered {
    pub l: Vec<f32>,
    pub r: Vec<f32>,
    /// Per step: its name, and the sample it starts on in the output.
    pub applied: Vec<(String, usize)>,
    /// The steps, handed back.
    pub steps: Vec<Step>,
    /// What a script did that is worth reading, at the sample it happened:
    /// `bar 12.00: acid cutoff 1.2khz`, a move asked for.
    pub log: Vec<(usize, String)>,
}

/// How `set render` moves between steps: the controls of `set play`, with
/// nobody at them.
pub struct Walk {
    /// A step lands on a bar that is a multiple of this. 1 lands every step
    /// exactly where the headers say; 8 is what `set play` does by default,
    /// and moves a step whose predecessor ends off the line to the next one.
    pub phrase: usize,
    /// Bars a change of tempo between steps takes. 0 jumps.
    pub ramp_bars: f32,
    /// Bars the outgoing step keeps playing under the new one; a step's own
    /// `# set: blend=` wins.
    pub blend_bars: f32,
    /// Walk it as `set play` does, which is how a take plays back: a step
    /// whose header says `auto` or `max=` moves on by itself, and nothing
    /// but the script asks for a step.
    pub live: bool,
    /// With `live`, `set play --auto`: every step moves on by itself.
    pub auto: bool,
}

impl Default for Walk {
    fn default() -> Self {
        Self { phrase: 1, ramp_bars: 4.0, blend_bars: 0.0, live: false, auto: false }
    }
}

/// The bar each step starts on, and the bar the set ends on: each step for
/// the bars its header gives, moved on to the next phrase line.
pub fn landings(steps: &[Step], phrase: usize) -> (Vec<usize>, usize) {
    let phrase = phrase.max(1);
    let mut at = 0usize;
    let mut lands = Vec::with_capacity(steps.len());
    for (i, s) in steps.iter().enumerate() {
        lands.push(at);
        at += s.length();
        if i + 1 < steps.len() {
            at = at.div_ceil(phrase) * phrase;
        }
    }
    (lands, at)
}

/// One thing a scripted performer does, at a bar from the top of the set.
#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    Cc(u8, u8),
    Pad(u8, u8),
    Key(u8, u8),
    /// The pitch strip, 0..16383 with 8192 at rest.
    Bend(u16),
    Next,
    Prev,
    Step(usize),
    /// A performance scene, called as its computer key would call it: it
    /// takes the keyboard on the bar the song's `keyboard` block says.
    Perform(String),
    /// Aftertouch: a held pad (`true`) or key pressed this hard; on one
    /// note, or with no note the whole channel's.
    Pressure(bool, Option<u8>, u8),
    /// The knobs' page: `voice next`, `voice prev`, `voice bass`.
    Voice(tatum_core::dsl::ast::VoiceMove),
    /// Every knob touched back to the text, as the reset key does.
    Reset,
    /// `l` down (`true`) and up: a loop of the bars it is held over.
    Loop(bool),
    /// Shift+L: keep a step that would move on by itself, or let it go.
    Keep,
    /// A track muted or unmuted from the screen.
    Mute(String),
    /// Tracks soloed together from the screen, or the solo taken off.
    Solo(Vec<String>),
    End,
}

impl Act {
    fn navigates(&self) -> bool {
        matches!(self, Act::Next | Act::Prev | Act::Step(_))
    }

    /// The act as a script line writes it, after the time: what
    /// [`parse_script`] reads back as the same act.
    pub fn script(&self) -> String {
        use tatum_core::dsl::ast::VoiceMove;
        match self {
            Act::Cc(cc, v) => format!("cc {} {}", cc, v),
            Act::Pad(n, v) => format!("pad {} {}", n, v),
            Act::Key(n, v) => format!("key {} {}", n, v),
            Act::Bend(v) => format!("bend {}", v),
            Act::Next => String::from("next"),
            Act::Prev => String::from("prev"),
            Act::Step(n) => format!("step {}", n),
            Act::Perform(scene) => format!("perform {}", scene),
            Act::Pressure(pad, note, v) => {
                let what = if *pad { "pad" } else { "key" };
                match note {
                    Some(n) => format!("press {} {} {}", what, n, v),
                    None => format!("press {} {}", what, v),
                }
            }
            Act::Voice(VoiceMove::Next) => String::from("voice next"),
            Act::Voice(VoiceMove::Prev) => String::from("voice prev"),
            Act::Voice(VoiceMove::To(page)) => format!("voice {}", page),
            Act::Reset => String::from("reset"),
            Act::Loop(true) => String::from("loop down"),
            Act::Loop(false) => String::from("loop up"),
            Act::Keep => String::from("keep"),
            Act::Mute(track) => format!("mute {}", track),
            Act::Solo(tracks) => format!("solo {}", tracks.join(" ")),
            Act::End => String::from("end"),
        }
    }
}

/// One line of a script: when, and what.
#[derive(Debug, Clone, PartialEq)]
pub struct Cue {
    /// Bars from the set's first downbeat, fractional.
    pub bar: f64,
    /// The sample it lands on, when the line gives one (`12.5@1058400`): a
    /// take writes it, so the act lands on the block it landed on live.
    pub sample: Option<usize>,
    pub act: Act,
}

impl Cue {
    #[cfg(test)]
    pub fn at(bar: f64, act: Act) -> Self {
        Self { bar, sample: None, act }
    }

    /// The line a take writes: `12.503906@529152  cc 74 64`.
    pub fn line(&self) -> String {
        match self.sample {
            Some(s) => format!("{:.6}@{}  {}", self.bar, s, self.act.script()),
            None => format!("{:.6}  {}", self.bar, self.act.script()),
        }
    }
}

/// How a take was played, from its `# take:` line, so it renders the way it
/// played without the flags written again.
#[derive(Debug, Clone, PartialEq)]
pub struct TakeHeader {
    pub phrase: usize,
    pub ramp_bars: f32,
    pub blend_bars: f32,
    pub auto: bool,
    /// The bars a step without `bars=` holds.
    pub bars: u32,
    /// The steps' text as they played, folded (see [`fingerprint`]).
    pub steps: Option<u64>,
}

impl TakeHeader {
    pub fn line(&self) -> String {
        let mut line = format!(
            "# take: phrase={} ramp={} blend={} auto={} bars={}",
            self.phrase,
            self.ramp_bars,
            self.blend_bars,
            if self.auto { "on" } else { "off" },
            self.bars
        );
        if let Some(f) = self.steps {
            line += &format!(" steps={:016x}", f);
        }
        line
    }

    /// Whether a `# take:` line says anything a take's header says.
    fn has_fields(rest: &str) -> bool {
        const KEYS: [&str; 6] = ["phrase", "ramp", "blend", "auto", "bars", "steps"];
        rest.split_whitespace().filter_map(|f| f.split_once('=')).any(|(k, _)| KEYS.contains(&k))
    }

    fn parse(rest: &str) -> Result<Self, String> {
        let mut take =
            TakeHeader { phrase: 8, ramp_bars: 4.0, blend_bars: 0.0, auto: false, bars: DEFAULT_BARS, steps: None };
        for field in rest.split_whitespace() {
            let Some((k, v)) = field.split_once('=') else { continue };
            let bad = || format!("`# take:` cannot read {}", field);
            match k {
                "phrase" => take.phrase = v.parse().ok().filter(|n| *n > 0).ok_or_else(bad)?,
                "ramp" => take.ramp_bars = v.parse().ok().filter(|n: &f32| *n >= 0.0).ok_or_else(bad)?,
                "blend" => take.blend_bars = v.parse().ok().filter(|n: &f32| *n >= 0.0).ok_or_else(bad)?,
                "auto" => take.auto = v == "on",
                "bars" => take.bars = v.parse().ok().filter(|n| *n > 0).ok_or_else(bad)?,
                "steps" => take.steps = Some(u64::from_str_radix(v, 16).map_err(|_| bad())?),
                _ => {}
            }
        }
        Ok(take)
    }

    /// The walk the take was played with.
    pub fn walk(&self) -> Walk {
        Walk {
            phrase: self.phrase,
            ramp_bars: self.ramp_bars,
            blend_bars: self.blend_bars,
            live: true,
            auto: self.auto,
        }
    }
}

/// The steps' text, folded to a number (FNV-1a): a take says which set it
/// was played on, so a render of it can say when the files have changed.
pub fn fingerprint(steps: &[Step]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for s in steps {
        for b in s.name().bytes().chain([0]).chain(s.src.bytes()).chain([0]) {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

/// A script read: its lines in the order they act, and the `# take:` line a
/// recorded take starts with.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Script {
    pub cues: Vec<Cue>,
    pub take: Option<TakeHeader>,
}

/// A performance script: one event a line, `<bar> <what>`, the bar counted
/// from 0 at the first downbeat of the set and fractional (`18.5` is the
/// middle of bar 19). `#` starts a comment.
///
/// A take (`set play --record`) writes `<bar>@<sample>`: the bar to read and
/// the sample of the set the act landed on, which is where it lands again. A
/// script whose every line has one acts in sample order, any other in bar
/// order. A take's first line, `# take: phrase=8 ramp=4 blend=0 auto=off
/// bars=32`, says how it was played, and `set render` plays it back so.
///
///     12.0  cc 74 64        a knob or fader to 64
///     18.0  pad 44 110      a pad down, struck at 110
///     19.0  pad 44 0        and up
///     20.0  key 48 100      a key down (channel 1), `key 48 0` up
///     21.0  bend 12000      the pitch strip
///     35.0  next            ask for the next step (also `prev`, `step 3`)
///     40.0  perform drop    call a scene, as its key in `keyboard` would
///     41.0  press pad 40 90 aftertouch on a held pad (or `press key 45 90`)
///     42.0  voice bass      the knobs' page (also `voice next`, `voice prev`)
///     64.0  end             stop here
///     70.0  loop down       `l`, a loop of bars (`loop up` closes it)
///     80.0  keep            Shift+L
///     81.0  mute bass       a track muted from the screen (`solo bass lead`)
pub fn parse_script(text: &str) -> Result<Script, String> {
    let mut out = Vec::new();
    let mut take = None;
    let mut first = true;
    for (n, raw) in text.lines().enumerate() {
        // A take's header is its first line, and says how it was played in
        // `key=value` fields; a `# take: second attempt` anywhere is a comment.
        let opening = first && !raw.trim().is_empty();
        if opening {
            first = false;
            if let Some(rest) = raw.trim().strip_prefix("# take:").filter(|r| TakeHeader::has_fields(r)) {
                take = Some(TakeHeader::parse(rest).map_err(|e| format!("script line {}: {}", n + 1, e))?);
                continue;
            }
        }
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let bad = |why: &str| format!("script line {}: {} (`{}`)", n + 1, why, line);
        let w: Vec<&str> = line.split_whitespace().collect();
        let (bar, sample) = match w[0].split_once('@') {
            Some((bar, at)) => {
                (bar, Some(at.parse::<usize>().map_err(|_| bad("a sample after the @, like 12.5@1058400"))?))
            }
            None => (w[0], None),
        };
        let bar: f64 =
            bar.parse().ok().filter(|b: &f64| *b >= 0.0).ok_or_else(|| bad("starts with a bar, like 12.5"))?;
        let byte = |i: usize| w.get(i).and_then(|v| v.parse::<u8>().ok()).filter(|v| *v <= 127);
        let act = match w.get(1).copied() {
            Some("cc") => Act::Cc(
                byte(2).ok_or_else(|| bad("cc <0-127> <0-127>"))?,
                byte(3).ok_or_else(|| bad("cc <0-127> <0-127>"))?,
            ),
            Some("pad") => Act::Pad(
                byte(2).ok_or_else(|| bad("pad <note> <velocity>"))?,
                byte(3).ok_or_else(|| bad("pad <note> <velocity>"))?,
            ),
            Some("key") => Act::Key(
                byte(2).ok_or_else(|| bad("key <note> <velocity>"))?,
                byte(3).ok_or_else(|| bad("key <note> <velocity>"))?,
            ),
            Some("bend") => Act::Bend(
                w.get(2)
                    .and_then(|v| v.parse::<u16>().ok())
                    .filter(|v| *v <= 16383)
                    .ok_or_else(|| bad("bend <0-16383>"))?,
            ),
            Some("next") => Act::Next,
            Some("prev") => Act::Prev,
            Some("step") => Act::Step(
                w.get(2)
                    .and_then(|v| v.parse::<usize>().ok())
                    .filter(|v| *v >= 1)
                    .ok_or_else(|| bad("step <n>, from 1"))?,
            ),
            Some("end") => Act::End,
            Some("perform") => Act::Perform(w.get(2).ok_or_else(|| bad("perform <scene>"))?.to_string()),
            Some("reset") => Act::Reset,
            // `press pad 40 90` on a note, `press pad 90` the channel's.
            Some("press") => {
                let usage = "press pad|key [<note>] <0-127>";
                let pad = match w.get(2) {
                    Some(&"pad") => true,
                    Some(&"key") => false,
                    _ => return Err(bad(usage)),
                };
                if w.len() == 4 {
                    Act::Pressure(pad, None, byte(3).ok_or_else(|| bad(usage))?)
                } else {
                    Act::Pressure(pad, Some(byte(3).ok_or_else(|| bad(usage))?), byte(4).ok_or_else(|| bad(usage))?)
                }
            }
            Some("loop") => match w.get(2).copied() {
                Some("down") => Act::Loop(true),
                Some("up") => Act::Loop(false),
                _ => return Err(bad("loop down|up")),
            },
            Some("keep") => Act::Keep,
            Some("mute") => Act::Mute(w.get(2).ok_or_else(|| bad("mute <track>"))?.to_string()),
            Some("solo") if w.len() > 2 => Act::Solo(w[2..].iter().map(|t| t.to_string()).collect()),
            Some("solo") => return Err(bad("solo <track> [<track> ...]")),
            Some("voice") => Act::Voice(match w.get(2).copied() {
                Some("next") => tatum_core::dsl::ast::VoiceMove::Next,
                Some("prev") => tatum_core::dsl::ast::VoiceMove::Prev,
                Some(p) => tatum_core::dsl::ast::VoiceMove::To(p.to_string()),
                None => return Err(bad("voice next|prev|<page>")),
            }),
            _ => {
                return Err(bad(
                    "expected cc, pad, key, press, bend, voice, reset, next, prev, step, perform, loop, keep, mute, solo or end",
                ))
            }
        };
        out.push(Cue { bar, sample, act });
    }
    // Stable: two events on one bar happen in the order written. A take's
    // samples order it, and keep it in order where a loop of bars sends the
    // bar count back.
    if out.iter().all(|c| c.sample.is_some()) {
        out.sort_by_key(|c| c.sample);
    } else {
        out.sort_by(|a, b| a.bar.total_cmp(&b.bar));
    }
    Ok(Script { cues: out, take })
}

/// Whether a script moves through the set itself: a `next`, `prev` or
/// `step` line, or a pad that some step maps to one.
pub fn script_navigates(steps: &[Step], script: &[Cue]) -> bool {
    let nav_pads: Vec<u8> = steps
        .iter()
        .filter_map(|s| tatum_core::dsl::parse(&s.src).ok())
        .flat_map(|song| song.midi)
        .filter_map(|m| match m.source {
            tatum_core::dsl::ast::MidiSource::Pad(n)
                if m.target == "next" || m.target == "prev" || m.target.starts_with("step.") =>
            {
                Some(n)
            }
            _ => None,
        })
        .collect();
    script.iter().any(|c| c.act.navigates() || matches!(c.act, Act::Pad(n, v) if v > 0 && nav_pads.contains(&n)))
}

/// Walk the set the way `set play` walks it, with a performer who asks for
/// each step one bar before it is due: the same `SetNav`, so the same phrase
/// lines, the same tempo ramps and the same blends. An hour rendered here is
/// the hour played live, not an approximation of it.
///
/// A step that needs a new engine is built in the bar before its line and
/// swaps on it; one that only changes values is sent on the line itself.
///
/// With a `script`, its events are played through the same planner the live
/// path uses, so the steps' `midi` blocks map them and knobs, toggles and
/// held pads carry from step to step as they do in `set play`. A script that
/// moves through the set itself (`next`, `prev`, `step`) is the only thing
/// that does; one that does not leaves that to the headers.
///
/// `walk.live` plays a take back: the walk `set play` made, through the same
/// [`Stage`](crate::stage::Stage) the take was recorded on, each act on the
/// sample it was written with, ending on its `end`.
pub fn render_set(steps: Vec<Step>, walk: &Walk, script: &[Cue]) -> Result<Rendered, String> {
    let (lands, header_end) = landings(&steps, walk.phrase);
    let scripted = walk.live || script_navigates(&steps, script);
    let explicit_end = script.iter().find(|c| c.act == Act::End);
    let last_event = script.iter().map(|c| c.bar).fold(0.0f64, f64::max);
    let mut stage = crate::stage::Stage::new(steps, walk)?;
    let (mut l, mut r) = (Vec::new(), Vec::new());
    let mut applied = Vec::new();
    let mut log = Vec::new();
    let (mut bl, mut br) = ([0.0f32; BLOCK_SIZE], [0.0f32; BLOCK_SIZE]);
    let mut asked = 1;
    let mut next_event = 0;
    // Where the step playing landed, in bars, for a scripted walk's end.
    let mut landed_at = 0usize;
    loop {
        let e = stage.player.engine().ok_or("the set did not load")?;
        let bar = e.current_bar();
        let at = e.position_bars();
        // The bar line coming up, and how far into this block it falls.
        let upcoming = e.global_step().div_ceil(e.steps_per_bar().max(1));
        let until = e.samples_until_bar();
        let end = match explicit_end {
            Some(c) => c.bar.ceil() as usize,
            None if scripted => {
                let played_out = landed_at + stage.nav.steps[stage.nav.current].length();
                if next_event < script.len() || stage.nav.waiting(bar).is_some() {
                    usize::MAX
                } else {
                    played_out.max(last_event.ceil() as usize)
                }
            }
            None => header_end,
        };
        let mut len = if upcoming >= end { until.min(BLOCK_SIZE) } else { BLOCK_SIZE };
        match explicit_end.and_then(|c| c.sample) {
            // A take ends on the sample it was stopped on.
            Some(s) => {
                if l.len() >= s {
                    break;
                }
                len = (s - l.len()).min(BLOCK_SIZE);
            }
            None => {
                if explicit_end.is_some_and(|c| at >= c.bar) || (upcoming >= end && until == 0) {
                    break;
                }
            }
        }
        if !scripted && applied.len() < lands.len() && upcoming == lands[applied.len()] && until < len.max(1) {
            applied.push((stage.nav.steps[applied.len()].name(), l.len() + until));
        }
        if scripted && applied.is_empty() {
            applied.push((stage.nav.steps[0].name(), 0));
        }
        if !scripted && asked < lands.len() && bar + 1 >= lands[asked] {
            stage.nav.ask(crate::setnav::Move::Next, bar);
            asked += 1;
        }
        while next_event < script.len() {
            let cue = &script[next_event];
            let due = match cue.sample {
                Some(s) => s <= l.len(),
                None => cue.bar <= at,
            };
            if !due {
                break;
            }
            next_event += 1;
            let said = stage.act(&cue.act);
            if !said.is_empty() {
                log.push((l.len(), format!("bar {:.2}: {}", cue.bar, said)));
            }
        }
        let turn = stage.block(&mut bl[..len], &mut br[..len])?;
        // Every step compiled when the gain was measured; a failure here is
        // a file that changed under the render.
        if let Some(bad) = turn.tick.iter().find(|s| !s.starts_with("now ") && !s.starts_with("auto ")) {
            return Err(bad.clone());
        }
        for line in turn.log {
            let now = line.contains(": now ");
            if !now || scripted {
                log.push((l.len(), line));
            }
        }
        if let Some(bar) = turn.landed {
            landed_at = bar;
            if scripted {
                applied.push((stage.nav.steps[stage.nav.current].name(), l.len()));
            }
        }
        l.extend_from_slice(&bl[..len]);
        r.extend_from_slice(&br[..len]);
        while stage.player.take_retired().is_some() {}
    }
    Ok(Rendered { l, r, applied, steps: stage.nav.steps, log })
}

pub(crate) fn describe(e: &tatum_core::song_engine::DslError) -> String {
    use tatum_core::song_engine::DslError;
    match e {
        DslError::Parse(errs) => {
            errs.iter().map(|x| format!("line {}: {}", x.line, x.message)).collect::<Vec<_>>().join("; ")
        }
        DslError::Compile(errs) => errs.iter().map(|x| x.message.clone()).collect::<Vec<_>>().join("; "),
    }
}

// ── Measurement ──────────────────────────────────────────────────────────────

pub struct Probe {
    pub peak: f32,
    pub rms_db: f32,
    pub clipped: usize,
    pub silent_tracks: usize,
    pub tracks: usize,
    /// Tracks actually making sound. A set that keeps stacking voices without
    /// taking any away ends up with everything on at once, and then the kick
    /// is one of ten things instead of the thing the rest hangs off.
    pub voices: usize,
    /// Energy below 140 Hz against 300 Hz-3 kHz, in dB. How far the floor
    /// stands over everything else. Measured, not judged: what counts as
    /// right is the genre's business, and the number is here to be compared.
    pub floor_db: f32,
    /// What the mix hands the master chain. Over 1.0 the limiter is not
    /// catching peaks any more, it is deciding how loud the step is -- and
    /// once that happens every step comes out the same loudness however the
    /// faders move, which takes the set's dynamics away from the set.
    pub master_in: f32,
}

/// A step's engine as a set plays it with no hand on the controller: the
/// tracks a `hold` pad keeps out are out. Probes and the gate listen to this.
pub fn at_rest(src: &str) -> Result<SongEngine, String> {
    let mut ast = tatum_core::dsl::parse(src).map_err(|e| describe(&tatum_core::song_engine::DslError::Parse(e)))?;
    tatum_core::midi::at_rest(&mut ast);
    let compiled = tatum_core::dsl::compiler::compile(&ast)
        .map_err(|e| describe(&tatum_core::song_engine::DslError::Compile(e)))?;
    Ok(SongEngine::from_compiled(compiled))
}

/// What a step sounds like on its own. The gate uses it to reject a state that
/// is silent, clipping, or the same as the one before it -- the three ways an
/// agent's turn can be wasted without anything reporting an error.
pub fn probe(src: &str, bars: u32) -> Result<Probe, String> {
    let mut e = at_rest(src)?;
    e.start();
    let (l, r) = e.render(bars);
    let voices = (0..e.track_count()).filter(|i| e.track_level(*i) > 0.0 && e.track_rms(*i) > 1e-5).count();
    let peak = l.iter().chain(r.iter()).fold(0.0f32, |m, v| m.max(v.abs()));
    let n = (l.len() + r.len()).max(1) as f64;
    let sum: f64 = l.iter().chain(r.iter()).map(|v| (*v as f64) * (*v as f64)).sum();
    let clipped = l.iter().chain(r.iter()).filter(|v| v.abs() >= 0.999).count();
    Ok(Probe {
        peak,
        rms_db: 10.0 * (sum / n).max(1e-20).log10() as f32,
        clipped,
        silent_tracks: e.silent_track_count(),
        tracks: e.track_count(),
        voices,
        floor_db: band_ratio(&l),
        master_in: e.master_input_peak_rms().0,
    })
}

/// Energy below 140 Hz against 300 Hz-3 kHz, in dB. A naive DFT at a handful
/// of probe frequencies would be cheaper, but the whole band is what the ear
/// weighs, so this walks it: one pass, no allocation beyond the accumulators.
fn band_ratio(x: &[f32]) -> f32 {
    // Two one-pole filters is enough to separate a floor from a midrange.
    let (mut lo, mut hi_lp, mut hi_hp) = (0.0f32, 0.0f32, 0.0f32);
    let (a_lo, a_hi) = (0.019f32, 0.38f32); // ~140 Hz and ~3 kHz at 44.1k
    let (mut e_lo, mut e_hi) = (0.0f64, 0.0f64);
    for &v in x {
        lo += a_lo * (v - lo);
        hi_lp += a_hi * (v - hi_lp);
        hi_hp += 0.041 * (v - hi_hp); // ~300 Hz
        let mid = hi_lp - hi_hp;
        e_lo += (lo * lo) as f64;
        e_hi += (mid * mid) as f64;
    }
    10.0 * (e_lo.max(1e-20) / e_hi.max(1e-20)).log10() as f32
}

/// How different two renders are, in dB. -inf is identical. The gate uses it
/// to catch a step that compiles, differs in the text, and changes nothing you
/// can hear -- an agent editing a parameter that is not reachable, say.
fn difference_db(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    if n == 0 {
        return f32::NEG_INFINITY;
    }
    let d: f64 = (0..n)
        .map(|i| {
            let x = (a[i] - b[i]) as f64;
            x * x
        })
        .sum();
    let e: f64 = (0..n).map(|i| (a[i] as f64) * (a[i] as f64)).sum();
    if e <= 0.0 {
        return f32::NEG_INFINITY;
    }
    10.0 * (d / e).max(1e-20).log10() as f32
}

// ── Commands ─────────────────────────────────────────────────────────────────

pub fn cmd(args: &[String]) {
    let sub = args.first().map(|s| s.as_str()).unwrap_or("");
    let rest = if args.is_empty() { &[][..] } else { &args[1..] };
    match sub {
        "render" => run(cmd_render(rest)),
        "check" => run(cmd_check(rest)),
        "next" => run(cmd_next(rest)),
        "play" => run(cmd_play(rest)),
        "controls" => run(crate::controls::cmd(rest)),
        _ => {
            eprintln!("usage:");
            eprintln!(
                "    tatum set render <dir> [-o out.wav] [--bars N] [--phrase 1] [--ramp 4] [--blend 0] [--perform script.txt]"
            );
            eprintln!("    tatum set controls <dir> [--step N] [--bars 2]   every control of every step, tried");
            eprintln!("    tatum set check  <dir> [--bars N] [--json]");
            eprintln!("    tatum set next   <dir> <candidate.synth> [--json]");
            eprintln!(
                "    tatum set play   <dir> [--tui] [--auto] [--phrase 8] [--ramp 4] [--blend 0] [--device <name>] [--midi <name>] [--record <name>]"
            );
            eprintln!();
            eprintln!("A set is a directory of numbered .synth files, each the whole rig at a");
            eprintln!("moment. `render` walks them the way `play` does, blends and tempo ramps");
            eprintln!("included, each step for the bars its header gives. `check` validates the");
            eprintln!("whole set. `next` validates one proposed step against the last one, which");
            eprintln!("is the gate an agent writing a set has to pass. `play` plays it live: the");
            eprintln!("space bar or a pad moves to the next step on the next phrase line;");
            eprintln!("`--record take1` writes take1.wav and take1.txt, a script of everything");
            eprintln!("played that `render --perform take1.txt` renders again sample for sample.");
            std::process::exit(2);
        }
    }
}

fn run(r: Result<(), String>) {
    if let Err(e) = r {
        eprintln!("error: {}", e);
        std::process::exit(1);
    }
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(|s| s.as_str())
}

/// Play a set live. Steps move on a key or a pad, landing on the next
/// multiple of `--phrase` bars; a tempo change between steps ramps over
/// `--ramp` bars (0 jumps).
fn cmd_play(args: &[String]) -> Result<(), String> {
    let (mut dir, mut phrase, mut ramp, mut blend) = (None, None, 4.0f32, 0.0f32);
    let (mut device, mut rate, mut midi, mut tui, mut glass) = (None, None, None, false, false);
    let mut auto = false;
    let mut record = None;
    let mut i = 0;
    while i < args.len() {
        let value = |i: usize| args.get(i + 1).map(|s| s.as_str());
        match args[i].as_str() {
            "--record" => {
                record = Some(value(i).ok_or("--record needs a name for the take, like take1")?);
                i += 1;
            }
            "--phrase" => {
                phrase = Some(
                    value(i)
                        .and_then(|v| v.parse().ok())
                        .filter(|&n| n > 0)
                        .ok_or("--phrase needs a number of bars")?,
                );
                i += 1;
            }
            "--ramp" => {
                ramp = value(i)
                    .and_then(|v| v.parse().ok())
                    .filter(|&n: &f32| n >= 0.0)
                    .ok_or("--ramp needs a number of bars")?;
                i += 1;
            }
            "--blend" => {
                blend = value(i)
                    .and_then(|v| v.parse().ok())
                    .filter(|&n: &f32| n >= 0.0)
                    .ok_or("--blend needs a number of bars")?;
                i += 1;
            }
            "--device" | "-d" => {
                device = Some(value(i).ok_or("--device needs a name")?);
                i += 1;
            }
            "--rate" => {
                rate = Some(value(i).and_then(|v| v.parse().ok()).ok_or("--rate needs a number in Hz")?);
                i += 1;
            }
            "--midi" => {
                midi = Some(value(i).ok_or("--midi needs part of an input's name")?);
                i += 1;
            }
            "--tui" => tui = true,
            "--auto" => auto = true,
            "--buffer" => {
                let n = value(i)
                    .and_then(|v| v.parse::<u32>().ok())
                    .filter(|n| (64..=8192).contains(n))
                    .ok_or("--buffer needs a number of frames, 64 to 8192 (like 1024)")?;
                let _ = crate::live::BUFFER.set(n);
                i += 1;
            }
            "--glass" => {
                tui = true;
                glass = true;
            }
            other if other.starts_with('-') => return Err(format!("unknown flag '{}'", other)),
            other => dir = Some(other),
        }
        i += 1;
    }
    let dir = dir.ok_or("usage: tatum set play <dir> [--tui] [--auto] [--phrase 8] [--ramp 4] [--record <name>]")?;
    let steps = load(Path::new(dir), DEFAULT_BARS)?;
    let first = steps[0].path.to_string_lossy().into_owned();
    // Walking on its own, a set keeps its headers' bars, which need not be a
    // multiple of 8: a step lands on the bar its predecessor ends.
    let phrase = phrase.unwrap_or(if auto { 1 } else { 8 });
    if let Some(name) = record {
        let header = TakeHeader {
            phrase,
            ramp_bars: ramp,
            blend_bars: blend,
            auto,
            bars: DEFAULT_BARS,
            steps: Some(fingerprint(&steps)),
        };
        return crate::take::run(steps, &header.walk(), &header, Path::new(name), device, rate, midi, tui, glass);
    }
    let mut nav = crate::setnav::SetNav::new(steps, phrase, ramp);
    nav.blend_bars = blend;
    nav.auto = auto;
    crate::live::run(&first, true, device, rate, midi, Default::default(), Some(nav), tui, glass)
}

fn cmd_render(args: &[String]) -> Result<(), String> {
    let usage =
        "usage: tatum set render <dir> [-o out.wav] [--bars N] [--phrase 1] [--ramp 4] [--blend 0] [--perform script.txt]";
    let dir = args.first().filter(|a| !a.starts_with('-')).ok_or(usage)?;
    let out = flag(args, "-o").unwrap_or("set.wav").to_string();
    let script = match flag(args, "--perform") {
        Some(path) => parse_script(&fs::read_to_string(path).map_err(|e| format!("{}: {}", path, e))?)
            .map_err(|e| format!("{}: {}", path, e))?,
        None => Script::default(),
    };
    // A take plays back the way it was played; a flag written anyway wins,
    // and the render is then not the take.
    let take = script.take.clone();
    let default = take.as_ref().map_or(Walk::default(), TakeHeader::walk);
    let number = |name: &str, default: f32| -> Result<f32, String> {
        match flag(args, name) {
            None => Ok(default),
            Some(v) => v.parse::<f32>().ok().filter(|n| *n >= 0.0).ok_or(format!("{} needs a number of bars", name)),
        }
    };
    let bars =
        flag(args, "--bars").and_then(|v| v.parse().ok()).unwrap_or(take.as_ref().map_or(DEFAULT_BARS, |t| t.bars));
    let walk = Walk {
        phrase: number("--phrase", default.phrase as f32)?.max(1.0) as usize,
        ramp_bars: number("--ramp", default.ramp_bars)?,
        blend_bars: number("--blend", default.blend_bars)?,
        ..default
    };
    let steps = load(Path::new(dir), bars)?;
    if let Some(take) = &take {
        let changed: Vec<&str> =
            ["--phrase", "--ramp", "--blend", "--bars"].into_iter().filter(|f| flag(args, f).is_some()).collect();
        if !changed.is_empty() {
            eprintln!("note: {} changes how the take was played: the render will not be the take", changed.join(", "));
        }
        if take.steps.is_some_and(|f| f != fingerprint(&steps)) {
            eprintln!("note: the steps have changed since the take was played: the render will not be the take");
        }
    }
    let script = script.cues;

    let (lands, end) = landings(&steps, walk.phrase);
    // Walked by the headers alone: each step where `landings` puts it.
    let by_headers = !walk.live && !script_navigates(&steps, &script);
    if take.is_some() {
        eprintln!("{}: {} steps, a take", dir, steps.len());
    } else if !by_headers {
        eprintln!("{}: {} steps, moved through by the script", dir, steps.len());
    } else {
        eprintln!("{}: {} steps, {} bars", dir, steps.len(), end);
    }
    let rendered = render_set(steps, &walk, &script)?;
    let secs = rendered.l.len() as f32 / SAMPLE_RATE;
    let clock = |sample: usize| {
        let t = sample as f32 / SAMPLE_RATE;
        format!("{:>3}:{:02}", (t / 60.0) as u32, (t % 60.0) as u32)
    };
    // One line per step, where it starts in the file: `m:ss  name`, then
    // its bars, phase and note. A listening guide is written against these.
    for (k, (name, at)) in rendered.applied.iter().enumerate() {
        let Some(i) = rendered.steps.iter().position(|s| s.name() == *name) else { continue };
        let step = &rendered.steps[i];
        let moved = (by_headers && rendered.applied.len() == rendered.steps.len() && k == i)
            .then(|| lands.get(i + 1).map(|n| n - lands[i]))
            .flatten()
            .filter(|b| *b != step.length());
        eprintln!(
            "  {}  {:<14} {:>3} bars{}  {:<10} {}{}",
            clock(*at),
            step.name(),
            step.bars,
            step.auto.map(|n| format!(" x{} auto", n)).or(step.max.map(|n| format!(" max x{}", n))).unwrap_or_default(),
            step.phase,
            step.note,
            moved.map(|b| format!("  (plays {} to reach the phrase line)", b)).unwrap_or_default()
        );
    }
    if !rendered.log.is_empty() {
        eprintln!("performance:");
        for (at, what) in &rendered.log {
            eprintln!("  {}  {}", clock(*at), what);
        }
    }
    if rendered.l.len() > tatum_core::wav::MAX_STEREO_16_FRAMES {
        eprintln!("note: the WAV stops at the most a WAV file can hold, about 6 h 46 min");
    }
    let bytes = tatum_core::wav::encode_stereo_16(&rendered.l, &rendered.r, SAMPLE_RATE as u32);
    fs::write(&out, bytes).map_err(|e| format!("{}: {}", out, e))?;
    // Whole minutes: `{:.0}` rounded 2:37 up to "3:37".
    eprintln!("wrote {} ({}:{:02})", out, (secs / 60.0) as u32, (secs % 60.0) as u32);
    Ok(())
}

fn cmd_check(args: &[String]) -> Result<(), String> {
    let dir = args.first().ok_or("usage: tatum set check <dir>")?;
    let bars = flag(args, "--bars").and_then(|v| v.parse().ok()).unwrap_or(DEFAULT_BARS);
    let json = args.iter().any(|a| a == "--json");
    let steps = load(Path::new(dir), bars)?;

    let mut planner = LivePlanner::new();
    planner.restart_lanes();
    let mut player = LivePlayer::new();
    let mut rows = Vec::new();
    let mut bad = 0;

    for (i, step) in steps.iter().enumerate() {
        let kind = match planner.plan(&step.src, player.generation()) {
            Ok(p) => {
                let k = p.describe();
                player.apply(p);
                if i == 0 {
                    player.start();
                }
                k
            }
            Err(e) => {
                bad += 1;
                rows.push((step, String::from("error"), None, describe(&e)));
                continue;
            }
        };
        let pr = probe(&step.src, 2)?;
        let mut why = String::new();
        if i > 0 && kind == "unchanged" {
            bad += 1;
            why = String::from("changes nothing");
        }
        if pr.peak < 1e-4 {
            bad += 1;
            why = String::from("silent");
        }
        if pr.clipped > 0 {
            bad += 1;
            why = format!("{} clipped samples", pr.clipped);
        }
        rows.push((step, String::from(kind), Some(pr), why));
    }

    if json {
        println!("{{\"steps\":[");
        for (i, (s, kind, pr, why)) in rows.iter().enumerate() {
            let (peak, rms, silent) = pr.as_ref().map_or((0.0, 0.0, 0), |p| (p.peak, p.rms_db, p.silent_tracks));
            println!("  {{\"file\":\"{}\",\"bars\":{},\"phase\":\"{}\",\"energy\":{},\"transition\":\"{}\",\"peak\":{:.4},\"rms_db\":{:.2},\"muted_tracks\":{},\"problem\":\"{}\"}}{}",
                s.name(), s.bars, s.phase,
                s.energy.map(|e| format!("{:.1}", e)).unwrap_or_else(|| String::from("null")),
                kind, peak, rms, silent, why, if i + 1 == rows.len() { "" } else { "," });
        }
        println!("],\"problems\":{}}}", bad);
    } else {
        println!(
            "{:<14} {:>5} {:<10} {:>6} {:<10} {:>7} {:>8} {:>6}  problem",
            "step", "bars", "phase", "energy", "transition", "peak", "rms dB", "muted"
        );
        for (s, kind, pr, why) in &rows {
            let (peak, rms, silent) = pr.as_ref().map_or((0.0, 0.0, 0), |p| (p.peak, p.rms_db, p.silent_tracks));
            let en = s.energy.map(|e| format!("{:.0}", e)).unwrap_or_else(|| String::from("-"));
            println!(
                "{:<14} {:>5} {:<10} {:>6} {:<10} {:>7.3} {:>8.1} {:>6}  {}",
                s.name(),
                s.bars,
                s.phase,
                en,
                kind,
                peak,
                rms,
                silent,
                why
            );
        }
    }
    if bad > 0 {
        return Err(format!("{} problems", bad));
    }
    Ok(())
}

/// True when a step is a build: a drum part whose name says it is a fill or a
/// roll. A build is a promise, and the only thing that can follow it is the
/// thing it promised. Two builds in a row means neither one explodes -- the
/// second steals the first one's arrival and then has nothing to arrive into.
fn is_a_build(src: &str) -> bool {
    src.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| l.split("play ").nth(1))
        .filter_map(|r| r.split_whitespace().next())
        .any(|p| p.contains("fill") || p.contains("roll_up") || p.contains("riser"))
}

/// The gate. An agent proposes the next state; this says yes or no and why.
fn cmd_next(args: &[String]) -> Result<(), String> {
    let dir = args.first().ok_or("usage: tatum set next <dir> <candidate.synth>")?;
    let cand_path = args.get(1).ok_or("usage: tatum set next <dir> <candidate.synth>")?;
    let json = args.iter().any(|a| a == "--json");
    let max_voices: Option<usize> = flag(args, "--max-voices").and_then(|v| v.parse().ok());
    let steps = load(Path::new(dir), DEFAULT_BARS)?;
    let prev = steps.last().ok_or("the set is empty")?;
    let cand = crate::include::Source::load(Path::new(cand_path))?.text;

    let mut problems: Vec<String> = Vec::new();

    // 1. Does it compile at all?
    let pr = match probe(&cand, 2) {
        Ok(p) => p,
        Err(e) => {
            emit(json, "rejected", "", &[format!("does not compile: {}", e)], None, f32::NEG_INFINITY);
            return Err(String::from("rejected"));
        }
    };

    // 2. How would the engine get there from the state before it?
    let mut planner = LivePlanner::new();
    planner.restart_lanes();
    let mut player = LivePlayer::new();
    let p0 = planner.plan(&prev.src, player.generation()).map_err(|e| describe(&e))?;
    player.apply(p0);
    player.start();
    let transition = match planner.plan(&cand, player.generation()) {
        Ok(p) => p.describe(),
        Err(e) => {
            emit(json, "rejected", "", &[format!("does not compile: {}", describe(&e))], None, f32::NEG_INFINITY);
            return Err(String::from("rejected"));
        }
    };
    if transition == "unchanged" {
        problems.push(String::from("changes nothing the engine can hear"));
    }

    // 3. Is it playable?
    if pr.peak < 1e-4 {
        problems.push(String::from("silent"));
    }
    if pr.clipped > 0 {
        problems.push(format!("{} clipped samples", pr.clipped));
    }
    if pr.silent_tracks == pr.tracks {
        problems.push(String::from("every track is muted"));
    }
    // Headroom. Once the mix arrives over full scale the limiter decides how
    // loud the step is, not the faders, and every step lands at the same
    // loudness however the set is arranged. Measured on one set: at 1.36 in,
    // the master crushed the crest from 4.7 to 3.4 and the whole second half
    // sat within 0.2 dB. Scaling the same steps to 0.6 recovered the crest to
    // 4.6 and reopened the gap between the quiet step and the loud one.
    if pr.master_in > 1.0 {
        problems.push(format!(
            "the mix hands the master {:.2}, over full scale: the limiter is arranging the set, not you. Pull levels or saturation back",
            pr.master_in
        ));
    }
    if let Some(max) = max_voices {
        if pr.voices > max {
            problems.push(format!(
                "{} voices sounding at once, more than {}: bring one in by taking one out",
                pr.voices, max
            ));
        }
    }

    // 4. Is the difference audible? A step that compiles, differs in the text
    //    and sounds the same has spent a turn for nothing.
    let a = at_rest(&prev.src)?;
    let b = at_rest(&cand)?;
    let (mut a, mut b) = (a, b);
    a.start();
    b.start();
    let (al, _) = a.render(2);
    let (bl, _) = b.render(2);
    let diff = difference_db(&al, &bl);
    if diff < -40.0 {
        problems.push(format!("inaudible: {:.0} dB from the step before it", diff));
    }

    // 5. A build has to explode. If the step before this one was a fill or a
    //    roll, it promised an arrival, and this step is the arrival.
    if is_a_build(&prev.src) {
        let before = probe(&prev.src, 2)?;
        if is_a_build(&cand) {
            problems.push(String::from(
                "the step before this one was already a build: two in a row means neither explodes",
            ));
        } else if pr.rms_db < before.rms_db + 1.5 {
            problems.push(format!(
                "the step before was a build and this does not land: {:.1} dB against {:.1} dB, needs at least 1.5 dB more",
                pr.rms_db, before.rms_db
            ));
        }
    }

    let verdict = if problems.is_empty() { "accepted" } else { "rejected" };
    emit(json, verdict, transition, &problems, Some(&pr), diff);
    if problems.is_empty() {
        Ok(())
    } else {
        Err(String::from("rejected"))
    }
}

fn emit(json: bool, verdict: &str, transition: &str, problems: &[String], pr: Option<&Probe>, diff: f32) {
    let (peak, rms, muted, voices, floor, min) = pr
        .map_or((0.0, 0.0, 0, 0, 0.0, 0.0), |p| (p.peak, p.rms_db, p.silent_tracks, p.voices, p.floor_db, p.master_in));
    if json {
        println!("{{\"verdict\":\"{}\",\"transition\":\"{}\",\"peak\":{:.4},\"rms_db\":{:.2},\"voices\":{},\"floor_db\":{:.1},\"master_in\":{:.2},\"muted_tracks\":{},\"difference_db\":{:.1},\"problems\":[{}]}}",
            verdict, transition, peak, rms, voices, floor, min, muted, diff,
            problems.iter().map(|p| format!("\"{}\"", p)).collect::<Vec<_>>().join(","));
    } else {
        println!("{}: {} transition, rms {:.1} dB, {} voices, floor {:+.1} dB over the mids, master in {:.2}, {:.0} dB from the step before",
            verdict, transition, rms, voices, floor, min, diff);
        for p in problems {
            println!("  - {}", p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_header_names_its_scene_and_a_script_calls_one() {
        assert_eq!(parse_perform("# set: bars=16 perform=drop phase=x\n"), Some(String::from("drop")));
        assert_eq!(parse_perform("# set: bars=16\n"), None);
        let script = parse_script("4.0 perform break\n").unwrap().cues;
        assert_eq!(script[0], Cue::at(4.0, Act::Perform(String::from("break"))));
        assert!(parse_script("4.0 perform\n").is_err());
    }

    const RIG: &str = r#"
tempo 120
scale C minor
module bass low { cutoff 0.5 sustain 0.8 }
pattern line { C2:0.9 - - -  C2:0.9 - - -  C2:0.9 - - -  C2:0.9 - - - }
track bass { play line using low level 0.5 out > lowpass(600, 0.1, wet=0.0) as lp > master }
master { in > out }
"#;

    #[test]
    fn the_header_travels_in_the_file() {
        let src = format!("# set: bars=48 phase=build energy=5\n# set-note: reese in\n{RIG}");
        let (bars, phase, energy, note, _) = parse_header(&src, 32);
        assert_eq!(bars, 48);
        assert_eq!(phase, "build");
        assert_eq!(energy, Some(5.0));
        assert_eq!(note, "reese in");
    }

    /// The header is a note to the tooling, not part of the language. A set
    /// someone wrote by hand should not fail to play because of a stray word.
    #[test]
    fn a_missing_or_odd_header_falls_back_instead_of_failing() {
        let (bars, phase, energy, note, _) = parse_header(RIG, 32);
        assert_eq!((bars, energy), (32, None));
        assert!(phase.is_empty() && note.is_empty());

        let (bars, ..) = parse_header("# set: bars=nope colour=blue\n", 32);
        assert_eq!(bars, 32, "an unreadable value keeps the default");
    }

    /// The whole point of a set: bringing a voice in or switching an effect on
    /// hands over without rebuilding the engine, so nothing restarts.
    #[test]
    fn level_and_wet_steps_stay_on_the_fast_path() {
        let steps = [RIG.to_string(), RIG.replace("wet=0.0", "wet=1.0"), RIG.replace("level 0.5", "level 0.2")];
        let mut planner = LivePlanner::new();
        let mut player = LivePlayer::new();
        player.apply(planner.plan(&steps[0], player.generation()).expect("first"));
        player.start();
        for s in &steps[1..] {
            let plan = planner.plan(s, player.generation()).expect("plans");
            assert_eq!(plan.describe(), "fast", "a level or wet step must not need a swap");
            player.apply(plan);
        }
    }

    /// A set in a fresh directory, one file per step, loaded the way `set
    /// render` loads it.
    fn set_of(files: &[String]) -> (PathBuf, Vec<Step>) {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("tatum-set-{}-{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for (i, src) in files.iter().enumerate() {
            fs::write(dir.join(format!("{:02}.synth", i + 1)), src).unwrap();
        }
        let steps = load(&dir, 32).unwrap();
        (dir, steps)
    }

    fn render(files: &[String], walk: &Walk) -> Rendered {
        let (dir, steps) = set_of(files);
        let out = render_set(steps, walk, &[]).expect("renders");
        let _ = fs::remove_dir_all(&dir);
        out
    }

    const BAR: usize = (SAMPLE_RATE * 2.0) as usize; // 120 BPM, 4/4

    fn rms_db(x: &[f32]) -> f32 {
        let e: f64 = x.iter().map(|v| (*v as f64) * (*v as f64)).sum::<f64>() / x.len().max(1) as f64;
        10.0 * e.max(1e-20).log10() as f32
    }

    /// Energy of the first difference, in dB: a crude measure of the top.
    fn top_db(x: &[f32]) -> f32 {
        let e: f64 = x.windows(2).map(|w| ((w[1] - w[0]) as f64).powi(2)).sum::<f64>() / x.len().max(1) as f64;
        10.0 * e.max(1e-20).log10() as f32
    }

    fn bar(x: &[f32], b: usize) -> &[f32] {
        &x[b * BAR..((b + 1) * BAR).min(x.len())]
    }

    /// And the walk produces one continuous render whose length is the bars
    /// the steps asked for, not the bars of whichever state happened to load,
    /// with each step starting where its predecessor's bars run out.
    #[test]
    fn the_walk_lasts_as_long_as_the_steps_say() {
        let head = "# set: bars=2\n";
        let out =
            render(&[format!("{head}{RIG}"), format!("{head}{}", RIG.replace("wet=0.0", "wet=1.0"))], &Walk::default());
        // Two steps of two bars at 120 BPM in 4/4 is eight seconds.
        let secs = out.l.len() as f32 / SAMPLE_RATE;
        assert!((secs - 8.0).abs() < 0.01, "expected 8 s, got {secs:.3}");
        assert_eq!(out.l.len(), out.r.len());
        assert!(out.l.iter().any(|v| v.abs() > 0.01), "the walk rendered silence");
        let starts: Vec<usize> = out.applied.iter().map(|a| a.1).collect();
        assert_eq!(starts.len(), 2);
        assert_eq!(starts[0], 0);
        assert!(starts[1].abs_diff(2 * BAR) < 2, "step two starts on sample {} of {}", starts[1], 2 * BAR);
    }

    /// `--phrase 4` moves a step whose predecessor ends off the line onto
    /// the next one, as `set play` would.
    #[test]
    fn a_phrase_moves_a_step_to_its_line() {
        let steps =
            [format!("# set: bars=3\n{RIG}"), format!("# set: bars=2\n{}", RIG.replace("level 0.5", "level 0.4"))];
        let (dir, loaded) = set_of(&steps);
        assert_eq!(landings(&loaded, 4), (vec![0, 4], 6));
        assert_eq!(landings(&loaded, 1), (vec![0, 3], 5));
        let _ = fs::remove_dir_all(&dir);
        let out = render(&steps, &Walk { phrase: 4, ..Walk::default() });
        assert!(out.applied[1].1.abs_diff(4 * BAR) < 2, "{:?}", out.applied);
        assert!((out.l.len() as f32 / SAMPLE_RATE - 12.0).abs() < 0.01);
    }

    /// A tempo change between steps ramps over `ramp_bars`, the way `set
    /// play` does it, rather than jumping on the line.
    #[test]
    fn a_tempo_change_ramps() {
        let steps =
            [format!("# set: bars=2\n{RIG}"), format!("# set: bars=4\n{}", RIG.replace("tempo 120", "tempo 150"))];
        let jump = render(&steps, &Walk { ramp_bars: 0.0, ..Walk::default() });
        let ramp = render(&steps, &Walk::default());
        // Four bars at 150 are 6.4 s; ramped from 120, longer.
        let (j, r) = (jump.l.len() as f32 / SAMPLE_RATE, ramp.l.len() as f32 / SAMPLE_RATE);
        assert!((j - 10.4).abs() < 0.05, "jumped: {j:.2} s");
        assert!(r > j + 0.3, "the ramp took no time: {r:.2} s against {j:.2}");
    }

    /// `# set: blend=2`: the step before keeps playing under the new one.
    #[test]
    fn a_blend_in_the_header_is_played() {
        let quiet = RIG.replace("level 0.5", "level 0");
        let second = |head: &str| {
            format!("# set: bars=4{head}\n{quiet}pattern hi {{ C5:0.9 - - - }}\ntrack hi {{ play hi using low level 0.3 out > master }}\n")
        };
        let cut = render(&[format!("# set: bars=2\n{RIG}"), second("")], &Walk::default());
        let blend = render(&[format!("# set: bars=2\n{RIG}"), second(" blend=2")], &Walk::default());
        // The bar after the line: the bass is gone from one and fading in the other.
        let (c, b) = (rms_db(bar(&cut.l, 2)), rms_db(bar(&blend.l, 2)));
        assert!(b > c + 3.0, "no blend heard: {b:.1} dB against {c:.1} cut");
    }

    /// A step's `auto ... over N` starts on the step's first bar, not on bar 0
    /// of the set, by which time it would be over.
    #[test]
    fn a_steps_lane_starts_on_its_first_bar() {
        let steps = [format!("# set: bars=2\n{RIG}"), format!("# set: bars=4\n{RIG}auto bass level 0 > 1 over 2\n")];
        let out = render(&steps, &Walk::default());
        let db: Vec<f32> = (0..6).map(|b| rms_db(bar(&out.l, b))).collect();
        assert!(db[2] < db[3] - 3.0, "the lane did not start with the step: {db:?}");
        assert!(db[3] < db[4] - 1.0, "{db:?}");
        assert!((db[4] - db[5]).abs() < 0.5, "and holds once there: {db:?}");
    }

    /// A node's wet swept in by one step's lane is back where the next step's
    /// text puts it when that step does not automate it.
    #[test]
    fn what_a_step_automated_returns_to_the_next_steps_text() {
        // A bright bass behind a lowpass that is switched out in the text.
        let rig =
            RIG.replace("cutoff 0.5", "cutoff 0.9").replace("lowpass(600, 0.1, wet=0.0)", "lowpass(150, 0.7, wet=0.0)");
        // Open for a bar, then closing, then closed.
        let a = format!("# set: bars=3\n{rig}auto bass.lp wet 0 > 0 > 1 over 2\n");
        let b = format!("# set: bars=3\n{rig}");
        let out = render(&[a, b], &Walk::default());
        let (open, dark, after) = (top_db(bar(&out.l, 0)), top_db(bar(&out.l, 2)), top_db(bar(&out.l, 4)));
        assert!(open - dark > 6.0, "the lane did not close the filter: {open:.1} then {dark:.1} dB");
        assert!((after - open).abs() < 1.0, "step two plays {after:.1} dB of top, its text {open:.1}");
    }

    #[test]
    fn a_script_reads_one_event_a_line() {
        let script = parse_script(
            "# a comment\n12.0 cc 74 64\n18 pad 44 110   # down\n19.0 pad 44 0\n\n20.5 key 48 100\n21 bend 12000\n35 next\n36 step 3\n37 prev\n64 end\n",
        )
        .expect("parses");
        assert_eq!(script.take, None);
        let script = script.cues;
        assert_eq!(script.len(), 9);
        assert_eq!(script[0], Cue::at(12.0, Act::Cc(74, 64)));
        assert_eq!(script[1], Cue::at(18.0, Act::Pad(44, 110)));
        assert_eq!(script[3], Cue::at(20.5, Act::Key(48, 100)));
        assert_eq!(script[5], Cue::at(35.0, Act::Next));
        assert_eq!(script[6], Cue::at(36.0, Act::Step(3)));
        assert_eq!(script[8], Cue::at(64.0, Act::End));
        for bad in ["x cc 1 2", "1 cc 200 1", "1 pad 3", "1 jump", "1 step 0"] {
            assert!(parse_script(bad).is_err(), "{bad}");
        }
    }

    /// A scripted performance goes through the steps' own `midi` blocks: a
    /// `hold` pad lets the bass in for the bar it is held, a knob turned in
    /// the first step is still where it was left in the second, and `next`
    /// moves the set on the next line.
    #[test]
    fn a_script_plays_the_set_through_its_midi_blocks() {
        let rig = format!("{RIG}midi {{\n  pad 41 > hold bass\n  cc 30 > bass level\n}}\n");
        let steps = [
            format!("# set: bars=2\n{rig}"),
            format!("# set: bars=2\n{}", rig.replace("C2:0.9 - - -  C2", "C2:0.9 - C2 -  C2")),
        ];
        let script =
            parse_script("1.0 pad 41 100\n2.0 pad 41 0\n2.5 cc 30 20\n3.0 next\n3.0 pad 41 100\n").unwrap().cues;
        let (dir, loaded) = set_of(&steps);
        let played = render_set(loaded, &Walk::default(), &script).expect("renders");
        let _ = fs::remove_dir_all(&dir);
        // `next` at bar 3 lands on bar 4; the second step then plays its 2 bars.
        assert_eq!(played.applied.len(), 2);
        assert!(played.applied[1].1.abs_diff(4 * BAR) <= BLOCK_SIZE, "{:?}", played.applied);
        // And the log says so in the script's own count, from 0: asked on 3,
        // in on 4 -- where it landed, not a bar later.
        let said: Vec<&str> = played.log.iter().map(|(_, s)| s.as_str()).collect();
        assert!(said.contains(&"bar 3.00: asked for 2/2 02.synth, lands on bar 4"), "{said:?}");
        assert!(said.contains(&"bar 4: now 2/2 02.synth"), "{said:?}");
        let now = played.log.iter().find(|(_, s)| s.starts_with("bar 4: now")).unwrap().0;
        assert_eq!(now, played.applied[1].1, "the log and the marker agree");
        assert!((played.l.len() as f32 / SAMPLE_RATE - 12.0).abs() < 0.05, "{} s", played.l.len() as f32 / SAMPLE_RATE);
        let db: Vec<f32> = (0..6).map(|b| rms_db(bar(&played.l, b))).collect();
        assert!(db[0] < -80.0, "the bass played before its pad: {db:?}");
        assert!(db[1] > -40.0, "the pad did not let the bass in: {db:?}");
        assert!(db[2] < db[1] - 20.0, "the bass stayed in after the pad: {db:?}");
        // Held again from bar 3, through the swap: in, at the knob's level,
        // well under the text's, which bar 1 played at.
        assert!(db[4] > -40.0 && db[4] < db[1] - 6.0, "step two at the text's level, not the knob's: {db:?}");
        assert!(played.log.iter().any(|(_, s)| s.contains("bass level")), "{:?}", played.log);
    }

    /// The set's gain and the gate's probe hear a step as it plays: a track a
    /// `hold` pad keeps out is not in them, whatever its level.
    #[test]
    fn a_held_out_track_does_not_move_the_gain_or_the_probe() {
        let with_pad = |level: &str| {
            format!(
                "# set: bars=2\n{}\npattern hit {{ C3:0.9 - - - }}\ntrack zap {{ play hit using low level {level} out > master }}\nmidi {{\n  pad 41 > hold zap\n}}\n",
                RIG
            )
        };
        let (dir, quiet) = set_of(&[with_pad("0.2")]);
        let (dir2, loud) = set_of(&[with_pad("1.5")]);
        assert!((set_gain(&quiet).unwrap() - set_gain(&loud).unwrap()).abs() < 1e-4);
        let (a, b) = (probe(&quiet[0].src, 2).unwrap(), probe(&loud[0].src, 2).unwrap());
        assert!((a.rms_db - b.rms_db).abs() < 1e-3, "{} vs {}", a.rms_db, b.rms_db);
        let _ = (fs::remove_dir_all(&dir), fs::remove_dir_all(&dir2));
    }
}
