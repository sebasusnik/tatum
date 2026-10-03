//! `instrument` blocks: the chain of nodes an instrument is built from, the
//! nodes defined inline in it, and the parameter list every node takes.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;

use crate::dsl::ast::*;
use crate::dsl::error::ParseError;
use crate::dsl::lexer::Token;

use super::Parser;

impl Parser {
    // ── Instrument ──

    pub(super) fn parse_instrument(&mut self, instruments: &mut Vec<InstrumentDef>) {
        let line = Line(self.span().line);
        let name = match self.expect_ident() {
            Some(n) => n,
            None => return,
        };
        if !self.expect(&Token::LBrace) {
            return;
        }

        let mut inst = InstrumentDef { name, line, gain: None, nodes: Vec::new(), connections: Vec::new() };

        loop {
            self.skip_newlines();
            if self.at_block_end() {
                break;
            }

            // Check for instrument-level "gain <number>" property
            if let Token::Ident(ref word) = self.peek().clone() {
                if word == "gain" {
                    let saved = self.pos;
                    self.advance();
                    if let Token::Number(_) = self.peek() {
                        inst.gain = self.expect_number();
                        continue;
                    } else {
                        self.pos = saved;
                    }
                }
            }

            // Each line inside instrument is either:
            // 1. A node declaration + chain: "osc saw(55) as osc1"
            // 2. A connection chain: "osc1 > mix > lowpass(900) > out"
            self.parse_instrument_line(&mut inst);
        }

        self.expect(&Token::RBrace);
        instruments.push(inst);
    }

    fn parse_instrument_line(&mut self, inst: &mut InstrumentDef) {
        // Collect a chain of nodes/references separated by >
        // Each element is either:
        //   - A reference to existing node (identifier)
        //   - An inline node definition: kind(params) as alias
        let mut chain: Vec<ChainElement> = Vec::new();
        self.skip_newlines();
        let line = Line(self.span().line);

        loop {
            self.skip_newlines();
            if self.at_block_end() || matches!(self.peek(), Token::Newline) {
                break;
            }

            let elem = self.parse_chain_element();
            chain.push(elem);

            // Check for > (chain continues)
            self.skip_newlines();
            if matches!(self.peek(), Token::Arrow) {
                self.advance();
            } else {
                break;
            }
        }

        // Process chain: create NodeDefs and ConnectionDefs
        let mut prev_name: Option<String> = None;
        for elem in chain {
            match elem {
                ChainElement::NodeDef(mut node_def) => {
                    // If the node has no alias, generate one
                    if node_def.alias.is_none() {
                        node_def.alias = Some(format!("_anon_{}", inst.nodes.len()));
                    }
                    let name = node_def.alias.clone().unwrap();
                    inst.nodes.push(node_def);
                    if let Some(ref prev) = prev_name {
                        inst.connections.push(ConnectionDef { from: prev.clone(), to: name.clone(), line });
                    }
                    prev_name = Some(name);
                }
                ChainElement::Ref(name) => {
                    if let Some(ref prev) = prev_name {
                        inst.connections.push(ConnectionDef { from: prev.clone(), to: name.clone(), line });
                    }
                    prev_name = Some(name);
                }
            }
        }
    }

    fn parse_chain_element(&mut self) -> ChainElement {
        match self.peek().clone() {
            // Special built-in nodes
            Token::Out => {
                self.advance();
                ChainElement::Ref(String::from("out"))
            }
            Token::In => {
                self.advance();
                ChainElement::Ref(String::from("in"))
            }
            Token::Mix => {
                self.advance();
                // "mix" could be a reference or a node def (if followed by params)
                ChainElement::Ref(String::from("mix"))
            }
            Token::Master => {
                self.advance();
                ChainElement::Ref(String::from("master"))
            }

            // Identifier: could be a reference to existing node, or a DSP keyword
            Token::Ident(ref word) => {
                let word = word.clone();
                // Check if this is a DSP keyword (followed by parentheses = node creation)
                let is_dsp_keyword = is_dsp_keyword(&word);

                if is_dsp_keyword {
                    self.parse_inline_node_def(word)
                } else {
                    // Just a reference
                    self.advance();
                    ChainElement::Ref(word)
                }
            }

            _ => {
                let s = self.span().clone();
                self.errors.push(ParseError {
                    line: s.line,
                    col: s.col,
                    message: format!("expected node or identifier in chain, got {:?}", s.token),
                });
                self.advance();
                ChainElement::Ref(String::from("_error"))
            }
        }
    }

    fn parse_inline_node_def(&mut self, kind: String) -> ChainElement {
        let line = Line(self.span().line);
        self.advance(); // consume the keyword

        let mut params = Vec::new();

        // Parse optional waveform argument for osc (before parentheses)
        if kind == "osc" || kind == "fixosc" || kind == "pitch_osc" {
            self.skip_newlines();
            if let Token::Ident(ref wf) = self.peek().clone() {
                if is_waveform(wf) {
                    let wf = wf.clone();
                    self.advance();
                    params.push(Param::Waveform(wf));
                }
            }
        }

        // Parse optional (params)
        if matches!(self.peek(), Token::LParen) {
            self.advance();
            self.parse_param_list_for(&mut params, &kind);
            self.expect(&Token::RParen);
        }

        // Parse optional "as alias"
        let alias = if matches!(self.peek(), Token::As) {
            self.advance();
            self.expect_ident()
        } else {
            None
        };

        let node_def = NodeDef { kind, alias, params, line };
        ChainElement::NodeDef(node_def)
    }

    pub(super) fn parse_param_list_for(&mut self, params: &mut Vec<Param>, kind: &str) {
        let mut positional = 0usize;
        loop {
            self.skip_newlines();
            if matches!(self.peek(), Token::RParen | Token::Eof) {
                break;
            }

            // Keywords that are also option names (`mix=0.5`, `level=`)
            let keyword_name = match self.peek() {
                Token::Mix => Some("mix"),
                Token::Level => Some("level"),
                Token::Pan => Some("pan"),
                Token::Velocity => Some("velocity"),
                _ => None,
            };
            if let Some(kn) = keyword_name {
                let saved_pos = self.pos;
                self.advance();
                if matches!(self.peek(), Token::Eq) {
                    self.advance();
                    if let Some(val) = self.named_arg_value(kind, kn) {
                        params.push(Param::Named(String::from(kn), val));
                    }
                    continue;
                }
                self.pos = saved_pos;
            }
            // Check for named param: name=value
            if let Token::Ident(name) = self.peek().clone() {
                let saved_pos = self.pos;
                self.advance();
                if matches!(self.peek(), Token::Eq) {
                    self.advance();
                    if let Some(val) = self.named_arg_value(kind, &name) {
                        params.push(Param::Named(name, val));
                    }
                } else {
                    // Not named — restore and parse as positional
                    self.pos = saved_pos;
                    if is_waveform(&name) || is_vowel(&name) || is_filter_mode(&name) {
                        self.advance();
                        params.push(Param::Waveform(name));
                    } else {
                        let s = self.span();
                        let (l, c) = (s.line, s.col);
                        self.errors.push(ParseError {
                            line: l,
                            col: c,
                            message: format!(
                                "unexpected '{}' in arguments (use name=value, a number, or a waveform word)",
                                name
                            ),
                        });
                        self.advance();
                    }
                }
            } else if matches!(self.peek(), Token::DrumGhost(_)) {
                // `o` lexes as a ghost hit; inside arguments it is the vowel word
                self.advance();
                params.push(Param::Waveform(String::from("o")));
            } else if matches!(self.peek(), Token::DrumHit | Token::DrumAccent) {
                let s = self.span();
                let (l, c) = (s.line, s.col);
                self.errors.push(ParseError {
                    line: l,
                    col: c,
                    message: String::from(
                        "unexpected 'x' in arguments (use name=value, a number, a waveform word or a vowel)",
                    ),
                });
                self.advance();
            } else if let Token::Quantity(raw, suffix) = self.peek().clone() {
                let sp = self.span();
                let (l, c) = (sp.line, sp.col);
                self.advance();
                match self.resolve_arg_quantity(kind, None, positional, raw, &suffix) {
                    Ok(v) => params.push(Param::Float(v)),
                    Err(message) => self.errors.push(ParseError { line: l, col: c, message }),
                }
                positional += 1;
                if matches!(self.peek(), Token::Comma) {
                    self.advance();
                }
                continue;
            } else if let Token::Number(_) = self.peek() {
                positional += 1;
                // Could be a number or a rhythm division (1/4)
                let n = self.expect_number().unwrap_or(0.0);
                if matches!(self.peek(), Token::Slash) {
                    self.advance();
                    let den = self.expect_number().unwrap_or(4.0);
                    params.push(Param::RhythmDiv(n as u8, den as u8));
                } else if matches!(self.peek(), Token::Star) {
                    // Expression: n * m
                    self.advance();
                    let m = self.expect_number().unwrap_or(1.0);
                    params.push(Param::Expr(Expr::Mul(Box::new(Expr::Num(n)), Box::new(Expr::Num(m)))));
                } else {
                    params.push(Param::Float(n));
                }
            } else if matches!(self.peek(), Token::Rest) {
                // Negative number: - followed by number
                self.advance();
                if let Token::Number(n) = self.peek() {
                    let n = *n;
                    self.advance();
                    positional += 1;
                    params.push(Param::Float(-n));
                } else if let Token::Quantity(raw, suffix) = self.peek().clone() {
                    let sp = self.span();
                    let (l, c) = (sp.line, sp.col);
                    self.advance();
                    match self.resolve_arg_quantity(kind, None, positional, -raw, &suffix) {
                        Ok(v) => params.push(Param::Float(v)),
                        Err(message) => self.errors.push(ParseError { line: l, col: c, message }),
                    }
                    positional += 1;
                }
            } else {
                break;
            }

            // Optional comma
            if matches!(self.peek(), Token::Comma) {
                self.advance();
            }
        }
    }
}

// ── Helper enums ──

enum ChainElement {
    NodeDef(NodeDef),
    Ref(String),
}

/// Check if a word is a DSP keyword (node type that can be instantiated).
fn is_dsp_keyword(word: &str) -> bool {
    matches!(
        word,
        "osc"
            | "fixosc"
            | "pitch_osc"
            | "noise"
            | "lfo"
            | "adsr"
            | "perc"
            | "lowpass"
            | "highpass"
            | "bandpass"
            | "ladder"
            | "gain"
            | "saturate"
            | "drive"
            | "chorus"
            | "bitcrush"
            | "tapestop"
            | "delay"
            | "reverb"
            | "compressor"
            | "limiter"
            | "tilt"
            | "eq"
    )
}

/// Vowel words accepted by the `vowel` node.
fn is_vowel(word: &str) -> bool {
    matches!(word, "a" | "e" | "i" | "o" | "u")
}

/// Check if a word is a waveform name.
fn is_waveform(word: &str) -> bool {
    matches!(word, "sine" | "saw" | "square" | "triangle" | "pulse")
}

/// Filter modes, for the nodes that take one as a bare word (`autowah`).
fn is_filter_mode(word: &str) -> bool {
    matches!(word, "lowpass" | "bandpass" | "highpass" | "lp" | "bp" | "hp")
}
