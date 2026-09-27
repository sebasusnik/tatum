//! Patterns: notes, degrees, chords and chord symbols, subdivisions,
//! ratchets, ties and rests, drum lanes, and the parameter locks a step can
//! carry.

use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;

use crate::dsl::ast::*;
use crate::math;
use crate::dsl::error::ParseError;
use crate::dsl::lexer::Token;

use super::{Parser, describe_token};

impl Parser {
    // ── Pattern ──

    pub(super) fn parse_pattern_or_scene(&mut self, song: &mut Song) {
        // "pattern" can be either a note pattern or a scene definition
        // Peek ahead: if the body contains "track" keywords, it's a scene
        let name = match self.expect_ident() {
            Some(n) => n,
            None => return,
        };
        if !self.expect(&Token::LBrace) { return; }

        // Look ahead to determine if this is a scene or a note pattern
        let saved_pos = self.pos;
        let mut is_scene = false;
        let mut depth = 1;
        let mut scan = self.pos;
        while scan < self.tokens.len() && depth > 0 {
            match &self.tokens[scan].token {
                Token::LBrace => depth += 1,
                Token::RBrace => depth -= 1,
                Token::Track | Token::Extends | Token::Tempo | Token::Auto => { is_scene = true; break; }
                _ => {}
            }
            scan += 1;
        }
        self.pos = saved_pos;

        if is_scene {
            self.parse_scene_body(name, &mut song.scenes);
        } else {
            self.parse_pattern_body(name, &mut song.patterns);
        }
    }

    fn parse_pattern_body(&mut self, name: String, patterns: &mut Vec<PatternDef>) {
        let mut rows: Vec<Vec<Step>> = Vec::new();
        let mut lane_labels: Vec<String> = Vec::new();
        let mut current_row: Vec<Step> = Vec::new();
        let mut first_row = true;
        let mut labeled_mode = false;
        let mut pending_slide = false;

        // Skip leading newlines only
        self.skip_newlines();

        loop {
            if self.at_block_end() {
                if !current_row.is_empty() {
                    rows.push(current_row);
                }
                break;
            }

            // At the start of a new row, check for labeled drum lane: ident ":"
            if current_row.is_empty() {
                if let Token::Ident(ref label) = self.peek().clone() {
                    let saved_pos = self.pos;
                    let label = label.clone();
                    self.advance();
                    // `Fm9:0.6` at row start is a chord with velocity, not a drum lane
                    let is_chord = crate::dsl::chords::parse(&label).is_some();
                    if matches!(self.peek(), Token::Colon) && !is_chord {
                        self.advance(); // consume ':'
                        if first_row {
                            labeled_mode = true;
                        }
                        if labeled_mode {
                            lane_labels.push(label);
                        } else {
                            // Mixed mode: first row wasn't labeled but this one is — error
                            let s = self.span();
                            self.errors.push(ParseError {
                                line: s.line, col: s.col,
                                message: format!("pattern '{}': mixed labeled/unlabeled rows", name),
                            });
                        }
                        first_row = false;
                        // Continue parsing the rest of this row normally
                        continue;
                    } else {
                        // Not a label — restore position
                        self.pos = saved_pos;
                    }
                }
                if first_row {
                    // First row is not labeled
                    first_row = false;
                }
            }

            match self.peek().clone() {
                // Slide marker: `~1.5` glides into the note instead of retriggering
                Token::Tilde => {
                    self.advance();
                    pending_slide = true;
                }
                // Chord: [A3 C4 E4]:0.35(plock)
                Token::LBracket => {
                    self.advance(); // consume [
                    let mut notes = Vec::new();
                    loop {
                        match self.peek().clone() {
                            Token::RBracket => { self.advance(); break; }
                            Token::Eof => break,
                            Token::Note(ref n) => {
                                let n = n.clone();
                                self.check_note_range(&n);
                                self.advance();
                                let vel = if matches!(self.peek(), Token::Colon) {
                                    self.advance();
                                    self.expect_number()
                                } else {
                                    None
                                };
                                notes.push(NoteStep {
                                    note: NoteRef::Absolute(n),
                                    velocity: vel,
                                    plock: PLock::default(),
                                    slide: false,
                                });
                            }
                            Token::Number(v) => {
                                self.advance();
                                let degree = math::floor(v) as u8;
                                let octave = ((v - math::floor(v)) * 10.0 + 0.5) as u8;
                                if (1..=7).contains(&degree) {
                                    let vel = if matches!(self.peek(), Token::Colon) {
                                        self.advance();
                                        self.expect_number()
                                    } else {
                                        None
                                    };
                                    notes.push(NoteStep {
                                        note: NoteRef::Degree(degree, octave),
                                        velocity: vel,
                                        plock: PLock::default(),
                                        slide: false,
                                    });
                                }
                            }
                            _ => { self.advance(); } // skip unexpected tokens inside chord
                        }
                    }
                    // Optional shared velocity after ]: [A3 C4 E4]:0.35
                    let velocity = if matches!(self.peek(), Token::Colon) {
                        self.advance();
                        self.expect_number()
                    } else {
                        None
                    };
                    // Optional plock after chord
                    let plock = if matches!(self.peek(), Token::LParen) {
                        self.parse_plock_params()
                    } else {
                        PLock::default()
                    };
                    current_row.push(Step::Chord(ChordStep { notes, velocity, plock }));
                }
                // `<B4 C#5 D5>` -- notes in sequence inside one step. The
                // closing `>` lexes as Arrow; a routing chain never appears in
                // a pattern body, so there is nothing to disambiguate.
                Token::LAngle => {
                    self.advance();
                    let mut subs: Vec<NoteStep> = Vec::new();
                    let mut slide_next = false;
                    loop {
                        match self.peek() {
                            Token::Arrow => { self.advance(); break; }
                            Token::RBrace | Token::Eof => break,
                            Token::Tilde => { self.advance(); slide_next = true; }
                            Token::Note(ref nn) => {
                                let nn = nn.clone();
                                self.check_note_range(&nn);
                                self.advance();
                                let velocity = if matches!(self.peek(), Token::Colon) {
                                    self.advance();
                                    self.expect_number()
                                } else { None };
                                subs.push(NoteStep {
                                    note: NoteRef::Absolute(nn),
                                    velocity,
                                    plock: PLock::default(),
                                    slide: core::mem::take(&mut slide_next),
                                });
                            }
                            Token::Number(v) => {
                                let v = *v;
                                self.advance();
                                let degree = math::floor(v) as u8;
                                let octave = ((v - math::floor(v)) * 10.0 + 0.5) as u8;
                                if (1..=7).contains(&degree) {
                                    let velocity = if matches!(self.peek(), Token::Colon) {
                                        self.advance();
                                        self.expect_number()
                                    } else { None };
                                    subs.push(NoteStep {
                                        note: NoteRef::Degree(degree, octave),
                                        velocity,
                                        plock: PLock::default(),
                                        slide: core::mem::take(&mut slide_next),
                                    });
                                }
                            }
                            _ => { self.advance(); }
                        }
                    }
                    let sp = self.span();
                    if subs.is_empty() {
                        self.errors.push(ParseError { line: sp.line, col: sp.col,
                            message: String::from("subdivision group `<...>` is empty: it needs at least one note") });
                    } else if subs.len() > crate::dsl::compiler::MAX_SUBDIV {
                        self.errors.push(ParseError { line: sp.line, col: sp.col,
                            message: format!("subdivision group has {} notes, the most one step can hold is {}",
                                subs.len(), crate::dsl::compiler::MAX_SUBDIV) });
                    } else {
                        // the group inherits the pending `~` for its first note
                        if core::mem::take(&mut pending_slide) {
                            subs[0].slide = true;
                        }
                        current_row.push(Step::Subdiv(subs));
                    }
                }
                Token::Note(ref n) if note_is_chord_symbol(n) || (self.peek_is_slash_next() && crate::dsl::chords::parse(n).is_some()) => {
                    let n = n.clone();
                    self.advance();
                    if let Some(step) = self.chord_symbol_step(&n) {
                        current_row.push(step);
                    }
                }
                Token::Note(ref n) => {
                    let n = n.clone();
                    self.check_note_range(&n);
                    self.advance();
                    let velocity = if matches!(self.peek(), Token::Colon) {
                        self.advance();
                        self.expect_number()
                    } else {
                        None
                    };
                    let plock = if matches!(self.peek(), Token::LParen) {
                        self.parse_plock_params()
                    } else {
                        PLock::default()
                    };
                    // `A4*3` -- the same note three times inside the step. The
                    // drum lanes already spell a roll this way; this is the same
                    // idea for pitches, desugared into a subdivision group so the
                    // engine only has to know about one thing.
                    let ratchet = self.parse_ratchet_count();
                    let base = NoteStep {
                        note: NoteRef::Absolute(n),
                        velocity,
                        plock,
                        slide: core::mem::take(&mut pending_slide),
                    };
                    if ratchet > 1 {
                        let mut subs = vec![base];
                        for _ in 1..ratchet {
                            subs.push(NoteStep { slide: false, ..subs[0].clone() });
                        }
                        current_row.push(Step::Subdiv(subs));
                    } else {
                        current_row.push(Step::Note(base));
                    }
                }
                // Scale degree: 1.3 = degree 1, octave 3
                Token::Number(v) => {
                    self.advance();
                    let degree = math::floor(v) as u8;
                    let octave = ((v - math::floor(v)) * 10.0 + 0.5) as u8;
                    if (1..=7).contains(&degree) {
                        let velocity = if matches!(self.peek(), Token::Colon) {
                            self.advance();
                            self.expect_number()
                        } else {
                            None
                        };
                        let plock = if matches!(self.peek(), Token::LParen) {
                            self.parse_plock_params()
                        } else {
                            PLock::default()
                        };
                        current_row.push(Step::Note(NoteStep {
                            note: NoteRef::Degree(degree, octave),
                            velocity,
                            plock,
                            slide: core::mem::take(&mut pending_slide),
                        }));
                    }
                }
                Token::DrumHit | Token::DrumAccent | Token::DrumGhost => {
                    let default_vel = match self.peek() {
                        Token::DrumAccent => 1.0,
                        Token::DrumGhost => 0.35,
                        _ => 0.8,  // DrumHit
                    };
                    self.advance();
                    // Optional probability: g?0.4 or x?0.5
                    let probability = if matches!(self.peek(), Token::Question) {
                        self.advance();
                        self.expect_number().unwrap_or(1.0)
                    } else {
                        1.0
                    };
                    // Optional velocity override: x:0.6
                    let velocity = if matches!(self.peek(), Token::Colon) {
                        self.advance();
                        self.expect_number().unwrap_or(default_vel)
                    } else {
                        default_vel
                    };
                    // Optional roll: x*3 (retrigger count within step)
                    let roll = if matches!(self.peek(), Token::Star) {
                        self.advance();
                        let n = self.expect_number().unwrap_or(1.0);
                        (n as u8).max(1)
                    } else {
                        1
                    };
                    let plock = if matches!(self.peek(), Token::LParen) {
                        self.parse_plock_params()
                    } else {
                        PLock::default()
                    };
                    current_row.push(Step::DrumHit(DrumStep { velocity, probability, roll, plock }));
                }
                Token::Rest => {
                    self.advance();
                    let n = self.repeat_count();
                    for _ in 0..n { current_row.push(Step::Rest); }
                }
                Token::Tie => {
                    self.advance();
                    let n = self.repeat_count();
                    for _ in 0..n { current_row.push(Step::Tie); }
                }
                Token::Newline => {
                    // A drum lane may be written across several lines. Ending
                    // the row at the first newline turned the continuation into
                    // an unlabelled row, which the compiler then dropped: the
                    // pattern compiled clean and played half of what was written.
                    let lane_continues = labeled_mode
                        && !current_row.is_empty()
                        && !self.next_line_starts_a_lane()
                        && !matches!(self.peek_past_newlines(), Token::RBrace | Token::Eof);
                    if !current_row.is_empty() && !lane_continues {
                        rows.push(current_row);
                        current_row = Vec::new();
                    }
                    self.advance();
                }
                // Chord symbol: Fm9, Dbmaj7/2, Csus4:0.6
                Token::Ident(ref word) if crate::dsl::chords::parse(word).is_some() => {
                    let word = word.clone();
                    self.advance();
                    if let Some(step) = self.chord_symbol_step(&word) {
                        current_row.push(step);
                    }
                }
                _ => {
                    let s = self.span();
                    let (l, c) = (s.line, s.col);
                    self.errors.push(ParseError {
                        line: l, col: c,
                        message: format!("pattern '{}': unexpected {} (expected a note, degree, chord symbol, `-`, `..` or `~`)", name, describe_token(self.peek())),
                    });
                    self.advance();
                }
            }
        }

        self.expect(&Token::RBrace);
        if !lane_labels.is_empty() && lane_labels.len() != rows.len() {
            let s = self.span();
            self.errors.push(ParseError {
                line: s.line, col: s.col,
                message: format!(
                    "pattern '{}': {} lane labels but {} rows. Every row of a drum pattern needs its own `lane:` label.",
                    name, lane_labels.len(), rows.len()
                ),
            });
        }
        let def = PatternDef { name, rows, lane_labels };
        match patterns.iter().position(|p| p.name == def.name) {
            Some(i) => patterns[i] = def,
            None => patterns.push(def),
        }
    }

    /// Is the token after the current one a `/`? (`E5/3` is a power chord, `E5` a note.)
    fn peek_is_slash_next(&self) -> bool {
        self.tokens.get(self.pos + 1).is_some_and(|s| matches!(s.token, Token::Slash))
    }

    /// After a chord symbol: optional `/octave`, `:velocity`, `(plocks)`.
    fn chord_symbol_step(&mut self, symbol: &str) -> Option<Step> {
        let mut octave = 3u8;
        if matches!(self.peek(), Token::Slash) {
            self.advance();
            match self.peek().clone() {
                Token::Number(n) if (0.0..=8.0).contains(&n) => { self.advance(); octave = n as u8; }
                _ => {
                    let s = self.span();
                    let (l, c) = (s.line, s.col);
                    self.errors.push(ParseError { line: l, col: c, message: format!("chord {}: expected an octave 0..8 after '/'", symbol) });
                }
            }
        }
        let velocity = if matches!(self.peek(), Token::Colon) {
            self.advance();
            self.expect_number()
        } else {
            None
        };
        let plock = if matches!(self.peek(), Token::LParen) { self.parse_plock_params() } else { PLock::default() };
        let midi = crate::dsl::chords::notes(symbol, octave)?;
        let notes = midi.into_iter().map(|m| NoteStep {
            note: NoteRef::Midi(m),
            velocity: None,
            plock: PLock::default(),
            slide: false,
        }).collect();
        Some(Step::Chord(ChordStep { notes, velocity, plock }))
    }

    /// Parse per-step parameter locks: `(cutoff=80, edepth=5000, res=0.8, gate=0.4)`
    fn parse_plock_params(&mut self) -> PLock {
        self.advance(); // consume LParen
        let mut plock = PLock::default();

        loop {
            self.skip_newlines();
            if matches!(self.peek(), Token::RParen | Token::Eof) { break; }

            if let Token::Ident(name) = self.peek().clone() {
                let saved_pos = self.pos;
                self.advance();
                if matches!(self.peek(), Token::Eq) {
                    self.advance();
                    if let Some(val) = self.parse_expr_value() {
                        match name.as_str() {
                            "cutoff" => plock.cutoff = Some(val),
                            "edepth" => plock.env_depth = Some(val),
                            "res" => plock.resonance = Some(val),
                            "gate" => plock.gate = Some(val),
                            other => {
                                let sp = self.tokens[saved_pos].clone();
                                self.errors.push(ParseError {
                                    line: sp.line,
                                    col: sp.col,
                                    message: format!(
                                        "unknown step parameter '{}'. A step lock takes cutoff, edepth, res or gate",
                                        other
                                    ),
                                });
                            }
                        }
                    }
                } else {
                    self.pos = saved_pos;
                    break;
                }
            } else {
                break;
            }

            if matches!(self.peek(), Token::Comma) {
                self.advance();
            }
        }

        self.expect(&Token::RParen);
        plock
    }

    /// A note name past the top of MIDI (G9) is an error at the note;
    /// `C999` used to play as C4.
    fn check_note_range(&mut self, name: &str) {
        if !crate::dsl::compiler::note_in_midi_range(name) {
            let (line, col) = { let s = self.span(); (s.line, s.col) };
            self.errors.push(ParseError {
                line, col,
                message: format!("note {} is above what MIDI can play (G9 is the top)", name),
            });
        }
    }

    /// `*N` after a note: repeat it N times inside the step. Returns 1 when
    /// there is no `*`, and clamps to what one step can hold.
    fn parse_ratchet_count(&mut self) -> usize {
        if !matches!(self.peek(), Token::Star) { return 1; }
        self.advance();
        match self.peek() {
            Token::Number(v) => {
                let n = *v as usize;
                self.advance();
                n.clamp(1, crate::dsl::compiler::MAX_SUBDIV)
            }
            _ => 1,
        }
    }

    /// The next token, looking past newlines (without consuming them).
    /// In a drum pattern, does a new lane start after the newlines ahead?
    /// `hat:` begins one; anything else is the current lane continuing onto
    /// another line.
    fn next_line_starts_a_lane(&self) -> bool {
        let mut i = self.pos;
        while i < self.tokens.len() && matches!(self.tokens[i].token, Token::Newline) {
            i += 1;
        }
        if i + 1 >= self.tokens.len() {
            return false;
        }
        matches!(self.tokens[i].token, Token::Ident(_)) && matches!(self.tokens[i + 1].token, Token::Colon)
    }
}

/// `C7`, `Db9`, `F#11`: a note token whose "octave" is 7 or more is read as a
/// chord symbol (real notes above octave 6 are not used in patterns).
fn note_is_chord_symbol(token: &str) -> bool {
    let digits: String = token.chars().skip_while(|c| !c.is_ascii_digit()).collect();
    match digits.parse::<u32>() {
        Ok(n) => n >= 7 && crate::dsl::chords::parse(token).is_some(),
        Err(_) => false,
    }
}
