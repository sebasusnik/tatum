//! `auto` lanes, which move a parameter over bars, and `arrange`, which
//! lays scenes out in time.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::dsl::ast::*;
use crate::dsl::error::ParseError;
use crate::dsl::lexer::Token;

use super::Parser;

impl Parser {
    // ── Automation ──

    /// `auto target val > val [> val ...]`, and at the top level of a song
    /// without scenes `... over N`: a scene lane spans its scene, a top-level
    /// one has nothing to span but the loop, so it says how long it is.
    pub(super) fn parse_automation(&mut self, automations: &mut Vec<AutomationDef>, top_level: bool) {
        // Target can be "instrument.param" (with dot), or "reverb_mix"
        let (line, col) = {
            let s = self.span();
            (s.line, s.col)
        };
        let mut target = match self.expect_ident() {
            Some(t) => t,
            None => return,
        };

        // A dotted target arrives as separate Ident tokens: the lexer drops a
        // `.` that sits between two identifiers, so `bass.lp wet` is three
        // idents in a row. Collect them until the first keyframe number and
        // rejoin, which handles `reverb_mix`, `tines.mod_index` and the
        // three-part `<track>.<node>.wet` with the same rule.
        loop {
            self.skip_newlines();
            let part = match self.peek().clone() {
                Token::Ident(ref s) => s.clone(),
                Token::Level => alloc::string::String::from("level"),
                Token::Velocity => alloc::string::String::from("velocity"),
                Token::Pan => alloc::string::String::from("pan"),
                Token::Mix => alloc::string::String::from("mix"),
                _ => break,
            };
            let saved = self.pos;
            self.advance();
            // Only a part if a keyframe or another part follows it; otherwise
            // it belongs to whatever comes next and we have to give it back.
            let continues = matches!(
                self.peek(),
                Token::Number(_)
                    | Token::Rest
                    | Token::Ident(_)
                    | Token::Level
                    | Token::Velocity
                    | Token::Pan
                    | Token::Mix
            );
            if !continues {
                self.pos = saved;
                break;
            }
            target = alloc::format!("{}.{}", target, part);
            if matches!(self.peek(), Token::Number(_) | Token::Rest) {
                break;
            }
        }

        // Parse keyframes: val > val [> val]
        let mut keyframes = Vec::new();

        // Handle negative first value
        let negative = if matches!(self.peek(), Token::Rest) {
            self.advance();
            true
        } else {
            false
        };
        if let Some(v) = self.expect_number() {
            keyframes.push(if negative { -v } else { v });
        }

        // Parse additional keyframes: > val
        loop {
            self.skip_newlines();
            if !matches!(self.peek(), Token::Arrow) {
                break;
            }
            self.advance(); // consume >

            let negative = if matches!(self.peek(), Token::Rest) {
                self.advance();
                true
            } else {
                false
            };
            if let Some(v) = self.expect_number() {
                keyframes.push(if negative { -v } else { v });
            } else {
                break;
            }
        }

        // `over 8`. The keyframe loop has already stepped past a line end,
        // so this only looks at the same line when there was no `>` left.
        let over = if matches!(self.peek(), Token::Ident(ref w) if w == "over") {
            let s = self.span();
            let (l, c) = (s.line, s.col);
            self.advance();
            let bars = match self.peek().clone() {
                Token::Number(n) if (1.0..=1024.0).contains(&n) && (n as u32) as f32 == n => {
                    self.advance();
                    Some(n as u32)
                }
                _ => {
                    self.errors.push(ParseError {
                        line: l,
                        col: c,
                        message: String::from("`over` takes a whole number of bars, 1 to 1024: `over 8`"),
                    });
                    self.recover_to_line_end();
                    None
                }
            };
            if !top_level {
                self.errors.push(ParseError {
                    line: l,
                    col: c,
                    message: String::from(
                        "`over` is for an `auto` outside any scene: a scene's lane already runs over the whole scene",
                    ),
                });
            }
            bars
        } else {
            None
        };
        if top_level && over.is_none() && !keyframes.is_empty() {
            self.errors.push(ParseError {
                line,
                col,
                message: format!(
                    "a top-level `auto` needs its length: `auto {} {} over 8` runs over 8 bars from the bar this text starts playing, then holds its last value",
                    target.replacen('.', " ", 1).replace(".wet", " wet"),
                    keyframes.iter().map(|k| format!("{}", k)).collect::<Vec<_>>().join(" > ")
                ),
            });
        }

        if !keyframes.is_empty() {
            automations.push(AutomationDef { target, keyframes, over });
        }
    }

    // ── Arrange ──

    pub(super) fn parse_arrange(&mut self, arrangement: &mut Vec<ArrangeEntry>) {
        if !self.expect(&Token::LBrace) {
            return;
        }

        loop {
            self.skip_newlines();
            if self.at_block_end() {
                break;
            }

            if let Some(scene_name) = self.expect_ident() {
                let repeat = if let Token::Repeat(n) = self.peek() {
                    let n = *n;
                    self.advance();
                    n
                } else {
                    1
                };
                arrangement.push(ArrangeEntry { scene_name, repeat });
            } else {
                self.advance();
            }
        }

        self.expect(&Token::RBrace);
    }
}
