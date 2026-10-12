//! Scales, and keeping what the keys play inside one.
//!
//! The table is the one the `scale` line reads, so a pattern's degrees and a
//! locked key agree on what a scale is. Every scale here has seven notes: a
//! pattern counts its degrees in sevens.

/// Every scale the text can name, as semitones above the root.
pub const SCALES: &[(&str, [u8; 7])] = &[
    ("major", [0, 2, 4, 5, 7, 9, 11]),
    ("ionian", [0, 2, 4, 5, 7, 9, 11]),
    ("minor", [0, 2, 3, 5, 7, 8, 10]),
    ("aeolian", [0, 2, 3, 5, 7, 8, 10]),
    ("dorian", [0, 2, 3, 5, 7, 9, 10]),
    ("phrygian", [0, 1, 3, 5, 7, 8, 10]),
    ("lydian", [0, 2, 4, 6, 7, 9, 11]),
    ("mixolydian", [0, 2, 4, 5, 7, 9, 10]),
    ("locrian", [0, 1, 3, 5, 6, 8, 10]),
    ("harmonic_minor", [0, 2, 3, 5, 7, 8, 11]),
    // Phrygian with a major third: the fifth mode of harmonic minor, the
    // sound of half of psytrance.
    ("phrygian_dominant", [0, 1, 4, 5, 7, 8, 10]),
    ("hungarian_minor", [0, 2, 3, 6, 7, 8, 11]),
    ("double_harmonic", [0, 1, 4, 5, 7, 8, 11]),
];

/// The names, for an error that lists them.
pub fn names() -> impl Iterator<Item = &'static str> {
    SCALES.iter().map(|(n, _)| *n)
}

pub fn intervals(kind: &str) -> Option<[u8; 7]> {
    SCALES.iter().find(|(n, _)| *n == kind).map(|(_, i)| *i)
}

/// The pitch class of a root as the text writes it: `E`, `F#`, `Bb`.
pub fn root_pc(name: &str) -> Option<u8> {
    let mut chars = name.chars();
    let base = match chars.next()? {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    let pc = match chars.as_str() {
        "" => base,
        "#" => base + 1,
        "b" => base + 11,
        _ => return None,
    };
    Some(pc % 12)
}

/// How a locked key finds its note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lock {
    /// Every key plays as written.
    Off,
    /// A key outside the scale plays the nearest note in it; halfway
    /// between two, the lower one.
    Snap,
    /// The white keys are the scale's degrees, C the root, in the octave of
    /// the key; the black keys play nothing.
    White,
}

impl Lock {
    pub fn from_word(w: &str) -> Option<Self> {
        match w {
            "off" => Some(Self::Off),
            "snap" => Some(Self::Snap),
            "white" => Some(Self::White),
            _ => None,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Snap => "snap",
            Self::White => "white",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scale {
    /// Pitch class of the root, 0 for C.
    pub root: u8,
    pub intervals: [u8; 7],
}

impl Scale {
    pub fn new(root: u8, intervals: [u8; 7]) -> Self {
        Self { root: root % 12, intervals }
    }

    /// `Scale::named("E", "phrygian")`.
    pub fn named(root: &str, kind: &str) -> Option<Self> {
        Some(Self::new(root_pc(root)?, intervals(kind)?))
    }

    /// Where `note` sits above the root, within the octave.
    fn offset(&self, note: u8) -> u8 {
        (note as i32 - self.root as i32).rem_euclid(12) as u8
    }

    pub fn contains(&self, note: u8) -> bool {
        self.intervals.contains(&self.offset(note))
    }

    /// The note in the scale nearest `note`; halfway between two, the lower,
    /// which in the dark modes is the darker. Never leaves 0..=127.
    pub fn snap(&self, note: u8) -> u8 {
        let off = self.offset(note) as i32;
        // The scale's notes around this octave, the root of the next one
        // included, so B in C major can go up to C.
        let mut best = (i32::MAX, 0i32);
        for iv in self.intervals.iter().map(|&i| i as i32).chain([12]) {
            let d = iv - off;
            // Strictly nearer, or as near and lower.
            if d.abs() < best.0 || (d.abs() == best.0 && d < best.1) {
                best = (d.abs(), d);
            }
            // The root of the octave below, for a note just above it.
            let below = iv - 12 - off;
            if below.abs() < best.0 || (below.abs() == best.0 && below < best.1) {
                best = (below.abs(), below);
            }
        }
        let out = note as i32 + best.1;
        if (0..=127).contains(&out) {
            out as u8
        } else {
            // Off the end of the keyboard: the nearest in the other direction.
            let step = if out > 127 { -1 } else { 1 };
            let mut n = out;
            while !(0..=127).contains(&n) || !self.contains(n as u8) {
                n += step;
            }
            n as u8
        }
    }

    /// A white key as a degree of the scale: C is the root, D the second, up
    /// to B the seventh, in the octave the key is in. A black key is `None`,
    /// and so is a degree past the top of MIDI.
    pub fn white(&self, note: u8) -> Option<u8> {
        let degree = match note % 12 {
            0 => 0,
            2 => 1,
            4 => 2,
            5 => 3,
            7 => 4,
            9 => 5,
            11 => 6,
            _ => return None,
        };
        let out = (note / 12) as u16 * 12 + self.root as u16 + self.intervals[degree] as u16;
        (out <= 127).then_some(out as u8)
    }

    /// What a key plays under `lock`: `None` for a key that plays nothing.
    pub fn lock(&self, lock: Lock, note: u8) -> Option<u8> {
        match lock {
            Lock::Off => Some(note),
            Lock::Snap => Some(self.snap(note)),
            Lock::White => self.white(note),
        }
    }

    /// The note `steps` degrees from `note`, which is in the scale (snapped
    /// first if not): what a bend that lands in the scale aims at.
    pub fn degrees_from(&self, note: u8, steps: i32) -> u8 {
        let start = self.snap(note);
        let mut n = start as i32;
        let mut left = steps;
        let dir = steps.signum();
        while left != 0 {
            n += dir;
            if !(0..=127).contains(&n) {
                return (n - dir) as u8;
            }
            if self.contains(n as u8) {
                left -= dir;
            }
        }
        n as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    fn e_phrygian() -> Scale {
        Scale::named("E", "phrygian").unwrap()
    }

    /// Note names from MIDI numbers, for assertions that read like music.
    fn n(name: &str) -> u8 {
        crate::dsl::compiler::note_name_to_midi(name)
    }

    #[test]
    fn roots_and_kinds_are_read_as_written() {
        assert_eq!(root_pc("E"), Some(4));
        assert_eq!(root_pc("F#"), Some(6));
        assert_eq!(root_pc("Bb"), Some(10));
        assert_eq!(root_pc("Cb"), Some(11));
        assert_eq!(root_pc("H"), None);
        assert_eq!(root_pc("E#x"), None);
        assert_eq!(intervals("phrygian_dominant"), Some([0, 1, 4, 5, 7, 8, 10]));
        assert_eq!(intervals("harmonic_minor"), Some([0, 2, 3, 5, 7, 8, 11]));
        assert_eq!(intervals("lydian"), Some([0, 2, 4, 6, 7, 9, 11]));
        assert!(intervals("blues").is_none());
        for (name, iv) in SCALES {
            assert_eq!(iv[0], 0, "{name} starts on its root");
            assert!(iv.windows(2).all(|w| w[0] < w[1] && w[1] - w[0] <= 3), "{name} climbs");
        }
    }

    #[test]
    fn a_note_in_the_scale_is_left_alone() {
        let s = e_phrygian();
        for name in ["E2", "F2", "G2", "A2", "B2", "C3", "D3", "E3"] {
            assert_eq!(s.snap(n(name)), n(name), "{name}");
            assert!(s.contains(n(name)));
        }
    }

    #[test]
    fn a_note_outside_snaps_to_the_nearest_and_down_on_a_tie() {
        let s = e_phrygian();
        // E phrygian: E F G A B C D. Every black key but none of the
        // naturals is outside it.
        assert_eq!(s.snap(n("F#2")), n("F2"), "between F and G: down");
        assert_eq!(s.snap(n("G#2")), n("G2"), "between G and A: down");
        assert_eq!(s.snap(n("A#2")), n("A2"), "between A and B: down");
        assert_eq!(s.snap(n("C#3")), n("C3"), "between C and D: down");
        assert_eq!(s.snap(n("D#3")), n("D3"), "between D and the root: down");

        // Phrygian dominant: E F G# A B C D. G is a semitone from both F#
        // (out) and G#: it goes to G#, the one note a semitone away.
        let pd = Scale::named("E", "phrygian_dominant").unwrap();
        assert_eq!(pd.snap(n("G3")), n("G#3"));
        assert_eq!(pd.snap(n("F#3")), n("F3"), "a semitone from F, a whole tone from G#");
        assert_eq!(pd.snap(n("D#3")), n("D3"));
    }

    #[test]
    fn snapping_wraps_the_octave() {
        // C major: B is in, and C# goes down to C across no boundary; in A
        // harmonic minor G# is the seventh and A the root above it.
        let c = Scale::named("C", "major").unwrap();
        assert_eq!(c.snap(n("C#4")), n("C4"));
        assert_eq!(c.snap(n("A#3")), n("A3"));
        let a = Scale::named("A", "harmonic_minor").unwrap();
        assert_eq!(a.snap(n("G4")), n("G#4"), "G is out of A harmonic minor; G# is a semitone up, F a whole tone down");
        assert_eq!(a.snap(n("A#3")), n("A3"));
    }

    #[test]
    fn every_key_lands_in_the_scale_for_every_scale_and_root() {
        for (kind, _) in SCALES {
            for root in 0..12u8 {
                let s = Scale::new(root, intervals(kind).unwrap());
                for note in 0..=127u8 {
                    let out = s.snap(note);
                    assert!(s.contains(out), "{kind} root {root}: {note} -> {out} is outside");
                    assert!((out as i32 - note as i32).abs() <= 2, "{kind} root {root}: {note} -> {out} moved far");
                }
            }
        }
    }

    #[test]
    fn snapping_never_leaves_midi() {
        let s = Scale::named("B", "locrian").unwrap();
        assert!(s.contains(s.snap(127)));
        assert!(s.snap(127) <= 127);
        assert!(s.contains(s.snap(0)));
    }

    #[test]
    fn white_keys_are_the_degrees_and_black_keys_nothing() {
        let s = e_phrygian();
        let whites: Vec<u8> =
            ["C3", "D3", "E3", "F3", "G3", "A3", "B3", "C4"].iter().map(|k| s.white(n(k)).unwrap()).collect();
        let want: Vec<u8> = ["E3", "F3", "G3", "A3", "B3", "C4", "D4", "E4"].iter().map(|k| n(k)).collect();
        assert_eq!(whites, want);
        for black in ["C#3", "D#3", "F#3", "G#3", "A#3"] {
            assert_eq!(s.white(n(black)), None, "{black}");
        }
        assert_eq!(s.lock(Lock::White, n("C#3")), None);
        assert_eq!(s.lock(Lock::Off, n("C#3")), Some(n("C#3")));
        assert_eq!(s.lock(Lock::Snap, n("C#3")), Some(n("C3")));
        // The top octave: a degree past 127 plays nothing.
        assert_eq!(s.white(127), None);
    }

    #[test]
    fn degrees_count_scale_steps() {
        let s = e_phrygian();
        assert_eq!(s.degrees_from(n("E3"), 1), n("F3"));
        assert_eq!(s.degrees_from(n("E3"), 2), n("G3"));
        assert_eq!(s.degrees_from(n("E3"), -1), n("D3"));
        assert_eq!(s.degrees_from(n("E3"), 7), n("E4"));
        assert_eq!(s.degrees_from(n("F#3"), 1), n("G3"), "from the snapped note");
        assert_eq!(s.degrees_from(n("E3"), 0), n("E3"));
    }
}
