extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Keywords
    Tempo,
    Meter,
    Scale,
    Bus,
    Instrument,
    Module,
    Pattern,
    Track,
    Master,
    Arrange,
    Scene,
    Extends,
    Play,
    Using,
    Velocity,
    Level,
    Pan,
    As,
    In,
    Out,
    Mix,
    Sidechain,
    Swing,
    Humanize,
    Auto,

    // Literals
    Ident(String),     // bass, osc1, etc
    Number(f32),       // 120, 0.01, 55
    Note(String),      // A1, C#4, G0
    DrumHit,           // x  (normal velocity 0.8)
    DrumAccent,        // X  (accent velocity 1.0)
    DrumGhost,         // o  (ghost note velocity 0.35)
    Rest,              // -
    Tie,               // ..

    // Operators
    Arrow,             // >
    Eq,                // =
    Star,              // *
    Slash,             // /
    Plus,              // +

    // Delimiters
    LBrace,            // {
    RBrace,            // }
    LBracket,          // [
    RBracket,          // ]
    LParen,            // (
    RParen,            // )
    Comma,             // ,
    Colon,             // :

    // Special
    Repeat(u32),       // x2, x4
    Newline,

    Eof,
}

#[derive(Debug, Clone)]
pub struct Span {
    pub token: Token,
    pub line: usize,
    pub col: usize,
}

/// Tokenize source text into a stream of tokens.
pub fn tokenize(source: &str) -> Vec<Span> {
    let mut tokens = Vec::new();
    let mut line = 1usize;
    let mut col = 1usize;
    let chars: Vec<char> = source.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        let ch = chars[i];

        // Skip whitespace (except newlines)
        if ch == ' ' || ch == '\t' || ch == '\r' {
            i += 1;
            col += 1;
            continue;
        }

        // Newline
        if ch == '\n' {
            tokens.push(Span { token: Token::Newline, line, col });
            line += 1;
            col = 1;
            i += 1;
            continue;
        }

        // Comment: # to end of line
        if ch == '#' {
            while i < len && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }

        let start_col = col;

        // Two-char tokens
        if ch == '.' && i + 1 < len && chars[i + 1] == '.' {
            tokens.push(Span { token: Token::Tie, line, col: start_col });
            i += 2;
            col += 2;
            continue;
        }

        // Single-char tokens
        match ch {
            '>' => { tokens.push(Span { token: Token::Arrow, line, col: start_col }); i += 1; col += 1; continue; }
            '=' => { tokens.push(Span { token: Token::Eq, line, col: start_col }); i += 1; col += 1; continue; }
            '*' => { tokens.push(Span { token: Token::Star, line, col: start_col }); i += 1; col += 1; continue; }
            '/' => { tokens.push(Span { token: Token::Slash, line, col: start_col }); i += 1; col += 1; continue; }
            '+' => { tokens.push(Span { token: Token::Plus, line, col: start_col }); i += 1; col += 1; continue; }
            '{' => { tokens.push(Span { token: Token::LBrace, line, col: start_col }); i += 1; col += 1; continue; }
            '}' => { tokens.push(Span { token: Token::RBrace, line, col: start_col }); i += 1; col += 1; continue; }
            '[' => { tokens.push(Span { token: Token::LBracket, line, col: start_col }); i += 1; col += 1; continue; }
            ']' => { tokens.push(Span { token: Token::RBracket, line, col: start_col }); i += 1; col += 1; continue; }
            '(' => { tokens.push(Span { token: Token::LParen, line, col: start_col }); i += 1; col += 1; continue; }
            ')' => { tokens.push(Span { token: Token::RParen, line, col: start_col }); i += 1; col += 1; continue; }
            ',' => { tokens.push(Span { token: Token::Comma, line, col: start_col }); i += 1; col += 1; continue; }
            ':' => { tokens.push(Span { token: Token::Colon, line, col: start_col }); i += 1; col += 1; continue; }
            '-' => { tokens.push(Span { token: Token::Rest, line, col: start_col }); i += 1; col += 1; continue; }
            _ => {}
        }

        // Number (including negative after context, and decimals)
        if ch.is_ascii_digit() || (ch == '.' && i + 1 < len && chars[i + 1].is_ascii_digit()) {
            let start = i;
            while i < len && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            if let Ok(n) = s.parse::<f32>() {
                tokens.push(Span { token: Token::Number(n), line, col: start_col });
            }
            col += i - start;
            continue;
        }

        // Identifier or keyword or note name
        if ch.is_ascii_alphabetic() || ch == '_' {
            let start = i;
            while i < len && (chars[i].is_ascii_alphanumeric() || chars[i] == '_' || chars[i] == '#') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let word_len = word.len();

            // Check for repeat pattern: x followed by digits (but only standalone "x2", "x4", etc)
            if word.starts_with('x') && word.len() > 1 && word[1..].chars().all(|c| c.is_ascii_digit()) {
                if let Ok(n) = word[1..].parse::<u32>() {
                    tokens.push(Span { token: Token::Repeat(n), line, col: start_col });
                    col += word_len;
                    continue;
                }
            }

            // Check for drum hit: standalone "x", accent "X", ghost "o"
            if word == "x" {
                tokens.push(Span { token: Token::DrumHit, line, col: start_col });
                col += word_len;
                continue;
            }
            if word == "X" {
                tokens.push(Span { token: Token::DrumAccent, line, col: start_col });
                col += word_len;
                continue;
            }
            if word == "o" {
                tokens.push(Span { token: Token::DrumGhost, line, col: start_col });
                col += word_len;
                continue;
            }

            // Check for note name: letter (A-G) + optional # + digit
            if is_note_name(&word) {
                tokens.push(Span { token: Token::Note(word), line, col: start_col });
                col += word_len;
                continue;
            }

            // Keywords
            let tok = match word.as_str() {
                "tempo" => Token::Tempo,
                "meter" => Token::Meter,
                "scale" => Token::Scale,
                "bus" => Token::Bus,
                "instrument" => Token::Instrument,
                "module" => Token::Module,
                "pattern" => Token::Pattern,
                "track" => Token::Track,
                "master" => Token::Master,
                "arrange" => Token::Arrange,
                "scene" => Token::Scene,
                "extends" => Token::Extends,
                "play" => Token::Play,
                "using" => Token::Using,
                "velocity" => Token::Velocity,
                "level" => Token::Level,
                "pan" => Token::Pan,
                "as" => Token::As,
                "in" => Token::In,
                "out" => Token::Out,
                "mix" => Token::Mix,
                "sidechain" => Token::Sidechain,
                "swing" => Token::Swing,
                "humanize" => Token::Humanize,
                "auto" => Token::Auto,
                _ => Token::Ident(word),
            };

            tokens.push(Span { token: tok, line, col: start_col });
            col += word_len;
            continue;
        }

        // Unknown character — skip
        i += 1;
        col += 1;
    }

    tokens.push(Span { token: Token::Eof, line, col });
    tokens
}

/// Check if a word looks like a note name: A-G, optional #/b, followed by a digit.
fn is_note_name(s: &str) -> bool {
    let chars: Vec<char> = s.chars().collect();
    if chars.is_empty() { return false; }

    // First char must be A-G
    let first = chars[0].to_ascii_uppercase();
    if first < 'A' || first > 'G' { return false; }

    let mut i = 1;
    // Optional # or b
    if i < chars.len() && (chars[i] == '#' || chars[i] == 'b') {
        i += 1;
    }
    // Must have at least one digit
    if i >= chars.len() { return false; }
    // Remaining chars must all be digits
    chars[i..].iter().all(|c| c.is_ascii_digit())
}
