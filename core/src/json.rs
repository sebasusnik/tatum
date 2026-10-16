//! Just enough JSON for the command surface: a reader for the flat objects
//! a message is (`{"op":"try_knob","target":"bass.cutoff","value":96}`,
//! a map of file texts, a list of MIDI bytes) and a writer for the
//! answers. The core has no dependencies and this keeps it that way; it
//! is not a general JSON library and does not try to be.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

pub use crate::dsl::error::json_escape as escape;

/// A parsed JSON value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Value>),
    Obj(Vec<(String, Value)>),
}

impl Value {
    pub fn parse(text: &str) -> Result<Value, String> {
        let mut p = Parser { s: text.as_bytes(), at: 0 };
        p.skip_ws();
        let v = p.value()?;
        p.skip_ws();
        if p.at != p.s.len() {
            return Err(format!("trailing characters at {}", p.at));
        }
        Ok(v)
    }

    /// A member of an object, by key.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Obj(members) => members.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn num(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            _ => None,
        }
    }

    pub fn int(&self) -> Option<i64> {
        self.num().map(|n| n as i64)
    }

    pub fn bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn arr(&self) -> Option<&[Value]> {
        match self {
            Value::Arr(items) => Some(items),
            _ => None,
        }
    }

    pub fn obj(&self) -> Option<&[(String, Value)]> {
        match self {
            Value::Obj(members) => Some(members),
            _ => None,
        }
    }
}

struct Parser<'a> {
    s: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn skip_ws(&mut self) {
        while self.at < self.s.len() && matches!(self.s[self.at], b' ' | b'\n' | b'\r' | b'\t') {
            self.at += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.at).copied()
    }

    fn expect(&mut self, c: u8) -> Result<(), String> {
        if self.peek() == Some(c) {
            self.at += 1;
            Ok(())
        } else {
            Err(format!("expected '{}' at {}", c as char, self.at))
        }
    }

    fn value(&mut self) -> Result<Value, String> {
        match self.peek() {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => self.string().map(Value::Str),
            Some(b't') => self.word("true", Value::Bool(true)),
            Some(b'f') => self.word("false", Value::Bool(false)),
            Some(b'n') => self.word("null", Value::Null),
            Some(c) if c == b'-' || c.is_ascii_digit() => self.number(),
            Some(c) => Err(format!("unexpected '{}' at {}", c as char, self.at)),
            None => Err(String::from("unexpected end")),
        }
    }

    fn word(&mut self, w: &str, v: Value) -> Result<Value, String> {
        if self.s[self.at..].starts_with(w.as_bytes()) {
            self.at += w.len();
            Ok(v)
        } else {
            Err(format!("expected {} at {}", w, self.at))
        }
    }

    fn number(&mut self) -> Result<Value, String> {
        let start = self.at;
        while self.at < self.s.len() && matches!(self.s[self.at], b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E') {
            self.at += 1;
        }
        let text = core::str::from_utf8(&self.s[start..self.at]).map_err(|_| "bad number")?;
        text.parse::<f64>().map(Value::Num).map_err(|_| format!("bad number '{}' at {}", text, start))
    }

    fn string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            let c = self.peek().ok_or("unterminated string")?;
            self.at += 1;
            match c {
                b'"' => return Ok(out),
                b'\\' => {
                    let e = self.peek().ok_or("unterminated escape")?;
                    self.at += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'n' => out.push('\n'),
                        b't' => out.push('\t'),
                        b'r' => out.push('\r'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'u' => {
                            let mut code = self.hex4()?;
                            // A surrogate pair is two escapes in a row.
                            if (0xD800..0xDC00).contains(&code) && self.s[self.at..].starts_with(b"\\u") {
                                self.at += 2;
                                let low = self.hex4()?;
                                code = 0x10000 + ((code - 0xD800) << 10) + (low.wrapping_sub(0xDC00) & 0x3FF);
                            }
                            out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                        }
                        other => return Err(format!("bad escape '\\{}'", other as char)),
                    }
                }
                _ => {
                    // Copy the whole UTF-8 sequence the byte starts.
                    let start = self.at - 1;
                    let len = utf8_len(c);
                    let end = (start + len).min(self.s.len());
                    out.push_str(core::str::from_utf8(&self.s[start..end]).map_err(|_| "bad utf-8")?);
                    self.at = end;
                }
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let digits = self.s.get(self.at..self.at + 4).ok_or("short \\u escape")?;
        let text = core::str::from_utf8(digits).map_err(|_| "bad \\u escape")?;
        self.at += 4;
        u32::from_str_radix(text, 16).map_err(|_| format!("bad \\u escape '{}'", text))
    }

    fn array(&mut self) -> Result<Value, String> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.at += 1;
            return Ok(Value::Arr(items));
        }
        loop {
            self.skip_ws();
            items.push(self.value()?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b']') => {
                    self.at += 1;
                    return Ok(Value::Arr(items));
                }
                _ => return Err(format!("expected ',' or ']' at {}", self.at)),
            }
        }
    }

    fn object(&mut self) -> Result<Value, String> {
        self.expect(b'{')?;
        let mut members = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.at += 1;
            return Ok(Value::Obj(members));
        }
        loop {
            self.skip_ws();
            let key = self.string()?;
            self.skip_ws();
            self.expect(b':')?;
            self.skip_ws();
            let value = self.value()?;
            members.push((key, value));
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b'}') => {
                    self.at += 1;
                    return Ok(Value::Obj(members));
                }
                _ => return Err(format!("expected ',' or '}}' at {}", self.at)),
            }
        }
    }
}

fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

/// An object being written: `Obj::new().str("op", "ok").num("bar", 3.0).finish()`.
pub struct Obj {
    text: String,
}

impl Default for Obj {
    fn default() -> Self {
        Self::new()
    }
}

impl Obj {
    pub fn new() -> Self {
        Self { text: String::from("{") }
    }

    fn key(&mut self, key: &str) {
        if self.text.len() > 1 {
            self.text.push(',');
        }
        self.text.push('"');
        self.text.push_str(&escape(key));
        self.text.push_str("\":");
    }

    pub fn str(mut self, key: &str, value: &str) -> Self {
        self.key(key);
        self.text.push('"');
        self.text.push_str(&escape(value));
        self.text.push('"');
        self
    }

    pub fn opt_str(self, key: &str, value: Option<&str>) -> Self {
        match value {
            Some(v) => self.str(key, v),
            None => self.raw(key, "null"),
        }
    }

    pub fn num(mut self, key: &str, value: f64) -> Self {
        self.key(key);
        self.text.push_str(&number(value));
        self
    }

    pub fn int(mut self, key: &str, value: i64) -> Self {
        self.key(key);
        self.text.push_str(&value.to_string());
        self
    }

    pub fn bool(mut self, key: &str, value: bool) -> Self {
        self.key(key);
        self.text.push_str(if value { "true" } else { "false" });
        self
    }

    /// A value already written as JSON: another object, an array.
    pub fn raw(mut self, key: &str, json: &str) -> Self {
        self.key(key);
        self.text.push_str(json);
        self
    }

    pub fn strs(self, key: &str, values: &[String]) -> Self {
        let items: Vec<String> = values.iter().map(|v| format!("\"{}\"", escape(v))).collect();
        self.raw(key, &array(&items))
    }

    pub fn finish(mut self) -> String {
        self.text.push('}');
        self.text
    }
}

/// An array of values already written as JSON.
pub fn array(items: &[String]) -> String {
    format!("[{}]", items.join(","))
}

/// A number as JSON writes it: no exponent for the sizes that come up,
/// and `null` for what JSON cannot say.
pub fn number(v: f64) -> String {
    if !v.is_finite() {
        return String::from("null");
    }
    if v == (v as i64) as f64 && v.abs() < 1e15 {
        return (v as i64).to_string();
    }
    // Four decimals cover a knob position, a level, a tempo.
    let mut s = format!("{:.4}", v);
    while s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn a_message_parses_into_its_members() {
        let v = Value::parse(r#"{"op":"try_knob","target":"bass.cutoff","value":96,"on":true,"bytes":[176,74,127]}"#)
            .unwrap();
        assert_eq!(v.get("op").and_then(Value::str), Some("try_knob"));
        assert_eq!(v.get("value").and_then(Value::int), Some(96));
        assert_eq!(v.get("on").and_then(Value::bool), Some(true));
        let bytes: Vec<i64> = v.get("bytes").unwrap().arr().unwrap().iter().filter_map(Value::int).collect();
        assert_eq!(bytes, [176, 74, 127]);
        assert_eq!(v.get("nope"), None);
    }

    #[test]
    fn strings_keep_their_escapes_and_their_accents() {
        let v = Value::parse(r#"{"text":"tempo 120\n# niño \"x\" é 🎵"}"#).unwrap();
        assert_eq!(v.get("text").and_then(Value::str), Some("tempo 120\n# niño \"x\" é 🎵"));
        let back = Obj::new().str("text", v.get("text").unwrap().str().unwrap()).finish();
        assert_eq!(Value::parse(&back).unwrap(), v);
    }

    #[test]
    fn what_is_not_json_is_an_error() {
        assert!(Value::parse("{\"a\":}").is_err());
        assert!(Value::parse("[1,2").is_err());
        assert!(Value::parse("{} x").is_err());
    }

    #[test]
    fn numbers_write_plainly() {
        assert_eq!(number(3.0), "3");
        assert_eq!(number(0.5), "0.5");
        assert_eq!(number(124.25), "124.25");
        assert_eq!(number(f64::NAN), "null");
        assert_eq!(
            Obj::new().int("a", -2).num("b", 0.1).strs("c", &vec![String::from("x")]).finish(),
            r#"{"a":-2,"b":0.1,"c":["x"]}"#
        );
    }
}
