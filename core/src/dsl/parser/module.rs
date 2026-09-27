//! `module` definitions: a module's type and the parameters it sets, each
//! checked against what that module has and read in its unit.

use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;

use crate::dsl::ast::*;
use crate::dsl::error::ParseError;
use crate::params::{self, ModuleKind};
use crate::dsl::lexer::Token;

use super::{Parser, describe_token};

impl Parser {
    // ── Module definition ──

    pub(super) fn parse_module_def(&mut self, module_defs: &mut Vec<ModuleDef>) {
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
                                params.push(ModuleParam { name: key, value, line, bare: false });
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
                            params.push(ModuleParam { name: key, value: 0.0, line, bare: false });
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
                            Ok(value) => params.push(ModuleParam { name: key, value, line, bare: false }),
                            Err(message) => self.errors.push(ParseError { line: l, col: c, message }),
                        },
                        // Unknown param: let the compiler name it with a suggestion.
                        None => params.push(ModuleParam { name: key, value: raw, line, bare: false }),
                    }
                    continue;
                }

                if let Some(val) = self.expect_number() {
                    let val = if negative { -val } else { val };
                    params.push(ModuleParam { name: key, value: val, line, bare: true });
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
        let def = ModuleDef { module_type, name, params, op_envelopes };
        match module_defs.iter().position(|m| m.name == def.name) {
            Some(i) => {
                // Parameters the redefinition does not name keep their value.
                let mut merged = module_defs[i].clone();
                merged.module_type = def.module_type;
                for np in def.params {
                    match merged.params.iter_mut().find(|p| p.name == np.name) {
                        Some(old) => old.value = np.value,
                        None => merged.params.push(np),
                    }
                }
                if !def.op_envelopes.is_empty() { merged.op_envelopes = def.op_envelopes; }
                module_defs[i] = merged;
            }
            None => module_defs.push(def),
        }
    }
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
