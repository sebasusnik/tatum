pub mod ast;
pub mod error;
pub mod lexer;
pub mod parser;
pub mod compiler;
pub mod diff;
pub mod lint;

use ast::Song;
use error::ParseResult;
use lexer::tokenize;
use parser::Parser;

/// Parse a .synth source string into a Song AST.
pub fn parse(source: &str) -> ParseResult<Song> {
    let tokens = tokenize(source);
    let parser = Parser::new(tokens);
    parser.parse_song()
}
