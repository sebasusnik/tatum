//! `--tui`: the live session on one screen, meant for half a terminal with
//! the editor in the other half.
//!
//! Top to bottom: where the song is (bar, beat, scene, BPM), the arrangement
//! or the set's steps with what is queued and how many bars until it lands,
//! the mix as a scrolling spectrogram, every track as a lane of its own five
//! bands scrolling with it, and at the bottom the knobs being turned and what
//! the session has to say -- a save that did not compile stays up, in red,
//! until one does.
//!
//! The colours are `tatum debug`'s: the same inferno ramp, log frequency and
//! 100 Hz / 1 kHz / 10 kHz lines, so the live screen and the pictures read
//! the same way.

pub mod edit;
pub mod fft;
pub mod palette;
pub mod shot;
pub mod telemetry;

use std::collections::VecDeque;
use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::{DefaultTerminal, Frame};

use palette::{inferno, BG, DIM, ERR, GOLD, HOT, KEYCAP, OK, PANEL, TEXT};
pub use telemetry::Telemetry;
use telemetry::BANDS;

/// What the screen knows about the song from its text, compiled on the
/// screen's own thread: names, bars and scenes.
pub struct SongInfo {
    pub title: String,
    pub tracks: Vec<String>,
    pub steps_per_bar: usize,
    /// Scene name and length in bars, in arrangement order. Empty for a live
    /// set, which loops.
    pub scenes: Vec<(String, u32)>,
    /// Each track's `play` line as written: `acid_riff16 every 4 rev`.
    pub plays: Vec<String>,
    /// Whether each track plays a drum pattern, for the palette.
    pub drums: Vec<bool>,
    /// The song's own patterns, by name, and whether each is a drum pattern:
    /// what the help shows a track could play instead.
    pub patterns: Vec<(String, bool)>,
    /// Every compiled pattern, transformed versions too, by the index the
    /// engine plays them at: what the grid draws.
    pub compiled: Vec<tatum_core::dsl::compiler::CompiledPattern>,
}

impl SongInfo {
    pub fn from_source(title: &str, text: &str) -> Option<Self> {
        let song = tatum_core::dsl::parse(text).ok()?;
        let compiled = tatum_core::dsl::compiler::compile(&song).ok()?;
        Some(Self {
            title: title.to_string(),
            tracks: compiled.tracks.iter().map(|t| t.name.clone()).collect(),
            steps_per_bar: (compiled.globals.meter.0 as usize * 4).max(1),
            scenes: compiled
                .arrangement
                .iter()
                .filter_map(|&(i, n)| compiled.scenes.get(i).map(|s| (s.name.clone(), n)))
                .collect(),
            plays: compiled.tracks.iter().map(|t| t.play_text.clone()).collect(),
            drums: compiled
                .tracks
                .iter()
                .map(|t| compiled.patterns.get(t.pattern_idx).is_some_and(|p| !p.lanes.is_empty()))
                .collect(),
            // Transformed versions carry a ` [` in their name; they are not
            // something to write on a line.
            patterns: compiled
                .patterns
                .iter()
                .filter(|p| !p.name.contains(" ["))
                .map(|p| (p.name.clone(), !p.lanes.is_empty()))
                .collect(),
            compiled: compiled.patterns.clone(),
        })
    }

    fn total_bars(&self) -> u32 {
        self.scenes.iter().map(|s| s.1).sum()
    }

    /// The scene playing at `bar`, its index, and the bar it started on.
    fn scene_at(&self, bar: usize) -> Option<(usize, usize)> {
        let mut start = 0usize;
        for (i, (_, n)) in self.scenes.iter().enumerate() {
            if bar < start + *n as usize {
                return Some((i, start));
            }
            start += *n as usize;
        }
        None
    }
}

/// A set's steps as the screen shows them.
pub struct SetView {
    pub steps: Vec<String>,
    pub current: usize,
    /// The step asked for and the bars until it lands.
    pub next: Option<(usize, usize)>,
    pub phrase: usize,
}

/// What a key asks for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Key {
    Quit,
    Next,
    Prev,
    Step(usize),
    /// Change the selected track's `play` line.
    Edit(edit::Op),
    /// Put back what the last edit from the screen changed.
    Undo,
    /// Mute or unmute the chosen track.
    Mute,
    /// Solo the chosen track, or take the solo off.
    Solo,
}

#[derive(Clone, Copy)]
pub enum Tone {
    Info,
    Good,
    Bad,
}

impl Tone {
    fn color(self) -> Color {
        match self {
            Tone::Info => TEXT,
            Tone::Good => OK,
            Tone::Bad => ERR,
        }
    }
}

/// Seconds the newest log line holds the footer before the keys come back.
const NEWS_SECS: f32 = 4.0;

/// The step list `g` opens: where the cursor is, and the number being typed.
struct Picker {
    cursor: usize,
    typed: String,
}

/// Steps of a set on screen at once, one per number key.
const STEPS_SHOWN: usize = 9;

/// Log-spaced rows of the spectrogram, before they are fitted to the screen.
const SPEC_BINS: usize = 160;
const SPEC_LOW_HZ: f32 = 30.0;
const SPEC_HIGH_HZ: f32 = 16_000.0;
/// dB span the colour ramp covers. Measured on three songs at the level the
/// engine brings every song to: the median bin sits near -52 dB, the loudest
/// near -9. With these the room between notes is near black, the body of the
/// mix purple and what is loud orange to yellow, the way the pictures look.
const SPEC_FLOOR_DB: f32 = -75.0;
const SPEC_TOP_DB: f32 = -12.0;
const LANE_FLOOR_DB: f32 = -66.0;
const LANE_TOP_DB: f32 = -12.0;
/// A new column every this long: at 100 columns, four seconds of history.
const COLUMN: Duration = Duration::from_millis(40);
const HISTORY: usize = 600;

pub struct Screen {
    /// Lent out while a frame is drawn, since drawing borrows the rest.
    term: Option<DefaultTerminal>,
    telemetry: Arc<Telemetry>,
    pub song: Option<SongInfo>,
    pub set: Option<SetView>,
    device: String,
    midi: Vec<String>,
    fft: fft::Fft,
    samples: Vec<f32>,
    power: Vec<f32>,
    spectrum: VecDeque<[f32; SPEC_BINS]>,
    /// For each column of history, the bar that began in it, if one did.
    bar_marks: VecDeque<Option<usize>>,
    marked_bar: Option<usize>,
    lanes: Vec<VecDeque<[f32; BANDS]>>,
    /// Per lane and column, the note the track held: the lane's colour.
    lane_notes: Vec<VecDeque<Option<u8>>>,
    /// The note each track held last, so a tail keeps its note's colour.
    last_note: Vec<Option<u8>>,
    lane_peak: Vec<f32>,
    last_column: Instant,
    log: VecDeque<(f32, String, Tone)>,
    knobs: VecDeque<(Instant, String)>,
    error: Option<Vec<String>>,
    started: Instant,
    /// The track the transform keys act on, by index.
    selected: Option<usize>,
    help: bool,
    /// The session log, open over everything, `l` toggles it.
    log_open: bool,
    /// The step ← → and 1-9 have moved to, not yet asked for: Enter goes.
    browse: Option<usize>,
    transformed: Vec<bool>,
    muted: Vec<bool>,
    /// The tracks soloed from the keyboard, by name, for their mark.
    pub soloed: Vec<String>,
    /// Tracks added to the selection with Shift+↑↓, besides `selected`.
    marked: Vec<usize>,
    /// `g`: the list of every step of the set, to go to any of them.
    picker: Option<Picker>,
    /// The first step of the nine on screen, which 1-9 reach; ← → move it.
    view: usize,
    /// The step playing when `view` last followed it.
    followed: Option<usize>,
    /// Paint sound with cell backgrounds only, one pixel per cell, so a
    /// terminal that makes cell backgrounds translucent (Ghostty's
    /// `background-opacity-cells`) shows the window behind all of it. The
    /// half-block pixels put half the sound in the foreground, which stays
    /// opaque.
    pub glass: bool,
    bar: usize,
    step: usize,
    tempo: f32,
    finished: bool,
}

impl Screen {
    pub fn new(telemetry: Arc<Telemetry>, device: String, midi: Vec<String>) -> io::Result<Self> {
        let mut screen = Self::headless(telemetry, device, midi);
        screen.term = Some(ratatui::try_init()?);
        Ok(screen)
    }

    /// A screen with no terminal, drawn only by `snapshot`: for pictures of
    /// it and for tests.
    pub fn headless(telemetry: Arc<Telemetry>, device: String, midi: Vec<String>) -> Self {
        let fft = fft::Fft::new(4096);
        let n = fft.len();
        Self {
            term: None,
            telemetry,
            song: None,
            set: None,
            device,
            midi,
            fft,
            samples: vec![0.0; n],
            power: vec![0.0; n / 2],
            spectrum: VecDeque::with_capacity(HISTORY),
            bar_marks: VecDeque::with_capacity(HISTORY),
            marked_bar: None,
            lanes: Vec::new(),
            lane_notes: Vec::new(),
            last_note: Vec::new(),
            lane_peak: Vec::new(),
            last_column: Instant::now(),
            log: VecDeque::new(),
            knobs: VecDeque::new(),
            error: None,
            started: Instant::now(),
            selected: None,
            help: false,
            log_open: false,
            browse: None,
            transformed: Vec::new(),
            muted: Vec::new(),
            soloed: Vec::new(),
            marked: Vec::new(),
            picker: None,
            view: 0,
            followed: None,
            glass: false,
            bar: 0,
            step: 0,
            tempo: 0.0,
            finished: false,
        }
    }

    pub fn set_song(&mut self, song: SongInfo) {
        if self.song.as_ref().map(|s| &s.tracks) != Some(&song.tracks) {
            self.lanes = vec![VecDeque::with_capacity(HISTORY); song.tracks.len()];
            self.lane_notes = vec![VecDeque::with_capacity(HISTORY); song.tracks.len()];
            self.last_note = vec![None; song.tracks.len()];
            self.lane_peak = vec![0.0; song.tracks.len()];
        }
        self.song = Some(song);
    }

    pub fn say(&mut self, line: impl Into<String>, tone: Tone) {
        self.log.push_back((self.started.elapsed().as_secs_f32(), line.into(), tone));
        while self.log.len() > 50 {
            self.log.pop_front();
        }
    }

    /// A save that did not compile: shown until the next one that does.
    pub fn error(&mut self, lines: Vec<String>) {
        self.say("save rejected: still playing the last good version", Tone::Bad);
        self.error = Some(lines);
    }

    pub fn clear_error(&mut self) {
        self.error = None;
    }

    pub fn knob(&mut self, reading: String) {
        self.knobs.retain(|(_, r)| r.split(' ').take(2).ne(reading.split(' ').take(2)));
        self.knobs.push_back((Instant::now(), reading));
        while self.knobs.len() > 6 {
            self.knobs.pop_front();
        }
    }

    pub fn position(&mut self, bar: usize, step: usize, tempo: f32) {
        self.bar = bar;
        self.step = step;
        self.tempo = tempo;
    }

    pub fn finished(&mut self) {
        self.finished = true;
    }

    /// Keys waiting, without blocking.
    pub fn keys(&mut self) -> Vec<Key> {
        let mut out = Vec::new();
        while event::poll(Duration::ZERO).unwrap_or(false) {
            let Ok(Event::Key(k)) = event::read() else { continue };
            if k.kind != KeyEventKind::Press {
                continue;
            }
            // The step list takes every key while it is open.
            if let Some(key) = self.picker_key(k.code) {
                out.extend(key);
                continue;
            }
            use edit::{Op, Toggle};
            let key = match k.code {
                KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => Some(Key::Quit),
                // Esc steps back -- the help, then the selection -- and never
                // quits: a set is not ended by a stray key.
                KeyCode::Esc => {
                    if self.help {
                        self.help = false;
                    } else if self.log_open {
                        self.log_open = false;
                    } else if self.browse.is_some() {
                        self.browse = None;
                    } else {
                        self.selected = None;
                        self.marked.clear();
                    }
                    None
                }
                KeyCode::Char('q') | KeyCode::Char('Q') => Some(Key::Quit),
                KeyCode::Char('?') => {
                    self.help = !self.help;
                    None
                }
                KeyCode::Char('l') => {
                    self.log_open = !self.log_open;
                    None
                }
                KeyCode::Char('g') | KeyCode::Tab => {
                    if let Some(set) = &self.set {
                        let cursor = set.next.map(|n| n.0).unwrap_or(set.current);
                        self.picker = Some(Picker { cursor, typed: String::new() });
                    }
                    None
                }
                // Shift+↑↓ adds the next track to the selection; ↑↓ alone
                // moves it and leaves one track chosen.
                KeyCode::Up if k.modifiers.contains(KeyModifiers::SHIFT) => {
                    self.extend(-1);
                    None
                }
                KeyCode::Down if k.modifiers.contains(KeyModifiers::SHIFT) => {
                    self.extend(1);
                    None
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.marked.clear();
                    self.select(-1);
                    None
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.marked.clear();
                    self.select(1);
                    None
                }
                // ← → and 1-9 only look through the set; Enter asks for the
                // step looked at. Space goes straight to the next one.
                KeyCode::Right => {
                    self.browse_by(1);
                    None
                }
                KeyCode::Left => {
                    self.browse_by(-1);
                    None
                }
                KeyCode::Char(c @ '1'..='9') => {
                    let count = self.set.as_ref().map_or(0, |s| s.steps.len());
                    let step = self.view + (c as usize - '1' as usize);
                    if step < count {
                        self.browse = Some(step);
                        // It is on screen already: the row stays where the
                        // number was pressed.
                        self.followed = Some(step);
                    }
                    None
                }
                KeyCode::Enter => self.browse.take().map(|b| Key::Step(b + 1)),
                KeyCode::Char(' ') | KeyCode::Char('n') => Some(Key::Next),
                KeyCode::Char('p') => Some(Key::Prev),
                KeyCode::Char('r') => Some(Key::Edit(Op::Toggle(Toggle::Rev))),
                KeyCode::Char('f') => Some(Key::Edit(Op::Toggle(Toggle::Fast))),
                KeyCode::Char('h') => Some(Key::Edit(Op::Toggle(Toggle::Slow))),
                KeyCode::Char('e') => Some(Key::Edit(Op::Toggle(Toggle::EveryRev))),
                KeyCode::Char('d') => Some(Key::Edit(Op::Degrade)),
                KeyCode::Char(']') => Some(Key::Edit(Op::Shift(1))),
                KeyCode::Char('[') => Some(Key::Edit(Op::Shift(-1))),
                KeyCode::Char('+') | KeyCode::Char('=') => Some(Key::Edit(Op::Up(1))),
                KeyCode::Char('-') => Some(Key::Edit(Op::Up(-1))),
                KeyCode::Char('x') => Some(Key::Edit(Op::Clear)),
                KeyCode::Char('u') => Some(Key::Undo),
                KeyCode::Char('m') => Some(Key::Mute),
                KeyCode::Char('s') => Some(Key::Solo),
                _ => None,
            };
            // A track key with nothing selected picks the first lane.
            if matches!(key, Some(Key::Edit(_) | Key::Mute | Key::Solo)) && self.selected.is_none() {
                self.select(1);
            }
            out.extend(key);
        }
        out
    }

    /// A key while the step list is open: `Some` when the list took it, with
    /// the step it asks for when that is what the key did.
    fn picker_key(&mut self, code: KeyCode) -> Option<Option<Key>> {
        let count = self.set.as_ref().map_or(0, |s| s.steps.len());
        let picker = self.picker.as_mut()?;
        if count == 0 {
            self.picker = None;
            return Some(None);
        }
        match code {
            KeyCode::Esc | KeyCode::Char('g') | KeyCode::Tab => self.picker = None,
            KeyCode::Up | KeyCode::Char('k') => {
                picker.cursor = (picker.cursor + count - 1) % count;
                picker.typed.clear();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                picker.cursor = (picker.cursor + 1) % count;
                picker.typed.clear();
            }
            KeyCode::PageUp => picker.cursor = picker.cursor.saturating_sub(10),
            KeyCode::PageDown => picker.cursor = (picker.cursor + 10).min(count - 1),
            KeyCode::Home => picker.cursor = 0,
            KeyCode::End => picker.cursor = count - 1,
            // Typing a number moves to it: `1` then `4` is step 14.
            KeyCode::Char(c @ '0'..='9') => {
                picker.typed.push(c);
                if picker.typed.parse::<usize>().map_or(true, |n| n > count) {
                    picker.typed = c.to_string();
                }
                if let Ok(n) = picker.typed.parse::<usize>() {
                    if (1..=count).contains(&n) {
                        picker.cursor = n - 1;
                    }
                }
            }
            KeyCode::Backspace => {
                picker.typed.pop();
            }
            KeyCode::Enter => {
                let step = picker.cursor + 1;
                self.picker = None;
                return Some(Some(Key::Step(step)));
            }
            _ => {}
        }
        Some(None)
    }

    /// Every step of the set, the one playing and the one queued marked, to
    /// choose where to go.
    fn picker_overlay(&self, f: &mut Frame, area: Rect) {
        let (Some(picker), Some(set)) = (self.picker.as_ref(), self.set.as_ref()) else { return };
        let w = area.width.clamp(30, 60);
        let h = area.height.saturating_sub(4).clamp(6, (set.steps.len() as u16) + 3);
        let pop = Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h };
        let rows = (h - 3) as usize;
        let first = picker.cursor.saturating_sub(rows / 2).min(set.steps.len().saturating_sub(rows));
        let mut lines: Vec<Line> = Vec::new();
        for (i, name) in set.steps.iter().enumerate().skip(first).take(rows) {
            let queued = set.next.map(|n| n.0) == Some(i);
            let mark = if i == set.current {
                "▶ playing"
            } else if queued {
                "· next"
            } else {
                ""
            };
            let here = i == picker.cursor;
            let style = if here {
                Style::new().fg(Color::Black).bg(GOLD).add_modifier(Modifier::BOLD)
            } else if i == set.current {
                Style::new().fg(GOLD)
            } else if queued {
                Style::new().fg(HOT)
            } else {
                Style::new().fg(TEXT)
            };
            let label = format!(" {:>3}  {:<24}{:>10} ", i + 1, step_name(name), mark);
            lines.push(Line::styled(label, style));
        }
        let typed = if picker.typed.is_empty() { String::new() } else { format!("  go to {}", picker.typed) };
        lines.push(Line::styled(format!(" ↑↓ or a number · Enter goes there · Esc{}", typed), Style::new().fg(DIM)));
        f.render_widget(ratatui::widgets::Clear, pop);
        f.render_widget(
            Paragraph::new(lines).block(
                Block::bordered()
                    .border_style(Style::new().fg(GOLD))
                    .title(Span::styled(
                        format!(" go to a step · {} steps · lands on the next phrase ", set.steps.len()),
                        Style::new().fg(GOLD),
                    ))
                    .style(Style::new().bg(PANEL)),
            ),
            pop,
        );
    }

    /// Look one step further along the set, from the step looked at, else
    /// the one queued, else the one playing.
    fn browse_by(&mut self, by: isize) {
        let Some(set) = &self.set else { return };
        let from = self.browse.or(set.next.map(|n| n.0)).unwrap_or(set.current);
        self.browse = Some(from.saturating_add_signed(by).min(set.steps.len().saturating_sub(1)));
    }

    /// Keep the step looked at -- else the one queued, else the one playing
    /// -- among the nine on screen, with one to spare on each side.
    fn follow(&mut self) {
        let Some(set) = &self.set else { return };
        let target = self.browse.or(set.next.map(|n| n.0)).unwrap_or(set.current);
        if self.followed == Some(target) {
            return;
        }
        self.followed = Some(target);
        let last = set.steps.len().saturating_sub(STEPS_SHOWN);
        if target < self.view + 1 {
            self.view = target.saturating_sub(1);
        } else if target + 2 > self.view + STEPS_SHOWN {
            self.view = (target + 2).saturating_sub(STEPS_SHOWN);
        }
        self.view = self.view.min(last);
    }

    /// Move the selection through the lanes on screen.
    fn select(&mut self, by: i32) {
        let active = self.active_lanes();
        if active.is_empty() {
            return;
        }
        let at = self.selected.and_then(|s| active.iter().position(|&a| a == s));
        let next = match at {
            None => 0,
            Some(i) => (i as i32 + by).rem_euclid(active.len() as i32) as usize,
        };
        self.selected = Some(active[next]);
    }

    /// Keep the track chosen now in the selection and move to the next one.
    fn extend(&mut self, by: i32) {
        if let Some(i) = self.selected {
            if !self.marked.contains(&i) {
                self.marked.push(i);
            }
        }
        self.select(by);
        if let Some(i) = self.selected {
            if !self.marked.contains(&i) {
                self.marked.push(i);
            }
        }
    }

    /// Every selected track, name and `play` line: the one the cursor is on
    /// first, then those added with Shift.
    pub fn selected_tracks(&self) -> Vec<(String, String)> {
        let Some(song) = self.song.as_ref() else { return Vec::new() };
        let mut idx: Vec<usize> = self.selected.into_iter().collect();
        for &m in &self.marked {
            if !idx.contains(&m) {
                idx.push(m);
            }
        }
        idx.into_iter().filter_map(|i| Some((song.tracks.get(i)?.clone(), song.plays.get(i)?.clone()))).collect()
    }

    /// Read the telemetry and draw a frame.
    pub fn draw(&mut self) -> io::Result<()> {
        self.analyse();
        self.follow();
        let Some(mut term) = self.term.take() else { return Ok(()) };
        let result = term.draw(|f| self.render(f)).map(|_| ());
        self.term = Some(term);
        result
    }

    fn analyse(&mut self) {
        let n = self.lanes.len().min(self.telemetry.tracks());
        self.transformed = (0..n).map(|i| self.telemetry.transformed(i)).collect();
        self.muted = (0..n).map(|i| self.telemetry.muted(i)).collect();
        if self.last_column.elapsed() < COLUMN {
            return;
        }
        self.last_column = Instant::now();
        self.column();
    }

    /// Take one column of history from the telemetry. Live, `draw` calls it
    /// every `COLUMN`; a snapshot calls it on the audio's own clock.
    pub fn column(&mut self) {
        self.telemetry.latest(&mut self.samples);
        self.fft.power_db(&self.samples, &mut self.power);
        let hz_per_bin = tatum_core::SAMPLE_RATE / self.fft.len() as f32;
        let mut column = [SPEC_FLOOR_DB; SPEC_BINS];
        for (row, c) in column.iter_mut().enumerate() {
            let lo = log_hz(row as f32 / SPEC_BINS as f32);
            let hi = log_hz((row + 1) as f32 / SPEC_BINS as f32);
            let (a, b) = ((lo / hz_per_bin) as usize, (hi / hz_per_bin).ceil() as usize);
            let b = b.max(a + 1).min(self.power.len());
            *c = self.power[a.min(b - 1)..b].iter().copied().fold(f32::MIN, f32::max);
        }
        push_capped(&mut self.spectrum, column);
        let mark = (self.marked_bar != Some(self.bar)).then_some(self.bar);
        self.marked_bar = Some(self.bar);
        push_capped(&mut self.bar_marks, mark);

        let n = self.lanes.len().min(self.telemetry.tracks());
        for i in 0..n {
            let (peak, bands) = self.telemetry.take(i);
            let mut db = [LANE_FLOOR_DB; BANDS];
            for (d, p) in db.iter_mut().zip(bands) {
                *d = 10.0 * (p + 1e-12).log10();
            }
            push_capped(&mut self.lanes[i], db);
            if let Some(n) = self.telemetry.take_note(i) {
                self.last_note[i] = Some(n);
            }
            push_capped(&mut self.lane_notes[i], self.last_note[i]);
            // A meter that falls slowly, the way a needle does.
            self.lane_peak[i] = peak.max(self.lane_peak[i] * 0.85);
        }
    }

    fn render(&self, f: &mut Frame) {
        let area = f.area();
        f.buffer_mut().set_style(area, Style::new().bg(BG).fg(TEXT));
        // The lanes take two rows each when there is room, one when not, and
        // the spectrogram everything left over.
        let active = self.active_lanes();
        let quiet = self.lanes.len() > active.len();
        let tracks = active.len() as u16;
        let rest = area.height.saturating_sub(1 + 2 + 5 + 1);
        let lane_rows = if rest >= tracks * 2 + 14 {
            tracks * 2
        } else if rest >= tracks + 10 {
            tracks
        } else {
            rest.saturating_sub(10)
        } + quiet as u16;
        let [header, timeline, spectrum, lanes, bottom, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Min(6),
            Constraint::Length(lane_rows),
            Constraint::Length(5),
            Constraint::Length(1),
        ])
        .areas(area);
        self.header(f, header);
        self.timeline(f, timeline);
        self.spectrogram(f.buffer_mut(), spectrum);
        self.lanes(f.buffer_mut(), lanes, &active);
        self.bottom(f, bottom);
        // The newest word from the session shows here for a few seconds,
        // then the keys come back; `l` has the whole log.
        let now = self.started.elapsed().as_secs_f32();
        match self.log.back().filter(|(t, _, _)| now - t < NEWS_SECS) {
            Some((_, line, tone)) => {
                let text: String = format!(" {}", line).chars().take(footer.width as usize).collect();
                f.render_widget(Paragraph::new(text).style(Style::new().fg(tone.color()).bg(BG)), footer);
            }
            None => {
                let hint = if self.set.is_some() {
                    " ← → 1-9 look at a step · Enter goes · ? every key · l log · q quit"
                } else {
                    " ? every key · l log · q quit"
                };
                f.render_widget(Paragraph::new(hint).style(Style::new().fg(DIM).bg(BG)), footer);
            }
        }
        if self.help {
            self.help_overlay(f, area);
        } else if self.log_open {
            self.log_overlay(f, area);
        }
        self.picker_overlay(f, area);
    }

    /// The chosen track's pattern as it plays this loop -- transformed when
    /// the loop is -- one bar of it, with the step playing lit. Returns
    /// whether it drew, so the knobs keep their place when nothing is chosen.
    fn grid(&self, f: &mut Frame, area: Rect) -> bool {
        use tatum_core::dsl::compiler::CompiledStep;
        let (Some(song), Some(i)) = (self.song.as_ref(), self.selected) else { return false };
        let (pat_idx, step) = self.telemetry.position(i);
        let Some(pat) = song.compiled.get(pat_idx) else { return false };
        let title = format!(
            " {} · {} ",
            song.tracks.get(i).map(String::as_str).unwrap_or(""),
            song.plays.get(i).map(String::as_str).unwrap_or("")
        );
        let on = self.transformed.get(i).copied().unwrap_or(false);
        let block = Block::new()
            .borders(Borders::TOP)
            .border_style(Style::new().fg(PANEL))
            .title(Span::styled(title, Style::new().fg(if on { HOT } else { GOLD })))
            .style(Style::new().bg(BG));
        let inner = block.inner(area);
        f.render_widget(block, area);
        if inner.height < 2 || inner.width < 16 {
            return true;
        }
        let buf = f.buffer_mut();
        let len = pat.len().max(1);
        let bar = (step / 16) * 16;
        let shown = 16.min(len - bar.min(len - 1));
        let cw = (inner.width as usize / 16).max(1);
        let rows_px = (inner.height as usize - 1) * 2;
        // Pixels: rows_px high, one column per cell.
        let mut px = vec![vec![0.0f32; shown * cw]; rows_px];
        let mut put = |row: usize, col: usize, v: f32| {
            if row < rows_px && col < shown * cw {
                px[row][col] = px[row][col].max(v);
            }
        };
        let bright = |vel: f32| 0.55 + 0.45 * vel.clamp(0.0, 1.0);
        if pat.lanes.is_empty() {
            let window = &pat.steps[bar..bar + shown];
            let notes = |s: &CompiledStep| -> Vec<u8> {
                match s {
                    CompiledStep::NoteOn { midi_note, .. } => vec![*midi_note],
                    CompiledStep::Chord { notes, count, .. } => {
                        notes[..*count as usize].iter().map(|n| n.midi_note).collect()
                    }
                    CompiledStep::Subdiv { notes, count, .. } => {
                        notes[..*count as usize].iter().filter(|n| n.velocity > 0.0).map(|n| n.midi_note).collect()
                    }
                    _ => vec![],
                }
            };
            let all: Vec<u8> = window.iter().flat_map(notes).collect();
            let (lo, hi) = (all.iter().copied().min().unwrap_or(60), all.iter().copied().max().unwrap_or(60));
            let row_of = |m: u8| {
                if hi == lo {
                    rows_px / 2
                } else {
                    (rows_px - 1) - ((m - lo) as usize * (rows_px - 1)) / (hi - lo) as usize
                }
            };
            let mut held: Vec<u8> = Vec::new();
            for (k, s) in window.iter().enumerate() {
                match s {
                    CompiledStep::NoteOn { midi_note, velocity, .. } => {
                        for c in 0..cw {
                            put(row_of(*midi_note), k * cw + c, bright(*velocity));
                        }
                        held = vec![*midi_note];
                    }
                    CompiledStep::Chord { notes: cn, count, .. } => {
                        for n in &cn[..*count as usize] {
                            for c in 0..cw {
                                put(row_of(n.midi_note), k * cw + c, bright(n.velocity));
                            }
                        }
                        held = cn[..*count as usize].iter().map(|n| n.midi_note).collect();
                    }
                    CompiledStep::Subdiv { notes: sn, count, .. } => {
                        let n = *count as usize;
                        for (j, note) in sn[..n].iter().enumerate() {
                            if note.velocity > 0.0 {
                                put(row_of(note.midi_note), k * cw + (j * cw) / n, bright(note.velocity));
                            }
                        }
                        held = notes(s).last().map(|m| vec![*m]).unwrap_or_default();
                    }
                    CompiledStep::Tie => {
                        for m in &held {
                            for c in 0..cw {
                                put(row_of(*m), k * cw + c, 0.3);
                            }
                        }
                    }
                    _ => held.clear(),
                }
            }
        } else {
            // A band of rows per drum, the first lane (the kick) at the bottom;
            // a hit fills its step across the band.
            let lanes = pat.lanes.len().min(rows_px);
            for (li, lane) in pat.lanes.iter().take(lanes).enumerate() {
                let top = rows_px - (li + 1) * rows_px / lanes;
                let bottom = rows_px - li * rows_px / lanes;
                let mut mark = |col: usize, v: f32| {
                    for row in top..bottom {
                        put(row, col, v);
                    }
                };
                for (k, s) in lane.steps[bar..bar + shown].iter().enumerate() {
                    match s {
                        CompiledStep::DrumHit { velocity, .. } => {
                            for c in 0..cw.saturating_sub(1).max(1) {
                                mark(k * cw + c, bright(*velocity));
                            }
                        }
                        CompiledStep::DrumSub { hits, count, .. } => {
                            let n = *count as usize;
                            for (j, h) in hits[..n].iter().enumerate() {
                                if *h > 0.0 {
                                    mark(k * cw + (j * cw) / n, bright(*h));
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        let playing = step.saturating_sub(bar);
        for y in 0..rows_px / 2 {
            for (x, (&top, &bottom)) in px[y * 2].iter().zip(&px[y * 2 + 1]).enumerate() {
                if let Some(c) = buf.cell_mut((inner.x + x as u16, inner.y + y as u16)) {
                    pixels(c, top, bottom, 0.05, self.glass);
                    if x / cw == playing {
                        let (fg, bg) = (lighten(c.fg), lighten(c.bg));
                        c.set_fg(fg).set_bg(if bg == Color::Reset { PANEL } else { bg });
                    }
                }
            }
        }
        // The count under it: the beats, and where the step is.
        let y = inner.y + inner.height - 1;
        for k in 0..shown {
            let x = inner.x + (k * cw) as u16;
            let (ch, color) = if k == playing {
                ("▲", GOLD)
            } else if k % 4 == 0 {
                ("·", TEXT)
            } else {
                ("·", PANEL)
            };
            let label = if k % 4 == 0 && k != playing { format!("{}", k / 4 + 1) } else { ch.to_string() };
            buf.set_string(x, y, label, Style::new().fg(color).bg(BG));
        }
        if len > 16 {
            buf.set_string(
                inner.x + (shown * cw) as u16 + 1,
                y,
                format!("bar {}/{}", bar / 16 + 1, len.div_ceil(16)),
                Style::new().fg(DIM).bg(BG),
            );
        }
        true
    }

    /// `?`: the words, the keys for them, and what the selected track could
    /// play instead.
    fn log_overlay(&self, f: &mut Frame, area: Rect) {
        let w = area.width.clamp(40, 100);
        // As tall as the log, up to most of the screen.
        let h = (self.log.len() as u16 + 2).clamp(4, area.height.clamp(8, 24));
        let pop = Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h };
        let lines: Vec<Line> = self
            .log
            .iter()
            .rev()
            .take(h.saturating_sub(2) as usize)
            .rev()
            .map(|(t, line, tone)| {
                Line::from(vec![
                    Span::styled(format!(" {:>4}:{:02} ", *t as u32 / 60, *t as u32 % 60), Style::new().fg(DIM)),
                    Span::styled(line.clone(), Style::new().fg(tone.color())),
                ])
            })
            .collect();
        f.render_widget(ratatui::widgets::Clear, pop);
        f.render_widget(
            Paragraph::new(lines).block(
                Block::bordered()
                    .border_style(Style::new().fg(GOLD))
                    .title(Span::styled(" session  ·  l or Esc closes ", Style::new().fg(GOLD)))
                    .style(Style::new().bg(PANEL)),
            ),
            pop,
        );
    }

    fn help_overlay(&self, f: &mut Frame, area: Rect) {
        let w = area.width.clamp(40, 82);
        let h = area.height.clamp(10, 30);
        let pop = Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h };
        let key = |k: &str, word: &str, what: &str| {
            Line::from(vec![
                Span::styled(format!(" {:<7}", k), Style::new().fg(GOLD).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{:<14}", word), Style::new().fg(HOT)),
                Span::styled(what.to_string(), Style::new().fg(TEXT)),
            ])
        };
        let mut lines = vec![
            key("← →", "a step", "look at the one before / after, through the whole set"),
            key("1-9", "a step", "look at the one in that place on screen"),
            key("Enter", "go", "to the step looked at, on the next phrase · Esc stays"),
            key("space", "next step", "g or Tab: every step, to pick one"),
            key("↑ ↓", "a track", "shift ↑ ↓ adds tracks · Esc back"),
            key("m s", "mute / solo", "the chosen tracks, at once, not written"),
            Line::raw(""),
            Line::styled(" the chosen tracks' `play` lines, written in the file:", Style::new().fg(DIM)),
            key("r", "rev", "the pattern backwards"),
            key("f", "fast 2", "twice in the same length"),
            key("h", "slow 2", "half speed, twice as long"),
            key("[ ]", "shift -1 / 1", "a step earlier / later"),
            key("- +", "up -1 / 1", "a degree of the scale down / up"),
            key("e", "every 4 rev", "backwards on the last of every 4 loops"),
            key("d", "degrade", "drop 25%, then 50% of the notes"),
            key("x", "", "every transform off"),
            key("u", "", "undo the last change made from here"),
            key("l", "log", "what happened this session"),
            Line::raw(""),
            Line::styled(
                " in the file: iter 4 · ply 2 · octave 1 · up 5st · sometimes 30% rev · play a, b",
                Style::new().fg(DIM),
            ),
        ];
        if let (Some(song), Some(i)) = (self.song.as_ref(), self.selected) {
            let drum = song.drums.get(i).copied().unwrap_or(false);
            let current = song.plays.get(i).and_then(|p| p.split([' ', ',']).next()).unwrap_or("");
            let prefix = current.split('_').next().unwrap_or(current);
            let mut same: Vec<&str> =
                song.patterns.iter().filter(|(n, d)| *d == drum && n != current).map(|(n, _)| n.as_str()).collect();
            same.sort_by_key(|n| !n.starts_with(prefix));
            let mut line = format!(" {} could play: ", song.tracks.get(i).map(String::as_str).unwrap_or(""));
            for n in same {
                if line.chars().count() + n.len() + 3 > w as usize - 2 {
                    line += "…";
                    break;
                }
                line += n;
                line += " · ";
            }
            lines.push(Line::raw(""));
            lines.push(Line::styled(line.trim_end_matches(" · ").to_string(), Style::new().fg(OK)));
        }
        f.render_widget(ratatui::widgets::Clear, pop);
        f.render_widget(
            Paragraph::new(lines).block(
                Block::bordered()
                    .border_style(Style::new().fg(GOLD))
                    .title(Span::styled(" every key  ·  ? or Esc closes ", Style::new().fg(GOLD)))
                    .style(Style::new().bg(PANEL)),
            ),
            pop,
        );
    }

    fn header(&self, f: &mut Frame, area: Rect) {
        let spb = self.song.as_ref().map_or(16, |s| s.steps_per_bar);
        let beats = (spb / 4).max(1);
        let beat = (self.step % spb) / 4;
        let mut spans = vec![
            Span::styled(" ◆ TATUM ", Style::new().fg(Color::Black).bg(GOLD).add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled(
                self.song.as_ref().map_or("…".to_string(), |s| s.title.clone()),
                Style::new().fg(TEXT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("   {:.0} BPM   bar ", self.tempo), Style::new().fg(DIM)),
            Span::styled(format!("{:<4}", self.bar + 1), Style::new().fg(GOLD).add_modifier(Modifier::BOLD)),
        ];
        for b in 0..beats {
            let (glyph, color) = if b == beat { ("● ", if b == 0 { GOLD } else { HOT }) } else { ("○ ", DIM) };
            spans.push(Span::styled(glyph, Style::new().fg(color)));
        }
        if let Some((scene, start, len)) = self.current_scene() {
            spans.push(Span::styled("  ▸ ", Style::new().fg(DIM)));
            spans.push(Span::styled(scene, Style::new().fg(HOT).add_modifier(Modifier::BOLD)));
            spans.push(Span::styled(format!(" {}/{}", self.bar + 1 - start, len), Style::new().fg(DIM)));
        }
        if self.finished {
            spans.push(Span::styled("  ■ finished", Style::new().fg(DIM)));
        }
        let t = self.started.elapsed().as_secs();
        let right = format!("{}  {}:{:02} ", self.device, t / 60, t % 60);
        let left_len: usize = spans.iter().map(|s| s.content.chars().count()).sum();
        let pad = (area.width as usize).saturating_sub(left_len + right.chars().count());
        spans.push(Span::raw(" ".repeat(pad)));
        spans.push(Span::styled(right, Style::new().fg(DIM)));
        f.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    /// The tracks heard in the last few seconds, in the song's order. A rig
    /// keeps a dozen voices loaded and silent; their lanes would be most of
    /// the screen and say nothing.
    fn active_lanes(&self) -> Vec<usize> {
        const RECENT: usize = 150;
        (0..self.lanes.len())
            .filter(|&i| {
                // A muted track keeps its lane, so it can be brought back,
                // but only when it would be playing something: a solo mutes
                // every other track, the silent ones too.
                if self.muted.get(i).copied().unwrap_or(false) {
                    return self.would_play(i);
                }
                let hist = &self.lanes[i];
                hist.iter().rev().take(RECENT).any(|col| col.iter().any(|&db| db > LANE_FLOOR_DB + 6.0))
            })
            .collect()
    }

    /// Whether the pattern track `i` is on has a note or a hit in it.
    fn would_play(&self, i: usize) -> bool {
        let Some(song) = self.song.as_ref() else { return false };
        let Some(pat) = song.compiled.get(self.telemetry.position(i).0) else { return false };
        pat.steps.iter().any(|s| s.is_onset()) || pat.lanes.iter().any(|l| l.steps.iter().any(|s| s.is_onset()))
    }

    fn current_scene(&self) -> Option<(String, usize, u32)> {
        let song = self.song.as_ref()?;
        let (i, start) = song.scene_at(self.bar)?;
        Some((song.scenes[i].0.clone(), start, song.scenes[i].1))
    }

    fn timeline(&self, f: &mut Frame, area: Rect) {
        let buf = f.buffer_mut();
        if let Some(set) = &self.set {
            self.steps(buf, area, set);
            return;
        }
        let Some(song) = self.song.as_ref() else { return };
        let total = song.total_bars().max(1) as f32;
        if song.scenes.is_empty() {
            let text = " live set: loops until you stop it";
            buf.set_string(area.x, area.y, text, Style::new().fg(DIM));
            return;
        }
        let w = area.width as f32;
        let current = song.scene_at(self.bar).map(|s| s.0);
        let mut x0 = 0f32;
        for (i, (name, bars)) in song.scenes.iter().enumerate() {
            let x1 = x0 + *bars as f32 / total * w;
            let (a, b) = (x0.round() as u16, (x1.round() as u16).max(x0.round() as u16 + 1));
            let playing = current == Some(i);
            let bg = if playing {
                HOT
            } else if i % 2 == 0 {
                PANEL
            } else {
                Color::Rgb(30, 24, 48)
            };
            let fg = if playing { Color::Black } else { DIM };
            for x in a..b.min(area.width) {
                if let Some(c) = buf.cell_mut((area.x + x, area.y + 1)) {
                    c.set_char(' ').set_bg(bg);
                }
            }
            let label: String = name.chars().take((b - a).saturating_sub(1) as usize).collect();
            buf.set_string(area.x + a, area.y + 1, &label, Style::new().fg(fg).bg(bg));
            x0 = x1;
        }
        // The playhead, above the lane, moving through the bar.
        let spb = song.steps_per_bar as f32;
        let pos = (self.bar as f32 + (self.step % song.steps_per_bar) as f32 / spb) / total;
        let x = ((pos * w) as u16).min(area.width.saturating_sub(1));
        if let Some(c) = buf.cell_mut((area.x + x, area.y)) {
            c.set_char('▼').set_fg(GOLD);
        }
    }

    fn steps(&self, buf: &mut Buffer, area: Rect, set: &SetView) {
        // Nine chips of one width, each with the key that reaches it; ← →
        // slide them along the set.
        let chip_w = (area.width.saturating_sub(2) / STEPS_SHOWN as u16).max(6);
        for slot in 0..STEPS_SHOWN {
            let i = self.view + slot;
            let Some(name) = set.steps.get(i) else { break };
            let x = area.x + 1 + slot as u16 * chip_w;
            let queued = set.next.map(|n| n.0) == Some(i);
            let style = if i == set.current {
                Style::new().fg(Color::Black).bg(GOLD).add_modifier(Modifier::BOLD)
            } else if self.browse == Some(i) {
                // Looked at, not asked for: white until Enter.
                Style::new().fg(Color::Black).bg(TEXT)
            } else if queued {
                // Flashes on the beat while it waits.
                if (self.step / 4).is_multiple_of(2) {
                    Style::new().fg(Color::Black).bg(HOT)
                } else {
                    Style::new().fg(HOT).bg(PANEL)
                }
            } else {
                Style::new().fg(DIM).bg(PANEL)
            };
            // The key that reaches the chip, drawn as a keycap apart from it,
            // so it never reads as part of the step's number.
            buf.set_string(
                x,
                area.y + 1,
                format!("{}", slot + 1),
                Style::new().fg(TEXT).bg(KEYCAP).add_modifier(Modifier::BOLD),
            );
            // The step's number in the set, marked as one, then its name.
            let width = chip_w as usize - 3;
            let number = format!(" #{} ", i + 1);
            let name: String = step_name(name).chars().take(width.saturating_sub(number.chars().count())).collect();
            let label = format!("{:<width$}", name, width = width.saturating_sub(number.chars().count()));
            buf.set_string(x + 2, area.y + 1, &number, style.add_modifier(Modifier::BOLD));
            buf.set_string(
                x + 2 + number.chars().count() as u16,
                area.y + 1,
                &label,
                style.remove_modifier(Modifier::BOLD),
            );
        }
        // More steps either side: an arrow says so.
        if self.view > 0 {
            buf.set_string(area.x, area.y + 1, "‹", Style::new().fg(GOLD));
        }
        if self.view + STEPS_SHOWN < set.steps.len() {
            buf.set_string(area.x + area.width - 1, area.y + 1, "›", Style::new().fg(GOLD));
        }
        // Where you stand in the whole set, and where you are going.
        let total = set.steps.len();
        let here = format!(" ▶ {}/{} {}", set.current + 1, total, step_name(&set.steps[set.current]));
        buf.set_string(area.x, area.y, &here, Style::new().fg(GOLD).add_modifier(Modifier::BOLD));
        let x = area.x + here.chars().count() as u16;
        let (going, color) = match set.next {
            Some((n, bars)) => (
                format!(
                    "   →  {}/{} {}  in {} bar{}",
                    n + 1,
                    total,
                    step_name(&set.steps[n]),
                    bars,
                    if bars == 1 { "" } else { "s" }
                ),
                HOT,
            ),
            None if self.browse.is_some() => (String::new(), DIM),
            None => (format!("   ← → to choose where to go · phrase {} bars", set.phrase), DIM),
        };
        buf.set_string(x, area.y, &going, Style::new().fg(color));
        if let Some(b) = self.browse.filter(|&b| Some(b) != set.next.map(|n| n.0)) {
            let x = x + going.chars().count() as u16;
            let look = format!("   ◇ {}/{} {} · Enter goes, Esc stays", b + 1, total, step_name(&set.steps[b]));
            buf.set_string(x, area.y, &look, Style::new().fg(TEXT));
        }
    }

    fn spectrogram(&self, buf: &mut Buffer, area: Rect) {
        if area.height < 2 || area.width < 8 {
            return;
        }
        // The last pixel row is left empty: a line one pixel high between
        // the mix and the lanes, so the two read as separate pictures. A
        // glass cell is one pixel, so there it is the whole last row.
        let rows = if self.glass { (area.height as usize - 1) * 2 } else { area.height as usize * 2 - 1 };
        let label_w = 5u16;
        let w = area.width.saturating_sub(label_w) as usize;
        let t = |db: f32| (db - SPEC_FLOOR_DB) / (SPEC_TOP_DB - SPEC_FLOOR_DB);
        for x in 0..w {
            let col = (self.spectrum.len() + x).checked_sub(w).and_then(|i| self.spectrum.get(i));
            for y in 0..area.height as usize {
                let sample = |py: usize| {
                    let Some(col) = col else { return 0.0 };
                    if py >= rows {
                        return 0.0;
                    }
                    // Top of the screen is the highest frequency.
                    let pos = 1.0 - (py as f32 + 0.5) / rows as f32;
                    let fbin = pos * (SPEC_BINS - 1) as f32;
                    let (i, fr) = (fbin as usize, fbin.fract());
                    let v = col[i] + (col[(i + 1).min(SPEC_BINS - 1)] - col[i]) * fr;
                    t(v)
                };
                let (top, bottom) = (sample(y * 2), sample(y * 2 + 1));
                if let Some(c) = buf.cell_mut((area.x + label_w + x as u16, area.y + y as u16)) {
                    pixels(c, top, bottom, CLEAR_MIX, self.glass);
                }
            }
        }
        // Bar lines, faint, with the bar's number at the top: the pictures'
        // ruler, so the scroll reads in bars rather than seconds.
        for x in 0..w {
            let Some(Some(bar)) = (self.bar_marks.len() + x).checked_sub(w).and_then(|i| self.bar_marks.get(i)) else {
                continue;
            };
            for y in 0..area.height {
                if let Some(c) = buf.cell_mut((area.x + label_w + x as u16, area.y + y)) {
                    let (fg, bg) = (lighten(c.fg), lighten(c.bg));
                    c.set_fg(fg).set_bg(bg);
                }
            }
            let label = format!("{}", bar + 1);
            if x + label.len() < w {
                buf.set_string(area.x + label_w + x as u16 + 1, area.y, &label, Style::new().fg(DIM));
            }
        }
        // The guide lines of the pictures: 100 Hz, 1 kHz, 10 kHz.
        for (hz, label) in [(100.0, "100"), (1000.0, "1k"), (10_000.0, "10k")] {
            let pos = ((hz as f32).ln() - SPEC_LOW_HZ.ln()) / (SPEC_HIGH_HZ.ln() - SPEC_LOW_HZ.ln());
            let y = ((1.0 - pos) * area.height as f32) as u16;
            if y < area.height {
                buf.set_string(area.x, area.y + y, format!("{:>4}", label), Style::new().fg(DIM).bg(BG));
                for x in 0..w as u16 {
                    if let Some(c) = buf.cell_mut((area.x + label_w + x, area.y + y)) {
                        if c.symbol() == "▀" && x % 2 == 0 {
                            c.set_char('▔');
                        }
                    }
                }
            }
        }
        buf.set_string(area.x, area.y, " mix", Style::new().fg(GOLD).bg(BG));
    }

    fn lanes(&self, buf: &mut Buffer, area: Rect, active: &[usize]) {
        let Some(song) = self.song.as_ref() else { return };
        if area.height == 0 {
            return;
        }
        // The last row names the tracks that are loaded and silent.
        let quiet: Vec<&str> = (0..song.tracks.len().min(self.lanes.len()))
            .filter(|i| !active.contains(i))
            .map(|i| song.tracks[i].as_str())
            .collect();
        let rows = area.height as usize - (!quiet.is_empty()) as usize;
        if !quiet.is_empty() {
            let line = format!(" silent  {}", quiet.join(" · "));
            let line: String = line.chars().take(area.width as usize).collect();
            buf.set_string(area.x, area.y + rows as u16, line, Style::new().fg(DIM).bg(BG));
        }
        let n = active.len();
        if n == 0 || rows == 0 {
            return;
        }
        let lane_h = (rows / n).clamp(1, 2);
        let label_w = 16u16;
        let w = area.width.saturating_sub(label_w) as usize;
        let t = |db: f32| (db - LANE_FLOOR_DB) / (LANE_TOP_DB - LANE_FLOOR_DB);
        for (row, &i) in active.iter().enumerate() {
            let Some(name) = song.tracks.get(i) else { continue };
            let y0 = area.y + (row * lane_h) as u16;
            if y0 + lane_h as u16 > area.y + rows as u16 {
                break;
            }
            let hist = &self.lanes[i];
            let notes = &self.lane_notes[i];
            let drum = song.drums.get(i).copied().unwrap_or(false);
            let rows = lane_h * 2;
            for x in 0..w {
                let col = (hist.len() + x).checked_sub(w).and_then(|j| hist.get(j));
                let note = (notes.len() + x).checked_sub(w).and_then(|j| notes.get(j)).copied().flatten();
                for y in 0..lane_h {
                    let pix = |py: usize| {
                        let Some(col) = col else { return 0.0 };
                        // Top pixel is air, bottom is sub.
                        let band = ((rows - 1 - py) * BANDS) / rows;
                        t(col[band])
                    };
                    if let Some(c) = buf.cell_mut((area.x + label_w + x as u16, y0 + y as u16)) {
                        pixels_with(c, pix(y * 2), pix(y * 2 + 1), CLEAR_LANE, self.glass, |t| {
                            lane_colour(t, note, drum)
                        });
                    }
                }
            }
            // The spectrogram's bar lines carry on down through the lanes.
            for x in 0..w {
                if let Some(Some(_)) = (self.bar_marks.len() + x).checked_sub(w).and_then(|j| self.bar_marks.get(j)) {
                    for y in 0..lane_h {
                        if let Some(c) = buf.cell_mut((area.x + label_w + x as u16, y0 + y as u16)) {
                            let (fg, bg) = (faint(c.fg), faint(c.bg));
                            c.set_fg(fg).set_bg(bg);
                        }
                    }
                }
            }
            // Name and a level meter in the gutter.
            let peak_db = 20.0 * (self.lane_peak[i] + 1e-9).log10();
            let lvl = ((peak_db + 48.0) / 48.0).clamp(0.0, 1.0);
            let active = lvl > 0.05;
            let label: String = name.chars().take(10).collect();
            let selected = self.selected == Some(i) || self.marked.contains(&i);
            buf.set_string(
                area.x + 1,
                y0,
                format!("{:<10}", label),
                Style::new()
                    .fg(if selected {
                        GOLD
                    } else if active {
                        TEXT
                    } else {
                        DIM
                    })
                    .bg(BG)
                    .add_modifier(if selected { Modifier::BOLD } else { Modifier::empty() }),
            );
            if selected {
                buf.set_string(area.x, y0, "▶", Style::new().fg(GOLD).bg(BG));
            }
            // M on a muted lane, S on the soloed one, where the meter was.
            let is_muted = self.muted.get(i).copied().unwrap_or(false);
            let is_solo = self.soloed.contains(name);
            if is_solo {
                buf.set_string(
                    area.x + 11,
                    y0,
                    " S  ",
                    Style::new().fg(Color::Black).bg(GOLD).add_modifier(Modifier::BOLD),
                );
            } else if is_muted {
                buf.set_string(
                    area.x + 11,
                    y0,
                    " M  ",
                    Style::new().fg(Color::Black).bg(ERR).add_modifier(Modifier::BOLD),
                );
            }
            // What the `play` line does to the pattern, lit on the loops it
            // changes: the second row of a lane, or a mark when there is one.
            let play = song.plays.get(i).map(String::as_str).unwrap_or("");
            let tail = play.split_once(' ').map(|(_, t)| t).unwrap_or("");
            if !tail.is_empty() {
                let on = self.transformed.get(i).copied().unwrap_or(false);
                let style = Style::new().fg(if on { HOT } else { DIM }).bg(BG);
                if lane_h >= 2 {
                    let t: String = tail.chars().take(label_w as usize - 2).collect();
                    buf.set_string(area.x + 1, y0 + 1, format!("⟲ {}", t), style);
                } else if !selected {
                    buf.set_string(area.x, y0, "⟲", style);
                }
            }
            let cells = if is_muted || is_solo { 0 } else { 4usize };
            let lit = (lvl * cells as f32).round() as usize;
            for k in 0..cells {
                let color = if k < lit { inferno(0.45 + 0.55 * (k as f32 / cells as f32)) } else { PANEL };
                if let Some(c) = buf.cell_mut((area.x + 11 + k as u16, y0)) {
                    c.set_char('▮').set_fg(color).set_bg(BG);
                }
            }
        }
    }

    fn bottom(&self, f: &mut Frame, area: Rect) {
        // The knobs always; beside them the error when the file has one,
        // else the chosen track's pattern. The log is behind `l`.
        let side = self.error.is_some() || self.selected.is_some();
        let [knobs, right] = if side {
            Layout::horizontal([Constraint::Percentage(35), Constraint::Percentage(65)]).areas(area)
        } else {
            [area, Rect::default()]
        };
        let block = |title: &str, color: Color| {
            Block::new()
                .borders(Borders::TOP)
                .border_style(Style::new().fg(PANEL))
                .title(Span::styled(format!(" {} ", title), Style::new().fg(color)))
                .style(Style::new().bg(BG))
        };
        let mut lines: Vec<Line> = Vec::new();
        for (at, reading) in self.knobs.iter().rev() {
            let age = at.elapsed().as_secs_f32();
            let color = if age < 1.5 { GOLD } else { DIM };
            lines.push(Line::styled(format!(" {}", reading), Style::new().fg(color)));
        }
        if lines.is_empty() {
            let midi = if self.midi.is_empty() { "no midi input".to_string() } else { self.midi.join(", ") };
            lines.push(Line::styled(format!(" {}", midi), Style::new().fg(DIM)));
        }
        f.render_widget(Paragraph::new(lines).block(block("knobs", HOT)), knobs);

        if let Some(err) = &self.error {
            let lines: Vec<Line> = err.iter().map(|l| Line::styled(format!(" {}", l), Style::new().fg(ERR))).collect();
            f.render_widget(
                Paragraph::new(lines).block(block("error — still playing the last good version", ERR)),
                right,
            );
            return;
        }
        self.grid(f, right);
    }
}

impl Screen {
    /// The screen drawn into a buffer of `width` x `height` cells.
    pub fn snapshot(&self, width: u16, height: u16) -> Buffer {
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).expect("test backend");
        term.draw(|f| self.render(f)).expect("test backend");
        term.backend().buffer().clone()
    }

    /// For a picture: step `n` looked at, as ← → would leave it.
    pub fn look_at(&mut self, n: usize) {
        self.browse = Some(n.saturating_sub(1));
    }

    /// For a picture: the session log open, as `l` would.
    pub fn open_log(&mut self) {
        self.log_open = true;
    }

    /// For a picture: the step list open at step `n`, as `g` and typing would.
    pub fn open_picker(&mut self, n: usize) {
        self.picker = Some(Picker { cursor: n.saturating_sub(1), typed: n.to_string() });
    }

    /// For a picture: select a track by name and open the help, as the keys
    /// would, and take the transform marks from the telemetry.
    pub fn draw_offline(&mut self, select: Option<&str>, help: bool) {
        if let (Some(name), Some(song)) = (select, self.song.as_ref()) {
            self.selected = song.tracks.iter().position(|t| t == name);
        }
        self.help = help;
        let n = self.lanes.len().min(self.telemetry.tracks());
        self.transformed = (0..n).map(|i| self.telemetry.transformed(i)).collect();
        self.muted = (0..n).map(|i| self.telemetry.muted(i)).collect();
        self.follow();
    }

    pub fn column_period() -> Duration {
        COLUMN
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        if self.term.is_some() {
            ratatui::restore();
        }
    }
}

/// `13-respiro.synth` as `respiro`: the number is already on the chip.
fn step_name(file: &str) -> &str {
    let stem = file.strip_suffix(".synth").unwrap_or(file);
    match stem.split_once('-') {
        Some((n, rest)) if n.chars().all(|c| c.is_ascii_digit()) => rest,
        _ => stem,
    }
}

/// Below this a pixel is left to the terminal's own background: a
/// transparent terminal stays transparent wherever nothing sounds, and only
/// the sound is painted. The mix is always full of quiet energy, so it keeps
/// only what has body; a lane is silent between its notes and keeps it all.
const CLEAR_MIX: f32 = 0.3;
const CLEAR_LANE: f32 = 0.1;

/// Two stacked pixels in one cell: the upper half block carries the top one
/// in its foreground and the cell's background the bottom one. A silent half
/// is not painted, so when only the bottom one sounds the lower half block is
/// used instead and the top stays clear.
fn pixels(c: &mut ratatui::buffer::Cell, top: f32, bottom: f32, clear: f32, glass: bool) {
    pixels_with(c, top, bottom, clear, glass, inferno);
}

/// `pixels` with the colour of a level chosen by `colour`.
fn pixels_with(
    c: &mut ratatui::buffer::Cell,
    top: f32,
    bottom: f32,
    clear: f32,
    glass: bool,
    colour: impl Fn(f32) -> Color,
) {
    if glass {
        // One pixel a cell, the louder of the two, all of it background.
        let v = top.max(bottom);
        let bg = if v > clear { colour(v) } else { Color::Reset };
        c.set_char(' ').set_fg(Color::Reset).set_bg(bg);
        return;
    }
    match (top > clear, bottom > clear) {
        (true, true) => c.set_char('▀').set_fg(colour(top)).set_bg(colour(bottom)),
        (true, false) => c.set_char('▀').set_fg(colour(top)).set_bg(Color::Reset),
        (false, true) => c.set_char('▄').set_fg(colour(bottom)).set_bg(Color::Reset),
        (false, false) => c.set_char(' ').set_fg(Color::Reset).set_bg(Color::Reset),
    };
}

/// A lane's colour: the hue of the note it plays -- the twelve notes around
/// the colour wheel, C red, E yellow-green, G# blue-violet -- and the level
/// as brightness. A drum, which has no note, is a cool cyan; a track that has
/// not played a note yet, grey. The mix above keeps the heat colours, so the
/// two are never read as one picture.
fn lane_colour(t: f32, note: Option<u8>, drum: bool) -> Color {
    let v = t.clamp(0.0, 1.0).powf(0.8);
    let (hue, sat) = match (drum, note) {
        (true, _) => (190.0, 0.45),
        (false, Some(n)) => ((n % 12) as f32 * 30.0, 0.75),
        (false, None) => (0.0, 0.0),
    };
    // A bright level washes towards white, the way the heat ramp tops out.
    let sat = sat * (1.0 - 0.35 * (v - 0.75).max(0.0) / 0.25);
    palette::hsv(hue, sat, v)
}

/// A bar line over a cell: a quarter of the way to white. A clear cell stays
/// clear -- a line painted there is a dark stripe across the glass -- and the
/// bar's number at the top still marks it.
fn lighten(c: Color) -> Color {
    match c {
        Color::Rgb(r, g, b) => {
            let up = |v: u8| v + (255 - v) / 4;
            Color::Rgb(up(r), up(g), up(b))
        }
        other => other,
    }
}

/// A bar line through a lane: an eighth of the way, so an empty lane is not
/// crossed by a grey bar.
fn faint(c: Color) -> Color {
    match c {
        Color::Rgb(r, g, b) => {
            let up = |v: u8| v + (255 - v) / 10;
            Color::Rgb(up(r), up(g), up(b))
        }
        other => other,
    }
}

fn log_hz(pos: f32) -> f32 {
    (SPEC_LOW_HZ.ln() + pos * (SPEC_HIGH_HZ.ln() - SPEC_LOW_HZ.ln())).exp()
}

fn push_capped<T>(q: &mut VecDeque<T>, v: T) {
    if q.len() == HISTORY {
        q.pop_front();
    }
    q.push_back(v);
}

#[cfg(test)]
mod tests {
    use super::*;

    const SONG: &str = r#"
tempo 120
module bass low { cutoff 400hz }
module keys pad { voice_mode poly }
pattern line { A2 ..*15 }
pattern rest { -*16 }
track bass { play line using low }
track pad { play rest using pad }
scene a { track bass { play line using low } track pad { play rest using pad } }
scene b { track bass { play line using low } }
arrange { a x2 b x2 }
"#;

    fn text(buf: &Buffer) -> String {
        let a = buf.area;
        (0..a.height)
            .map(|y| (0..a.width).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn a_frame_shows_the_song_its_scenes_and_what_plays() {
        let telemetry = Arc::new(Telemetry::new());
        let mut screen = Screen::headless(Arc::clone(&telemetry), "test".into(), vec![]);
        screen.set_song(SongInfo::from_source("demo.synth", SONG).unwrap());
        // A 440 Hz sine in the mix, and the bass track loud in its low band.
        let sine: Vec<f32> = (0..8192)
            .map(|i| (i as f32 * 440.0 / tatum_core::SAMPLE_RATE * std::f32::consts::TAU).sin() * 0.5)
            .collect();
        telemetry.push(&sine, &sine);
        for _ in 0..40 {
            let mut engine = tatum_core::song_engine::SongEngine::from_source(SONG).unwrap();
            engine.set_band_metering(true);
            engine.start();
            engine.render_steps(4);
            telemetry.meter(&mut engine);
            screen.column();
        }
        screen.position(2, 32, 120.0);
        let buf = screen.snapshot(120, 40);
        let shown = text(&buf);
        assert!(shown.contains("TATUM"), "no header:\n{shown}");
        assert!(shown.contains("demo.synth"), "no title");
        assert!(shown.contains("120 BPM"), "no tempo");
        assert!(shown.contains("bar 3"), "no bar");
        // The arrangement: the second scene is playing at bar 3.
        assert!(shown.contains("▸ b"), "the scene playing is not named:\n{shown}");
        // The bass has a lane; the pad, silent the whole time, is listed as such.
        assert!(shown.contains(" bass"), "no lane for the bass:\n{shown}");
        assert!(shown.contains("silent  pad"), "the silent track is not listed:\n{shown}");
        // The spectrogram lit up somewhere: some cell is well above black.
        let lit = (0..buf.area.height).any(|y| {
            (0..buf.area.width).any(|x| {
                matches!(buf[(x, y)].fg, Color::Rgb(r, _, _) if r > 200) && matches!(buf[(x, y)].symbol(), "▀" | "▄")
            })
        });
        assert!(lit, "nothing in the spectrogram is lit");
    }

    #[test]
    fn step_names_lose_their_number_and_extension() {
        assert_eq!(step_name("13-respiro.synth"), "respiro");
        assert_eq!(step_name("intro.synth"), "intro");
        assert_eq!(step_name("dub-techno"), "dub-techno");
    }
}
