use crate::math;

#[derive(Clone, Copy, PartialEq)]
pub enum Scale {
    Major,
    Minor,
    Dorian,
    Mixolydian,
    PentatonicMinor,
}

impl Scale {
    /// Returns the intervals (semitones from root) for this scale.
    pub fn intervals(&self) -> [u8; 7] {
        match self {
            Scale::Major => [0, 2, 4, 5, 7, 9, 11],
            Scale::Minor => [0, 2, 3, 5, 7, 8, 10],
            Scale::Dorian => [0, 2, 3, 5, 7, 9, 10],
            Scale::Mixolydian => [0, 2, 4, 5, 7, 9, 10],
            Scale::PentatonicMinor => [0, 3, 5, 7, 10, 12, 15],
        }
    }
}

#[derive(Clone, Copy)]
pub struct HarmonyContext {
    pub root: u8,        // MIDI note of root (e.g., 57 = A3)
    pub scale: Scale,
    pub chord_degree: u8, // 0-based scale degree (0 = I, 1 = II, etc.)
}

impl HarmonyContext {
    pub fn new(root: u8, scale: Scale) -> Self {
        Self {
            root,
            scale,
            chord_degree: 0,
        }
    }

    /// Get the MIDI notes of the scale starting from root.
    pub fn scale_notes(&self) -> [u8; 7] {
        let intervals = self.scale.intervals();
        let mut notes = [0u8; 7];
        let mut i = 0;
        while i < 7 {
            notes[i] = self.root + intervals[i];
            i += 1;
        }
        notes
    }

    /// Get chord notes (triad + optional 7th) for the current degree.
    /// Returns up to 4 notes.
    pub fn chord_notes(&self) -> [Option<u8>; 4] {
        let scale = self.scale_notes();
        let deg = (self.chord_degree % 7) as usize;

        let root_note = scale[deg];
        let third = scale[(deg + 2) % 7]
            + if (deg + 2) >= 7 { 12 } else { 0 };
        let fifth = scale[(deg + 4) % 7]
            + if (deg + 4) >= 7 { 12 } else { 0 };
        let seventh = scale[(deg + 6) % 7]
            + if (deg + 6) >= 7 { 12 } else { 0 };

        [
            Some(root_note),
            Some(third),
            Some(fifth),
            Some(seventh),
        ]
    }

    /// Get bass note for current chord (root of the chord).
    pub fn bass_note(&self) -> u8 {
        let scale = self.scale_notes();
        let deg = (self.chord_degree % 7) as usize;
        scale[deg]
    }

    /// Set chord by common progression step.
    pub fn set_chord_degree(&mut self, degree: u8) {
        self.chord_degree = degree % 7;
    }

    /// Get frequency for a given scale degree and octave offset.
    pub fn degree_freq(&self, degree: u8, octave_offset: i8) -> f32 {
        let scale = self.scale_notes();
        let deg = (degree % 7) as usize;
        let note = (scale[deg] as i32 + octave_offset as i32 * 12) as u8;
        math::midi_to_freq(note)
    }
}

/// Common chord progressions as arrays of scale degrees.
pub struct Progressions;

impl Progressions {
    /// Am -> F -> C -> G (i -> VI -> III -> VII in minor)
    /// As scale degrees: 0 -> 5 -> 2 -> 6
    pub fn minor_pop() -> [u8; 4] {
        [0, 5, 2, 6]
    }

    #[allow(dead_code)]
    /// I -> V -> vi -> IV in major
    pub fn major_pop() -> [u8; 4] {
        [0, 4, 5, 3]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_a_minor_scale() {
        let ctx = HarmonyContext::new(57, Scale::Minor); // A3
        let notes = ctx.scale_notes();
        // A minor: A B C D E F G = 57 59 60 62 64 65 67
        assert_eq!(notes, [57, 59, 60, 62, 64, 65, 67]);
    }

    #[test]
    fn test_chord_notes() {
        let mut ctx = HarmonyContext::new(57, Scale::Minor);
        ctx.set_chord_degree(0);
        let chord = ctx.chord_notes();
        // Am chord: A(57) C(60) E(64) G(67)
        assert_eq!(chord[0], Some(57));
        assert_eq!(chord[1], Some(60));
        assert_eq!(chord[2], Some(64));
        assert_eq!(chord[3], Some(67));
    }
}
