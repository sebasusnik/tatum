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
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

pub type ParseResult<T> = Result<T, Vec<ParseError>>;
pub type CompileResult<T> = Result<T, Vec<CompileError>>;
