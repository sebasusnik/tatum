//! Tracks: what a track plays and through what, its arp, and the `>` chain
//! that routes it. A track defined twice keeps what the second one leaves out.

use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;

use crate::dsl::ast::*;
use crate::dsl::error::ParseError;
use crate::dsl::lexer::Token;

use super::{Parser, describe_token};

impl Parser {
    // ── Track ──

    /// A redefinition only says what it changes; everything it leaves out
    /// keeps the value the earlier definition gave it. This is the same rule a
    /// scene track already follows, and without it a step of a live set that
    /// wants to move one fader has to restate the track's whole chain -- and
    /// then it owns that chain, so a later fix to the rig never reaches it.
    /// `play`, `using` and the `out >` chain read as unset when absent.
    fn merge_track(old: &TrackDef, new: TrackDef) -> TrackDef {
        TrackDef {
            name: new.name,
            play: if new.play.is_empty() { old.play.clone() } else { new.play },
            using_instrument: if new.using_instrument.is_empty() {
                old.using_instrument.clone()
            } else {
                new.using_instrument
            },
            routing: if new.routing.is_empty() { old.routing.clone() } else { new.routing },
            velocity: new.velocity.or(old.velocity),
            level: new.level.or(old.level),
            pan: new.pan.or(old.pan),
            gate: new.gate.or(old.gate),
            delay_send: new.delay_send.or(old.delay_send),
            reverb_send: new.reverb_send.or(old.reverb_send),
            sidechain: new.sidechain.or(old.sidechain),
            sidechain_source: new.sidechain_source.or_else(|| old.sidechain_source.clone()),
            arp: new.arp.or_else(|| old.arp.clone()),
        }
    }

    pub(super) fn parse_track(&mut self, tracks: &mut Vec<TrackDef>) {
        let name = match self.expect_ident() {
            Some(n) => n,
            None => return,
        };
        if !self.expect(&Token::LBrace) { return; }

        let mut track = TrackDef {
            name,
            play: String::new(),
            using_instrument: String::new(),
            velocity: None,
            level: None,
            pan: None,
            gate: None,
            routing: Vec::new(),
            delay_send: None,
            reverb_send: None,
            sidechain_source: None,
            sidechain: None,
            arp: None,
        };

        loop {
            self.skip_newlines();
            if self.at_block_end() { break; }

            match self.peek().clone() {
                Token::Play => {
                    self.advance();
                    if let Some(pat) = self.expect_ident() {
                        track.play = pat;
                    }
                }
                Token::Using => {
                    self.advance();
                    if let Some(inst) = self.expect_ident() {
                        track.using_instrument = inst;
                    }
                }
                Token::Velocity => {
                    self.advance();
                    if let Some(v) = self.expect_number() {
                        track.velocity = Some(v);
                    }
                }
                Token::Level => {
                    self.advance();
                    if let Some(v) = self.expect_number() {
                        track.level = Some(v);
                    }
                }
                Token::Pan => {
                    self.advance();
                    // Pan can be negative (e.g., pan -0.5)
                    let negative = if matches!(self.peek(), Token::Rest) {
                        self.advance();
                        true
                    } else {
                        false
                    };
                    if let Some(v) = self.expect_number() {
                        track.pan = Some(if negative { -v } else { v });
                    }
                }
                Token::Out => {
                    // Parse routing chain: out > drive(0.2) > master
                    self.parse_routing_chain(&mut track.routing);
                }
                Token::Ident(ref word) if word == "gate" => {
                    self.advance();
                    if let Some(v) = self.expect_number() {
                        track.gate = Some(v);
                    }
                }
                Token::Ident(ref word) if word == "delay_send" => {
                    self.advance();
                    if let Some(v) = self.expect_number() {
                        track.delay_send = Some(v);
                    }
                }
                Token::Ident(ref word) if word == "reverb_send" => {
                    self.advance();
                    if let Some(v) = self.expect_number() {
                        track.reverb_send = Some(v);
                    }
                }
                // `sidechain` is a top-level keyword and also a track option.
                Token::Sidechain | Token::Ident(_) if matches!(self.peek(), Token::Sidechain)
                    || matches!(self.peek(), Token::Ident(ref w) if w == "sidechain") =>
                {
                    self.advance();
                    if let Some(v) = self.expect_number() {
                        track.sidechain = Some(v);
                    }
                    track.sidechain_source = self.parse_sidechain_source();
                }
                Token::Ident(ref word) if word == "arp" => {
                    self.advance();
                    track.arp = self.parse_arp_clause();
                }
                other => {
                    let sp = self.span();
                    let (l, c) = (sp.line, sp.col);
                    self.errors.push(ParseError {
                        line: l, col: c,
                        message: format!(
                            "track '{}': unexpected {} (a track takes play, using, level, pan, gate, velocity, delay_send, reverb_send, sidechain, arp or `out > ...`)",
                            track.name, describe_token(&other)
                        ),
                    });
                    self.advance();
                    self.recover_to_line_end();
                }
            }
        }

        self.expect(&Token::RBrace);
        // A later definition of the same name REPLACES the earlier one, in
        // place. In place matters: the order of these lists is the order the
        // engine builds and mixes them, and a live swap inherits state by
        // position as well as by name, so reordering on a redefinition would
        // hand a voice's state to a different voice. This is what lets a file
        // say `use "rig.synth"` and then override three tracks, instead of
        // every step of a set carrying a copy of the whole rig.
        match tracks.iter().position(|t| t.name == track.name) {
            Some(i) => tracks[i] = Self::merge_track(&tracks[i], track),
            None => tracks.push(track),
        }
    }

    /// `arp <mode> [rate=N] [gate=F] [octaves=N]` — mode is up, down, updown or off.
    fn parse_arp_clause(&mut self) -> Option<ArpDef> {
        let mode = self.expect_ident()?;
        let mut def = ArpDef { mode, rate: None, gate: None, octaves: None };
        while let Token::Ident(key) = self.peek().clone() {
            // Only consume `ident =` pairs; a bare ident belongs to the next clause.
            let saved = self.pos;
            self.advance();
            if !matches!(self.peek(), Token::Eq) {
                self.pos = saved;
                break;
            }
            self.advance();
            let value = match self.expect_number() {
                Some(v) => v,
                None => break,
            };
            match key.as_str() {
                "rate" => def.rate = Some(value),
                "gate" => def.gate = Some(value),
                "octaves" => def.octaves = Some(value),
                other => {
                    let s = self.span();
                    let (l, c) = (s.line, s.col);
                    self.errors.push(ParseError {
                        line: l, col: c,
                        message: format!("arp: unknown option '{}' (expected rate, gate, octaves)", other),
                    });
                }
            }
        }
        Some(def)
    }

    fn parse_routing_chain(&mut self, routing: &mut Vec<RoutingNode>) {
        // Consume "out"
        self.advance();

        loop {
            // A chain may wrap: `out > a(..)\n    > b(..) > master`. Only swallow
            // the newlines when a `>` really follows, so a chain that simply ends
            // still hands the newline back to the track body.
            if matches!(self.peek(), Token::Newline) && matches!(self.peek_past_newlines(), Token::Arrow) {
                self.skip_newlines();
            }
            if !matches!(self.peek(), Token::Arrow) { break; }
            self.advance(); // >
            self.skip_newlines(); // `>` at end of line, node on the next

            let kind = match self.peek().clone() {
                Token::Ident(ref name) => { let n = name.clone(); self.advance(); n }
                Token::Master => { self.advance(); String::from("master") }
                other => {
                    let s = self.span();
                    let (l, c) = (s.line, s.col);
                    self.errors.push(ParseError {
                        line: l, col: c,
                        message: format!("routing: expected an effect or destination after '>', got {}", describe_token(&other)),
                    });
                    break;
                }
            };

            let mut params = Vec::new();
            if matches!(self.peek(), Token::LParen) {
                self.advance();
                self.parse_param_list_for(&mut params, &kind);
                self.expect(&Token::RParen);
            }

            let label = if matches!(self.peek(), Token::As) {
                self.advance();
                self.expect_node_label()
            } else {
                None
            };

            routing.push(RoutingNode { kind, params, label });
        }
    }
}
