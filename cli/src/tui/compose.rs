//! The compose screen: the loop's rows -- the groove and a voice per role --
//! and the knobs of the row chosen, over the live screen's own spectrogram
//! and lanes. All it knows is the `View` the session hands it each frame;
//! every key that changes the loop goes back to the session as an `Action`.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use super::palette::{BG, DIM, ERR, GOLD, HOT, KEYCAP, OK, PANEL, TEXT};
use super::{Key, Screen};

/// One row of the loop: the groove, or a role and the voice it holds.
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
}

/// One knob of the chosen row.
pub struct MacroRow {
    pub label: String,
    pub position: u8,
    pub reading: String,
    pub held: bool,
}

pub struct View {
    pub rows: Vec<Row>,
    pub macros: Vec<MacroRow>,
    pub comparing: bool,
    pub knob_ccs: Vec<u8>,
}

/// What a key asks of the session.
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
}

#[derive(Clone, Copy, PartialEq)]
enum Pane {
    Rows,
    Knobs,
}

/// Where the hands are on the compose screen.
pub struct Panel {
    pub view: View,
    /// The row chosen: 0 is the groove.
    pub focus: usize,
    pane: Pane,
    knob: usize,
    /// `s`: the name being typed for the voice to save.
    prompt: Option<String>,
    pub help: bool,
}

/// Cells of a knob's bar.
const BAR: usize = 20;

impl Panel {
    pub fn new(view: View) -> Self {
        let focus = 1.min(view.rows.len().saturating_sub(1));
        Self { view, focus, pane: Pane::Rows, knob: 0, prompt: None, help: false }
    }

    /// For a picture: the knobs pane on knob `n`.
    pub fn on_knob(&mut self, n: usize) {
        self.pane = Pane::Knobs;
        self.knob = n.min(self.view.macros.len().saturating_sub(1));
    }

    /// Rows the panel needs on the screen.
    pub fn height(&self) -> u16 {
        (1 + self.view.rows.len() + 1 + self.view.macros.len().max(1) + 1) as u16
    }

    /// A key while the panel is up: `Some` when it took it, with what it
    /// asks for. A few keys are left to the screen: quit, the log, MIDI in.
    pub fn key(&mut self, k: KeyEvent) -> Option<Option<Key>> {
        if let Some(name) = self.prompt.as_mut() {
            match k.code {
                KeyCode::Esc => self.prompt = None,
                KeyCode::Enter => {
                    let name = self.prompt.take().unwrap_or_default();
                    return Some(Some(Key::Compose(Action::Save { row: self.focus, name })));
                }
                KeyCode::Backspace => {
                    name.pop();
                }
                KeyCode::Char(c) if c.is_ascii_alphanumeric() || c == '_' || c == '-' => name.push(c),
                _ => {}
            }
            return Some(None);
        }
        if self.help {
            if matches!(k.code, KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')) {
                self.help = false;
            }
            return Some(None);
        }
        let rows = self.view.rows.len();
        let knobs = self.view.macros.len();
        let shift = k.modifiers.contains(KeyModifiers::SHIFT);
        let key = match k.code {
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => return None,
            KeyCode::Char('q') | KeyCode::Char('j') | KeyCode::Char('i') | KeyCode::Esc => return None,
            KeyCode::Char('?') => {
                self.help = true;
                None
            }
            KeyCode::Tab => {
                self.pane = if self.pane == Pane::Rows { Pane::Knobs } else { Pane::Rows };
                None
            }
            KeyCode::Up | KeyCode::Down => {
                let by: i64 = if k.code == KeyCode::Up { -1 } else { 1 };
                match self.pane {
                    Pane::Rows if rows > 0 => {
                        self.focus = (self.focus as i64 + by).rem_euclid(rows as i64) as usize;
                        self.knob = 0;
                    }
                    Pane::Knobs if knobs > 0 => self.knob = (self.knob as i64 + by).rem_euclid(knobs as i64) as usize,
                    _ => {}
                }
                None
            }
            KeyCode::Left | KeyCode::Right => {
                let by: i32 = if k.code == KeyCode::Left { -1 } else { 1 };
                match self.pane {
                    Pane::Rows => Some(Key::Compose(Action::Choose { row: self.focus, by })),
                    Pane::Knobs => self.turn(by * if shift { 1 } else { 4 }),
                }
            }
            KeyCode::PageUp => self.turn(16),
            KeyCode::PageDown => self.turn(-16),
            KeyCode::Home => self.turn_to(0),
            KeyCode::End => self.turn_to(127),
            KeyCode::Char(c @ '1'..='9') => {
                let n = c as usize - '1' as usize;
                if n < knobs {
                    self.knob = n;
                    self.pane = Pane::Knobs;
                }
                None
            }
            KeyCode::Char('m') => Some(Key::Compose(Action::Mute(self.focus))),
            KeyCode::Char('x') => Some(Key::Compose(Action::Reset(self.focus))),
            KeyCode::Char('a') => Some(Key::Compose(Action::Compare)),
            KeyCode::Char('s') => {
                if let Some(name) = self.view.rows.get(self.focus).and_then(|r| r.name.clone()) {
                    self.prompt = Some(name);
                }
                None
            }
            _ => None,
        };
        Some(key)
    }

    /// The knob under the cursor moved `by` steps of its travel.
    fn turn(&mut self, by: i32) -> Option<Key> {
        let m = self.view.macros.get(self.knob)?;
        self.turn_to((m.position as i32 + by).clamp(0, 127) as u8)
    }

    fn turn_to(&mut self, value: u8) -> Option<Key> {
        self.pane = Pane::Knobs;
        self.view.macros.get(self.knob)?;
        Some(Key::Compose(Action::Turn { row: self.focus, knob: self.knob, value }))
    }

    pub fn render(&self, screen: &Screen, f: &mut Frame, area: Rect) {
        let w = area.width as usize;
        let mut lines: Vec<Line> = Vec::new();
        let ccs: Vec<String> = self.view.knob_ccs.iter().map(|c| c.to_string()).collect();
        let title = " LOOP".to_string();
        let right = format!("knobs 1-{} on cc {} ", ccs.len(), ccs.join(" "));
        lines.push(Line::from(vec![
            Span::styled(title.clone(), Style::new().fg(HOT).add_modifier(Modifier::BOLD)),
            Span::raw(" ".repeat(w.saturating_sub(title.chars().count() + right.chars().count()))),
            Span::styled(right, Style::new().fg(DIM)),
        ]));
        for (i, row) in self.view.rows.iter().enumerate() {
            let here = i == self.focus;
            let on = here && self.pane == Pane::Rows;
            let base = Style::new().bg(if on { GOLD } else { BG });
            let fg = |c: Color| base.fg(if on { Color::Black } else { c });
            let muted = screen.muted_track(&row.track);
            let name = row.name.clone().unwrap_or_else(|| "—".into());
            let which = match row.index {
                Some(i) => format!("{}/{}", i, row.count),
                None => String::new(),
            };
            let mut spans = vec![
                Span::styled(if here { " > " } else { "   " }, fg(GOLD)),
                Span::styled(format!("{:<6}", row.role), fg(TEXT).add_modifier(Modifier::BOLD)),
                Span::styled("  < ", fg(DIM)),
                Span::styled(
                    format!("{:<14}", name),
                    if muted {
                        fg(KEYCAP)
                    } else {
                        fg(if row.name.is_some() { HOT } else { DIM }).add_modifier(Modifier::BOLD)
                    },
                ),
                Span::styled(" >  ", fg(DIM)),
            ];
            let tail = if muted {
                "muted".to_string()
            } else if row.name.is_some() {
                format!("{:<6} {}", row.kind, row.doc)
            } else {
                format!("nothing · ← → picks one of {}", row.count)
            };
            let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
            let room = w.saturating_sub(used + 10);
            spans.push(Span::styled(
                format!("{:<room$}", tail.chars().take(room).collect::<String>()),
                fg(if muted { ERR } else { DIM }),
            ));
            spans.push(Span::styled(if row.held { " ● " } else { "   " }, fg(OK)));
            spans.push(Span::styled(format!("{:>6}", which), fg(DIM)));
            lines.push(Line::from(spans));
        }
        let chosen = self.view.rows.get(self.focus);
        let heading = match chosen.and_then(|r| r.name.as_ref().map(|n| (r.role.clone(), n.clone()))) {
            Some((role, name)) => format!(" {} · {} ", role, name),
            None => " nothing chosen ".to_string(),
        };
        let rule = "─".repeat(w.saturating_sub(heading.chars().count() + 2));
        lines.push(Line::from(vec![
            Span::styled(" ─", Style::new().fg(PANEL)),
            Span::styled(heading, Style::new().fg(HOT).add_modifier(Modifier::BOLD)),
            Span::styled(rule, Style::new().fg(PANEL)),
        ]));
        if self.view.macros.is_empty() {
            lines.push(Line::styled("   no knobs: choose a voice first", Style::new().fg(DIM)));
        }
        for (i, m) in self.view.macros.iter().enumerate() {
            let on = self.pane == Pane::Knobs && i == self.knob;
            let base = Style::new().bg(if on { GOLD } else { BG });
            let fg = |c: Color| base.fg(if on { Color::Black } else { c });
            let filled = (m.position as usize * BAR + 63) / 127;
            let bar = format!("{}{}", "█".repeat(filled.min(BAR)), "░".repeat(BAR - filled.min(BAR)));
            let number = if i < 9 { format!(" {} ", i + 1) } else { "   ".to_string() };
            lines.push(Line::from(vec![
                Span::styled(number, fg(if i < self.view.knob_ccs.len() { GOLD } else { DIM })),
                Span::styled(format!("{:<13}", m.label), fg(TEXT).add_modifier(Modifier::BOLD)),
                Span::styled("▐", fg(PANEL)),
                Span::styled(bar, fg(if m.held { OK } else { HOT })),
                Span::styled("▌ ", fg(PANEL)),
                Span::styled(format!("{:<12}", m.reading), fg(TEXT)),
                Span::styled(if m.held { "● turned" } else { "" }, fg(OK)),
            ]));
        }
        let hint = if self.view.comparing {
            Line::styled(" A: hearing the text's values · a goes back to yours", Style::new().fg(HOT))
        } else {
            Line::styled(
                " ↑↓ row · ←→ choose · Tab knobs · ←→ turn · 1-9 a knob · m mute · s save · a compare · x reset · ? keys",
                Style::new().fg(DIM),
            )
        };
        lines.push(hint);
        f.render_widget(Paragraph::new(lines).style(Style::new().bg(BG)), area);
    }

    /// The help and the save prompt, over everything.
    pub fn overlays(&self, f: &mut Frame, area: Rect) {
        if let Some(name) = &self.prompt {
            let role = self.view.rows.get(self.focus).map(|r| r.role.as_str()).unwrap_or("voice");
            let from = self.view.rows.get(self.focus).and_then(|r| r.name.as_deref()).unwrap_or("");
            let w = area.width.clamp(40, 70);
            let pop = Rect { x: area.x + (area.width - w) / 2, y: area.y + area.height / 3, width: w, height: 5 };
            let lines = vec![
                Line::from(vec![
                    Span::styled(format!(" voices/{}/", role), Style::new().fg(DIM)),
                    Span::styled(name.clone(), Style::new().fg(TEXT).add_modifier(Modifier::BOLD)),
                    Span::styled("_", Style::new().fg(GOLD)),
                    Span::styled(".synth", Style::new().fg(DIM)),
                ]),
                Line::styled(
                    format!(" the same name writes over {} · another is a new voice · Enter saves · Esc", from),
                    Style::new().fg(DIM),
                ),
            ];
            f.render_widget(Clear, pop);
            f.render_widget(
                Paragraph::new(lines).block(
                    Block::bordered()
                        .border_style(Style::new().fg(GOLD))
                        .title(Span::styled(" save as it sounds now ", Style::new().fg(GOLD)))
                        .style(Style::new().bg(PANEL)),
                ),
                pop,
            );
        }
        if self.help {
            let keys: &[(&str, &str)] = &[
                ("↑ ↓", "choose a row: the groove, or a role"),
                ("← →", "on a row: the next or previous voice of its library, on the next bar"),
                ("Tab", "the knobs of the chosen row; ↑↓ moves, ← → turns (Shift: a step)"),
                ("1-9", "straight to a knob; PgUp PgDn by 16, Home End to the ends"),
                ("knobs", "the controller's knobs turn knobs 1-8 of the chosen row"),
                ("m", "mute the row's track, or bring it back"),
                ("s", "save the row's voice as it sounds now, under a name"),
                ("a", "A/B: hear every knob at the text's value, then back"),
                ("x", "every knob of the row back to the text"),
                ("j  i", "the session's log; what the controller sends"),
                ("q", "quit; the session file stays as the loop was"),
            ];
            let w = area.width.clamp(50, 90);
            let h = (keys.len() as u16 + 4).min(area.height);
            let pop = Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h };
            let mut lines = vec![Line::styled(
                " a loop to try voices in: pick, turn, save; the file is yours after",
                Style::new().fg(DIM),
            )];
            lines.push(Line::raw(""));
            for (key, what) in keys {
                lines.push(Line::from(vec![
                    Span::styled(format!(" {:<7}", key), Style::new().fg(GOLD).add_modifier(Modifier::BOLD)),
                    Span::styled(*what, Style::new().fg(TEXT)),
                ]));
            }
            f.render_widget(Clear, pop);
            f.render_widget(
                Paragraph::new(lines).block(
                    Block::new()
                        .borders(Borders::ALL)
                        .border_style(Style::new().fg(GOLD))
                        .title(Span::styled(" compose · ? or Esc closes ", Style::new().fg(GOLD)))
                        .style(Style::new().bg(PANEL)),
                ),
                pop,
            );
        }
    }
}
