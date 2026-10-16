//! Patterns written back as text: a step as its token, a pattern as its
//! block body. What a grid edits is the parsed pattern; what it saves is
//! this, put back into the file between the pattern's braces. Every form
//! the parser reads is written the way the docs spell it, except a chord
//! symbol (`Fm9`), which the parser resolves to notes and comes back as
//! `[F3 Ab3 C4 Eb4 G4]`, and a ratchet (`a*3`), which comes back as the
//! `<a a a>` it is sugar for.

use alloc::string::String;
use alloc::vec::Vec;

use super::ast::{ChordStep, DrumStep, NoteRef, NoteStep, PLock, PatternDef, Step};

/// A MIDI note as a name the parser reads back to it: `C4` is 60, sharps
/// for the black keys.
pub fn note_name(midi: u8) -> String {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    let octave = midi as i32 / 12 - 1;
    format!("{}{}", NAMES[midi as usize % 12], octave)
}

/// A number as a pattern writes it: `0.8`, `0.35`, `1`.
fn amount(v: f32) -> String {
    let mut s = format!("{:.3}", v);
    while s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    s
}

fn plock_text(p: &PLock) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(v) = p.cutoff {
        parts.push(format!("cutoff={}", amount(v)));
    }
    if let Some(v) = p.env_depth {
        parts.push(format!("edepth={}", amount(v)));
    }
    if let Some(v) = p.resonance {
        parts.push(format!("res={}", amount(v)));
    }
    if let Some(v) = p.gate {
        parts.push(format!("gate={}", amount(v)));
    }
    let mut out = String::new();
    if let Some(p) = p.probability {
        out.push_str(&format!("?{}", amount(p)));
    }
    if !parts.is_empty() {
        out.push_str(&format!("({})", parts.join(", ")));
    }
    out
}

pub fn note_ref_text(n: &NoteRef) -> String {
    match n {
        NoteRef::Absolute(name) => name.clone(),
        NoteRef::Degree(d, o) => format!("{}.{}", d, o),
        NoteRef::Midi(m) => note_name(*m),
    }
}

fn note_text(n: &NoteStep) -> String {
    let mut out = String::new();
    if n.slide {
        out.push('~');
    }
    out.push_str(&note_ref_text(&n.note));
    if let Some(v) = n.velocity {
        out.push_str(&format!(":{}", amount(v)));
    }
    out.push_str(&plock_text(&n.plock));
    out
}

fn chord_text(c: &ChordStep) -> String {
    let notes: Vec<String> = c.notes.iter().map(note_text).collect();
    let mut out = format!("[{}]", notes.join(" "));
    if let Some(v) = c.velocity {
        out.push_str(&format!(":{}", amount(v)));
    }
    out.push_str(&plock_text(&c.plock));
    out
}

/// `X`, `x`, `o` for the three velocities the letters mean, `x:v` otherwise.
fn hit_letter(velocity: f32) -> String {
    let near = |a: f32, b: f32| (a - b).abs() < 0.001;
    if near(velocity, 1.0) {
        String::from("X")
    } else if near(velocity, 0.8) {
        String::from("x")
    } else if near(velocity, 0.35) {
        String::from("o")
    } else {
        format!("x:{}", amount(velocity))
    }
}

fn drum_text(d: &DrumStep) -> String {
    let mut out = hit_letter(d.velocity);
    if d.roll > 1 {
        out.push_str(&format!("*{}", d.roll));
    }
    if d.probability < 0.999 {
        out.push_str(&format!("?{}", amount(d.probability)));
    }
    out.push_str(&plock_text(&d.plock));
    out
}

/// One step as its token.
pub fn step_text(step: &Step) -> String {
    match step {
        Step::Rest => String::from("-"),
        Step::Tie => String::from(".."),
        Step::Note(n) => note_text(n),
        Step::Chord(c) => chord_text(c),
        Step::DrumHit(d) => drum_text(d),
        Step::Subdiv(notes) => {
            let parts: Vec<String> = notes.iter().map(note_text).collect();
            format!("<{}>", parts.join(" "))
        }
        Step::DrumSub(hits) => {
            let parts: Vec<String> =
                hits.iter().map(|&v| if v <= 0.0 { String::from("-") } else { hit_letter(v) }).collect();
            format!("<{}>", parts.join(" "))
        }
    }
}

/// A row of steps on one line, a space between tokens and a wider gap
/// every four, so a bar reads as beats.
fn row_text(steps: &[Step]) -> String {
    let mut out = String::new();
    for (i, s) in steps.iter().enumerate() {
        if i > 0 {
            out.push_str(if i % 4 == 0 { "   " } else { " " });
        }
        out.push_str(&step_text(s));
    }
    out
}

/// The body of a pattern block, between its braces: one line per row, or
/// one per drum lane with its label.
pub fn pattern_body(p: &PatternDef) -> String {
    let mut out = String::from("\n");
    if p.lane_labels.is_empty() {
        for row in &p.rows {
            out.push_str("    ");
            out.push_str(&row_text(row));
            out.push('\n');
        }
    } else {
        let width = p.lane_labels.iter().map(|l| l.len()).max().unwrap_or(0) + 1;
        for (label, row) in p.lane_labels.iter().zip(&p.rows) {
            out.push_str(&format!("    {:<width$} {}\n", format!("{}:", label), row_text(row), width = width));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::parse;

    fn round_trip(body: &str) -> PatternDef {
        let text = format!("pattern t {{\n{}\n}}\n", body);
        let song = parse(&text).unwrap();
        let written = format!("pattern t {{{}}}\n", pattern_body(&song.patterns[0]));
        let again = parse(&written).unwrap_or_else(|e| panic!("{}\n{:?}", written, e));
        assert_eq!(again.patterns[0], song.patterns[0], "written as:\n{}", written);
        song.patterns[0].clone()
    }

    #[test]
    fn every_melodic_form_survives_a_round_trip() {
        round_trip("1.2:0.9  -  ~5.2:0.8  ..  [1.3 3.3 5.3]:0.6  ..  ..  ..");
        round_trip("A2:0.8 .. ~B2:0.9 .. <D4:0.6 F#4:0.7 A4:0.8> C#5*4 - ..*3");
        round_trip("1.2:0.8(cutoff=0.4, edepth=0.3, res=0.6, gate=0.5) A2?0.5 1.3:0.8?0.25 [A3 C4]?0.5 -*4");
        round_trip("1.1 - - -   2.1 - - -\n    3.1 - - -   4.1 - - -");
    }

    #[test]
    fn every_drum_form_survives_a_round_trip() {
        let p = round_trip("kick: X - - -  x - - -  X - - -  x - x:0.6 -\n    snare: - - - -  X - - -  - - - -  X - - -\n    hat: o x x*2 x?0.4  <x o - X> x x x  x x x x  x x x x");
        assert_eq!(p.lane_labels, ["kick", "snare", "hat"]);
        assert_eq!(p.rows[2].len(), 16);
    }

    #[test]
    fn a_chord_symbol_comes_back_as_its_notes() {
        let song = parse("pattern t { Fm9:0.7 - - - }\n").unwrap();
        let body = pattern_body(&song.patterns[0]);
        assert!(body.contains("[F3 G#3 C4 D#4 G4]:0.7"), "{body}");
        assert_eq!(note_name(60), "C4");
        assert_eq!(note_name(61), "C#4");
        assert_eq!(note_name(33), "A1");
    }
}
