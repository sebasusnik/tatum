//! MIDI controls: what a `midi { }` block maps, resolved against one engine.
//!
//! Three kinds. A knob (`cc 74 > acid cutoff`) turns a 7-bit value into the
//! same [`FastOp`]s a text edit produces, and says what it now reads in the
//! target's own units; the registry is what makes that short, since every
//! parameter already says whether it spans 0..1, left to right, a gain or a
//! list of choices, and what real quantity it stands for. The keys
//! (`keys > solo`) play a track's instrument. A pad (`pad 36 > kick kick`)
//! hits one drum of a track.
//!
//! No I/O here: whatever reads the controller hands over numbers.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::dsl::ast::{BendRange, Hands, MidiSource, Param, RangeEnd, Song, ZoneKind, ZonePlay};
use crate::perform::scale::{Lock, Scale};
use crate::live::FastOp;
use crate::params::{self, ModuleKind, ParamSpec, Range};

/// One thing a knob moves, resolved to engine indices.
#[derive(Debug, Clone, Copy)]
pub enum Move {
    Module {
        instrument: usize,
        spec: &'static ParamSpec,
    },
    TrackLevel {
        track: usize,
    },
    TrackPan {
        track: usize,
    },
    NodeWet {
        track: usize,
        node: usize,
    },
    ReverbMix,
    DelayMix,
    ReverbFreeze,
    /// `stab delay_send`, `stab reverb_send`.
    TrackSend {
        track: usize,
        reverb: bool,
    },
    /// `master cutoff`, `master dj cutoff`, `bass lp cutoff`: a parameter of
    /// one node of an effect chain; `track` is `None` for the master.
    NodeParam {
        track: Option<usize>,
        node: usize,
        param: &'static str,
    },
    Tempo,
}

/// The parameters a knob can reach on a chain node: the ones `auto master`
/// sweeps. Each with the stretch a knob with no range covers, in the value
/// the node reads, and whether it sweeps geometrically (a cutoff: each turn
/// of the knob is the same interval to the ear, not the same number of Hz).
const NODE_PARAMS: &[(&str, f32, f32, bool)] = &[
    ("cutoff", 20.0, 20000.0, true),
    ("tilt", -1.0, 1.0, false),
    ("eq_low", -12.0, 12.0, false),
    ("eq_mid", -12.0, 12.0, false),
    ("eq_high", -12.0, 12.0, false),
    ("drive", 0.0, 2.0, false),
    ("gain", 0.0, 1.0, false),
    ("comp_threshold", -40.0, 0.0, false),
    // A vowel filter's place along a-e-i-o-u.
    ("position", 0.0, 1.0, false),
];

/// The widest a range on a node parameter may be written, which is what the
/// node itself accepts.
fn node_param_limits(param: &str) -> (f32, f32) {
    match param {
        "cutoff" => (20.0, 20000.0),
        "tilt" => (-1.0, 1.0),
        "eq_low" | "eq_mid" | "eq_high" => (-12.0, 12.0),
        "drive" | "gain" => (0.0, 10.0),
        "position" => (0.0, 1.0),
        _ => (-60.0, 0.0),
    }
}

fn node_param(name: &str) -> Option<&'static (&'static str, f32, f32, bool)> {
    NODE_PARAMS.iter().find(|p| p.0 == name)
}

/// The kinds of chain node that answer to a parameter, as the text names
/// them. The same list `auto master` checks against.
pub fn node_kinds_for(param: &str) -> &'static [&'static str] {
    crate::dsl::compiler::master_auto_node_kinds(param)
}

/// Where a knob's travel lands: two points, or three with the middle one at
/// half travel, as the engine reads the target. Three is what puts two
/// filters on one knob, DJ style: the lowpass closes over the left half and
/// the highpass opens over the right.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub points: [f32; 3],
    pub len: usize,
    /// Sweep geometrically between points, for a cutoff.
    pub log: bool,
}

impl Span {
    fn two(lo: f32, hi: f32, log: bool) -> Self {
        Self { points: [lo, hi, hi], len: 2, log }
    }

    /// The value at `x` along the travel, 0..1.
    pub fn at(&self, x: f32) -> f32 {
        let p = &self.points;
        let (a, b, t) = if self.len == 3 {
            if x < 0.5 {
                (p[0], p[1], x * 2.0)
            } else {
                (p[1], p[2], x * 2.0 - 1.0)
            }
        } else {
            (p[0], p[1], x)
        };
        if self.log && a > 0.0 && b > 0.0 {
            a * crate::math::pow(b / a, t)
        } else {
            a + (b - a) * t
        }
    }
}

/// The names one engine answers to, which is all resolution needs. The
/// engine gives every track that names a module its own copy of it, so
/// `instruments` can carry the same name more than once.
pub struct Names<'a> {
    pub instruments: &'a [String],
    pub tracks: &'a [String],
    /// Which instrument each track plays, `usize::MAX` for none.
    pub track_instruments: &'a [usize],
    /// The `as` names of each track's insert nodes, in chain order.
    pub track_nodes: &'a [Vec<Option<String>>],
    /// The note each track's patterns sound most often, if any.
    pub track_home: &'a [Option<u8>],
}

/// A controller and everything it moves in one engine.
#[derive(Debug, Clone)]
pub struct Knob {
    pub cc: u8,
    /// The target as the file writes it, dotted: `acid.cutoff`.
    pub target: String,
    pub moves: Vec<Move>,
    /// Where the travel lands, as the engine reads the target; from
    /// `cc 74 > acid cutoff 200hz..4khz`. `None`: all of it.
    pub span: Option<Span>,
    /// `guard`: a first touch waits for the knob to reach the target.
    pub guard: bool,
}

impl Knob {
    /// The operations that put this knob at `value`.
    pub fn ops(&self, value: u8) -> impl Iterator<Item = FastOp> + '_ {
        self.moves.iter().map(move |m| op(m, scaled(m, value, self.span)))
    }

    /// Where on its travel (0..127) this knob puts `target` at `value`, the
    /// value as the engine reads it: what a knob has to reach to take over
    /// a target it does not move yet. `None` when it cannot reach it.
    pub fn position_of(&self, value: f32) -> Option<u8> {
        let m = self.moves.first()?;
        let inverse = |a: f32, b: f32, log: bool| -> Option<f32> {
            if log && a > 0.0 && b > 0.0 && value > 0.0 {
                let x = crate::math::ln(value / a) / crate::math::ln(b / a);
                x.is_finite().then_some(x)
            } else if b != a {
                Some((value - a) / (b - a))
            } else {
                None
            }
        };
        let x = match self.span {
            Some(span) if span.len == 3 => {
                let p = span.points;
                inverse(p[0], p[1], span.log)
                    .map(|x| x * 0.5)
                    .filter(|x| (0.0..=0.5).contains(x))
                    .or_else(|| inverse(p[1], p[2], span.log).map(|x| 0.5 + x * 0.5))
            }
            Some(span) => inverse(span.points[0], span.points[1], span.log),
            None => match m {
                Move::Module { spec, .. } => match spec.range {
                    Range::Unit => Some(value),
                    Range::Bipolar => Some((value + 1.0) / 2.0),
                    Range::Gain { max } => Some(value / max),
                    Range::Choice(_) => None,
                },
                Move::TrackLevel { .. }
                | Move::NodeWet { .. }
                | Move::ReverbMix
                | Move::DelayMix
                | Move::TrackSend { .. } => Some(value),
                Move::TrackPan { .. } => Some((value + 1.0) / 2.0),
                Move::NodeParam { param, .. } => {
                    let &(_, lo, hi, log) = node_param(param)?;
                    inverse(lo, hi, log)
                }
                _ => None,
            },
        }?;
        (-0.01..=1.01).contains(&x).then(|| crate::math::floor(x.clamp(0.0, 1.0) * 127.0 + 0.5) as u8)
    }

    /// What puts every target of this knob back where the text has it: the
    /// value written, else the default the compiler gives it. A target with
    /// neither (a tempo) is left where it is.
    pub fn rest_ops(&self, song: &Song) -> Vec<FastOp> {
        let written = text_value(song, &self.target);
        self.moves
            .iter()
            .filter_map(|m| Some(op(m, rest_value(m, written)?)))
            .collect()
    }

    /// Where the knob has to be to sit on the value the text gives its
    /// target: what a knob that has not moved it yet takes over from.
    pub fn rest_position(&self, song: &Song) -> Option<u8> {
        let written = text_value(song, &self.target);
        self.position_of(rest_value(self.moves.first()?, written)?)
    }

    /// What the knob reads at `value`: `acid cutoff 1.2khz`, `pad level -6.0 dB`.
    pub fn reading(&self, value: u8) -> String {
        let label = self.target.replace('.', " ");
        match self.moves.first() {
            Some(m) => format!("{} {}", label, reading(m, scaled(m, value, self.span))),
            None => label,
        }
    }
}

/// The value the text gives a knob's target: the one written, else the
/// default the compiler gives it. A target with neither (a tempo) has none.
fn rest_value(m: &Move, written: Option<f32>) -> Option<f32> {
    Some(match m {
        Move::Module { spec, .. } => written.unwrap_or(spec.default),
        Move::TrackLevel { .. } => written.unwrap_or(0.8),
        Move::TrackPan { .. } | Move::TrackSend { .. } => written.unwrap_or(0.0),
        Move::NodeWet { .. } => written.unwrap_or(1.0),
        Move::ReverbMix | Move::DelayMix => written.unwrap_or(1.0),
        Move::ReverbFreeze => 0.0,
        Move::NodeParam { .. } => written?,
        Move::Tempo => return None,
    })
}

/// A track the keyboard plays.
#[derive(Debug, Clone, Copy)]
pub struct Keys {
    pub track: usize,
    pub instrument: usize,
    /// One note at a time. The `bass` module is: it glides from note to note,
    /// and it releases on any note-off, so the keyboard has to decide which
    /// key it is actually playing.
    pub mono: bool,
}

/// One pad: the drum channel note it answers to, and what it does.
#[derive(Debug, Clone, Copy)]
pub struct Pad {
    pub note: u8,
    pub action: PadAction,
    /// `q=bar`: press and release wait for the line. See `LivePlayer`.
    pub quantize: Option<crate::dsl::ast::Quantize>,
}

/// What a pad does. A drum is struck on the way down; `mute`, `throw` and
/// `freeze` last while the pad is held; `toggle` flips on each hit; the
/// last three move through a set and do nothing outside `tatum set play`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PadAction {
    /// `pad 36 > kick kick`. `drum` is the note the `beats` module plays it on.
    Drum {
        track: usize,
        drum: u8,
    },
    /// `pad 44 > mute kick`: silent while held.
    Mute {
        track: usize,
    },
    /// `pad 45 > toggle hats`: out on one hit, back on the next.
    Toggle {
        track: usize,
    },
    /// `pad 46 > throw stab`: the whole track into the delay while held.
    Throw {
        track: usize,
    },
    /// `pad 47 > freeze`: the reverb frozen while held.
    Freeze,
    /// `pad 40 > play grinder C2`: the track's instrument plays `note` while
    /// the pad is held, through the track's chain, fader and sends.
    Play {
        track: usize,
        note: u8,
    },
    /// `pad 41 > hold riser`: the inverse of `mute`, the track silent until
    /// the pad is held and sounding its pattern while it is.
    Hold {
        track: usize,
    },
    /// `pad 38 > repeat 1/16`: a beat repeat on the whole output while held,
    /// one division long, in sixteenths (4, 2, 1 or 0.5).
    Repeat {
        sixteenths: f32,
    },
    /// `pad 40 > tapestop`, `gate 1/16`, `crush`, `scream`, `cut`,
    /// `sweep`: an effect on the whole output while held, as hard as the pad
    /// was struck and then as hard as it is pressed.
    Fx(crate::perform::fx::OutFx),
    /// `pad 48 > next`, `pad 49 > prev`, `pad 50 > step 3`.
    Next,
    Prev,
    Step(usize),
}

impl PadAction {
    /// A set move rather than something the engine does.
    pub fn navigation(&self) -> bool {
        matches!(self, PadAction::Next | PadAction::Prev | PadAction::Step(_))
    }
}

/// What a pad line names, before any engine: the words after `>`.
pub fn pad_words(target: &str) -> Vec<&str> {
    target.split('.').collect()
}

/// The output effect a pad line names: `tapestop`, `gate` (`gate 1/8`: the
/// division, a sixteenth by default), `crush`, `scream`, `cut`, `sweep`.
pub fn out_fx(word: &str, rest: &[&str]) -> Option<crate::perform::fx::OutFx> {
    use crate::perform::fx::OutFx;
    match (word, rest) {
        ("tapestop", []) => Some(OutFx::TapeStop),
        ("gate", []) => Some(OutFx::Gate(1.0)),
        ("gate", [d]) => repeat_division(d).map(OutFx::Gate),
        ("crush", []) => Some(OutFx::Crush),
        ("scream", []) => Some(OutFx::Scream),
        ("cut", []) => Some(OutFx::Cut),
        ("sweep", []) => Some(OutFx::Sweep),
        _ => None,
    }
}

/// The tracks a `pad N > hold <track>` (or `key N > hold`) line keeps out
/// until the pad or key is held.
pub fn held_out(song: &Song) -> Vec<String> {
    song.midi
        .iter()
        .filter(|m| matches!(m.source, MidiSource::Pad(_) | MidiSource::Key(_)))
        .filter_map(|m| m.target.strip_prefix("hold.").map(String::from))
        .collect()
}

/// `song` as a live session starts it, with no hand on the controller: every
/// track a `hold` pad keeps out pulled to `level 0`, everywhere it is defined,
/// and the level lanes that would bring it back dropped. What measures a song
/// for a live session measures this, since this is what it hears; a render
/// of the file, which ignores the `midi` block, does not. True when it
/// changed anything.
pub fn at_rest(song: &mut Song) -> bool {
    let out = held_out(song);
    if out.is_empty() {
        return false;
    }
    let level_of_held = |target: &str| target.strip_suffix(".level").is_some_and(|t| out.iter().any(|o| o == t));
    for t in song.tracks.iter_mut().chain(song.scenes.iter_mut().flat_map(|s| s.tracks.iter_mut())) {
        if out.contains(&t.name) {
            t.level = Some(0.0);
        }
    }
    for sc in song.scenes.iter_mut() {
        sc.automations.retain(|a| !level_of_held(&a.target));
    }
    song.automations.retain(|a| !level_of_held(&a.target));
    true
}

/// `1/4`, `1/8`, `1/16`, `1/32` as sixteenths: the lengths a `repeat` pad
/// loops. Longer is a different gesture (a loop, not a roll), shorter is a
/// buzz the grid cannot place.
pub fn repeat_division(word: &str) -> Option<f32> {
    match word {
        "1/4" => Some(4.0),
        "1/8" => Some(2.0),
        "1/16" => Some(1.0),
        "1/32" => Some(0.5),
        _ => None,
    }
}

/// The note a `play` pad sounds: the one written (`C2`, `F#3`, or a drum
/// name on a `beats` track), else the scale's root -- in octave 2 on a
/// `bass` module, where a bass line lives, and octave 3 on anything else --
/// or the kick on a kit. `None` when what is written is neither.
pub fn play_note(song: &Song, track: &str, written: Option<&str>) -> Option<u8> {
    let kind = song
        .tracks
        .iter()
        .find(|t| t.name == track)
        .and_then(|t| song.module_defs.iter().find(|m| m.name == t.using_instrument))
        .map(|m| m.module_type.as_str());
    match (written, kind) {
        (Some(w), Some("beats")) => crate::dsl::compiler::drum_note(w),
        (Some(w), _) => {
            let first = w.chars().next()?;
            (matches!(first.to_ascii_uppercase(), 'A'..='G') && crate::dsl::compiler::note_in_midi_range(w))
                .then(|| crate::dsl::compiler::note_name_to_midi(w))
        }
        (None, Some("beats")) => crate::dsl::compiler::drum_note("kick"),
        (None, kind) => {
            let (_, root) = crate::dsl::compiler::scale_context(song);
            let octave: u8 = if kind == Some("bass") { 2 } else { 3 };
            Some(12 * (octave + 1) + root % 12)
        }
    }
}

/// A track a key zone plays, resolved to engine indices.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZoneTarget {
    pub track: usize,
    pub instrument: usize,
    /// One note at a time: the bass zone always, the lead zone on a `bass`
    /// module.
    pub mono: bool,
    pub roll: bool,
    /// The kick a roll strikes on the beat: track and drum note.
    pub kick: Option<(usize, u8)>,
    /// Semitones every key of the zone is moved by, in whole octaves: what
    /// puts a bass zone in the register of the line it plays over.
    pub shift: i8,
}

/// A stretch of the keyboard, by MIDI note, both ends in it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Zone {
    pub kind: ZoneKind,
    pub low: u8,
    pub high: u8,
    /// What it plays until a scene says otherwise.
    pub play: Option<ZoneTarget>,
}

impl Zone {
    pub fn contains(&self, note: u8) -> bool {
        (self.low..=self.high).contains(&note)
    }
}

/// A `perform` block resolved against one engine.
#[derive(Debug, Clone)]
pub struct Scene {
    pub name: String,
    pub scale: Option<Scale>,
    pub lock: Option<Lock>,
    pub bass: Option<ZoneTarget>,
    pub lead: Option<ZoneTarget>,
    /// Its `set` lines, each a knob with its value at the bottom of its
    /// travel: `ops(0)` puts it in place.
    pub sets: Vec<Knob>,
    /// Its `bend` lines.
    pub bends: Vec<(ZoneKind, BendRange, Hands)>,
    /// Its `wheel` and `cc` lines, with when each answers: while it plays,
    /// a controller named here moves these instead of what the `midi`
    /// block maps it to.
    pub knobs: Vec<(Hands, Knob)>,
}

/// Everything a `midi` block maps, resolved against one engine.
#[derive(Debug, Clone)]
pub struct Controls {
    pub knobs: Vec<Knob>,
    pub keys: Vec<Keys>,
    pub pads: Vec<Pad>,
    /// `key` lines: keys of the trigger zone, which act like pads.
    pub triggers: Vec<Pad>,
    pub zones: Vec<Zone>,
    pub scenes: Vec<Scene>,
    /// How the bass and lead zones keep to the scale, unless a scene says.
    pub lock: Lock,
    /// The song's scale; a scene may name its own.
    pub scale: Option<Scale>,
    /// The `midi` block's `bend` lines.
    pub bends: Vec<(ZoneKind, BendRange, Hands)>,
    /// The knobs' voices, in order: what each page moves.
    pub pages: Vec<Page>,
    /// `takeover pickup`.
    pub pickup: bool,
    /// Controllers that choose the page: `cc 114 > voice` picks by
    /// position, `cc 114 > voice step` steps like an encoder.
    pub voice_knobs: Vec<(u8, bool)>,
}

/// A page of knobs, resolved.
#[derive(Debug, Clone)]
pub struct Page {
    pub name: String,
    pub knobs: Vec<Knob>,
}

impl Page {
    /// The page's lines for controller `cc`; empty when the page leaves it
    /// to the `midi` block.
    pub fn lines(&self, cc: u8) -> impl Iterator<Item = &Knob> {
        self.knobs.iter().filter(move |k| k.cc == cc)
    }

    /// `cutoff · resonance · decay`: what its knobs move, in order, by the
    /// last word of each target.
    pub fn summary(&self) -> String {
        let mut seen: Vec<u8> = Vec::new();
        let mut words: Vec<String> = Vec::new();
        for k in &self.knobs {
            if seen.contains(&k.cc) {
                continue;
            }
            seen.push(k.cc);
            let w = k.target.rsplit('.').next().unwrap_or(&k.target);
            words.push(if k.moves.is_empty() { format!("({})", w) } else { String::from(w) });
        }
        words.join(" · ")
    }
}

impl Default for Controls {
    fn default() -> Self {
        Self {
            knobs: Vec::new(),
            keys: Vec::new(),
            pads: Vec::new(),
            triggers: Vec::new(),
            zones: Vec::new(),
            scenes: Vec::new(),
            lock: Lock::Snap,
            scale: None,
            bends: Vec::new(),
            pages: Vec::new(),
            pickup: false,
            voice_knobs: Vec::new(),
        }
    }
}

impl Controls {
    pub fn scene(&self, name: &str) -> Option<&Scene> {
        self.scenes.iter().find(|s| s.name == name)
    }

    /// The zone a key on the keyboard channel falls in.
    pub fn zone_of(&self, note: u8) -> Option<&Zone> {
        self.zones.iter().find(|z| z.contains(note))
    }

    /// What zone `kind` plays under `scene`: the scene's track, or the
    /// zone's own.
    pub fn zone_target(&self, kind: ZoneKind, scene: Option<&str>) -> Option<ZoneTarget> {
        let sc = scene.and_then(|n| self.scene(n));
        let from_scene = match kind {
            ZoneKind::Bass => sc.and_then(|s| s.bass),
            ZoneKind::Lead => sc.and_then(|s| s.lead),
            ZoneKind::Triggers => None,
        };
        from_scene.or_else(|| self.zones.iter().find(|z| z.kind == kind).and_then(|z| z.play))
    }

    /// The bend lines for `zone` under `scene`: the scene's, else the
    /// `midi` block's, else two semitones on the lead while it is played.
    pub fn bends_under(&self, zone: ZoneKind, scene: Option<&str>) -> Vec<(BendRange, Hands)> {
        let of = |list: &[(ZoneKind, BendRange, Hands)]| -> Vec<(BendRange, Hands)> {
            list.iter().filter(|b| b.0 == zone).map(|b| (b.1, b.2)).collect()
        };
        let from_scene = scene.and_then(|n| self.scene(n)).map(|s| of(&s.bends)).unwrap_or_default();
        if !from_scene.is_empty() {
            return from_scene;
        }
        let from_block = of(&self.bends);
        if !from_block.is_empty() {
            return from_block;
        }
        match zone {
            ZoneKind::Lead => Vec::from([(BendRange::Semitones(crate::live::BEND_SEMITONES), Hands::Playing(zone))]),
            _ => Vec::new(),
        }
    }

    /// The scene's own lines for controller `cc`, by index in its list:
    /// `None` when the scene has none and the `midi` block's line stands.
    pub fn scene_knobs(&self, cc: u8, scene: Option<&str>) -> Option<Vec<usize>> {
        let sc = scene.and_then(|n| self.scene(n))?;
        let lines: Vec<usize> = (0..sc.knobs.len()).filter(|&i| sc.knobs[i].1.cc == cc).collect();
        (!lines.is_empty()).then_some(lines)
    }

    /// The scale and lock the zones keep to under `scene`.
    pub fn lock_under(&self, scene: Option<&str>) -> (Option<Scale>, Lock) {
        let sc = scene.and_then(|n| self.scene(n));
        (sc.and_then(|s| s.scale).or(self.scale), sc.and_then(|s| s.lock).unwrap_or(self.lock))
    }
}

/// Every `midi { }` mapping in `song`, resolved against one engine. A knob
/// that resolves to nothing is kept with no moves rather than dropped, so a
/// knob on a track that only exists in another scene does not vanish; keys
/// and pads that do not resolve are left out.
pub fn resolve_all(song: &Song, names: &Names) -> Controls {
    let mut controls = Controls::default();
    let track = |name: &str| names.tracks.iter().position(|t| t == name);
    for m in &song.midi {
        match m.source {
            MidiSource::Cc(cc) => {
                let span = m.range.as_ref().and_then(|r| knob_span(song, &m.target, r).ok());
                controls.knobs.push(Knob {
                    cc,
                    target: m.target.clone(),
                    moves: resolve(song, names, &m.target),
                    span,
                    guard: m.guard,
                })
            }
            MidiSource::Keys => {
                let Some(t) = track(&m.target) else { continue };
                let Some(&instrument) = names.track_instruments.get(t).filter(|&&i| i != usize::MAX) else { continue };
                let module = song.tracks.iter().find(|d| d.name == m.target).map(|d| d.using_instrument.as_str());
                let mono = song
                    .module_defs
                    .iter()
                    .find(|d| Some(d.name.as_str()) == module)
                    .is_some_and(|d| d.module_type == "bass");
                controls.keys.push(Keys { track: t, instrument, mono });
            }
            MidiSource::Pad(note) | MidiSource::Key(note) => {
                if let Some(action) = pad_action(song, &track, &m.target) {
                    let pad = Pad { note, action, quantize: m.quantize };
                    if matches!(m.source, MidiSource::Key(_)) {
                        controls.triggers.push(pad);
                    } else {
                        controls.pads.push(pad);
                    }
                }
            }
        }
    }
    let zone_of = |kind: ZoneKind| song.perform.zones.iter().find(|z| z.kind == kind);
    controls.zones = song
        .perform
        .zones
        .iter()
        .map(|z| Zone {
            kind: z.kind,
            low: z.low,
            high: z.high,
            play: z.play.as_ref().and_then(|p| zone_target(song, names, p, z.kind, z.low)),
        })
        .collect();
    let target =
        |kind: ZoneKind| move |play: &ZonePlay| zone_of(kind).and_then(|z| zone_target(song, names, play, kind, z.low));
    controls.lock = song.perform.lock.unwrap_or(Lock::Snap);
    controls.scale = song.globals.scale.as_ref().and_then(|d| Scale::named(&d.root, &d.kind));
    controls.scenes = song
        .perform
        .scenes
        .iter()
        .map(|sc| Scene {
            name: sc.name.clone(),
            scale: sc.scale.as_ref().and_then(|d| Scale::named(&d.root, &d.kind)),
            lock: sc.lock,
            bass: sc.bass.as_ref().and_then(target(ZoneKind::Bass)),
            lead: sc.lead.as_ref().and_then(target(ZoneKind::Lead)),
            sets: sc
                .sets
                .iter()
                .map(|set| {
                    let ends = [set.value.clone(), set.value.clone()];
                    Knob {
                        cc: 0,
                        target: set.target.clone(),
                        moves: resolve(song, names, &set.target),
                        span: knob_span(song, &set.target, &ends).ok(),
                        guard: false,
                    }
                })
                .collect(),
            bends: sc.bends.iter().map(|b| (b.zone, b.range, b.hands)).collect(),
            knobs: sc
                .knobs
                .iter()
                .map(|k| {
                    let m = &k.map;
                    let span = m.range.as_ref().and_then(|r| knob_span(song, &m.target, r).ok());
                    let MidiSource::Cc(cc) = m.source else { unreachable!("a scene's knob is a controller") };
                    (
                        k.hands,
                        Knob {
                            cc,
                            target: m.target.clone(),
                            moves: resolve(song, names, &m.target),
                            span,
                            guard: m.guard,
                        },
                    )
                })
                .collect(),
        })
        .collect();
    controls.bends = song.perform.bends.iter().map(|b| (b.zone, b.range, b.hands)).collect();
    controls.pages = song
        .perform
        .pages
        .iter()
        .map(|p| Page {
            name: p.name.clone(),
            knobs: p
                .knobs
                .iter()
                .filter_map(|m| {
                    let MidiSource::Cc(cc) = m.source else { return None };
                    let span = m.range.as_ref().and_then(|r| knob_span(song, &m.target, r).ok());
                    Some(Knob {
                        cc,
                        target: m.target.clone(),
                        moves: resolve(song, names, &m.target),
                        span,
                        guard: m.guard,
                    })
                })
                .collect(),
        })
        .collect();
    controls.pickup = song.perform.pickup.unwrap_or(false);
    controls.voice_knobs = song
        .midi
        .iter()
        .filter_map(|m| match (m.source, m.target.as_str()) {
            (MidiSource::Cc(cc), "voice") => Some((cc, false)),
            (MidiSource::Cc(cc), "voice.step") => Some((cc, true)),
            _ => None,
        })
        .collect();
    controls
}

/// What a pad or a trigger key line does, resolved: `None` when it names
/// something this engine does not have.
fn pad_action(song: &Song, track: &impl Fn(&str) -> Option<usize>, target: &str) -> Option<PadAction> {
    match pad_words(target).as_slice() {
        ["freeze"] => Some(PadAction::Freeze),
        ["next"] => Some(PadAction::Next),
        ["prev"] => Some(PadAction::Prev),
        ["step", n] => n.parse::<usize>().ok().filter(|&n| n >= 1).map(PadAction::Step),
        ["mute", t] => track(t).map(|track| PadAction::Mute { track }),
        ["toggle", t] => track(t).map(|track| PadAction::Toggle { track }),
        ["throw", t] => track(t).map(|track| PadAction::Throw { track }),
        ["hold", t] => track(t).map(|track| PadAction::Hold { track }),
        ["repeat", d] => repeat_division(d).map(|sixteenths| PadAction::Repeat { sixteenths }),
        [fx, rest @ ..] if out_fx(fx, rest).is_some() => out_fx(fx, rest).map(PadAction::Fx),
        ["play", t, rest @ ..] if rest.len() <= 1 => match (track(t), play_note(song, t, rest.first().copied())) {
            (Some(track), Some(note)) => Some(PadAction::Play { track, note }),
            _ => None,
        },
        [t, drum] => match (track(t), crate::dsl::compiler::drum_note(drum)) {
            (Some(track), Some(drum)) => Some(PadAction::Drum { track, drum }),
            _ => None,
        },
        _ => None,
    }
}

/// A zone's track in this engine: `None` when the engine has no such track
/// or it plays no instrument. `low` is the zone's lowest key.
fn zone_target(song: &Song, names: &Names, play: &ZonePlay, kind: ZoneKind, low: u8) -> Option<ZoneTarget> {
    let t = names.tracks.iter().position(|n| *n == play.track)?;
    let instrument = *names.track_instruments.get(t).filter(|&&i| i != usize::MAX)?;
    let module = song.tracks.iter().find(|d| d.name == play.track).map(|d| d.using_instrument.as_str());
    let mono =
        song.module_defs.iter().find(|d| Some(d.name.as_str()) == module).is_some_and(|d| d.module_type == "bass");
    let kick = play.kick.as_ref().and_then(|k| {
        let kt = names.tracks.iter().position(|n| n == k)?;
        Some((kt, crate::dsl::compiler::drum_note("kick")?))
    });
    // The key the zone's root sits on: its lowest key on the scale's root.
    let root = song.globals.scale.as_ref().and_then(|d| crate::perform::scale::root_pc(&d.root)).unwrap_or(0);
    let root_key = low as i32 + (root as i32 - low as i32).rem_euclid(12);
    // Where that root should sound: in the octave written, else, for a bass
    // zone, the root nearest the note the track's line plays most, else, on
    // a `bass` module, octave 1, where a psytrance bass lives.
    let wanted = match (play.octave, kind) {
        (Some(o), _) => Some(12 * (o as i32 + 1) + root as i32),
        (None, ZoneKind::Bass) => names
            .track_home
            .get(t)
            .copied()
            .flatten()
            .or_else(|| sibling_home(names, t))
            .map(|home| {
                let below = home as i32 - (home as i32 - root as i32).rem_euclid(12);
                if home as i32 - below > 6 {
                    below + 12
                } else {
                    below
                }
            })
            .or_else(|| mono.then_some(24 + root as i32)),
        // A lead zone on a voice of the song's own -- a keyboard copy of the
        // track's lead -- plays where that lead plays; any other as written.
        (None, ZoneKind::Lead) => sibling_home(names, t).map(|home| {
            let below = home as i32 - (home as i32 - root as i32).rem_euclid(12);
            if home as i32 - below > 6 {
                below + 12
            } else {
                below
            }
        }),
        (None, _) => None,
    };
    let shift = wanted.map_or(0, |w| (w - root_key + 6).div_euclid(12) * 12);
    Some(ZoneTarget { track: t, instrument, mono, roll: play.roll, kick, shift: shift.clamp(-96, 96) as i8 })
}

/// The note another track playing the same module plays most: where a
/// track with no notes of its own, kept for the keys, belongs.
fn sibling_home(names: &Names, t: usize) -> Option<u8> {
    let inst = *names.track_instruments.get(t)?;
    let name = names.instruments.get(inst)?;
    (0..names.tracks.len())
        .filter(|&o| o != t)
        .filter(|&o| names.track_instruments.get(o).and_then(|&i| names.instruments.get(i)) == Some(name))
        .find_map(|o| names.track_home.get(o).copied().flatten())
}

/// What `target` moves in the engine `names` describes. Resolves the way
/// `auto` does: `level` finds a track by name first and otherwise every track
/// that plays the module, and a module parameter reaches every copy.
pub fn resolve(song: &Song, names: &Names, target: &str) -> Vec<Move> {
    let words: Vec<&str> = target.split('.').collect();
    let mut out = Vec::new();
    let track = |name: &str| names.tracks.iter().position(|t| t == name);
    match words.as_slice() {
        ["reverb_mix"] => out.push(Move::ReverbMix),
        ["delay_mix"] => out.push(Move::DelayMix),
        ["reverb_freeze"] => out.push(Move::ReverbFreeze),
        ["tempo"] => out.push(Move::Tempo),
        [t, which @ ("delay_send" | "reverb_send")] => {
            if let Some(t) = track(t) {
                out.push(Move::TrackSend { track: t, reverb: *which == "reverb_send" });
            }
        }
        ["master", param] if node_param(param).is_some() => {
            if let Some(node) = master_node(song, None, param) {
                out.push(Move::NodeParam { track: None, node, param: node_param(param).unwrap().0 });
            }
        }
        ["master", label, param] if node_param(param).is_some() => {
            if let Some(node) = master_node(song, Some(label), param) {
                out.push(Move::NodeParam { track: None, node, param: node_param(param).unwrap().0 });
            }
        }
        [t, label, param] if *param != "wet" && node_param(param).is_some() => {
            if let Some(t) = track(t) {
                let found =
                    names.track_nodes.get(t).and_then(|nodes| nodes.iter().position(|l| l.as_deref() == Some(*label)));
                if let Some(node) = found {
                    out.push(Move::NodeParam { track: Some(t), node, param: node_param(param).unwrap().0 });
                }
            }
        }
        [name, which @ ("level" | "pan")] => {
            let tracks: Vec<usize> = match track(name) {
                Some(t) => Vec::from([t]),
                None => (0..names.tracks.len())
                    .filter(|&t| {
                        names
                            .track_instruments
                            .get(t)
                            .and_then(|&i| names.instruments.get(i))
                            .is_some_and(|n| n == name)
                    })
                    .collect(),
            };
            for t in tracks {
                out.push(if *which == "level" { Move::TrackLevel { track: t } } else { Move::TrackPan { track: t } });
            }
        }
        [t, node, "wet"] => {
            if let Some(t) = track(t) {
                let found =
                    names.track_nodes.get(t).and_then(|nodes| nodes.iter().position(|l| l.as_deref() == Some(*node)));
                if let Some(n) = found {
                    out.push(Move::NodeWet { track: t, node: n });
                }
            }
        }
        [module, param] => {
            let spec = module_named(song, module)
                .and_then(|def| ModuleKind::from_str(&def.module_type))
                .and_then(|kind| params::lookup(kind, param));
            if let Some(spec) = spec {
                // A track's name stands for its module, every copy of it, as
                // the module's own name does: the hats of a kit that two
                // tracks play are wherever they sound.
                let name = module_named(song, module).map(|d| d.name.as_str()).unwrap_or(module);
                for (i, n) in names.instruments.iter().enumerate() {
                    if n == name {
                        out.push(Move::Module { instrument: i, spec });
                    }
                }
            }
        }
        _ => {}
    }
    out
}

/// The module `name` names: a module by that name, or else the one the track
/// of that name plays, so `bass cutoff` reaches every track set's bass
/// whatever each calls its module.
pub fn module_named<'a>(song: &'a Song, name: &str) -> Option<&'a crate::dsl::ast::ModuleDef> {
    song.module_defs.iter().find(|m| m.name == name).or_else(|| {
        let t = song.tracks.iter().find(|t| t.name == name)?;
        song.module_defs.iter().find(|m| m.name == t.using_instrument)
    })
}

/// The master node a knob on `param` reaches, by its position in the chain
/// the engine builds (which leaves out a `limiter`): the one named `label`,
/// or with no label the first that has the parameter, as `auto master` picks.
pub fn master_node(song: &Song, label: Option<&str>, param: &str) -> Option<usize> {
    let kinds = node_kinds_for(param);
    let chain = song.master.as_ref()?.chain.iter().filter(|n| n.kind != "limiter");
    for (i, n) in chain.enumerate() {
        let named = label.is_none_or(|l| n.label.as_deref() == Some(l));
        if named && kinds.contains(&n.kind.as_str()) {
            return Some(i);
        }
        if named && label.is_some() {
            return None;
        }
    }
    None
}

/// The value `target` has in the text, where the text gives it one. A knob
/// forgets what it was turned to when this changes: the text was edited on
/// purpose, and the edit should be what plays.
pub fn text_value(song: &Song, target: &str) -> Option<f32> {
    let words: Vec<&str> = target.split('.').collect();
    match words.as_slice() {
        [t, "level"] => song.tracks.iter().find(|d| &d.name == t)?.level,
        [t, "pan"] => song.tracks.iter().find(|d| &d.name == t)?.pan,
        [t, "delay_send"] => song.tracks.iter().find(|d| &d.name == t)?.delay_send,
        [t, "reverb_send"] => song.tracks.iter().find(|d| &d.name == t)?.reverb_send,
        ["tempo"] => Some(song.globals.tempo),
        [t, node, "wet"] => {
            let def = song.tracks.iter().find(|d| &d.name == t)?;
            let n = def.routing.iter().find(|n| n.label.as_deref() == Some(*node))?;
            n.params.iter().find_map(|p| match p {
                Param::Named(k, v) if k == "wet" => Some(*v),
                _ => None,
            })
        }
        [t, label, param] if *t != "master" && *param != "wet" => {
            let def = song.tracks.iter().find(|d| &d.name == t)?;
            let node = def.routing.iter().find(|n| n.label.as_deref() == Some(*label))?;
            let _ = param;
            node.params.iter().find_map(|p| match p {
                Param::Float(v) => Some(*v),
                _ => None,
            })
        }
        ["master", label, param] => {
            let node = song.master.as_ref()?.chain.iter().find(|n| n.label.as_deref() == Some(*label))?;
            let _ = param;
            node.params.iter().find_map(|p| match p {
                Param::Float(v) => Some(*v),
                _ => None,
            })
        }
        [module, param] => module_named(song, module)?.params.iter().find(|p| &p.name == param).map(|p| p.value),
        _ => None,
    }
}

/// Where a knob's range, as written, lands on `target` as the engine reads
/// it: two ends, or three with the middle at half travel. The ends are in the
/// units the text uses for that target: a module parameter takes its own
/// (`200hz`, `40ms`, `60%`) or a plain number as its line in the module
/// would; a level takes a gain (`0.8`), `-6db` or `%`; a pan, a send, a wet
/// and a mix take a number or `%`; a node's cutoff takes Hz, its eq and
/// threshold dB; `tempo` takes BPM. The error is for `tatum check`.
pub fn knob_span(song: &Song, target: &str, range: &[RangeEnd]) -> Result<Span, String> {
    if !(2..=3).contains(&range.len()) {
        return Err(String::from("a knob's range has two ends, or three with the middle at half travel"));
    }
    let words: Vec<&str> = target.split('.').collect();
    let spec = match words.as_slice() {
        [module, param] if !matches!(*param, "level" | "pan" | "delay_send" | "reverb_send") && *module != "master" => {
            module_named(song, module)
                .and_then(|def| ModuleKind::from_str(&def.module_type))
                .and_then(|kind| params::lookup(kind, param))
        }
        _ => None,
    };
    let node = match words.as_slice() {
        ["master", p] | [_, _, p] if *p != "wet" => node_param(p),
        _ => None,
    };
    let end = |e: &RangeEnd| -> Result<f32, String> {
        let unit = e.unit.as_deref();
        match (words.as_slice(), spec, node) {
            (["reverb_freeze"], _, _) => Err(String::from("reverb_freeze is on or off; it takes no range")),
            (["tempo"], _, _) => match unit {
                None | Some("bpm") if (20.0..=999.0).contains(&e.value) => Ok(e.value),
                None => Err(String::from("tempo runs from 20 to 999")),
                Some(u) => Err(format!("tempo takes beats per minute, not '{}'", u)),
            },
            (_, _, Some(&(param, _, _, _))) => {
                let v = match (param, unit) {
                    (_, None) => e.value,
                    ("cutoff", Some("hz")) => e.value,
                    ("cutoff", Some("khz")) => e.value * 1000.0,
                    ("eq_low" | "eq_mid" | "eq_high" | "comp_threshold", Some("db")) => e.value,
                    ("gain", Some("db")) => crate::math::pow(10.0, e.value / 20.0),
                    ("tilt" | "position", Some("%")) => e.value / 100.0,
                    (_, Some(u)) => return Err(format!("'{}' does not take '{}'", param, u)),
                };
                let (lo, hi) = node_param_limits(param);
                if (lo..=hi).contains(&v) {
                    Ok(v)
                } else {
                    Err(format!("{} is outside {}..{} for '{}'", e.value, lo, hi, param))
                }
            }
            (_, Some(spec), _) => {
                if let Range::Choice(_) = spec.range {
                    return Err(format!(
                        "'{}' is a choice; a knob steps through all of them, with no range",
                        spec.name
                    ));
                }
                let v = match unit {
                    Some(u) => spec.value_from_quantity(e.value, u)?,
                    None => e.value,
                };
                if spec.range.contains(v) {
                    Ok(v)
                } else {
                    Err(format!("{} is outside what '{}' takes", e.value, spec.name))
                }
            }
            ([_, "level"], _, _) => {
                let gain = match unit {
                    None => e.value,
                    Some("db") => crate::math::pow(10.0, e.value / 20.0),
                    Some("%") => e.value / 100.0,
                    Some(u) => return Err(format!("a level takes a gain, dB or %, not '{}'", u)),
                };
                if (0.0..=crate::song_engine::MAX_TRACK_LEVEL).contains(&gain) {
                    Ok(gain)
                } else {
                    Err(format!("a level runs from 0 to {} (+12 dB)", crate::song_engine::MAX_TRACK_LEVEL))
                }
            }
            (w, _, _) => {
                let (lo, hi) = if matches!(w, [_, "pan"]) { (-1.0, 1.0) } else { (0.0, 1.0) };
                let v = match unit {
                    None => e.value,
                    Some("%") => e.value / 100.0,
                    Some(u) => return Err(format!("this takes a plain number or %, not '{}'", u)),
                };
                if (lo..=hi).contains(&v) {
                    Ok(v)
                } else {
                    Err(format!("{} is outside {}..{}", e.value, lo, hi))
                }
            }
        }
    };
    let mut points = [0.0f32; 3];
    for (i, e) in range.iter().enumerate() {
        points[i] = end(e)?;
    }
    if range.len() == 2 {
        points[2] = points[1];
    }
    Ok(Span { points, len: range.len(), log: node.is_some_and(|n| n.3) })
}

/// Where a 7-bit controller value lands on a target, as the engine reads it.
fn scaled(m: &Move, value: u8, span: Option<Span>) -> f32 {
    let x = value.min(127) as f32 / 127.0;
    // A range written with the knob: from one end to the other in the values
    // the engine reads, so a module parameter still sweeps along its own
    // curve and a node's cutoff geometrically. Choices and the freeze switch
    // have none.
    if let Some(span) = span {
        let continuous = match m {
            Move::Module { spec, .. } => !matches!(spec.range, Range::Choice(_)),
            Move::ReverbFreeze => false,
            _ => true,
        };
        if continuous {
            return span.at(x);
        }
    }
    match m {
        Move::Module { spec, .. } => match spec.range {
            Range::Unit => x,
            Range::Bipolar => x * 2.0 - 1.0,
            Range::Gain { max } => x * max,
            // Evenly spaced across the travel, so every choice gets the same
            // stretch of the knob.
            Range::Choice(names) => {
                let n = names.len();
                let idx = crate::math::floor(x * (n.max(1) - 1) as f32 + 0.5) as usize;
                params::choice_value(idx.min(n.saturating_sub(1)), n)
            }
        },
        // A fader at the top is `level 1.0`, not the +12 dB ceiling a level
        // can be written up to: the top of the travel is where it sounds as
        // loud as the file would have it at full.
        Move::TrackLevel { .. } => x,
        Move::TrackPan { .. } => x * 2.0 - 1.0,
        Move::NodeWet { .. } | Move::ReverbMix | Move::DelayMix | Move::TrackSend { .. } => x,
        Move::NodeParam { param, .. } => {
            let &(_, lo, hi, log) = node_param(param).expect("a known node parameter");
            Span::two(lo, hi, log).at(x)
        }
        // `tatum check` wants a range on a tempo knob; with none it spans
        // the tempos music is usually played at.
        Move::Tempo => Span::two(60.0, 180.0, false).at(x),
        // Past half is on, which is also what a pad sending 0 and 127 means.
        Move::ReverbFreeze => {
            if value >= 64 {
                1.0
            } else {
                0.0
            }
        }
    }
}

fn op(m: &Move, v: f32) -> FastOp {
    match *m {
        Move::Module { instrument, spec } => FastOp::ModuleParam { instrument, id: spec.id, value: v },
        Move::TrackLevel { track } => FastOp::TrackLevel { track, level: v },
        Move::TrackPan { track } => FastOp::TrackPan { track, pan: v },
        Move::NodeWet { track, node } => FastOp::NodeWet { track, node, wet: v },
        Move::ReverbMix => FastOp::ReverbMix(v),
        Move::DelayMix => FastOp::DelayMix(v),
        Move::ReverbFreeze => FastOp::ReverbFreeze(v >= 0.5),
        Move::TrackSend { track, reverb } => FastOp::TrackSend { track, reverb, amount: v },
        Move::NodeParam { track, node, param } => FastOp::NodeParam { track, node, param, value: v },
        Move::Tempo => FastOp::Tempo(v),
    }
}

/// A value in the target's own terms: `1.2khz`, `-6.0 dB`, `unison`, `40% L`.
fn reading(m: &Move, v: f32) -> String {
    match m {
        Move::Module { spec, .. } => match (spec.range, spec.curve.unit()) {
            (Range::Choice(_), _) => spec.choice_name(v).map(String::from).unwrap_or_default(),
            (Range::Gain { .. }, _) => decibels(v),
            (_, Some(unit)) => params::write_amount(spec.curve.to_real(v), unit),
            (Range::Bipolar, None) => format!("{:.2}", v),
            (Range::Unit, None) => percent(v),
        },
        Move::TrackLevel { .. } => decibels(v),
        Move::TrackPan { .. } => {
            let amount = crate::math::floor(crate::math::abs(v) * 100.0 + 0.5) as i32;
            if amount == 0 {
                String::from("center")
            } else {
                format!("{}% {}", amount, if v < 0.0 { "L" } else { "R" })
            }
        }
        Move::NodeWet { .. } | Move::ReverbMix | Move::DelayMix | Move::TrackSend { .. } => percent(v),
        Move::NodeParam { param: "cutoff", .. } => params::write_amount(v, params::Unit::Hz),
        Move::NodeParam { param: "eq_low" | "eq_mid" | "eq_high" | "comp_threshold", .. } => format!("{:+.1} dB", v),
        Move::NodeParam { param: "gain", .. } => decibels(v),
        Move::NodeParam { param: "position", .. } => {
            let x = v.clamp(0.0, 1.0) * 4.0;
            let near = ["a", "e", "i", "o", "u"][crate::math::floor(x + 0.5) as usize];
            format!("{} ({:.0}%)", near, v * 100.0)
        }
        Move::NodeParam { .. } => format!("{:.2}", v),
        Move::Tempo => format!("{:.1} bpm", v),
        Move::ReverbFreeze => String::from(if v >= 0.5 { "frozen" } else { "off" }),
    }
}

fn percent(v: f32) -> String {
    format!("{}%", crate::math::floor(v * 100.0 + 0.5) as i32)
}

fn decibels(gain: f32) -> String {
    if gain <= 0.0001 {
        String::from("off")
    } else {
        format!("{:+.1} dB", 20.0 * crate::math::log10(gain))
    }
}
