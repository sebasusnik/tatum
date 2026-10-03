//! Notes by name, by scale degree and by drum: what a step, a chord, a lane
//! label or a pad resolves to as a MIDI note.

use alloc::string::String;
use alloc::vec::Vec;

use crate::dsl::ast::*;

// ── Note resolution ──

/// Resolve a NoteRef (absolute or scale degree) to a MIDI note number.
pub fn resolve_note(note: &crate::dsl::ast::NoteRef, scale_intervals: &[u8], root_midi: u8) -> u8 {
    match note {
        crate::dsl::ast::NoteRef::Absolute(name) => note_name_to_midi(name),
        crate::dsl::ast::NoteRef::Midi(m) => *m,
        crate::dsl::ast::NoteRef::Degree(degree, octave) => {
            // degree is 1-7, map to 0-indexed
            let deg_idx = (*degree as usize).saturating_sub(1) % scale_intervals.len();
            let semitones = scale_intervals[deg_idx];
            // Root MIDI is in octave 0 context — degree 1 octave 3 means root note in octave 3
            // root_midi is the root at octave 0 (just the pitch class, 0-11)
            let midi = ((*octave as i16 + 1) * 12) + root_midi as i16 + semitones as i16;
            midi.clamp(0, 127) as u8
        }
    }
}

/// Get scale intervals and root pitch class from the song's scale definition.
/// With none, C major. A name the table does not know is a compile error
/// (see `validate_scale`); here it falls back the same way.
pub fn scale_context(song: &Song) -> ([u8; 7], u8) {
    song.globals
        .scale
        .as_ref()
        .and_then(|d| crate::perform::scale::Scale::named(&d.root, &d.kind))
        .map_or(([0, 2, 4, 5, 7, 9, 11], 0), |s| (s.intervals, s.root))
}

/// The note a drum lane plays on a `beats` module, `None` for a name that is
/// not a drum. What a pad hits, and what a lane label compiles to.
pub fn drum_note(name: &str) -> Option<u8> {
    match drum_name_to_midi(name) {
        0 => None,
        n => Some(n),
    }
}

/// Map drum lane label to MIDI note number.
pub(super) fn drum_name_to_midi(name: &str) -> u8 {
    match name {
        "kick" | "bd" => 36,
        "snare" | "sd" => 38,
        "clap" | "cp" => 39,
        "hat" | "hh" | "hihat" => 42,
        "tom" | "lt" => 43,
        "tom2" | "mt" => 45,
        "tom3" | "ht" => 47,
        "openhat" | "oh" => 46,
        "crash" | "cr" => 49,
        _ => 0, // unknown: caller reports an error
    }
}

/// Convert note name (e.g., "A1", "C#4") to MIDI note number. The parser has
/// already refused names outside MIDI (`note_in_midi_range`); anything else is
/// held to 0..127.
pub fn note_name_to_midi(name: &str) -> u8 {
    note_name_midi(name).clamp(0, 127) as u8
}

/// Whether a note name lands on a MIDI note: C-1 up to G9.
pub fn note_in_midi_range(name: &str) -> bool {
    (0..=127).contains(&note_name_midi(name))
}

fn note_name_midi(name: &str) -> i32 {
    let chars: Vec<char> = name.chars().collect();
    if chars.is_empty() {
        return 60;
    } // default C4

    let base = match chars[0].to_ascii_uppercase() {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => 0,
    };

    let mut i = 1;
    let accidental = if i < chars.len() && chars[i] == '#' {
        i += 1;
        1
    } else if i < chars.len() && chars[i] == 'b' {
        i += 1;
        -1
    } else {
        0
    };

    // The lexer only makes a note of digits here; a run of them too long for
    // an i32 is out of range like any other.
    let octave: i32 = if i < chars.len() {
        let oct_str: String = chars[i..].iter().collect();
        oct_str.parse().unwrap_or(i32::MAX / 24)
    } else {
        4
    };

    // MIDI: C4 = 60
    (octave + 1) * 12 + base + accidental
}
