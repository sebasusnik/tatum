//! What the loop plays: a single hit, four on the floor, a held note or a
//! chord, on a note the page picks; on a drum module, a lane once, on every
//! beat, or a groove. Each is a bar of pattern written next to the sound and
//! rendered through the instrument alone -- no master chain, no loudness.

use tatum_core::song_engine::SongEngine;
use tatum_core::SAMPLE_RATE;

use super::sound::{Body, Sound};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pattern {
    Hit,
    Four,
    Held,
    Chord,
    /// Drums only: kick on the beat, hat between, clap on two and four.
    Groove,
}

impl Pattern {
    pub fn parse(s: &str) -> Option<Pattern> {
        Some(match s {
            "hit" => Pattern::Hit,
            "four" => Pattern::Four,
            "held" => Pattern::Held,
            "chord" => Pattern::Chord,
            "groove" => Pattern::Groove,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Pattern::Hit => "hit",
            Pattern::Four => "four",
            Pattern::Held => "held",
            Pattern::Chord => "chord",
            Pattern::Groove => "groove",
        }
    }
}

/// The drum lanes a `beats` module plays.
pub const LANES: [&str; 9] = ["kick", "snare", "clap", "hat", "openhat", "tom", "tom2", "tom3", "crash"];

/// The patterns that make sense for a sound.
pub fn patterns_for(sound: &Sound) -> &'static [Pattern] {
    if is_drums(sound) {
        &[Pattern::Hit, Pattern::Four, Pattern::Groove]
    } else {
        &[Pattern::Hit, Pattern::Four, Pattern::Held, Pattern::Chord]
    }
}

pub fn is_drums(sound: &Sound) -> bool {
    matches!(&sound.body, Body::Module(m) if m.kind == tatum_core::params::ModuleKind::Beats)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Audition {
    pub pattern: Pattern,
    /// A note name, checked: `C3`, `F#1`, `Bb4`.
    pub note: String,
    pub bpm: f32,
    /// The lane a drum module plays for a hit or four on the floor.
    pub lane: String,
}

impl Default for Audition {
    fn default() -> Self {
        Audition { pattern: Pattern::Hit, note: "C3".into(), bpm: 125.0, lane: "kick".into() }
    }
}

/// A note name as a pattern takes it, and nothing else: it is written into
/// the source that is rendered, so a name that is not one is refused rather
/// than spliced in.
pub fn midi_of(note: &str) -> Option<u8> {
    let b = note.as_bytes();
    let base = match b.first()? {
        b'C' => 0,
        b'D' => 2,
        b'E' => 4,
        b'F' => 5,
        b'G' => 7,
        b'A' => 9,
        b'B' => 11,
        _ => return None,
    };
    let (acc, rest) = match b.get(1) {
        Some(b'#') => (1, &note[2..]),
        Some(b'b') => (-1, &note[2..]),
        _ => (0, &note[1..]),
    };
    // One octave digit: C0 up to G9, the notes a pattern spells.
    if rest.len() != 1 || !rest.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let octave: i32 = rest.parse().ok()?;
    let midi = (octave + 1) * 12 + base + acc;
    (0..=127).contains(&midi).then_some(midi as u8)
}

fn name_of(midi: u8) -> String {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    format!("{}{}", NAMES[midi as usize % 12], midi as i32 / 12 - 1)
}

impl Audition {
    /// Set from the page's `key=value` lines: `pattern`, `note`, `bpm`,
    /// `lane`. Nothing is changed unless every line is good.
    pub fn apply(&mut self, body: &str, sound: &Sound) -> Result<(), String> {
        let mut next = self.clone();
        for line in body.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let (k, v) = line.split_once('=').ok_or_else(|| format!("'{line}': expected key=value"))?;
            let v = v.trim();
            match k.trim() {
                "pattern" => next.pattern = Pattern::parse(v).ok_or_else(|| format!("no pattern '{v}'"))?,
                "note" => {
                    midi_of(v).ok_or_else(|| format!("'{v}' is not a note (C1, F#2, Bb3)"))?;
                    next.note = v.to_string();
                }
                "bpm" => {
                    next.bpm = super::sound::finite(v)
                        .filter(|b| (40.0..=220.0).contains(b))
                        .ok_or_else(|| format!("'{v}': a tempo from 40 to 220"))?;
                }
                "lane" => {
                    if !LANES.contains(&v) {
                        return Err(format!("no lane '{v}' ({})", LANES.join(", ")));
                    }
                    next.lane = v.to_string();
                }
                other => return Err(format!("'{other}': pattern, note, bpm or lane")),
            }
        }
        if !patterns_for(sound).contains(&next.pattern) {
            return Err(format!("a {} sound has no '{}' to play", sound.kind(), next.pattern.as_str()));
        }
        *self = next;
        Ok(())
    }

    /// The pattern a sound plays now, fallen back to a hit when the sound
    /// has no such thing (a chord on a drum kit).
    pub fn pattern_for(&self, sound: &Sound) -> Pattern {
        if patterns_for(sound).contains(&self.pattern) {
            self.pattern
        } else {
            Pattern::Hit
        }
    }

    /// Two bars of pattern, the second empty so a tail rings out: the bar
    /// the page loops, with what spills over the bar line.
    fn pattern_text(&self, sound: &Sound, pattern: Pattern) -> String {
        let rests = "- - - -  - - - -  - - - -  - - - -";
        if is_drums(sound) {
            let lane = |l: &str, steps: &str| format!("  {l}: {steps}  {rests}\n");
            let mut o = String::from("pattern p {\n");
            match pattern {
                Pattern::Groove => {
                    o += &lane("kick", "X - - -  X - - -  X - - -  X - - -");
                    o += &lane("hat", "- - x -  - - x -  - - x -  - - x -");
                    o += &lane("clap", "- - - -  x - - -  - - - -  x - - -");
                }
                Pattern::Four => o += &lane(&self.lane, "X - - -  X - - -  X - - -  X - - -"),
                _ => o += &lane(&self.lane, "X - - -  - - - -  - - - -  - - - -"),
            }
            o.push_str("}\n");
            return o;
        }
        let n = &self.note;
        let bar = match pattern {
            Pattern::Four => format!("{n} - - -  {n} - - -  {n} - - -  {n} - - -"),
            Pattern::Held => format!("{n} ..*15"),
            Pattern::Chord => {
                // A minor triad on the note, held: on a poly sound, three notes.
                let m = midi_of(n).unwrap_or(48);
                let up = |s: u8| name_of(m.saturating_add(s).min(127));
                format!("[{} {} {}] ..*15", n, up(3), up(7))
            }
            _ => format!("{n} - - -  - - - -  - - - -  - - - -"),
        };
        format!("pattern p {{ {bar}  {rests} }}\n")
    }

    /// The song that plays the sound: the sound's own text, a pattern and
    /// one track. What Save compiles to check a sound before writing it.
    pub fn source(&self, sound: &Sound, pattern: Pattern, bpm: f32) -> String {
        format!(
            "tempo {}\n{}\n{}track t {{ play p using {} }}\n",
            super::sound::number(bpm),
            sound.text(),
            self.pattern_text(sound, pattern),
            sound.name
        )
    }

    /// One hit of the sound, alone, on the note (a drum module: its lane):
    /// up to two seconds, cut where it is 70 dB down. What is drawn and
    /// measured.
    pub fn hit(&self, sound: &Sound) -> Result<Vec<f32>, String> {
        let (l, _) = render(&self.source(sound, Pattern::Hit, 60.0), 1)?;
        let mut out = l;
        out.truncate((2.0 * SAMPLE_RATE) as usize);
        let start = out.iter().position(|v| *v != 0.0).unwrap_or(0);
        let mut y = out.split_off(start);
        let peak = y.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        if let Some(last) = y.iter().rposition(|v| v.abs() > peak * 3.2e-4) {
            y.truncate((last + (0.01 * SAMPLE_RATE) as usize).min(y.len()));
        }
        Ok(y)
    }

    /// The bar the loop plays, in stereo, with what rings past it, from the
    /// bar's first sample: the page starts one every bar.
    pub fn bar(&self, sound: &Sound) -> Result<(Vec<f32>, Vec<f32>), String> {
        let (mut l, mut r) = render(&self.source(sound, self.pattern_for(sound), self.bpm), 2)?;
        let peak = l.iter().chain(r.iter()).fold(0.0f32, |m, v| m.max(v.abs()));
        let last = l.iter().zip(&r).rposition(|(a, b)| a.abs().max(b.abs()) > peak * 3.2e-4).unwrap_or(0);
        let bar = (4.0 * 60.0 / self.bpm * SAMPLE_RATE) as usize;
        let end = (last + (0.01 * SAMPLE_RATE) as usize).max(bar).min(l.len());
        l.truncate(end);
        r.truncate(end);
        Ok((l, r))
    }
}

/// `bars` of the one track's instrument, before its insert chain.
fn render(src: &str, bars: u32) -> Result<(Vec<f32>, Vec<f32>), String> {
    let mut engine = SongEngine::from_source(src)?;
    engine.set_taps(true);
    let (mut l, mut r) = (Vec::new(), Vec::new());
    engine.render_each(bars, |e, _, _| {
        if let Some(t) = e.taps() {
            l.extend_from_slice(&t.dry[0].l[..t.len]);
            r.extend_from_slice(&t.dry[0].r[..t.len]);
        }
    });
    Ok((l, r))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    /// The level at `hz` over `x`, by Goertzel.
    fn tone(x: &[f32], hz: f32) -> f32 {
        let w = 2.0 * std::f32::consts::PI * hz / SAMPLE_RATE;
        let (mut s1, mut s2) = (0.0f32, 0.0f32);
        for v in x {
            let s = v + 2.0 * w.cos() * s1 - s2;
            s2 = s1;
            s1 = s;
        }
        (s1 * s1 + s2 * s2 - 2.0 * w.cos() * s1 * s2).sqrt() / x.len() as f32
    }

    #[test]
    fn notes_are_notes_and_nothing_else() {
        assert_eq!(midi_of("C4"), Some(60));
        assert_eq!(midi_of("A4"), Some(69));
        assert_eq!(midi_of("F#1"), Some(30));
        assert_eq!(midi_of("Bb3"), Some(58));
        assert_eq!(midi_of("C0"), Some(12));
        for bad in ["", "H3", "c3", "C", "C3 }", "C3\ntrack", "C#", "Z3", "C10", "G9x", "C3-", "C-1", "G#9"] {
            assert_eq!(midi_of(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn every_pattern_on_every_kind_of_sound_makes_sound() {
        let sounds = [
            "instrument k {\n  kick(F#1, body=70ms, hold=100ms, release=200ms) as k\n  k > out\n}\n",
            "module bass b { cutoff 800hz }\n",
            "module fm f { algorithm two_op level 2.0 }\n",
            "module keys k { voice_mode poly attack 5ms }\n",
            "module beats d { kick_level 1.0 }\n",
        ];
        for text in sounds {
            let s = Sound::parse(text).expect("parses");
            let mut a = Audition::default();
            assert!(rms(&a.hit(&s).expect("a hit")) > 1e-3, "{text}");
            for p in patterns_for(&s) {
                a.apply(&format!("pattern={}\nnote=A2\nbpm=128", p.as_str()), &s).expect("takes it");
                let (l, r) = a.bar(&s).expect("renders");
                let bar = (4.0 * 60.0 / 128.0 * SAMPLE_RATE) as usize;
                assert!(l.len() >= bar, "{text} {p:?}: a whole bar");
                assert!(rms(&l) + rms(&r) > 1e-3, "{text} {p:?} is silent");
            }
        }
    }

    #[test]
    fn four_on_the_floor_plays_four_and_a_drum_lane_is_its_lane() {
        let s = Sound::parse("module beats d { }\n").expect("parses");
        let mut a = Audition::default();
        a.apply("pattern=four\nbpm=120", &s).expect("takes it");
        let (l, _) = a.bar(&s).expect("renders");
        let beat = (0.5 * SAMPLE_RATE) as usize;
        for i in 0..4 {
            let at = &l[i * beat..i * beat + beat / 4];
            assert!(rms(at) > 0.01, "beat {i}");
        }
        assert!(a.apply("pattern=chord", &s).is_err(), "no chord on drums");
        assert!(a.apply("lane=cowbell", &s).is_err());
        a.apply("pattern=hit\nlane=hat", &s).expect("a hat");
        let hat = a.hit(&s).expect("renders");
        a.apply("lane=kick", &s).expect("a kick");
        let kick = a.hit(&s).expect("renders");
        assert!(tone(&kick, 60.0) > 3.0 * tone(&hat, 60.0), "the kick lane is the kick");
    }

    #[test]
    fn a_chord_on_a_poly_keys_module_is_several_notes() {
        let s =
            Sound::parse("module keys pad { voice_mode poly attack 5ms detune 0% chorus_mix 0% }\n").expect("parses");
        let mut a = Audition::default();
        a.apply("pattern=held\nnote=C3", &s).expect("held");
        let (held, _) = a.bar(&s).expect("renders");
        a.apply("pattern=chord", &s).expect("chord");
        let (chord, _) = a.bar(&s).expect("renders");
        let n = (1.5 * SAMPLE_RATE) as usize;
        let (held, chord) = (&held[..n], &chord[..n]);
        // C3, Eb3, G3.
        for hz in [130.81, 155.56, 196.0] {
            assert!(tone(chord, hz) > 0.002, "{hz} Hz in the chord: {}", tone(chord, hz));
        }
        assert!(tone(chord, 155.56) > 5.0 * tone(held, 155.56), "the third is the chord's, not the note's");
        assert!(tone(chord, 196.0) > 5.0 * tone(held, 196.0), "and the fifth");
    }

    #[test]
    fn a_bad_audition_changes_nothing() {
        let s = Sound::parse("module bass b { }\n").expect("parses");
        let mut a = Audition::default();
        for bad in ["note=C3 }", "bpm=NaN", "bpm=1000", "pattern=solo", "pattern=groove", "volume=3", "note"] {
            assert!(a.apply(&format!("pattern=four\n{bad}"), &s).is_err(), "{bad}");
            assert_eq!(a, Audition::default(), "{bad}");
        }
    }
}
