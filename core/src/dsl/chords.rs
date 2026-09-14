//! Chord symbols in patterns: `Fm9`, `Dbmaj7`, `Csus4`, `G7/2` (octave 2).
//! A symbol expands to a chord step with the root at the given octave and
//! the quality's intervals stacked above it.

extern crate alloc;
use alloc::vec::Vec;

/// (suffix, intervals in semitones from the root). Longest suffixes first so
/// `maj7` wins over `7` and `m7b5` over `m7`.
pub const QUALITIES: &[(&str, &[u8])] = &[
    ("maj13", &[0, 4, 7, 11, 14, 21]),
    ("mmaj7", &[0, 3, 7, 11]),
    ("madd9", &[0, 3, 7, 14]),
    ("7sus4", &[0, 5, 7, 10]),
    ("maj9", &[0, 4, 7, 11, 14]),
    ("maj7", &[0, 4, 7, 11]),
    ("add9", &[0, 4, 7, 14]),
    ("dim7", &[0, 3, 6, 9]),
    ("m7b5", &[0, 3, 6, 10]),
    ("sus2", &[0, 2, 7]),
    ("sus4", &[0, 5, 7]),
    ("m13", &[0, 3, 7, 10, 14, 21]),
    ("m11", &[0, 3, 7, 10, 14, 17]),
    ("aug", &[0, 4, 8]),
    ("dim", &[0, 3, 6]),
    ("maj", &[0, 4, 7]),
    ("min", &[0, 3, 7]),
    ("m9", &[0, 3, 7, 10, 14]),
    ("m7", &[0, 3, 7, 10]),
    ("m6", &[0, 3, 7, 9]),
    ("13", &[0, 4, 7, 10, 14, 21]),
    ("11", &[0, 4, 7, 10, 14, 17]),
    ("9", &[0, 4, 7, 10, 14]),
    ("7", &[0, 4, 7, 10]),
    ("6", &[0, 4, 7, 9]),
    ("5", &[0, 7]),
    ("m", &[0, 3, 7]),
    ("", &[0, 4, 7]),
];

/// Parse a symbol into (root pitch class 0..11, intervals). `None` if it is not a chord.
pub fn parse(symbol: &str) -> Option<(u8, &'static [u8])> {
    let mut chars = symbol.chars();
    let root = match chars.next()?.to_ascii_uppercase() {
        'C' => 0, 'D' => 2, 'E' => 4, 'F' => 5, 'G' => 7, 'A' => 9, 'B' => 11,
        _ => return None,
    };
    let rest = chars.as_str();
    let (root, rest) = match rest.chars().next() {
        Some('#') => ((root + 1) % 12, &rest[1..]),
        Some('b') => ((root + 11) % 12, &rest[1..]),
        _ => (root, rest),
    };
    QUALITIES.iter().find(|(q, _)| *q == rest).map(|(_, iv)| (root, *iv))
}

/// MIDI notes of a symbol with the root in `octave` (C3 = 48).
pub fn notes(symbol: &str, octave: u8) -> Option<Vec<u8>> {
    let (root, intervals) = parse(symbol)?;
    let base = 12u16 * (octave as u16 + 1) + root as u16;
    Some(intervals.iter().map(|i| (base + *i as u16).min(127) as u8).collect())
}

/// The quality suffixes, for docs and error messages.
pub fn quality_names() -> Vec<&'static str> {
    QUALITIES.iter().map(|(q, _)| if q.is_empty() { "(major)" } else { *q }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbols() {
        assert_eq!(notes("Fm9", 3), Some(alloc::vec![53, 56, 60, 63, 67]));
        assert_eq!(notes("Dbmaj7", 3), Some(alloc::vec![49, 53, 56, 60]));
        assert_eq!(notes("C", 4), Some(alloc::vec![60, 64, 67]));
        assert_eq!(notes("F#m7b5", 2), Some(alloc::vec![42, 45, 48, 52]));
        assert_eq!(notes("Gsus4", 3), Some(alloc::vec![55, 60, 62]));
        assert_eq!(parse("H7"), None);
        assert_eq!(parse("Cxyz"), None);
        assert_eq!(parse("cutoff"), None);
    }
}
