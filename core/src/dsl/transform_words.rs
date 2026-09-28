//! The words a `play` line can transform a pattern with, for suggestions.

pub const WORDS: [&str; 12] =
    ["rev", "fast", "slow", "shift", "up", "transpose", "octave", "degrade", "ply", "iter", "every", "sometimes"];

/// The transform word closest to `word`, if it is close enough to be a typo
/// of one. `palindrome` is left out on purpose: nothing short is near it.
pub fn closest(word: &str) -> Option<&'static str> {
    let aliases: [(&str, &str); 4] = [("reverse", "rev"), ("double", "fast"), ("half", "slow"), ("rotate", "shift")];
    if let Some((_, w)) = aliases.iter().find(|(a, _)| *a == word) {
        return Some(w);
    }
    WORDS
        .iter()
        .map(|w| (crate::params::levenshtein(word, w), *w))
        .filter(|(d, w)| *d <= 2 && *d < w.len())
        .min_by_key(|(d, _)| *d)
        .map(|(_, w)| w)
}
