//! Tokens in, a `Song` out, or every error found on the way.
//!
//! Recursive descent, one method per construct, in an `impl Parser` split
//! across one file per kind of block. Here: the parser itself, the token
//! helpers every block leans on, and the top level that hands each keyword to
//! the block that reads it.

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;

use crate::dsl::ast::*;
use crate::dsl::error::ParseError;
use crate::dsl::lexer::{Token, Span};

mod automation;
mod blocks;
mod globals;
mod instrument;
mod module;
mod pattern;
mod track;

/// Recursive descent parser for .synth files.
pub struct Parser {
    tokens: Vec<Span>,
    pos: usize,
    errors: Vec<ParseError>,
}

impl Parser {
    pub fn new(tokens: Vec<Span>) -> Self {
        Self { tokens, pos: 0, errors: Vec::new() }
    }

    // ── Helpers ──

    fn peek(&self) -> &Token {
        if self.pos < self.tokens.len() {
            &self.tokens[self.pos].token
        } else {
            &Token::Eof
        }
    }

    fn span(&self) -> &Span {
        &self.tokens[self.pos.min(self.tokens.len() - 1)]
    }

    fn advance(&mut self) -> &Token {
        let tok = &self.tokens[self.pos.min(self.tokens.len() - 1)].token;
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
        tok
    }

    fn skip_newlines(&mut self) {
        while matches!(self.peek(), Token::Newline) {
            self.pos += 1;
        }
    }

    fn expect(&mut self, expected: &Token) -> bool {
        self.skip_newlines();
        if core::mem::discriminant(self.peek()) == core::mem::discriminant(expected) {
            self.advance();
            true
        } else {
            let s = self.span();
            self.errors.push(ParseError {
                line: s.line,
                col: s.col,
                message: format!("expected {:?}, got {:?}", expected, self.peek()),
            });
            false
        }
    }

    /// A node label after `as`. Naming an autopan `pan` or a mixer `mix` is the
    /// obvious thing to write, and those are keyword tokens, so a plain
    /// identifier is too narrow here.
    fn expect_node_label(&mut self) -> Option<String> {
        self.skip_newlines();
        let name = match self.peek() {
            Token::Pan => "pan",
            Token::Level => "level",
            Token::Velocity => "velocity",
            _ => return self.expect_ident(),
        };
        self.advance();
        Some(String::from(name))
    }

    fn expect_ident(&mut self) -> Option<String> {
        self.skip_newlines();
        match self.peek().clone() {
            Token::Ident(s) => {
                let s = s.clone();
                self.advance();
                Some(s)
            }
            // Also accept keywords that could be used as identifiers in certain contexts
            Token::In | Token::Out | Token::Mix | Token::Master => {
                let name = match self.peek() {
                    Token::In => String::from("in"),
                    Token::Out => String::from("out"),
                    Token::Mix => String::from("mix"),
                    Token::Master => String::from("master"),
                    _ => unreachable!(),
                };
                self.advance();
                Some(name)
            }
            // `x`, `X` and `o` lex as drum hits but are fine as names outside patterns.
            Token::DrumHit | Token::DrumAccent | Token::DrumGhost(_) => {
                let name = match self.peek() {
                    Token::DrumHit => String::from("x"),
                    Token::DrumAccent => String::from("X"),
                    Token::DrumGhost(c) => String::from(*c),
                    _ => String::from("o"),
                };
                self.advance();
                Some(name)
            }
            _ => {
                let s = self.span();
                self.errors.push(ParseError {
                    line: s.line,
                    col: s.col,
                    message: format!("expected identifier, got {}", describe_token(self.peek())),
                });
                None
            }
        }
    }

    fn expect_number(&mut self) -> Option<f32> {
        self.skip_newlines();
        if let Token::Number(n) = self.peek() {
            let n = *n;
            self.advance();
            Some(n)
        } else if let Token::Quantity(n, suffix) = self.peek().clone() {
            // Do not silently drop the unit: say where units are allowed.
            let s = self.span();
            self.errors.push(ParseError {
                line: s.line, col: s.col,
                message: format!(
                    "'{}{}' has a unit, but this takes a plain number. Units work on module parameters and effect arguments.",
                    n, suffix
                ),
            });
            self.advance();
            Some(n)
        } else {
            let s = self.span();
            self.errors.push(ParseError {
                line: s.line,
                col: s.col,
                message: format!("expected number, got {:?}", self.peek()),
            });
            None
        }
    }

    /// A named argument's value, which may carry a unit (`cutoff=2khz`).
    fn named_arg_value(&mut self, kind: &str, name: &str) -> Option<f32> {
        // A minus sign used to drop out of the unit path entirely.
        let negative = matches!(self.peek(), Token::Rest) && matches!(self.peek_ahead(1), Token::Quantity(_, _));
        if negative {
            self.advance();
        }
        if let Token::Quantity(raw, suffix) = self.peek().clone() {
            let raw = if negative { -raw } else { raw };
            let sp = self.span();
            let (l, c) = (sp.line, sp.col);
            self.advance();
            return match self.resolve_arg_quantity(kind, Some(name), 0, raw, &suffix) {
                Ok(v) => Some(v),
                Err(message) => {
                    self.errors.push(ParseError { line: l, col: c, message });
                    None
                }
            };
        }
        let (line, col) = {
            let sp = self.span();
            (sp.line, sp.col)
        };
        let value = self.parse_expr_value();
        // A compressor's makeup is a linear gain inside, and a bare number
        // there was read as one: `makeup=4` is +12 dB. Every use in the corpus
        // once wrote it meaning decibels, so it takes decibels, said as such.
        if kind == "compressor" && name == "makeup" {
            if let Some(v) = value.filter(|v| *v > 0.0) {
                let db = 20.0 * crate::math::log10(v);
                self.errors.push(ParseError { line, col, message: format!(
                    "makeup takes decibels: write `makeup={db:.1}db` for what `makeup={v}` did (a gain of {v}), or `makeup={v}db` if you meant {v} dB"
                ) });
            } else {
                self.errors.push(ParseError {
                    line,
                    col,
                    message: String::from("makeup takes decibels, like `makeup=3db`"),
                });
            }
            return None;
        }
        value
    }

    fn resolve_arg_quantity(
        &self,
        kind: &str,
        name: Option<&str>,
        index: usize,
        raw: f32,
        suffix: &str,
    ) -> Result<f32, String> {
        match crate::nodes::arg_at(kind, name, index) {
            Some(arg) => arg.value_from_quantity(raw, suffix),
            None => Err(format!(
                "'{}{}' carries a unit, but {} has no argument there to give it meaning",
                raw,
                suffix,
                if kind.is_empty() { "this" } else { kind }
            )),
        }
    }

    /// Optional `*N` after a tie or rest: `..*15` is fifteen ties.
    fn repeat_count(&mut self) -> usize {
        if !matches!(self.peek(), Token::Star) {
            return 1;
        }
        self.advance();
        match self.peek().clone() {
            Token::Number(n) if (1.0..=256.0).contains(&n) => {
                self.advance();
                n as usize
            }
            _ => {
                let s = self.span();
                let (l, c) = (s.line, s.col);
                self.errors.push(ParseError {
                    line: l,
                    col: c,
                    message: String::from("expected a count 1..256 after '*'"),
                });
                1
            }
        }
    }

    fn at_block_end(&self) -> bool {
        matches!(self.peek(), Token::RBrace | Token::Eof)
    }

    /// The token `n` places ahead, without consuming anything.
    fn peek_ahead(&self, n: usize) -> &Token {
        let i = (self.pos + n).min(self.tokens.len().saturating_sub(1));
        &self.tokens[i].token
    }

    fn peek_past_newlines(&self) -> &Token {
        let mut i = self.pos;
        while i < self.tokens.len() && matches!(self.tokens[i].token, Token::Newline) {
            i += 1;
        }
        &self.tokens[i.min(self.tokens.len() - 1)].token
    }

    /// True when the tokens after the current one look like `ident =`.
    fn next_is_key_value(&self) -> bool {
        let n = self.tokens.len();
        let i = self.pos + 1;
        i + 1 < n && matches!(self.tokens[i].token, Token::Ident(_)) && matches!(self.tokens[i + 1].token, Token::Eq)
    }

    /// After a top-level error, skip the rest of the line so one mistake
    /// produces one diagnostic instead of one per token.
    fn recover_to_line_end(&mut self) {
        while !matches!(self.peek(), Token::Newline | Token::Eof) {
            self.pos += 1;
        }
    }

    fn parse_expr_value(&mut self) -> Option<f32> {
        // Handles negative values and simple arithmetic
        let negative = if matches!(self.peek(), Token::Rest) {
            self.advance();
            true
        } else {
            false
        };

        let val = self.expect_number()?;
        let val = if negative { -val } else { val };

        // Check for * or + continuation
        if matches!(self.peek(), Token::Star) {
            self.advance();
            let rhs = self.expect_number().unwrap_or(1.0);
            Some(val * rhs)
        } else if matches!(self.peek(), Token::Plus) {
            self.advance();
            let rhs = self.expect_number().unwrap_or(0.0);
            Some(val + rhs)
        } else {
            Some(val)
        }
    }

    // ── Top-level ──

    pub fn parse_song(mut self) -> Result<Song, Vec<ParseError>> {
        let mut song = Song {
            globals: Globals::default(),
            buses: Vec::new(),
            instruments: Vec::new(),
            module_defs: Vec::new(),
            patterns: Vec::new(),
            tracks: Vec::new(),
            bus_chains: Vec::new(),
            master: None,
            scenes: Vec::new(),
            arrangement: Vec::new(),
            grooves: Vec::new(),
            midi: Vec::new(),
            automations: Vec::new(),
            perform: Default::default(),
        };
        // Where the first top-level `auto` is, for the error if the song
        // turns out to have scenes.
        let mut first_auto: Option<(usize, usize)> = None;

        loop {
            self.skip_newlines();
            match self.peek().clone() {
                Token::Eof => break,
                Token::Tempo => {
                    self.advance();
                    self.parse_tempo(&mut song.globals);
                }
                Token::Meter => {
                    self.advance();
                    self.parse_meter(&mut song.globals);
                }
                Token::Scale => {
                    self.advance();
                    self.parse_scale(&mut song.globals);
                }
                Token::Sidechain => {
                    self.advance();
                    self.parse_sidechain(&mut song.globals);
                }
                Token::Swing => {
                    self.advance();
                    self.parse_swing(&mut song.globals);
                }
                Token::Humanize => {
                    self.advance();
                    self.parse_humanize(&mut song.globals);
                }
                Token::Bus => {
                    self.advance();
                    self.parse_bus_decl(&mut song.buses);
                }
                Token::Instrument => {
                    self.advance();
                    self.parse_instrument(&mut song.instruments);
                }
                Token::Module => {
                    self.advance();
                    self.parse_module_def(&mut song.module_defs);
                }
                Token::Pattern => {
                    self.advance();
                    self.parse_pattern_or_scene(&mut song);
                }
                Token::Track => {
                    self.advance();
                    self.parse_track(&mut song.tracks);
                }
                Token::Master => {
                    self.advance();
                    self.parse_master(&mut song);
                }
                Token::Arrange => {
                    self.advance();
                    self.parse_arrange(&mut song.arrangement);
                }
                Token::Scene => {
                    self.advance();
                    self.parse_scene(&mut song.scenes);
                }
                Token::Auto => {
                    let s = self.span();
                    first_auto.get_or_insert((s.line, s.col));
                    self.advance();
                    self.parse_automation(&mut song.automations, true);
                }
                // A bare identifier followed by { could be a bus chain or groove block
                Token::Ident(ref name) if name == "gain_comp" => {
                    self.advance();
                    song.globals.gain_comp = self.expect_number().map(|v| v.clamp(0.0, 1.0));
                }
                Token::Ident(ref name) if name == "groove" => {
                    self.advance();
                    self.parse_groove(&mut song.grooves);
                }
                // `midi {` would otherwise read as a bus chain named midi.
                Token::Ident(ref name) if name == "midi" && matches!(self.peek_ahead(1), Token::LBrace) => {
                    self.advance();
                    self.parse_midi(&mut song.midi, &mut song.perform);
                }
                // `perform drop {`: a scene of a performance.
                Token::Ident(ref name)
                    if name == "perform"
                        && matches!(
                            self.peek_ahead(1),
                            Token::Ident(_) | Token::DrumHit | Token::DrumAccent | Token::DrumGhost(_)
                        )
                        && matches!(self.peek_ahead(2), Token::LBrace) =>
                {
                    self.advance();
                    self.parse_perform(&mut song.perform);
                }
                Token::Ident(ref name) if name == "keyboard" && matches!(self.peek_ahead(1), Token::LBrace) => {
                    self.advance();
                    self.parse_keyboard(&mut song.perform);
                }
                // `delay key=value ...` / `reverb key=value ...` configure the global sends.
                // `reverb {` is still a bus chain, so only take this path on `ident =`.
                Token::Ident(ref name) if (name == "delay" || name == "reverb") && self.next_is_key_value() => {
                    let which = name.clone();
                    self.advance();
                    self.parse_send_fx_globals(&which, &mut song.globals);
                }
                Token::Ident(_) => {
                    self.try_parse_bus_chain_or_error(&mut song);
                }
                _ => {
                    let s = self.span().clone();
                    self.errors.push(ParseError {
                        line: s.line,
                        col: s.col,
                        message: format!("unexpected {} at top level", describe_token(&s.token)),
                    });
                    self.recover_to_line_end();
                }
            }
        }

        // A song with scenes has somewhere for a lane to live and a length
        // for it; one outside them would have neither a start nor an owner.
        if let (Some((line, col)), false) = (first_auto, song.scenes.is_empty()) {
            self.errors.push(ParseError {
                line,
                col,
                message: String::from(
                    "a top-level `auto` is for a song without scenes; this one has scenes, so put the lane inside the scene it belongs to (`scene build { auto ... }`), where it runs over the scene",
                ),
            });
        }

        if self.errors.is_empty() {
            Ok(song)
        } else {
            Err(self.errors)
        }
    }
}

/// Human-readable token for error messages.
fn describe_token(t: &Token) -> String {
    match t {
        Token::Ident(s) => format!("'{}'", s),
        Token::Number(n) => format!("number {}", n),
        Token::Quantity(n, u) => format!("'{}{}'", n, u),
        Token::Note(n) => format!("note {}", n),
        Token::DrumHit => String::from("'x'"),
        Token::DrumAccent => String::from("'X'"),
        Token::DrumGhost(_) => String::from("'o'"),
        Token::LBrace => String::from("'{'"),
        Token::RBrace => String::from("'}'"),
        Token::Newline => String::from("end of line"),
        Token::Eof => String::from("end of file"),
        other => format!("{:?}", other).to_lowercase(),
    }
}
