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

use palette::{inferno, BG, DIM, ERR, GOLD, HOT, OK, PANEL, TEXT};
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Quit,
    Next,
    Prev,
    Step(usize),
}

#[derive(Clone, Copy)]
pub enum Tone {
    Info,
    Good,
    Bad,
}

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
    lane_peak: Vec<f32>,
    last_column: Instant,
    log: VecDeque<(f32, String, Tone)>,
    knobs: VecDeque<(Instant, String)>,
    error: Option<Vec<String>>,
    started: Instant,
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
            lane_peak: Vec::new(),
            last_column: Instant::now(),
            log: VecDeque::new(),
            knobs: VecDeque::new(),
            error: None,
            started: Instant::now(),
            bar: 0,
            step: 0,
            tempo: 0.0,
            finished: false,
        }
    }

    pub fn set_song(&mut self, song: SongInfo) {
        if self.song.as_ref().map(|s| &s.tracks) != Some(&song.tracks) {
            self.lanes = vec![VecDeque::with_capacity(HISTORY); song.tracks.len()];
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
            let key = match k.code {
                KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => Some(Key::Quit),
                KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => Some(Key::Quit),
                KeyCode::Char(' ') | KeyCode::Right | KeyCode::Char('n') => Some(Key::Next),
                KeyCode::Left | KeyCode::Char('p') => Some(Key::Prev),
                KeyCode::Char(c @ '1'..='9') => Some(Key::Step(c as usize - '0' as usize)),
                _ => None,
            };
            out.extend(key);
        }
        out
    }

    /// Read the telemetry and draw a frame.
    pub fn draw(&mut self) -> io::Result<()> {
        self.analyse();
        let Some(mut term) = self.term.take() else { return Ok(()) };
        let result = term.draw(|f| self.render(f)).map(|_| ());
        self.term = Some(term);
        result
    }

    fn analyse(&mut self) {
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
        let hint = if self.set.is_some() {
            " space/→ next · ← back · 1-9 jump to a step · q quit · save the step to re-evaluate"
        } else {
            " save the file to re-evaluate it · q quit"
        };
        f.render_widget(Paragraph::new(hint).style(Style::new().fg(DIM).bg(BG)), footer);
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
                let hist = &self.lanes[i];
                hist.iter().rev().take(RECENT).any(|col| col.iter().any(|&db| db > LANE_FLOOR_DB + 6.0))
            })
            .collect()
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
        let mut x = area.x + 1;
        // Scroll so the step playing is on screen.
        let first = set.current.saturating_sub(3);
        for (i, name) in set.steps.iter().enumerate().skip(first) {
            let label = format!(" {} {} ", i + 1, step_name(name));
            let w = label.chars().count() as u16;
            if x + w > area.x + area.width {
                break;
            }
            let queued = set.next.map(|n| n.0) == Some(i);
            let style = if i == set.current {
                Style::new().fg(Color::Black).bg(GOLD).add_modifier(Modifier::BOLD)
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
            buf.set_string(x, area.y + 1, &label, style);
            x += w + 1;
        }
        let status = match set.next {
            Some((n, bars)) => format!(
                " ▸ {} lands in {} bar{}  (phrase {})",
                step_name(&set.steps[n]),
                bars,
                if bars == 1 { "" } else { "s" },
                set.phrase
            ),
            None => format!(" step {} of {} · phrase {} bars", set.current + 1, set.steps.len(), set.phrase),
        };
        buf.set_string(area.x, area.y, &status, Style::new().fg(if set.next.is_some() { HOT } else { DIM }));
    }

    fn spectrogram(&self, buf: &mut Buffer, area: Rect) {
        if area.height < 2 || area.width < 8 {
            return;
        }
        let rows = area.height as usize * 2;
        let label_w = 5u16;
        let w = area.width.saturating_sub(label_w) as usize;
        let t = |db: f32| (db - SPEC_FLOOR_DB) / (SPEC_TOP_DB - SPEC_FLOOR_DB);
        for x in 0..w {
            let col = (self.spectrum.len() + x).checked_sub(w).and_then(|i| self.spectrum.get(i));
            for y in 0..area.height as usize {
                let sample = |py: usize| {
                    let Some(col) = col else { return 0.0 };
                    // Top of the screen is the highest frequency.
                    let pos = 1.0 - (py as f32 + 0.5) / rows as f32;
                    let fbin = pos * (SPEC_BINS - 1) as f32;
                    let (i, fr) = (fbin as usize, fbin.fract());
                    let v = col[i] + (col[(i + 1).min(SPEC_BINS - 1)] - col[i]) * fr;
                    t(v)
                };
                let (top, bottom) = (sample(y * 2), sample(y * 2 + 1));
                if let Some(c) = buf.cell_mut((area.x + label_w + x as u16, area.y + y as u16)) {
                    pixels(c, top, bottom, CLEAR_MIX);
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
            let rows = lane_h * 2;
            for x in 0..w {
                let col = (hist.len() + x).checked_sub(w).and_then(|j| hist.get(j));
                for y in 0..lane_h {
                    let pix = |py: usize| {
                        let Some(col) = col else { return 0.0 };
                        // Top pixel is air, bottom is sub.
                        let band = ((rows - 1 - py) * BANDS) / rows;
                        t(col[band])
                    };
                    if let Some(c) = buf.cell_mut((area.x + label_w + x as u16, y0 + y as u16)) {
                        pixels(c, pix(y * 2), pix(y * 2 + 1), CLEAR_LANE);
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
            buf.set_string(
                area.x + 1,
                y0,
                format!("{:<10}", label),
                Style::new().fg(if active { TEXT } else { DIM }).bg(BG),
            );
            let cells = 4usize;
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
        let [knobs, log] = Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)]).areas(area);
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
                log,
            );
            return;
        }
        let height = log.height.saturating_sub(1) as usize;
        let lines: Vec<Line> = self
            .log
            .iter()
            .rev()
            .take(height)
            .rev()
            .map(|(t, line, tone)| {
                let color = match tone {
                    Tone::Info => TEXT,
                    Tone::Good => OK,
                    Tone::Bad => ERR,
                };
                Line::from(vec![
                    Span::styled(format!(" {:>4}:{:02} ", *t as u32 / 60, *t as u32 % 60), Style::new().fg(DIM)),
                    Span::styled(line.clone(), Style::new().fg(color)),
                ])
            })
            .collect();
        f.render_widget(Paragraph::new(lines).block(block("session", GOLD)), log);
    }
}

impl Screen {
    /// The screen drawn into a buffer of `width` x `height` cells.
    pub fn snapshot(&self, width: u16, height: u16) -> Buffer {
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).expect("test backend");
        term.draw(|f| self.render(f)).expect("test backend");
        term.backend().buffer().clone()
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
fn pixels(c: &mut ratatui::buffer::Cell, top: f32, bottom: f32, clear: f32) {
    match (top > clear, bottom > clear) {
        (true, true) => c.set_char('▀').set_fg(inferno(top)).set_bg(inferno(bottom)),
        (true, false) => c.set_char('▀').set_fg(inferno(top)).set_bg(Color::Reset),
        (false, true) => c.set_char('▄').set_fg(inferno(bottom)).set_bg(Color::Reset),
        (false, false) => c.set_char(' ').set_fg(Color::Reset).set_bg(Color::Reset),
    };
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
