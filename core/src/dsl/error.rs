extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

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
