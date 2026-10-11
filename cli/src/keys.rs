//! Single keys from the terminal, for `tatum set play` and a song with a
//! `keyboard` block: the space bar and the arrows move through the set, and
//! whatever the song binds calls its scenes, without an Enter after each.
//!
//! On a Unix terminal the line discipline is switched off for the session
//! (no canonical mode, no echo, Ctrl-C read as a key) and put back when the
//! guard drops, so quitting with Ctrl-C leaves the terminal as it was. Output
//! processing is left alone, so every other line printed still starts at the
//! left edge. Anywhere else a key is a line: type it and press Enter.

use std::io::{BufRead, IsTerminal, Read};
use std::sync::mpsc::Sender;

/// What a key asks for. Every key but the one that quits arrives by name,
/// and the session decides what it does: the song's `keyboard` block first,
/// then the keys a set always had.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Quit,
    Next,
    Prev,
    /// A step by its number on screen, counting from 1.
    Step(usize),
    Named(KeyName),
}

/// A key of the computer's keyboard, as a `keyboard` block names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyName {
    Char(char),
    F(u8),
    Space,
    Tab,
    Enter,
    Left,
    Right,
    Up,
    Down,
}

impl KeyName {
    /// The name the `keyboard` block writes: `a`, `7`, `f1`, `space`.
    pub fn word(self) -> String {
        match self {
            KeyName::Char(c) => c.to_ascii_lowercase().to_string(),
            KeyName::F(n) => format!("f{}", n),
            KeyName::Space => "space".into(),
            KeyName::Tab => "tab".into(),
            KeyName::Enter => "enter".into(),
            KeyName::Left => "left".into(),
            KeyName::Right => "right".into(),
            KeyName::Up => "up".into(),
            KeyName::Down => "down".into(),
        }
    }

    /// What the key does when the song binds nothing to it: the set's own
    /// keys, space or n or → for the next step, p or ← for the one before,
    /// a digit for a step by number.
    pub fn default_move(self) -> Option<Key> {
        match self {
            KeyName::Space | KeyName::Right | KeyName::Char('n' | 'N') => Some(Key::Next),
            KeyName::Left | KeyName::Char('p' | 'P') => Some(Key::Prev),
            KeyName::Char(c @ '1'..='9') => Some(Key::Step(c as usize - '0' as usize)),
            _ => None,
        }
    }
}

fn key_for(byte: u8) -> Option<Key> {
    match byte {
        b'q' | b'Q' | 3 => Some(Key::Quit), // 3 is Ctrl-C
        b' ' => Some(Key::Named(KeyName::Space)),
        b'\t' => Some(Key::Named(KeyName::Tab)),
        b'\r' | b'\n' => Some(Key::Named(KeyName::Enter)),
        b if b.is_ascii_alphanumeric() => Some(Key::Named(KeyName::Char(b as char))),
        _ => None,
    }
}

/// The key an escape sequence names: the arrows (`[C`), F1-F4 (`OP`..`OS`)
/// and F5-F12 (`[15~`..`[24~`).
fn escape_key(seq: &[u8]) -> Option<KeyName> {
    match seq {
        b"[A" => Some(KeyName::Up),
        b"[B" => Some(KeyName::Down),
        b"[C" => Some(KeyName::Right),
        b"[D" => Some(KeyName::Left),
        [b'O', c @ b'P'..=b'S'] => Some(KeyName::F(c - b'P' + 1)),
        [b'[', digits @ .., b'~'] => {
            let n: u8 = std::str::from_utf8(digits).ok()?.parse().ok()?;
            let f = match n {
                11..=15 => n - 10,
                17..=21 => n - 11,
                23 | 24 => n - 12,
                _ => return None,
            };
            Some(KeyName::F(f))
        }
        _ => None,
    }
}

/// Puts the terminal back when dropped.
pub struct Guard {
    #[cfg(unix)]
    saved: Option<libc::termios>,
}

impl Drop for Guard {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(saved) = self.saved.take() {
            // SAFETY: restores the attributes read from the same descriptor.
            unsafe {
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &saved);
            }
        }
    }
}

/// Start reading keys into `tx`. Single keys when stdin is a Unix terminal;
/// otherwise lines.
pub fn spawn(tx: Sender<Key>) -> Guard {
    #[cfg(unix)]
    if std::io::stdin().is_terminal() {
        if let Some(saved) = raw() {
            std::thread::spawn(move || {
                let mut stdin = std::io::stdin().lock();
                let mut byte = [0u8; 1];
                // The arrows and function keys arrive as escape sequences:
                // ESC, then `[` or `O`, then up to a letter or `~`.
                let mut escape: Option<Vec<u8>> = None;
                while stdin.read_exact(&mut byte).is_ok() {
                    let key = match (&mut escape, byte[0]) {
                        (None, 0x1b) => {
                            escape = Some(Vec::new());
                            continue;
                        }
                        (Some(seq), b) => {
                            seq.push(b);
                            let done = seq.len() >= 2 && (b.is_ascii_alphabetic() || b == b'~') || seq.len() > 5;
                            if !done {
                                continue;
                            }
                            let key = escape_key(seq).map(Key::Named);
                            escape = None;
                            key
                        }
                        (None, b) => key_for(b),
                    };
                    if let Some(key) = key {
                        if tx.send(key).is_err() || key == Key::Quit {
                            break;
                        }
                    }
                }
            });
            return Guard { saved: Some(saved) };
        }
    }
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            let key = match line.trim() {
                "" => Some(Key::Next),
                t => match t.parse::<usize>() {
                    Ok(n) if n >= 1 => Some(Key::Step(n)),
                    _ if t.len() > 1 && t.starts_with('f') => {
                        t[1..].parse::<u8>().ok().filter(|n| (1..=12).contains(n)).map(|n| Key::Named(KeyName::F(n)))
                    }
                    _ => t.bytes().next().and_then(key_for),
                },
            };
            if let Some(key) = key {
                if tx.send(key).is_err() || key == Key::Quit {
                    break;
                }
            }
        }
    });
    Guard {
        #[cfg(unix)]
        saved: None,
    }
}

/// Switch stdin to one key at a time, returning what it was.
#[cfg(unix)]
fn raw() -> Option<libc::termios> {
    // SAFETY: tcgetattr fills the struct it is given; zeroed is a valid
    // starting value for a plain C struct.
    unsafe {
        let mut saved: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(libc::STDIN_FILENO, &mut saved) != 0 {
            return None;
        }
        let mut t = saved;
        t.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG);
        t.c_cc[libc::VMIN] = 1;
        t.c_cc[libc::VTIME] = 0;
        if libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &t) != 0 {
            return None;
        }
        Some(saved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_sequences_name_the_arrows_and_function_keys() {
        assert_eq!(escape_key(b"[C"), Some(KeyName::Right));
        assert_eq!(escape_key(b"OP"), Some(KeyName::F(1)));
        assert_eq!(escape_key(b"OS"), Some(KeyName::F(4)));
        assert_eq!(escape_key(b"[15~"), Some(KeyName::F(5)));
        assert_eq!(escape_key(b"[17~"), Some(KeyName::F(6)));
        assert_eq!(escape_key(b"[24~"), Some(KeyName::F(12)));
        assert_eq!(escape_key(b"[16~"), None);
        assert_eq!(KeyName::F(3).word(), "f3");
        assert_eq!(KeyName::Char('A').word(), "a");
        assert_eq!(KeyName::Space.default_move(), Some(Key::Next));
        assert_eq!(KeyName::Char('7').default_move(), Some(Key::Step(7)));
        assert_eq!(KeyName::F(1).default_move(), None);
    }
}
