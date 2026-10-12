//! Playing a song from a controller as a performer does: the keyboard split
//! into zones, the notes kept in a scale, and scenes that change all of it.

pub mod scale;

/// Whether `key` names a key `keyboard` can bind: a letter, a digit, a
/// function key, or one of the named ones.
pub fn key_name_ok(key: &str) -> bool {
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => c.is_ascii_alphanumeric(),
        _ => {
            matches!(key, "space" | "tab" | "enter" | "left" | "right" | "up" | "down")
                || key.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()).is_some_and(|n| (1..=12).contains(&n))
        }
    }
}
pub mod fx;
