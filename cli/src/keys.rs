//! Single keys from the terminal, for `tatum set play`: the space bar and the
//! arrows move through the set without an Enter after each.
//!
//! On a Unix terminal the line discipline is switched off for the session
//! (no canonical mode, no echo, Ctrl-C read as a key) and put back when the
//! guard drops, so quitting with Ctrl-C leaves the terminal as it was. Output
//! processing is left alone, so every other line printed still starts at the
//! left edge. Anywhere else a key is a line: type it and press Enter.

use std::io::{BufRead, IsTerminal, Read};
use std::sync::mpsc::Sender;

/// What a key asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Quit,
    Next,
    Prev,
    /// A step by its number on screen, counting from 1.
    Step(usize),
}

fn key_for(byte: u8) -> Option<Key> {
    match byte {
        b'q' | b'Q' | 3 => Some(Key::Quit), // 3 is Ctrl-C
        b' ' | b'n' | b'N' => Some(Key::Next),
        b'p' | b'P' => Some(Key::Prev),
        b'1'..=b'9' => Some(Key::Step((byte - b'0') as usize)),
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
                // Arrow keys arrive as ESC [ C / ESC [ D.
                let mut escape = 0u8;
                while stdin.read_exact(&mut byte).is_ok() {
                    let key = match (escape, byte[0]) {
                        (0, 0x1b) => {
                            escape = 1;
                            continue;
                        }
                        (1, b'[') => {
                            escape = 2;
                            continue;
                        }
                        (2, b'C') => Some(Key::Next),
                        (2, b'D') => Some(Key::Prev),
                        (2, _) => None,
                        (_, b) => key_for(b),
                    };
                    escape = 0;
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
                t => t.bytes().next().and_then(key_for).map(|k| match (k, t.parse::<usize>()) {
                    (Key::Step(_), Ok(n)) => Key::Step(n),
                    (k, _) => k,
                }),
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
