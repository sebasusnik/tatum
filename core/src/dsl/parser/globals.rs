//! The lines that set something for the whole song: tempo, meter, scale,
//! sidechain, swing, humanize, and the global delay and reverb sends. Each
//! number is checked against the span the engine can play.

use alloc::string::String;
use alloc::format;

use crate::dsl::ast::*;
use crate::dsl::error::ParseError;
use crate::dsl::lexer::Token;

use super::{Parser, describe_token};

impl Parser {
    // ── Globals ──

    /// A number that has to fall inside `lo..=hi`, or an error at the number
    /// saying what the span is and why. The engine's runtime setters clamp to
    /// the same spans; a file that asks for more is told so rather than being
    /// quietly played at something else (a tempo of 0 used to hang a render,
    /// a meter of 0/4 rendered nothing).
    fn expect_number_in(&mut self, what: &str, lo: f32, hi: f32, meaning: &str) -> Option<f32> {
        self.skip_newlines();
        let (line, col) = {
            let s = self.span();
            (s.line, s.col)
        };
        let n = self.expect_number()?;
        if !(lo..=hi).contains(&n) {
            self.errors.push(ParseError {
                line,
                col,
                message: format!("{} {} is outside {}..{} ({})", what, n, lo, hi, meaning),
            });
            return None;
        }
        Some(n)
    }

    pub(super) fn expect_tempo(&mut self) -> Option<f32> {
        self.expect_number_in("tempo", 20.0, 999.0, "beats per minute")
    }

    pub(super) fn parse_tempo(&mut self, globals: &mut Globals) {
        if let Some(n) = self.expect_tempo() {
            globals.tempo = n;
        }
    }

    pub(super) fn parse_meter(&mut self, globals: &mut Globals) {
        let num = self.expect_number_in("meter", 1.0, 16.0, "beats in a bar");
        self.expect(&Token::Slash);
        self.skip_newlines();
        let (line, col) = {
            let s = self.span();
            (s.line, s.col)
        };
        let den = self.expect_number();
        if let Some(den) = den {
            if den != 4.0 {
                self.errors.push(ParseError {
                    line, col,
                    message: format!("meter /{}: only /4 is supported -- a beat is four sixteenth steps. Write 7/8 as 7/4 at half the tempo, or 6/8 as 3/4 with swing", den),
                });
            }
        }
        if let (Some(num), Some(4.0)) = (num, den) {
            if num != (num as u8) as f32 {
                self.errors.push(ParseError {
                    line,
                    col,
                    message: format!("meter {}/4: a whole number of beats", num),
                });
            } else {
                globals.meter = (num as u8, 4);
            }
        }
    }

    pub(super) fn parse_scale(&mut self, globals: &mut Globals) {
        if let Some(root) = self.expect_ident() {
            if let Some(kind) = self.expect_ident() {
                globals.scale = Some(ScaleDef { root, kind });
            }
        }
    }

    pub(super) fn parse_sidechain(&mut self, globals: &mut Globals) {
        if let Some(n) = self.expect_number() {
            globals.sidechain = n;
        }
        globals.sidechain_source = self.parse_sidechain_source();
        // `attack=` and `release=` shape the envelope every source follows.
        loop {
            let key = match self.peek().clone() {
                Token::Ident(ref k) if k == "attack" || k == "release" => k.clone(),
                _ => break,
            };
            let sp = self.span();
            let (l, c) = (sp.line, sp.col);
            self.advance();
            if !self.expect(&Token::Eq) {
                break;
            }
            let ms = match self.peek().clone() {
                Token::Quantity(v, ref suffix) if suffix == "ms" => {
                    self.advance();
                    Some(v)
                }
                Token::Quantity(v, ref suffix) if suffix == "s" || suffix == "sec" => {
                    self.advance();
                    Some(v * 1000.0)
                }
                Token::Number(v) => {
                    self.advance();
                    Some(v)
                }
                other => {
                    self.errors.push(ParseError {
                        line: l,
                        col: c,
                        message: format!("sidechain {}= takes a time like 80ms, got {}", key, describe_token(&other)),
                    });
                    None
                }
            };
            let Some(ms) = ms else { break };
            if !(0.1..=2000.0).contains(&ms) {
                self.errors.push(ParseError {
                    line: l,
                    col: c,
                    message: format!("sidechain {}= {}ms is outside 0.1ms..2000ms", key, ms),
                });
                continue;
            }
            if key == "attack" {
                globals.sidechain_attack_ms = Some(ms);
            } else {
                globals.sidechain_release_ms = Some(ms);
            }
        }
    }

    /// Optional `from=<track or module>` after a sidechain amount.
    pub(super) fn parse_sidechain_source(&mut self) -> Option<String> {
        if !matches!(self.peek(), Token::Ident(ref w) if w == "from") {
            return None;
        }
        self.advance();
        if !self.expect(&Token::Eq) {
            return None;
        }
        match self.peek().clone() {
            Token::Ident(name) => {
                self.advance();
                Some(name)
            }
            other => {
                let sp = self.span();
                let (l, c) = (sp.line, sp.col);
                self.errors.push(ParseError {
                    line: l,
                    col: c,
                    message: format!("sidechain from= needs a track or module name, got {}", describe_token(&other)),
                });
                None
            }
        }
    }

    pub(super) fn parse_swing(&mut self, globals: &mut Globals) {
        if let Some(n) = self.expect_number_in("swing", 0.5, 0.75, "0.5 is straight, 0.75 a hard shuffle") {
            globals.swing = Some(n);
        }
    }

    pub(super) fn parse_humanize(&mut self, globals: &mut Globals) {
        // humanize 0.5           → velocity humanization only
        // humanize 0.5 timing 0.3 → velocity + timing humanization
        let before = self.errors.len();
        let n = self.expect_number_in("humanize", 0.0, 1.0, "how much velocity varies");
        if n.is_none() && self.errors.len() == before {
            return;
        }
        globals.humanize = n;
        // Check for optional "timing" sub-keyword; read it even after a
        // velocity out of range, so the one mistake is the one error.
        self.skip_newlines();
        if let Token::Ident(ref w) = self.peek().clone() {
            if w == "timing" {
                self.advance();
                if let Some(t) = self.expect_number_in("humanize timing", 0.0, 1.0, "how much the step clock wanders") {
                    globals.humanize_timing = Some(t);
                }
            }
        }
    }

    /// Top-level `delay sync=dotted_eighth feedback=0.45 filter=0.5 time=0.3`
    /// or `reverb size=0.7 damp=0.4 predelay=20`.
    pub(super) fn parse_send_fx_globals(&mut self, which: &str, globals: &mut Globals) {
        loop {
            let key = match self.peek().clone() {
                Token::Ident(ref k) => k.clone(),
                Token::Sidechain => String::from("sidechain"),
                _ => break,
            };
            let s = self.span();
            let (line, col) = (s.line, s.col);
            self.advance();
            if !self.expect(&Token::Eq) {
                break;
            }
            // Value: number, or an identifier for `sync`
            let ident_value = match self.peek().clone() {
                Token::Ident(ref v) => {
                    let v = v.clone();
                    self.advance();
                    Some(v)
                }
                _ => None,
            };
            let num_value = if ident_value.is_none() { self.expect_number() } else { None };
            let mut bad = None;
            match (which, key.as_str()) {
                ("delay", "sync") => match ident_value {
                    Some(v) if ["free", "quarter", "dotted_eighth", "eighth", "sixteenth", "triplet_eighth"].contains(&v.as_str()) => {
                        globals.send_delay.sync = Some(v);
                    }
                    _ => bad = Some(String::from("delay sync: expected free | quarter | dotted_eighth | eighth | sixteenth | triplet_eighth")),
                },
                ("delay", "time") => globals.send_delay.time = num_value,
                ("delay", "feedback") => globals.send_delay.feedback = num_value,
                ("delay", "filter") => globals.send_delay.filter = num_value,
                ("delay", "sidechain") => globals.send_delay.sidechain = num_value,
                ("reverb", "sidechain") => globals.send_reverb.sidechain = num_value,
                ("reverb", "size") => globals.send_reverb.size = num_value,
                ("reverb", "damp") => globals.send_reverb.damp = num_value,
                ("reverb", "predelay") => globals.send_reverb.predelay = num_value,
                _ => bad = Some(format!(
                    "{}: unknown option '{}' (delay: sync, time, feedback, filter, sidechain; reverb: size, damp, predelay, sidechain)",
                    which, key
                )),
            }
            if let Some(message) = bad {
                self.errors.push(ParseError { line, col, message });
            }
        }
    }
}
