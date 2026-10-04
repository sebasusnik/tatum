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
    pub(super) fn parse_midi(&mut self, maps: &mut Vec<MidiMapDef>, perform: &mut PerformSetup) {
        if !self.expect(&Token::LBrace) {
            return;
        }
        let mut block: Vec<MidiMapDef> = Vec::new();
        let mut zones: Vec<ZoneDef> = Vec::new();
        loop {
            self.skip_newlines();
            if self.at_block_end() {
                break;
            }
            let line = self.span().line;
            let word = match self.peek() {
                Token::Ident(w) if w == "cc" || w == "keys" || w == "pad" || w == "key" => w.clone(),
                Token::Ident(w) if w == "zone" => {
                    self.advance();
                    if let Some(z) = self.parse_zone(line) {
                        zones.retain(|o| o.kind != z.kind);
                        zones.push(z);
                    }
                    continue;
                }
                Token::Ident(w) if w == "lock" => {
                    self.advance();
                    if let Some(l) = self.parse_lock() {
                        perform.lock = Some(l);
                    }
                    continue;
                }
                Token::Ident(w)
                    if w == "page"
                        && matches!(
                            self.peek_ahead(1),
                            Token::Ident(_) | Token::Master | Token::Mix | Token::In | Token::Out
                        ) =>
                {
                    self.advance();
                    if let Some(page) = self.parse_page(line) {
                        perform.pages.retain(|p| p.name != page.name);
                        perform.pages.push(page);
                    }
                    continue;
                }
                Token::Ident(w) if w == "takeover" => {
                    self.advance();
                    let s = self.span().clone();
                    match &s.token {
                        Token::Ident(w) if w == "pickup" || w == "jump" => {
                            perform.pickup = Some(w == "pickup");
                            self.advance();
                        }
                        other => {
                            self.errors.push(ParseError {
                                line: s.line,
                                col: s.col,
                                message: format!("takeover: pickup or jump, got {}", describe_token(other)),
                            });
                            self.recover_to_line_end();
                        }
                    }
                    continue;
                }
                Token::Ident(w) if w == "bend" => {
                    self.advance();
                    if let Some(b) = self.parse_bend(line) {
                        perform.bends.retain(|o| (o.zone, o.hands) != (b.zone, b.hands));
                        perform.bends.push(b);
                    }
                    continue;
                }
                other => {
                    let s = self.span().clone();
                    self.errors.push(ParseError {
                        line: s.line, col: s.col,
                        message: format!(
                            "midi: expected `cc <number> > <target>`, `keys > <track>`, `pad <note> > <track> <drum>`, `key <note> > <action>`, `zone <triggers|bass|lead> <low>..<high>`, `lock <snap|white|off>`, `bend <bass|lead> <range>`, `page <name> {{ ... }}` or `takeover <pickup|jump>`, got {}",
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
                let what = match word.as_str() {
                    "cc" => "a controller number",
                    "key" => "a key's note",
                    _ => "a pad's note",
                };
                match self.peek().clone() {
                    Token::Number(n) if (0.0..=127.0).contains(&n) && (n as u8) as f32 == n => {
                        self.advance();
                        match word.as_str() {
                            "cc" => MidiSource::Cc(n as u8),
                            "key" => MidiSource::Key(n as u8),
                            _ => MidiSource::Pad(n as u8),
                        }
                    }
                    // `key C1 > ...`: a key by its name.
                    Token::Note(name) if word == "key" && crate::dsl::compiler::note_in_midi_range(&name) => {
                        self.advance();
                        MidiSource::Key(crate::dsl::compiler::note_name_to_midi(&name))
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
                    Token::Ident(w)
                        if (w == "cc" || w == "pad" || w == "key")
                            && matches!(self.peek_ahead(1), Token::Number(_)) =>
                    {
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
                    Token::Note(n) if matches!(source, MidiSource::Pad(_) | MidiSource::Key(_)) => n.clone(),
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
                    MidiSource::Key(_) => "what the key does, like `hold riser` or `mute bass`",
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
            if matches!(source, MidiSource::Pad(_) | MidiSource::Key(_)) && words == ["step"] {
                if let Token::Number(n) = self.peek().clone() {
                    self.advance();
                    words.push(format!("{}", n as usize));
                }
            }
            // `pad 38 > repeat 1/16`: a division, read as the fraction it is.
            if matches!(source, MidiSource::Pad(_) | MidiSource::Key(_)) && (words == ["repeat"] || words == ["gate"]) {
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
        perform.zones.retain(|z| !zones.iter().any(|n| n.kind == z.kind));
        perform.zones.extend(zones);
    }

    /// `snap`, `white` or `off`, after `lock`.
    fn parse_lock(&mut self) -> Option<crate::perform::scale::Lock> {
        let s = self.span().clone();
        let lock = match self.peek() {
            Token::Ident(w) => crate::perform::scale::Lock::from_word(w),
            _ => None,
        };
        match lock {
            Some(l) => {
                self.advance();
                Some(l)
            }
            None => {
                self.errors.push(ParseError {
                    line: s.line,
                    col: s.col,
                    message: format!(
                        "lock: snap (to the nearest note of the scale), white (the white keys are the degrees) or off, got {}",
                        describe_token(&s.token)
                    ),
                });
                self.recover_to_line_end();
                None
            }
        }
    }

    /// `lead 24st`, `lead 2deg` or `lead off`, after `bend`.
    fn parse_bend(&mut self, line: usize) -> Option<BendDef> {
        let s = self.span().clone();
        let zone = match &s.token {
            Token::Ident(w) => ZoneKind::from_word(w).filter(|z| *z != ZoneKind::Triggers),
            _ => None,
        };
        let Some(zone) = zone else {
            self.errors.push(ParseError {
                line: s.line,
                col: s.col,
                message: format!("bend: the zone it bends, bass or lead, got {}", describe_token(&s.token)),
            });
            self.recover_to_line_end();
            return None;
        };
        self.advance();
        let r = self.span().clone();
        let range = match &r.token {
            Token::Quantity(n, u) if u == "st" && (0.0..=48.0).contains(n) => Some(BendRange::Semitones(*n)),
            Token::Quantity(n, u) if u == "deg" && (1.0..=7.0).contains(n) && (*n as u8) as f32 == *n => {
                Some(BendRange::Degrees(*n as u8))
            }
            Token::Ident(w) if w == "off" => Some(BendRange::Off),
            _ => None,
        };
        match range {
            Some(range) => {
                self.advance();
                let hands = match self.peek() {
                    Token::Ident(w) if w == "always" => Some(Hands::Always),
                    Token::Ident(w) if w == "idle" => Some(Hands::Idle),
                    _ => None,
                };
                if hands.is_some() {
                    self.advance();
                }
                Some(BendDef { zone, range, hands: hands.unwrap_or(Hands::Playing(zone)), line })
            }
            None => {
                self.errors.push(ParseError {
                    line: r.line,
                    col: r.col,
                    message: format!(
                        "bend {}: semitones each way up to 48 (`24st`), degrees of the scale from 1 to 7 (`2deg`), or off; got {}",
                        zone.word(),
                        describe_token(&r.token)
                    ),
                });
                self.recover_to_line_end();
                None
            }
        }
    }

    /// `page bass { cc 74 > bass cutoff ... }`, after `page`.
    fn parse_page(&mut self, line: usize) -> Option<PageDef> {
        let name = self.expect_ident()?;
        if !self.expect(&Token::LBrace) {
            self.recover_to_line_end();
            return None;
        }
        let mut knobs = Vec::new();
        loop {
            self.skip_newlines();
            if self.at_block_end() {
                break;
            }
            let s = self.span().clone();
            let cc = match (&s.token, self.peek_ahead(1).clone()) {
                (Token::Ident(w), Token::Number(n))
                    if w == "cc" && (0.0..=127.0).contains(&n) && (n as u8) as f32 == n =>
                {
                    Some(n as u8)
                }
                _ => None,
            };
            match cc {
                Some(cc) => {
                    self.pos += 2;
                    if let Some(k) = self.parse_scene_knob(cc, s.line) {
                        knobs.push(k.map);
                    }
                }
                None => {
                    self.errors.push(ParseError {
                        line: s.line,
                        col: s.col,
                        message: format!(
                            "page {}: knob lines, like `cc 74 > bass cutoff`, got {}",
                            name,
                            describe_token(&s.token)
                        ),
                    });
                    self.recover_to_line_end();
                }
            }
        }
        self.expect(&Token::RBrace);
        Some(PageDef { name, knobs, line })
    }

    /// `wheel lead > vox vowel position 0..1` or `cc 74 idle > master dj
    /// cutoff`, inside a scene, after `wheel` or `cc <n>`: a knob line like
    /// the `midi` block's, with when it answers before the `>` (`bass`,
    /// `lead`, `idle` or `always`; nothing is `always`).
    fn parse_scene_knob(&mut self, cc: u8, line: usize) -> Option<SceneKnob> {
        let hands = match self.peek() {
            Token::Ident(w) if w == "always" => Some(Hands::Always),
            Token::Ident(w) if w == "idle" => Some(Hands::Idle),
            Token::Ident(w) => match ZoneKind::from_word(w) {
                Some(z) if z != ZoneKind::Triggers => Some(Hands::Playing(z)),
                _ => None,
            },
            _ => None,
        };
        if hands.is_some() {
            self.advance();
        }
        let hands = hands.unwrap_or(Hands::Always);
        if !self.expect(&Token::Arrow) {
            self.recover_to_line_end();
            return None;
        }
        let mut words: Vec<String> = Vec::new();
        loop {
            let word = match self.peek() {
                Token::Ident(w) => w.clone(),
                Token::Level => String::from("level"),
                Token::Pan => String::from("pan"),
                Token::Mix => String::from("mix"),
                Token::Master => String::from("master"),
                Token::Tempo => String::from("tempo"),
                _ => break,
            };
            self.advance();
            words.push(word);
        }
        if words.is_empty() {
            let s = self.span().clone();
            self.errors.push(ParseError {
                line: s.line,
                col: s.col,
                message: String::from("a knob in a scene needs a target after `>`, like `wheel > vox vowel position`"),
            });
            self.recover_to_line_end();
            return None;
        }
        let range = self.parse_knob_range();
        let map = MidiMapDef { source: MidiSource::Cc(cc), target: words.join("."), range, quantize: None, line };
        Some(SceneKnob { hands, map })
    }

    /// A note as a zone writes it: a MIDI number or a name, `48` or `C2`.
    fn parse_zone_note(&mut self) -> Option<u8> {
        let s = self.span().clone();
        let n = match &s.token {
            Token::Number(n) if (0.0..=127.0).contains(n) && (*n as u8) as f32 == *n => Some(*n as u8),
            Token::Note(name) if crate::dsl::compiler::note_in_midi_range(name) => {
                Some(crate::dsl::compiler::note_name_to_midi(name))
            }
            _ => None,
        };
        match n {
            Some(n) => {
                self.advance();
                Some(n)
            }
            None => {
                self.errors.push(ParseError {
                    line: s.line,
                    col: s.col,
                    message: format!(
                        "zone: a note is a MIDI number from 0 to 127 or a name like C2, got {}",
                        describe_token(&s.token)
                    ),
                });
                self.recover_to_line_end();
                None
            }
        }
    }

    /// `zone bass 48..59 > bass roll kick=drums`, after `zone`.
    fn parse_zone(&mut self, line: usize) -> Option<ZoneDef> {
        let s = self.span().clone();
        let kind = match &s.token {
            Token::Ident(w) => ZoneKind::from_word(w),
            _ => None,
        };
        let Some(kind) = kind else {
            self.errors.push(ParseError {
                line: s.line,
                col: s.col,
                message: format!("zone: triggers, bass or lead, got {}", describe_token(&s.token)),
            });
            self.recover_to_line_end();
            return None;
        };
        self.advance();
        let low = self.parse_zone_note()?;
        if !matches!(self.peek(), Token::Tie) {
            let s = self.span().clone();
            self.errors.push(ParseError {
                line: s.line,
                col: s.col,
                message: format!(
                    "zone: the lowest and the highest note with `..` between them, like `48..59`, got {}",
                    describe_token(&s.token)
                ),
            });
            self.recover_to_line_end();
            return None;
        }
        self.advance();
        let high = self.parse_zone_note()?;
        if high < low {
            self.errors.push(ParseError {
                line: s.line,
                col: s.col,
                message: format!("zone {}: {}..{} runs downwards; write the lowest note first", kind.word(), low, high),
            });
        }
        let play = if matches!(self.peek(), Token::Arrow) {
            self.advance();
            Some(self.parse_zone_play(kind)?)
        } else {
            None
        };
        Some(ZoneDef { kind, low: low.min(high), high: high.max(low), play, line })
    }

    /// `bass roll kick=drums` after a zone's `>`: the track, then how.
    fn parse_zone_play(&mut self, kind: ZoneKind) -> Option<ZonePlay> {
        let s = self.span().clone();
        let track = match &s.token {
            Token::Ident(w) => w.clone(),
            _ => {
                self.errors.push(ParseError {
                    line: s.line,
                    col: s.col,
                    message: format!("{}: the track the zone plays, got {}", kind.word(), describe_token(&s.token)),
                });
                self.recover_to_line_end();
                return None;
            }
        };
        self.advance();
        let mut play = ZonePlay { track, roll: false, kick: None, octave: None };
        loop {
            let s = self.span().clone();
            match &s.token {
                Token::Ident(w) if w == "roll" || w == "notes" => {
                    play.roll = w == "roll";
                    self.advance();
                }
                Token::Ident(w) if w == "octave" => {
                    self.advance();
                    let negative = matches!(self.peek(), Token::Rest);
                    if negative {
                        self.advance();
                    }
                    match self.peek().clone() {
                        Token::Number(n) if (0.0..=8.0).contains(&n) && (n as i8) as f32 == n => {
                            self.advance();
                            play.octave = Some(if negative { -(n as i8) } else { n as i8 });
                        }
                        other => {
                            let s = self.span().clone();
                            self.errors.push(ParseError {
                                line: s.line,
                                col: s.col,
                                message: format!(
                                    "octave: the octave the zone's root plays in, a whole number like 1, got {}",
                                    describe_token(&other)
                                ),
                            });
                            self.recover_to_line_end();
                            return None;
                        }
                    }
                }
                Token::Ident(w) if w == "kick" && matches!(self.peek_ahead(1), Token::Eq) => {
                    self.pos += 2;
                    match self.peek().clone() {
                        Token::Ident(t) => {
                            self.advance();
                            play.kick = Some(t);
                        }
                        other => {
                            let s = self.span().clone();
                            self.errors.push(ParseError {
                                line: s.line,
                                col: s.col,
                                message: format!(
                                    "kick=: the drum track whose kick a roll strikes, got {}",
                                    describe_token(&other)
                                ),
                            });
                            self.recover_to_line_end();
                            return None;
                        }
                    }
                }
                Token::Newline | Token::Eof | Token::RBrace => break,
                // The next mapping on the same line.
                Token::Ident(w) if matches!(w.as_str(), "cc" | "pad" | "key" | "keys" | "zone" | "lock") => break,
                other => {
                    self.errors.push(ParseError {
                        line: s.line,
                        col: s.col,
                        message: format!(
                            "{}: after the track, `roll` or `notes`, `kick=<track>` and `octave <n>`; got {}",
                            kind.word(),
                            describe_token(other)
                        ),
                    });
                    self.recover_to_line_end();
                    return None;
                }
            }
        }
        if play.kick.is_some() && !play.roll {
            self.errors.push(ParseError {
                line: s.line,
                col: s.col,
                message: String::from("kick= goes with `roll`: it is the kick on the beat the roll leaves for it"),
            });
        }
        if play.roll && kind != ZoneKind::Bass {
            self.errors.push(ParseError {
                line: s.line,
                col: s.col,
                message: String::from("`roll` is for the bass zone"),
            });
        }
        Some(play)
    }

    // ── Performance scenes ──
    // perform drop {
    //     scale E phrygian_dominant      the zones keep to this; with none, the song's
    //     lock white                     snap, white or off
    //     bass > bass roll kick=drums    what the bass zone plays, and how
    //     lead > zapper                  what the lead zone plays
    //     set reverb_mix 20%             a value put in place as the scene comes in
    // }

    /// After `perform`. A later block with the same name replaces it.
    pub(super) fn parse_perform(&mut self, setup: &mut PerformSetup) {
        let line = self.span().line;
        let Some(name) = self.expect_ident() else {
            self.recover_to_line_end();
            return;
        };
        if !self.expect(&Token::LBrace) {
            return;
        }
        let mut def = PerformDef {
            name,
            scale: None,
            lock: None,
            bass: None,
            lead: None,
            sets: Vec::new(),
            bends: Vec::new(),
            knobs: Vec::new(),
            line,
        };
        // A scene's knob lines: several on one controller make a macro.
        let mut knobs: Vec<SceneKnob> = Vec::new();
        loop {
            self.skip_newlines();
            if self.at_block_end() {
                break;
            }
            let s = self.span().clone();
            match s.token.clone() {
                Token::Scale => {
                    self.advance();
                    let root = self.expect_ident();
                    let kind = self.expect_ident();
                    if let (Some(root), Some(kind)) = (root, kind) {
                        def.scale = Some(ScaleDef { root, kind });
                    } else {
                        self.recover_to_line_end();
                    }
                }
                Token::Ident(w) if w == "lock" => {
                    self.advance();
                    def.lock = self.parse_lock();
                }
                Token::Ident(w) if (w == "bass" || w == "lead") && matches!(self.peek_ahead(1), Token::Arrow) => {
                    self.pos += 2;
                    let kind = if w == "bass" { ZoneKind::Bass } else { ZoneKind::Lead };
                    let play = self.parse_zone_play(kind);
                    if kind == ZoneKind::Bass {
                        def.bass = play;
                    } else {
                        def.lead = play;
                    }
                }
                Token::Ident(w) if w == "bend" => {
                    self.advance();
                    if let Some(b) = self.parse_bend(s.line) {
                        def.bends.retain(|o| (o.zone, o.hands) != (b.zone, b.hands));
                        def.bends.push(b);
                    }
                }
                Token::Ident(w) if w == "wheel" => {
                    self.advance();
                    knobs.extend(self.parse_scene_knob(1, s.line));
                }
                Token::Ident(w) if w == "cc" => {
                    self.advance();
                    match self.peek().clone() {
                        Token::Number(n) if (0.0..=127.0).contains(&n) && (n as u8) as f32 == n => {
                            self.advance();
                            knobs.extend(self.parse_scene_knob(n as u8, s.line));
                        }
                        other => {
                            let e = self.span().clone();
                            self.errors.push(ParseError {
                                line: e.line,
                                col: e.col,
                                message: format!(
                                    "cc: a controller number from 0 to 127, got {}",
                                    describe_token(&other)
                                ),
                            });
                            self.recover_to_line_end();
                        }
                    }
                }
                Token::Ident(w) if w == "set" => {
                    self.advance();
                    let mut words: Vec<String> = Vec::new();
                    loop {
                        let word = match self.peek() {
                            Token::Ident(w) => w.clone(),
                            Token::Level => String::from("level"),
                            Token::Pan => String::from("pan"),
                            Token::Mix => String::from("mix"),
                            Token::Master => String::from("master"),
                            Token::Tempo => String::from("tempo"),
                            _ => break,
                        };
                        self.advance();
                        words.push(word);
                    }
                    if words.is_empty() {
                        self.errors.push(ParseError {
                            line: s.line,
                            col: s.col,
                            message: String::from("set: a target and a value, like `set reverb_mix 20%`"),
                        });
                        self.recover_to_line_end();
                        continue;
                    }
                    if let Some(value) = self.parse_range_end() {
                        def.sets.retain(|p| p.target != words.join("."));
                        def.sets.push(PerformSet { target: words.join("."), value, line: s.line });
                    }
                }
                other => {
                    self.errors.push(ParseError {
                        line: s.line,
                        col: s.col,
                        message: format!(
                            "perform: scale, lock, `bass > <track>`, `lead > <track>`, `set <target> <value>`, `bend <zone> <range>`, `wheel > <target>` or `cc <n> > <target>`, got {}",
                            describe_token(&other)
                        ),
                    });
                    self.recover_to_line_end();
                }
            }
        }
        self.expect(&Token::RBrace);
        def.knobs = knobs;
        setup.scenes.retain(|o| o.name != def.name);
        setup.scenes.push(def);
    }

    // ── The computer's keys ──
    // keyboard {
    //     f1 > perform intro     a scene, on the next bar (or what `quantize` says)
    //     space > next           the set: next, prev, step 3
    //     quantize bar           bar, phrase (8 bars) or a number of bars
    // }

    /// After `keyboard`. A later binding of the same key replaces it.
    pub(super) fn parse_keyboard(&mut self, setup: &mut PerformSetup) {
        if !self.expect(&Token::LBrace) {
            return;
        }
        loop {
            self.skip_newlines();
            if self.at_block_end() {
                break;
            }
            let s = self.span().clone();
            if matches!(&s.token, Token::Ident(w) if w == "quantize") && !matches!(self.peek_ahead(1), Token::Arrow) {
                self.advance();
                let q = self.span().clone();
                let bars = match &q.token {
                    Token::Ident(w) if w == "bar" => Some(1),
                    Token::Ident(w) if w == "phrase" => Some(8),
                    Token::Number(n) if *n >= 1.0 && (*n as u32) as f32 == *n => Some(*n as u32),
                    _ => None,
                };
                match bars {
                    Some(b) => {
                        self.advance();
                        setup.scene_bars = Some(b);
                    }
                    None => {
                        self.errors.push(ParseError {
                            line: q.line,
                            col: q.col,
                            message: format!(
                                "quantize: bar, phrase (8 bars) or a whole number of bars, got {}",
                                describe_token(&q.token)
                            ),
                        });
                        self.recover_to_line_end();
                    }
                }
                continue;
            }
            let key = match &s.token {
                Token::Ident(w) => Some(w.to_ascii_lowercase()),
                Token::Note(n) => Some(n.to_ascii_lowercase()),
                Token::Number(n) if (0.0..=9.0).contains(n) && (*n as u8) as f32 == *n => Some(format!("{}", *n as u8)),
                Token::DrumHit | Token::DrumAccent => Some(String::from("x")),
                Token::DrumGhost(c) => Some(String::from(*c)),
                Token::Play => Some(String::from("play")),
                _ => None,
            };
            let Some(key) = key.filter(|k| crate::perform::key_name_ok(k)) else {
                self.errors.push(ParseError {
                    line: s.line,
                    col: s.col,
                    message: format!(
                        "keyboard: a key is a letter, a digit, f1..f12, space, tab, enter, left, right, up or down; got {}",
                        describe_token(&s.token)
                    ),
                });
                self.recover_to_line_end();
                continue;
            };
            self.advance();
            if !self.expect(&Token::Arrow) {
                self.recover_to_line_end();
                continue;
            }
            let a = self.span().clone();
            let action = match &a.token {
                Token::Ident(w) if w == "perform" => {
                    self.advance();
                    self.expect_ident().map(KeyAction::Perform)
                }
                Token::Ident(w) if w == "voice" => {
                    self.advance();
                    match self.peek().clone() {
                        Token::Ident(w) if w == "next" => {
                            self.advance();
                            Some(KeyAction::Voice(VoiceMove::Next))
                        }
                        Token::Ident(w) if w == "prev" => {
                            self.advance();
                            Some(KeyAction::Voice(VoiceMove::Prev))
                        }
                        _ => self.expect_ident().map(|n| KeyAction::Voice(VoiceMove::To(n))),
                    }
                }
                Token::Ident(w) if w == "next" => {
                    self.advance();
                    Some(KeyAction::Next)
                }
                Token::Ident(w) if w == "prev" => {
                    self.advance();
                    Some(KeyAction::Prev)
                }
                Token::Ident(w) if w == "step" => {
                    self.advance();
                    match self.peek().clone() {
                        Token::Number(n) if n >= 1.0 && (n as usize) as f32 == n => {
                            self.advance();
                            Some(KeyAction::Step(n as usize))
                        }
                        _ => None,
                    }
                }
                _ => None,
            };
            match action {
                Some(action) => {
                    setup.keyboard.retain(|b| b.key != key);
                    setup.keyboard.push(KeyBinding { key, action, line: s.line });
                }
                None => {
                    self.errors.push(ParseError {
                        line: a.line,
                        col: a.col,
                        message: format!(
                            "keyboard: a key does `perform <scene>`, `voice next|prev|<page>`, `next`, `prev` or `step <n>`; got {}",
                            describe_token(&a.token)
                        ),
                    });
                    self.recover_to_line_end();
                }
            }
        }
        self.expect(&Token::RBrace);
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
        if !matches!(source, MidiSource::Pad(_) | MidiSource::Key(_)) {
            err(self, String::from("midi: q= quantizes a pad or a trigger key; a knob or the keys act at once"));
        } else if matches!(
            first,
            Some("repeat" | "next" | "prev" | "step" | "tapestop" | "gate" | "crush" | "scream" | "cut" | "sweep")
        ) {
            err(self, format!(
                "midi: `{}` keeps its own time (a repeat waits for its division, a set move for the phrase, an effect acts at once); it takes no q=",
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
