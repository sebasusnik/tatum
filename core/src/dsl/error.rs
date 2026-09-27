extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use alloc::format;

#[derive(Debug, Clone)]
pub struct ParseError {
    pub line: usize,
    pub col: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}:{}: {}", self.line, self.col, self.message)
    }
}

#[derive(Debug, Clone)]
pub struct CompileError {
    pub message: String,
    /// Source line (1-based) when known, 0 otherwise.
    pub line: usize,
}

impl CompileError {
    pub fn new(message: String) -> Self {
        Self { message, line: 0 }
    }

    pub fn at(line: usize, message: String) -> Self {
        Self { message, line }
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line > 0 {
            write!(f, "line {}: {}", self.line, self.message)
        } else {
            write!(f, "{}", self.message)
        }
    }
}

pub type ParseResult<T> = Result<T, Vec<ParseError>>;
pub type CompileResult<T> = Result<T, Vec<CompileError>>;

/// Structured error from DSL parsing or compilation, preserving line/col info.
#[derive(Debug, Clone)]
pub enum DslError {
    Parse(Vec<ParseError>),
    Compile(Vec<CompileError>),
}

impl DslError {
    /// Serialize to a JSON string for WASM → JS communication.
    pub fn to_json(&self) -> String {
        let mut out = String::from(r#"{"ok":false,"errors":["#);
        match self {
            DslError::Parse(errs) => {
                for (i, e) in errs.iter().enumerate() {
                    if i > 0 { out.push(','); }
                    out.push_str(&format!(
                        r#"{{"line":{},"col":{},"msg":"{}"}}"#,
                        e.line, e.col, json_escape(&e.message),
                    ));
                }
            }
            DslError::Compile(errs) => {
                for (i, e) in errs.iter().enumerate() {
                    if i > 0 { out.push(','); }
                    out.push_str(&format!(
                        r#"{{"line":{},"col":0,"msg":"{}"}}"#,
                        e.line, json_escape(&e.message),
                    ));
                }
            }
        }
        out.push_str("]}");
        out
    }
}

/// Escape `s` for the inside of a JSON string.
pub fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str(r#"\""#),
            '\\' => out.push_str(r"\\"),
            '\n' => out.push_str(r"\n"),
            '\r' => out.push_str(r"\r"),
            '\t' => out.push_str(r"\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            _ => out.push(c),
        }
    }
    out
}
