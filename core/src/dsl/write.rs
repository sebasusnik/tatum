//! Patterns written back as text: a step as its token, a pattern as its
//! block body. What a grid edits is the parsed pattern; what it saves is
//! this, put back into the file between the pattern's braces. Every form
//! the parser reads is written the way the docs spell it, except a chord
//! symbol (`Fm9`), which the parser resolves to notes and comes back as
//! `[F3 Ab3 C4 Eb4 G4]`, and a ratchet (`a*3`), which comes back as the
//! `<a a a>` it is sugar for.

use alloc::string::String;
use alloc::vec::Vec;

use super::ast::{ChordStep, DrumStep, NoteRef, NoteStep, PLock, Param, PatternDef, RoutingNode, Step};

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

/// One argument of a chain node, in the unit its spec says it is in:
/// `700hz`, `1.2khz`, `40ms`, `-3db`; a linear gain (a compressor's
/// `makeup`) as the decibels the parser wants for it; anything else bare.
fn arg_text(kind: &str, name: Option<&str>, index: usize, v: f32) -> String {
    let doc = crate::nodes::arg_at(kind, name, index).map_or("", |a| a.doc);
    if doc.starts_with("Hz") {
        if v.abs() >= 1000.0 {
            format!("{}khz", amount(v / 1000.0))
        } else {
            format!("{}hz", amount(v))
        }
    } else if doc.starts_with("ms") {
        format!("{}ms", amount(v))
    } else if doc.starts_with("dB") {
        format!("{}db", amount(v))
    } else if doc.starts_with("linear gain") && v > 0.0 {
        format!("{}db", amount(20.0 * crate::math::log10(v)))
    } else {
        amount(v)
    }
}

/// A chain node as the routing line writes it: `lowpass(700hz, 0.1) as lp`.
/// Numbers in order are the positionals, words (`1/8`, `saw`) as they were.
pub fn chain_node_text(node: &RoutingNode) -> String {
    let mut args = Vec::new();
    let mut index = 0;
    for p in &node.params {
        args.push(match p {
            Param::Float(v) => {
                index += 1;
                arg_text(&node.kind, None, index - 1, *v)
            }
            Param::Expr(e) => {
                index += 1;
                arg_text(&node.kind, None, index - 1, e.eval())
            }
            Param::Named(name, v) => format!("{}={}", name, arg_text(&node.kind, Some(name), 0, *v)),
            Param::Waveform(w) => w.clone(),
            Param::RhythmDiv(n, d) => format!("{}/{}", n, d),
        });
    }
    let mut out = node.kind.clone();
    if !args.is_empty() {
        out.push_str(&format!("({})", args.join(", ")));
    }
    if let Some(label) = &node.label {
        out.push_str(" as ");
        out.push_str(label);
    }
    out
}

/// A track's routing line, a node per line under the first:
/// `out > saturate(0.3) as drive\n        > master`.
pub fn routing_text(nodes: &[RoutingNode], indent: &str) -> String {
    let mut out = String::from("out");
    for (i, node) in nodes.iter().enumerate() {
        if i == 0 {
            out.push_str(" > ");
        } else {
            out.push('\n');
            out.push_str(indent);
            out.push_str("    > ");
        }
        out.push_str(&chain_node_text(node));
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

    fn close(a: &Param, b: &Param) -> bool {
        let near = |x: f32, y: f32| (x - y).abs() <= 1e-3 * x.abs().max(1.0);
        match (a, b) {
            (Param::Float(x), Param::Float(y)) => near(*x, *y),
            (Param::Expr(e), Param::Float(y)) => near(e.eval(), *y),
            (Param::Named(k, x), Param::Named(l, y)) => k == l && near(*x, *y),
            _ => a == b,
        }
    }

    /// Every chain in the repo's songs and voices, written and read back,
    /// is the chain it was: kinds, labels, words, numbers in their units.
    #[test]
    fn every_chain_in_the_corpus_survives_a_round_trip() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut files = Vec::new();
        for dir in [
            "examples",
            "compose/grooves",
            "compose/voices/bass",
            "compose/voices/lead",
            "compose/voices/pad",
            "compose/voices/perc",
        ] {
            for entry in std::fs::read_dir(root.join(dir)).unwrap().flatten() {
                if entry.path().extension().is_some_and(|e| e == "synth") {
                    files.push(entry.path());
                }
            }
        }
        let mut chains = 0;
        for path in files {
            let text = std::fs::read_to_string(&path).unwrap();
            let Ok(song) = parse(&text) else { continue };
            for t in song.tracks.iter().filter(|t| !t.routing.is_empty()) {
                let probe = format!("track x {{\n    {}\n}}\n", routing_text(&t.routing, "    "));
                let back = parse(&probe).unwrap_or_else(|e| panic!("{}: {:?}\n{}", path.display(), e, probe));
                let got = &back.tracks[0].routing;
                assert_eq!(got.len(), t.routing.len(), "{}\n{}", path.display(), probe);
                for (a, b) in t.routing.iter().zip(got) {
                    assert_eq!(a.kind, b.kind, "{}", path.display());
                    assert_eq!(a.label, b.label, "{}", path.display());
                    assert_eq!(a.params.len(), b.params.len(), "{}: {}", path.display(), probe);
                    for (x, y) in a.params.iter().zip(&b.params) {
                        assert!(close(x, y), "{}: {:?} came back {:?}\n{}", path.display(), x, y, probe);
                    }
                }
                chains += 1;
            }
        }
        assert!(chains > 50, "only {chains} chains");
    }
}
