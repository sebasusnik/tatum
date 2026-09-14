extern crate alloc;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;

use crate::dsl::ast::*;
use crate::math;
use crate::dsl::error::ParseError;
use crate::params::{self, ModuleKind};
use crate::dsl::lexer::{Token, Span};

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

    fn expect_ident(&mut self) -> Option<String> {
        self.skip_newlines();
        match self.peek().clone() {
            Token::Ident(s) => { let s = s.clone(); self.advance(); Some(s) }
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
            Token::DrumHit | Token::DrumAccent | Token::DrumGhost => {
                let name = match self.peek() {
                    Token::DrumHit => String::from("x"),
                    Token::DrumAccent => String::from("X"),
                    _ => String::from("o"),
                };
                self.advance();
                Some(name)
            }
            _ => {
                let s = self.span();
                self.errors.push(ParseError {
                    line: s.line, col: s.col,
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
                line: s.line, col: s.col,
                message: format!("expected number, got {:?}", self.peek()),
            });
            None
        }
    }

    /// A named argument's value, which may carry a unit (`cutoff=2khz`).
    fn named_arg_value(&mut self, kind: &str, name: &str) -> Option<f32> {
        if let Token::Quantity(raw, suffix) = self.peek().clone() {
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
        self.parse_expr_value()
    }

    fn resolve_arg_quantity(&self, kind: &str, name: Option<&str>, index: usize, raw: f32, suffix: &str)
        -> Result<f32, String>
    {
        match crate::nodes::arg_at(kind, name, index) {
            Some(arg) => arg.value_from_quantity(raw, suffix),
            None => Err(format!(
                "'{}{}' carries a unit, but {} has no argument there to give it meaning",
                raw, suffix,
                if kind.is_empty() { "this" } else { kind }
            )),
        }
    }

    /// Optional `*N` after a tie or rest: `..*15` is fifteen ties.
    fn repeat_count(&mut self) -> usize {
        if !matches!(self.peek(), Token::Star) { return 1; }
        self.advance();
        match self.peek().clone() {
            Token::Number(n) if n >= 1.0 && n <= 256.0 => { self.advance(); n as usize }
            _ => {
                let s = self.span();
                let (l, c) = (s.line, s.col);
                self.errors.push(ParseError { line: l, col: c, message: String::from("expected a count 1..256 after '*'") });
                1
            }
        }
    }

    fn at_block_end(&self) -> bool {
        matches!(self.peek(), Token::RBrace | Token::Eof)
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
        };

        loop {
            self.skip_newlines();
            match self.peek().clone() {
                Token::Eof => break,
                Token::Tempo => { self.advance(); self.parse_tempo(&mut song.globals); }
                Token::Meter => { self.advance(); self.parse_meter(&mut song.globals); }
                Token::Scale => { self.advance(); self.parse_scale(&mut song.globals); }
                Token::Sidechain => { self.advance(); self.parse_sidechain(&mut song.globals); }
                Token::Swing => { self.advance(); self.parse_swing(&mut song.globals); }
                Token::Humanize => { self.advance(); self.parse_humanize(&mut song.globals); }
                Token::Bus => { self.advance(); self.parse_bus_decl(&mut song.buses); }
                Token::Instrument => { self.advance(); self.parse_instrument(&mut song.instruments); }
                Token::Module => { self.advance(); self.parse_module_def(&mut song.module_defs); }
                Token::Pattern => { self.advance(); self.parse_pattern_or_scene(&mut song); }
                Token::Track => { self.advance(); self.parse_track(&mut song.tracks); }
                Token::Master => { self.advance(); self.parse_master(&mut song); }
                Token::Arrange => { self.advance(); self.parse_arrange(&mut song.arrangement); }
                Token::Scene => { self.advance(); self.parse_scene(&mut song.scenes); }
                // A bare identifier followed by { could be a bus chain or groove block
                Token::Ident(ref name) if name == "gain_comp" => {
                    self.advance();
                    song.globals.gain_comp = self.expect_number().map(|v| v.clamp(0.0, 1.0));
                }
                Token::Ident(ref name) if name == "groove" => {
                    self.advance();
                    self.parse_groove(&mut song.grooves);
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
                        line: s.line, col: s.col,
                        message: format!("unexpected {} at top level", describe_token(&s.token)),
                    });
                    self.recover_to_line_end();
                }
            }
        }

        if self.errors.is_empty() {
            Ok(song)
        } else {
            Err(self.errors)
        }
    }

    // ── Globals ──

    fn parse_tempo(&mut self, globals: &mut Globals) {
        if let Some(n) = self.expect_number() {
            globals.tempo = n;
        }
    }

    fn parse_meter(&mut self, globals: &mut Globals) {
        if let Some(num) = self.expect_number() {
            self.expect(&Token::Slash);
            if let Some(den) = self.expect_number() {
                globals.meter = (num as u8, den as u8);
            }
        }
    }

    fn parse_scale(&mut self, globals: &mut Globals) {
        if let Some(root) = self.expect_ident() {
            if let Some(kind) = self.expect_ident() {
                globals.scale = Some(ScaleDef { root, kind });
            }
        }
    }

    fn parse_sidechain(&mut self, globals: &mut Globals) {
        if let Some(n) = self.expect_number() {
            globals.sidechain = n;
        }
    }

    fn parse_swing(&mut self, globals: &mut Globals) {
        if let Some(n) = self.expect_number() {
            globals.swing = Some(n);
        }
    }

    fn parse_humanize(&mut self, globals: &mut Globals) {
        // humanize 0.5           → velocity humanization only
        // humanize 0.5 timing 0.3 → velocity + timing humanization
        if let Some(n) = self.expect_number() {
            globals.humanize = Some(n);
            // Check for optional "timing" sub-keyword
            self.skip_newlines();
            if let Token::Ident(ref w) = self.peek().clone() {
                if w == "timing" {
                    self.advance();
                    if let Some(t) = self.expect_number() {
                        globals.humanize_timing = Some(t);
                    }
                }
            }
        }
    }

    // ── Bus declaration ──

    fn parse_bus_decl(&mut self, buses: &mut Vec<BusDef>) {
        if let Some(name) = self.expect_ident() {
            buses.push(BusDef { name });
        }
    }

    // ── Instrument ──

    fn parse_instrument(&mut self, instruments: &mut Vec<InstrumentDef>) {
        let name = match self.expect_ident() {
            Some(n) => n,
            None => return,
        };
        if !self.expect(&Token::LBrace) { return; }

        let mut inst = InstrumentDef {
            name,
            gain: None,
            nodes: Vec::new(),
            connections: Vec::new(),
        };

        loop {
            self.skip_newlines();
            if self.at_block_end() { break; }

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

        loop {
            self.skip_newlines();
            if self.at_block_end() || matches!(self.peek(), Token::Newline) { break; }

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
                        inst.connections.push(ConnectionDef {
                            from: prev.clone(),
                            to: name.clone(),
                        });
                    }
                    prev_name = Some(name);
                }
                ChainElement::Ref(name) => {
                    if let Some(ref prev) = prev_name {
                        inst.connections.push(ConnectionDef {
                            from: prev.clone(),
                            to: name.clone(),
                        });
                    }
                    prev_name = Some(name);
                }
            }
        }
    }

    fn parse_chain_element(&mut self) -> ChainElement {
        match self.peek().clone() {
            // Special built-in nodes
            Token::Out => { self.advance(); ChainElement::Ref(String::from("out")) }
            Token::In => { self.advance(); ChainElement::Ref(String::from("in")) }
            Token::Mix => {
                self.advance();
                // "mix" could be a reference or a node def (if followed by params)
                ChainElement::Ref(String::from("mix"))
            }
            Token::Master => { self.advance(); ChainElement::Ref(String::from("master")) }

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
                    line: s.line, col: s.col,
                    message: format!("expected node or identifier in chain, got {:?}", s.token),
                });
                self.advance();
                ChainElement::Ref(String::from("_error"))
            }
        }
    }

    fn parse_inline_node_def(&mut self, kind: String) -> ChainElement {
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

        let node_def = NodeDef { kind, alias, params };
        ChainElement::NodeDef(node_def)
    }

    fn parse_param_list_for(&mut self, params: &mut Vec<Param>, kind: &str) {
        let mut positional = 0usize;
        loop {
            self.skip_newlines();
            if matches!(self.peek(), Token::RParen | Token::Eof) { break; }

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
                    if is_waveform(&name) || is_vowel(&name) {
                        self.advance();
                        params.push(Param::Waveform(name));
                    } else {
                        let s = self.span();
                        let (l, c) = (s.line, s.col);
                        self.errors.push(ParseError {
                            line: l, col: c,
                            message: format!("unexpected '{}' in arguments (use name=value, a number, or a waveform word)", name),
                        });
                        self.advance();
                    }
                }
            } else if matches!(self.peek(), Token::DrumGhost) {
                // `o` lexes as a ghost hit; inside arguments it is the vowel word
                self.advance();
                params.push(Param::Waveform(String::from("o")));
            } else if matches!(self.peek(), Token::DrumHit | Token::DrumAccent) {
                let s = self.span();
                let (l, c) = (s.line, s.col);
                self.errors.push(ParseError {
                    line: l, col: c,
                    message: String::from("unexpected 'x' in arguments (use name=value, a number, a waveform word or a vowel)"),
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
                if matches!(self.peek(), Token::Comma) { self.advance(); }
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
                    params.push(Param::Expr(Expr::Mul(
                        Box::new(Expr::Num(n)),
                        Box::new(Expr::Num(m)),
                    )));
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

    // ── Pattern ──

    fn parse_pattern_or_scene(&mut self, song: &mut Song) {
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
                                let v = v;
                                self.advance();
                                let degree = math::floor(v) as u8;
                                let octave = ((v - math::floor(v)) * 10.0 + 0.5) as u8;
                                if degree >= 1 && degree <= 7 {
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
                Token::Note(ref n) if note_is_chord_symbol(n) || (self.peek_is_slash_next() && crate::dsl::chords::parse(n).is_some()) => {
                    let n = n.clone();
                    self.advance();
                    if let Some(step) = self.chord_symbol_step(&n) {
                        current_row.push(step);
                    }
                }
                Token::Note(ref n) => {
                    let n = n.clone();
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
                    current_row.push(Step::Note(NoteStep {
                        note: NoteRef::Absolute(n),
                        velocity,
                        plock,
                        slide: core::mem::take(&mut pending_slide),
                    }));
                }
                // Scale degree: 1.3 = degree 1, octave 3
                Token::Number(v) => {
                    let v = v;
                    self.advance();
                    let degree = math::floor(v) as u8;
                    let octave = ((v - math::floor(v)) * 10.0 + 0.5) as u8;
                    if degree >= 1 && degree <= 7 {
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
                    if !current_row.is_empty() {
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
        patterns.push(PatternDef { name, rows, lane_labels });
    }

    /// Is the token after the current one a `/`? (`E5/3` is a power chord, `E5` a note.)
    fn peek_is_slash_next(&self) -> bool {
        self.tokens.get(self.pos + 1).map_or(false, |s| matches!(s.token, Token::Slash))
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

    // ── Track ──

    fn parse_track(&mut self, tracks: &mut Vec<TrackDef>) {
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
                Token::Ident(ref word) if word == "sidechain" => {
                    self.advance();
                    if let Some(v) = self.expect_number() {
                        track.sidechain = Some(v);
                    }
                }
                Token::Ident(ref word) if word == "arp" => {
                    self.advance();
                    track.arp = self.parse_arp_clause();
                }
                _ => { self.advance(); }
            }
        }

        self.expect(&Token::RBrace);
        tracks.push(track);
    }

    /// `arp <mode> [rate=N] [gate=F] [octaves=N]` — mode is up, down, updown or off.
    fn parse_arp_clause(&mut self) -> Option<ArpDef> {
        let mode = match self.expect_ident() {
            Some(m) => m,
            None => return None,
        };
        let mut def = ArpDef { mode, rate: None, gate: None, octaves: None };
        loop {
            let key = match self.peek().clone() {
                Token::Ident(ref k) => k.clone(),
                _ => break,
            };
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

    /// The next token, looking past newlines (without consuming them).
    fn peek_past_newlines(&self) -> &Token {
        let mut i = self.pos;
        while i < self.tokens.len() && matches!(self.tokens[i].token, Token::Newline) {
            i += 1;
        }
        &self.tokens[i.min(self.tokens.len() - 1)].token
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

            routing.push(RoutingNode { kind, params });
        }
    }

    // ── Groove block ──
    // groove jungle_breaks {
    //     hat swing 0.62
    //     snare nudge 0.01
    //     kick nudge -0.005
    // }

    fn parse_groove(&mut self, grooves: &mut Vec<GrooveDef>) {
        let name = self.expect_ident().unwrap_or_else(|| String::from("default"));
        if !self.expect(&Token::LBrace) { return; }

        let mut lanes = Vec::new();
        loop {
            self.skip_newlines();
            if self.at_block_end() { break; }

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
                        _ => { self.advance(); }
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

    // ── Bus chain ──

    /// True when the tokens after the current one look like `ident =`.
    fn next_is_key_value(&self) -> bool {
        let n = self.tokens.len();
        let i = self.pos + 1;
        i + 1 < n
            && matches!(self.tokens[i].token, Token::Ident(_))
            && matches!(self.tokens[i + 1].token, Token::Eq)
    }

    /// Top-level `delay sync=dotted_eighth feedback=0.45 filter=0.5 time=0.3`
    /// or `reverb size=0.7 damp=0.4 predelay=20`.
    fn parse_send_fx_globals(&mut self, which: &str, globals: &mut Globals) {
        loop {
            let key = match self.peek().clone() {
                Token::Ident(ref k) => k.clone(),
                Token::Sidechain => String::from("sidechain"),
                _ => break,
            };
            let s = self.span();
            let (line, col) = (s.line, s.col);
            self.advance();
            if !self.expect(&Token::Eq) { break; }
            // Value: number, or an identifier for `sync`
            let ident_value = match self.peek().clone() {
                Token::Ident(ref v) => { let v = v.clone(); self.advance(); Some(v) }
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

    fn try_parse_bus_chain_or_error(&mut self, song: &mut Song) {
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
            line: s.line, col: s.col,
            message: format!("unexpected {} at top level (expected tempo, scale, module, pattern, track, scene, arrange, ...)", describe_token(&s.token)),
        });
        self.recover_to_line_end();
    }

    /// After a top-level error, skip the rest of the line so one mistake
    /// produces one diagnostic instead of one per token.
    fn recover_to_line_end(&mut self) {
        while !matches!(self.peek(), Token::Newline | Token::Eof) {
            self.pos += 1;
        }
    }

    fn parse_bus_chain_body(&mut self, bus_name: String, chains: &mut Vec<BusChainDef>) {
        let mut chain: Vec<ChainNode> = Vec::new();

        // Parse: in > effect(params) > ... > out/master
        loop {
            self.skip_newlines();
            if self.at_block_end() { break; }

            match self.peek().clone() {
                Token::In => { self.advance(); }
                Token::Arrow => { self.advance(); }
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
                    chain.push(ChainNode { kind: name, params });
                }
                _ => { self.advance(); }
            }
        }

        self.expect(&Token::RBrace);
        chains.push(BusChainDef { bus_name, chain });
    }

    // ── Master ──

    fn parse_master(&mut self, song: &mut Song) {
        if !self.expect(&Token::LBrace) { return; }
        let mut chain: Vec<ChainNode> = Vec::new();

        loop {
            self.skip_newlines();
            if self.at_block_end() { break; }

            match self.peek().clone() {
                Token::In => { self.advance(); }
                Token::Arrow => { self.advance(); }
                Token::Out => { self.advance(); }
                Token::Ident(ref name) => {
                    let name = name.clone();
                    self.advance();
                    let mut params = Vec::new();
                    if matches!(self.peek(), Token::LParen) {
                        self.advance();
                        self.parse_param_list_for(&mut params, &name);
                        self.expect(&Token::RParen);
                    }
                    chain.push(ChainNode { kind: name, params });
                }
                _ => { self.advance(); }
            }
        }

        self.expect(&Token::RBrace);
        song.master = Some(MasterDef { chain });
    }

    // ── Scene ──

    fn parse_scene(&mut self, scenes: &mut Vec<SceneDef>) {
        let name = match self.expect_ident() {
            Some(n) => n,
            None => return,
        };
        if !self.expect(&Token::LBrace) { return; }
        self.parse_scene_body(name, scenes);
    }

    fn parse_scene_body(&mut self, name: String, scenes: &mut Vec<SceneDef>) {
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
            if self.at_block_end() { break; }

            match self.peek().clone() {
                Token::Extends => {
                    self.advance();
                    if let Some(parent) = self.expect_ident() {
                        scene.extends = Some(parent);
                    }
                }
                Token::Tempo => {
                    self.advance();
                    if let Some(t) = self.expect_number() {
                        scene.tempo = Some(t);
                    }
                }
                Token::Track => {
                    self.advance();
                    self.parse_track(&mut scene.tracks);
                }
                Token::Auto => {
                    self.advance();
                    self.parse_automation(&mut scene.automations);
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
                            line, col,
                            message: format!("scene '{}': unexpected '{}' (expected track, auto, tempo, or <override> = value)", scene.name, target),
                        });
                    }
                }
                _ => { self.advance(); }
            }
        }

        self.expect(&Token::RBrace);
        scenes.push(scene);
    }

    // ── Module definition ──

    fn parse_module_def(&mut self, module_defs: &mut Vec<ModuleDef>) {
        // module <type> <name> { key value ... }
        let module_type = match self.expect_ident() {
            Some(t) => t,
            None => return,
        };
        let name = match self.expect_ident() {
            Some(n) => n,
            None => return,
        };
        if !self.expect(&Token::LBrace) { return; }

        let mut params = Vec::new();
        let mut op_envelopes = Vec::new();

        loop {
            self.skip_newlines();
            if self.at_block_end() { break; }

            // Some parameter names are also keywords elsewhere in the language
            // (`level`, `pan`, `velocity`, `mix`, `sidechain`, `swing`). Treat
            // them as plain names inside a module block.
            let keyword_param = match self.peek() {
                Token::Level => Some("level"),
                Token::Pan => Some("pan"),
                Token::Velocity => Some("velocity"),
                Token::Mix => Some("mix"),
                Token::Sidechain => Some("sidechain"),
                Token::Swing => Some("swing"),
                _ => None,
            };
            if let Some(ref key) = self.peek().clone().into_ident_or(keyword_param) {
                let key = key.clone();
                let line = self.span().line;
                self.advance();

                // Check for op_envelope shorthand: op0_envelope 0.001 0.12 0.05 0.08
                if key.starts_with("op") && key.ends_with("_envelope") {
                    let op_idx_str: String = key.chars()
                        .skip(2)
                        .take_while(|c| c.is_ascii_digit())
                        .collect();
                    if let Ok(op_idx) = op_idx_str.parse::<usize>() {
                        let a = self.expect_number().unwrap_or(0.01);
                        let d = self.expect_number().unwrap_or(0.1);
                        let s = self.expect_number().unwrap_or(0.7);
                        let r = self.expect_number().unwrap_or(0.3);
                        op_envelopes.push(OpEnvelopeDef { op_index: op_idx, a, d, s, r });
                        continue;
                    }
                }

                // Handle negative values
                let negative = if matches!(self.peek(), Token::Rest) {
                    self.advance();
                    true
                } else {
                    false
                };

                // Symbolic values for choice params: `waveform half_sine`, `voice_mode unison`
                if let Token::Ident(ref word) = self.peek().clone() {
                    let word = word.clone();
                    let spec = ModuleKind::from_str(&module_type)
                        .and_then(|k| params::lookup(k, &key));
                    match spec {
                        Some(spec) => match spec.value_from_name(&word) {
                            Some(value) => {
                                self.advance();
                                params.push(ModuleParam { name: key, value, line });
                            }
                            None => {
                                let s = self.span();
                                let (l, c) = (s.line, s.col);
                                self.errors.push(ParseError {
                                    line: l, col: c,
                                    message: format!("'{}' has no option '{}' (choices: {})", key, word, spec.range.describe()),
                                });
                                self.advance();
                            }
                        },
                        // Unknown param: let the compiler report it with a suggestion.
                        None => {
                            self.advance();
                            params.push(ModuleParam { name: key, value: 0.0, line });
                        }
                    }
                    continue;
                }

                if let Token::Quantity(raw, suffix) = self.peek().clone() {
                    let sp = self.span();
                    let (l, c) = (sp.line, sp.col);
                    self.advance();
                    let raw = if negative { -raw } else { raw };
                    let spec = ModuleKind::from_str(&module_type)
                        .and_then(|k| params::lookup(k, &key));
                    match spec {
                        Some(spec) => match spec.value_from_quantity(raw, &suffix) {
                            Ok(value) => params.push(ModuleParam { name: key, value, line }),
                            Err(message) => self.errors.push(ParseError { line: l, col: c, message }),
                        },
                        // Unknown param: let the compiler name it with a suggestion.
                        None => params.push(ModuleParam { name: key, value: raw, line }),
                    }
                    continue;
                }

                if let Some(val) = self.expect_number() {
                    let val = if negative { -val } else { val };
                    params.push(ModuleParam { name: key, value: val, line });
                }
            } else {
                let sp = self.span();
                let (l, c) = (sp.line, sp.col);
                self.errors.push(ParseError {
                    line: l, col: c,
                    message: format!("module '{}': unexpected {} (expected `name value`)", name, describe_token(self.peek())),
                });
                self.recover_to_line_end();
            }
        }

        self.expect(&Token::RBrace);
        module_defs.push(ModuleDef { module_type, name, params, op_envelopes });
    }

    // ── Automation ──

    fn parse_automation(&mut self, automations: &mut Vec<AutomationDef>) {
        // auto target val > val [> val]
        // Target can be "instrument.param" (with dot), or "reverb_mix"
        let mut target = match self.expect_ident() {
            Some(t) => t,
            None => return,
        };

        // Check for dot-separated target: name.param (lexer doesn't handle dots in idents)
        // The dot gets tokenized... actually the lexer eats '.' as part of numbers or Tie (..)
        // We need to handle "ident . ident" → "ident.ident" by checking for Number(.) after ident
        // Actually, looking at the lexer, a '.' followed by non-digit is skipped.
        // Let's check if next token gives us continuation
        loop {
            // The lexer would have produced a number starting with . if followed by digits
            // For "funk_bass.cutoff", the '.' is between two identifiers
            // Since '.' isn't handled as a single char token, it gets skipped by the lexer
            // The two parts "funk_bass" and "cutoff" come as separate Ident tokens
            // But actually, looking at lexer, identifiers include _ but not .
            // So "funk_bass.cutoff" → Ident("funk_bass"), then '.' gets skipped, then Ident("cutoff")
            // We need to peek and see if there's an unexpected ident right after (the '.' was eaten)

            // Workaround: if next token is an Ident and it's NOT a number/arrow, treat as continuation
            // Actually let's just check if the raw source has a dot by looking at column positions
            // Simplest approach: check if next token is an Ident that could be a param name
            self.skip_newlines();
            // Extract param name from Ident or keyword tokens (level, velocity, etc.)
            let param_name = match self.peek().clone() {
                Token::Ident(ref s) => Some(s.clone()),
                Token::Level => Some(alloc::string::String::from("level")),
                Token::Velocity => Some(alloc::string::String::from("velocity")),
                Token::Pan => Some(alloc::string::String::from("pan")),
                _ => None,
            };
            if let Some(part_clone) = param_name {
                // Check if this could be a dotted continuation
                // Heuristic: if the next thing after this ident is a number (the first keyframe),
                // then this ident is the param part of a dotted target
                let saved = self.pos;
                self.advance();
                if let Token::Number(_) | Token::Rest = self.peek() {
                    // This is the param part
                    target = alloc::format!("{}.{}", target, part_clone);
                    break;
                } else {
                    // Not a param part, restore
                    self.pos = saved;
                    break;
                }
            } else {
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
            if !matches!(self.peek(), Token::Arrow) { break; }
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

        if !keyframes.is_empty() {
            automations.push(AutomationDef { target, keyframes });
        }
    }

    // ── Arrange ──

    fn parse_arrange(&mut self, arrangement: &mut Vec<ArrangeEntry>) {
        if !self.expect(&Token::LBrace) { return; }

        loop {
            self.skip_newlines();
            if self.at_block_end() { break; }

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

// ── Helper enums ──

enum ChainElement {
    NodeDef(NodeDef),
    Ref(String),
}

/// Check if a word is a DSP keyword (node type that can be instantiated).
fn is_dsp_keyword(word: &str) -> bool {
    matches!(word,
        "osc" | "fixosc" | "pitch_osc" | "noise" | "lfo" |
        "adsr" | "perc" |
        "lowpass" | "highpass" | "bandpass" | "ladder" |
        "gain" |
        "saturate" | "drive" | "chorus" | "bitcrush" | "tapestop" |
        "delay" | "reverb" |
        "compressor" | "limiter" |
        "tilt" | "eq"
    )
}

impl Token {
    /// The token as a parameter name: identifiers as themselves, and the
    /// listed keywords as their word.
    fn into_ident_or(self, keyword: Option<&str>) -> Option<String> {
        match self {
            Token::Ident(s) => Some(s),
            _ => keyword.map(String::from),
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
        Token::DrumGhost => String::from("'o'"),
        Token::LBrace => String::from("'{'"),
        Token::RBrace => String::from("'}'"),
        Token::Newline => String::from("end of line"),
        Token::Eof => String::from("end of file"),
        other => format!("{:?}", other).to_lowercase(),
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

/// Vowel words accepted by the `vowel` node.
fn is_vowel(word: &str) -> bool {
    matches!(word, "a" | "e" | "i" | "o" | "u")
}

/// Check if a word is a waveform name.
fn is_waveform(word: &str) -> bool {
    matches!(word, "sine" | "saw" | "square" | "triangle" | "pulse")
}

