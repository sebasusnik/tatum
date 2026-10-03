//! The smaller blocks: bus declarations, `groove`, `midi`, bus chains,
//! `master` and `scene`.

use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;

use crate::dsl::ast::*;
use crate::dsl::error::ParseError;
use crate::dsl::lexer::Token;

use super::{Parser, describe_token};

impl Parser {
    // ── Bus declaration ──

    pub(super) fn parse_bus_decl(&mut self, buses: &mut Vec<BusDef>) {
        if let Some(name) = self.expect_ident() {
            buses.push(BusDef { name });
        }
    }

    // ── Groove block ──
    // groove jungle_breaks {
    //     hat swing 0.62
    //     snare nudge 0.01
    //     kick nudge -0.005
    // }

    pub(super) fn parse_groove(&mut self, grooves: &mut Vec<GrooveDef>) {
        let name = self.expect_ident().unwrap_or_else(|| String::from("default"));
        if !self.expect(&Token::LBrace) {
            return;
        }

        let mut lanes = Vec::new();
        loop {
            self.skip_newlines();
            if self.at_block_end() {
                break;
            }

            if let Token::Ident(ref drum_name) = self.peek().clone() {
                let drum_name = drum_name.clone();
                self.advance();

                let mut swing = None;
                let mut nudge = None;

                // Parse key-value pairs on this line
                loop {
                    match self.peek().clone() {
                        Token::Ident(ref key) if key == "swing" => {
                            self.advance();
                            swing = self.expect_number();
                        }
                        Token::Ident(ref key) if key == "nudge" || key == "delay" => {
                            self.advance();
                            // Handle negative numbers: if we see a Rest token (-) followed by a number
                            if matches!(self.peek(), Token::Rest) {
                                self.advance(); // consume '-'
                                nudge = self.expect_number().map(|n| -n);
                            } else {
                                nudge = self.expect_number();
                            }
                        }
                        Token::Newline | Token::RBrace | Token::Eof => break,
                        _ => {
                            self.advance();
                        }
                    }
                }

                lanes.push(GrooveLane { drum_name, swing, nudge });
            } else {
                self.advance();
            }
        }

        self.expect(&Token::RBrace);
        grooves.push(GrooveDef { name, lanes });
    }

    // ── MIDI ──
    // midi {
    //     cc 74 > acid cutoff      a knob or a fader
    //     keys > solo              the keyboard plays a track
    //     pad 36 > kick kick       a pad hits one drum of a track
    //     pad 40 > play drone C2   a pad plays a note on a track while held
    //     pad 41 > hold riser      a track sounds only while the pad is held
    //     pad 38 > repeat 1/16     a beat repeat on the output while held
    // }

    /// A later block replaces what an earlier one said about the same knob,
    /// the keys or the same pad, and leaves the rest alone -- the redefinition
    /// rule the rest of the language follows, so a set step can remap one
    /// knob of the rig it `use`s. Inside one block, a source named twice
    /// moves both targets.
    pub(super) fn parse_midi(&mut self, maps: &mut Vec<MidiMapDef>) {
        if !self.expect(&Token::LBrace) {
            return;
        }
        let mut block: Vec<MidiMapDef> = Vec::new();
        loop {
            self.skip_newlines();
            if self.at_block_end() {
                break;
            }
            let line = self.span().line;
            let word = match self.peek() {
                Token::Ident(w) if w == "cc" || w == "keys" || w == "pad" => w.clone(),
                other => {
                    let s = self.span().clone();
                    self.errors.push(ParseError {
                        line: s.line, col: s.col,
                        message: format!(
                            "midi: expected `cc <number> > <target>`, `keys > <track>` or `pad <note> > <track> <drum>`, got {}",
                            describe_token(other)),
                    });
                    self.recover_to_line_end();
                    continue;
                }
            };
            self.advance();
            let source = if word == "keys" {
                MidiSource::Keys
            } else {
                let what = if word == "cc" { "a controller number" } else { "a pad's note" };
                match self.peek().clone() {
                    Token::Number(n) if (0.0..=127.0).contains(&n) && (n as u8) as f32 == n => {
                        self.advance();
                        if word == "cc" {
                            MidiSource::Cc(n as u8)
                        } else {
                            MidiSource::Pad(n as u8)
                        }
                    }
                    other => {
                        let s = self.span().clone();
                        self.errors.push(ParseError {
                            line: s.line,
                            col: s.col,
                            message: format!(
                                "midi: {} is a whole number from 0 to 127, got {}",
                                what,
                                describe_token(&other)
                            ),
                        });
                        self.recover_to_line_end();
                        continue;
                    }
                }
            };
            if !self.expect(&Token::Arrow) {
                self.recover_to_line_end();
                continue;
            }
            // The target runs to the end of the line, in the words `auto`
            // takes. The start of another mapping ends it, so several can
            // share a line.
            let mut words: Vec<String> = Vec::new();
            loop {
                let word = match self.peek() {
                    Token::Ident(w) if (w == "cc" || w == "pad") && matches!(self.peek_ahead(1), Token::Number(_)) => {
                        break
                    }
                    Token::Ident(w) if w == "keys" && matches!(self.peek_ahead(1), Token::Arrow) => break,
                    Token::Ident(w) if w == "q" && matches!(self.peek_ahead(1), Token::Eq) => break,
                    Token::Ident(w) => w.clone(),
                    Token::Level => String::from("level"),
                    Token::Pan => String::from("pan"),
                    Token::Velocity => String::from("velocity"),
                    Token::Mix => String::from("mix"),
                    Token::Master => String::from("master"),
                    Token::Tempo => String::from("tempo"),
                    Token::Play => String::from("play"),
                    // `pad 40 > play grinder C2`: the note a pad plays.
                    Token::Note(n) if matches!(source, MidiSource::Pad(_)) => n.clone(),
                    _ => break,
                };
                self.advance();
                words.push(word);
            }
            if words.is_empty() {
                let s = self.span().clone();
                let example = match source {
                    MidiSource::Cc(_) => "`acid cutoff` or `pad level`",
                    MidiSource::Keys => "a track, like `solo`",
                    MidiSource::Pad(_) => "a track and a drum, like `kick kick`",
                };
                self.errors.push(ParseError {
                    line: s.line,
                    col: s.col,
                    message: format!("midi: {} needs a target after `>`: {}", word, example),
                });
                self.recover_to_line_end();
                continue;
            }
            // `pad 50 > step 3`: the one target with a number in it.
            if matches!(source, MidiSource::Pad(_)) && words == ["step"] {
                if let Token::Number(n) = self.peek().clone() {
                    self.advance();
                    words.push(format!("{}", n as usize));
                }
            }
            // `pad 38 > repeat 1/16`: a division, read as the fraction it is.
            if matches!(source, MidiSource::Pad(_)) && words == ["repeat"] {
                if let (Token::Number(a), Token::Slash, Token::Number(b)) =
                    (self.peek().clone(), self.peek_ahead(1).clone(), self.peek_ahead(2).clone())
                {
                    self.pos += 3;
                    words.push(format!("{}/{}", a as u32, b as u32));
                }
            }
            let range = if matches!(source, MidiSource::Cc(_)) { self.parse_knob_range() } else { None };
            let quantize = self.parse_pad_quantize(source, &words);
            block.push(MidiMapDef { source, target: words.join("."), range, quantize, line });
        }
        self.expect(&Token::RBrace);
        maps.retain(|m| !block.iter().any(|b| b.source == m.source));
        maps.extend(block);
    }

    /// `q=bar`, `q=beat`, `q=1/8`, `q=1/16` or `q=off` after a pad's target.
    fn parse_pad_quantize(&mut self, source: MidiSource, words: &[String]) -> Option<Quantize> {
        if !(matches!(self.peek(), Token::Ident(w) if w == "q") && matches!(self.peek_ahead(1), Token::Eq)) {
            return None;
        }
        let s = self.span().clone();
        self.pos += 2;
        let word = match (self.peek().clone(), self.peek_ahead(1).clone(), self.peek_ahead(2).clone()) {
            (Token::Number(a), Token::Slash, Token::Number(b)) => {
                self.pos += 3;
                format!("{}/{}", a as u32, b as u32)
            }
            (Token::Ident(w), _, _) => {
                self.pos += 1;
                w
            }
            (other, _, _) => describe_token(&other),
        };
        let err = |p: &mut Self, message: String| p.errors.push(ParseError { line: s.line, col: s.col, message });
        let Some(q) = Quantize::from_word(&word) else {
            err(self, format!("midi: q= is bar, beat, 1/8, 1/16 or off, got {}", word));
            return None;
        };
        let first = words.first().map(String::as_str);
        if !matches!(source, MidiSource::Pad(_)) {
            err(self, String::from("midi: q= quantizes a pad; a knob or the keys act at once"));
        } else if matches!(first, Some("repeat" | "next" | "prev" | "step")) {
            err(self, format!(
                "midi: `{}` already waits for its line (a repeat for its division, a set move for the phrase); it takes no q=",
                first.unwrap_or_default()
            ));
        }
        q
    }

    /// `200hz..4khz` after a knob's target, if one follows; or three points,
    /// `20hz..20hz..2khz`, the middle one at half travel.
    fn parse_knob_range(&mut self) -> Option<Vec<RangeEnd>> {
        let starts = match self.peek() {
            Token::Number(_) | Token::Quantity(_, _) => true,
            Token::Rest => matches!(self.peek_ahead(1), Token::Number(_) | Token::Quantity(_, _)),
            _ => false,
        };
        if !starts {
            return None;
        }
        let low = self.parse_range_end()?;
        if !matches!(self.peek(), Token::Tie) {
            let s = self.span().clone();
            self.errors.push(ParseError {
                line: s.line,
                col: s.col,
                message: format!(
                    "midi: a knob's range is two values with `..` between them, like `200hz..4khz`, got {}",
                    describe_token(&s.token)
                ),
            });
            self.recover_to_line_end();
            return None;
        }
        self.advance();
        let mut ends = Vec::from([low, self.parse_range_end()?]);
        if matches!(self.peek(), Token::Tie) {
            self.advance();
            ends.push(self.parse_range_end()?);
        }
        Some(ends)
    }

    fn parse_range_end(&mut self) -> Option<RangeEnd> {
        let negative = matches!(self.peek(), Token::Rest);
        if negative {
            self.advance();
        }
        let sign = if negative { -1.0 } else { 1.0 };
        match self.peek().clone() {
            Token::Number(n) => {
                self.advance();
                Some(RangeEnd { value: sign * n, unit: None })
            }
            Token::Quantity(n, unit) => {
                self.advance();
                Some(RangeEnd { value: sign * n, unit: Some(unit) })
            }
            other => {
                let s = self.span().clone();
                self.errors.push(ParseError {
                    line: s.line,
                    col: s.col,
                    message: format!("midi: expected a value for the knob's range, got {}", describe_token(&other)),
                });
                self.recover_to_line_end();
                None
            }
        }
    }

    // ── Bus chain ──

    pub(super) fn try_parse_bus_chain_or_error(&mut self, song: &mut Song) {
        // Identifier at top level followed by { — bus chain or scene
        let saved_pos = self.pos;
        if let Token::Ident(ref name) = self.peek().clone() {
            let name = name.clone();
            self.advance();
            self.skip_newlines();
            if matches!(self.peek(), Token::LBrace) {
                self.advance();
                self.parse_bus_chain_body(name, &mut song.bus_chains);
                return;
            }
        }
        self.pos = saved_pos;
        let s = self.span().clone();
        self.errors.push(ParseError {
            line: s.line,
            col: s.col,
            message: format!(
                "unexpected {} at top level (expected tempo, scale, module, pattern, track, scene, arrange, ...)",
                describe_token(&s.token)
            ),
        });
        self.recover_to_line_end();
    }

    fn parse_bus_chain_body(&mut self, bus_name: String, chains: &mut Vec<BusChainDef>) {
        let mut chain: Vec<ChainNode> = Vec::new();

        // Parse: in > effect(params) > ... > out/master
        loop {
            self.skip_newlines();
            if self.at_block_end() {
                break;
            }

            match self.peek().clone() {
                Token::In => {
                    self.advance();
                }
                Token::Arrow => {
                    self.advance();
                }
                Token::Out | Token::Master => {
                    self.advance();
                    // End of chain
                }
                Token::Ident(ref name) => {
                    let name = name.clone();
                    self.advance();
                    let mut params = Vec::new();
                    if matches!(self.peek(), Token::LParen) {
                        self.advance();
                        self.parse_param_list_for(&mut params, &name);
                        self.expect(&Token::RParen);
                    }
                    let label = if matches!(self.peek(), Token::As) {
                        self.advance();
                        self.expect_node_label()
                    } else {
                        None
                    };
                    chain.push(ChainNode { kind: name, params, label });
                }
                _ => {
                    self.advance();
                }
            }
        }

        self.expect(&Token::RBrace);
        let def = BusChainDef { bus_name, chain };
        match chains.iter().position(|c| c.bus_name == def.bus_name) {
            Some(i) => chains[i] = def,
            None => chains.push(def),
        }
    }

    // ── Master ──

    pub(super) fn parse_master(&mut self, song: &mut Song) {
        if !self.expect(&Token::LBrace) {
            return;
        }
        let mut chain: Vec<ChainNode> = Vec::new();

        loop {
            self.skip_newlines();
            if self.at_block_end() {
                break;
            }

            match self.peek().clone() {
                Token::In => {
                    self.advance();
                }
                Token::Arrow => {
                    self.advance();
                }
                Token::Out => {
                    self.advance();
                }
                Token::Ident(ref name) => {
                    let name = name.clone();
                    self.advance();
                    let mut params = Vec::new();
                    if matches!(self.peek(), Token::LParen) {
                        self.advance();
                        self.parse_param_list_for(&mut params, &name);
                        self.expect(&Token::RParen);
                    }
                    let label = if matches!(self.peek(), Token::As) {
                        self.advance();
                        self.expect_node_label()
                    } else {
                        None
                    };
                    chain.push(ChainNode { kind: name, params, label });
                }
                _ => {
                    self.advance();
                }
            }
        }

        self.expect(&Token::RBrace);
        song.master = Some(MasterDef { chain });
    }

    // ── Scene ──

    pub(super) fn parse_scene(&mut self, scenes: &mut Vec<SceneDef>) {
        let name = match self.expect_ident() {
            Some(n) => n,
            None => return,
        };
        if !self.expect(&Token::LBrace) {
            return;
        }
        self.parse_scene_body(name, scenes);
    }

    pub(super) fn parse_scene_body(&mut self, name: String, scenes: &mut Vec<SceneDef>) {
        let mut scene = SceneDef {
            name,
            extends: None,
            tempo: None,
            overrides: Vec::new(),
            automations: Vec::new(),
            tracks: Vec::new(),
        };

        loop {
            self.skip_newlines();
            if self.at_block_end() {
                break;
            }

            match self.peek().clone() {
                Token::Extends => {
                    self.advance();
                    if let Some(parent) = self.expect_ident() {
                        scene.extends = Some(parent);
                    }
                }
                Token::Tempo => {
                    self.advance();
                    if let Some(t) = self.expect_tempo() {
                        scene.tempo = Some(t);
                    }
                }
                Token::Track => {
                    self.advance();
                    self.parse_track(&mut scene.tracks);
                }
                Token::Auto => {
                    self.advance();
                    self.parse_automation(&mut scene.automations, false);
                }
                // Override: reverb_mix = 0.2 or delay_mix = 0.18
                Token::Ident(ref target) => {
                    let target = target.clone();
                    let s = self.span();
                    let (line, col) = (s.line, s.col);
                    self.advance();
                    if matches!(self.peek(), Token::Eq) {
                        self.advance();
                        if let Some(val) = self.expect_number() {
                            scene.overrides.push(Override { target, value: val });
                        }
                    } else {
                        self.errors.push(ParseError {
                            line,
                            col,
                            message: format!(
                                "scene '{}': unexpected '{}' (expected track, auto, tempo, or <override> = value)",
                                scene.name, target
                            ),
                        });
                    }
                }
                _ => {
                    self.advance();
                }
            }
        }

        self.expect(&Token::RBrace);
        scenes.push(scene);
    }
}
