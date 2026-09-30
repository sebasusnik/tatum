//! Transforming a pattern on its `play` line: `rev`, `fast 2`, `every 4 rev`,
//! `a, b`, and the pattern notation that came with them (`?` on notes, hits
//! inside a step, Euclidean rhythms).

use tatum_core::dsl::{self, ast::Transform, compiler};
use tatum_core::dsl::compiler::{CompiledStep, CompiledSong};
use tatum_core::song_engine::SongEngine;

fn song(body: &str) -> String {
    format!("tempo 120\nscale A minor\nmodule bass b {{ cutoff 800hz }}\nmodule beats kit {{ }}\n{}\n", body)
}

fn compile(src: &str) -> CompiledSong {
    compiler::compile(&dsl::parse(src).expect("parses")).expect("compiles")
}

/// The steps of the pattern track 0 plays, as a string: a note name for an
/// onset, `..` for a tie and `-` for a rest.
fn shown(c: &CompiledSong, pattern_idx: usize) -> String {
    let p = &c.patterns[pattern_idx];
    let steps = if p.lanes.is_empty() { &p.steps } else { &p.lanes[0].steps };
    steps
        .iter()
        .map(|s| match s {
            CompiledStep::NoteOn { midi_note, .. } => format!("{}", midi_note),
            CompiledStep::Subdiv { notes, count, .. } => {
                let inner: Vec<String> = notes[..*count as usize]
                    .iter()
                    .map(|n| if n.velocity > 0.0 { format!("{}", n.midi_note) } else { "-".into() })
                    .collect();
                format!("<{}>", inner.join(" "))
            }
            CompiledStep::DrumHit { .. } => "x".into(),
            CompiledStep::DrumSub { hits, count, .. } => {
                let inner: Vec<&str> =
                    hits[..*count as usize].iter().map(|v| if *v > 0.0 { "x" } else { "-" }).collect();
                format!("<{}>", inner.join(""))
            }
            CompiledStep::Chord { .. } => "chord".into(),
            CompiledStep::Tie => "..".into(),
            CompiledStep::Rest => "-".into(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn played(src: &str) -> String {
    let c = compile(src);
    shown(&c, c.tracks[0].pattern_idx)
}

// A2 = 45, C3 = 48, E3 = 52, G3 = 55.
const LINE: &str = "pattern p { A2 .. .. - C3 - E3 - }";

#[test]
fn the_play_line_reads_its_transforms_in_order() {
    let ast =
        dsl::parse(&song("pattern p { A2 - } pattern q { C3 - }\ntrack t { play p, q every 4 rev shift 2 using b }"))
            .unwrap();
    let t = &ast.tracks[0];
    assert_eq!(t.play, "p");
    assert_eq!(t.play_also, vec!["q".to_string()]);
    assert_eq!(t.transforms, vec![Transform::Every(4, Box::new(Transform::Rev)), Transform::Shift(2)]);
    assert_eq!(t.using_instrument, "b", "the track's next option is still read");
}

#[test]
fn a_misspelt_transform_says_which_one_was_meant() {
    let errs = dsl::parse(&song(&format!("{}\ntrack t {{ play p reverse using b }}", LINE))).unwrap_err();
    assert!(errs.iter().any(|e| e.message.contains("Did you mean 'rev'")), "{:?}", errs);
}

#[test]
fn rev_keeps_a_tied_note_whole() {
    assert_eq!(played(&song(&format!("{}\ntrack t {{ play p rev using b }}", LINE))), "- 52 - 48 - 45 .. ..");
}

#[test]
fn shift_rotates_and_wraps() {
    assert_eq!(played(&song(&format!("{}\ntrack t {{ play p shift 2 using b }}", LINE))), "52 - 45 .. .. - 48 -");
    assert_eq!(played(&song(&format!("{}\ntrack t {{ play p shift -1 using b }}", LINE))), ".. .. - 48 - 52 - 45");
}

#[test]
fn fast_plays_the_pattern_twice_in_its_own_length() {
    let src = song("pattern p { A2 - C3 - E3 - G3 - }\ntrack t { play p fast 2 using b }");
    assert_eq!(played(&src), "45 48 52 55 45 48 52 55");
    // Two onsets in one step become a run inside it.
    let src = song("pattern p { A2 C3 E3 - }\ntrack t { play p fast 2 using b }");
    assert_eq!(played(&src), "<45 48> 52 <45 48> 52");
}

#[test]
fn slow_stretches_every_step_and_holds_notes() {
    let src = song("pattern p { A2 C3 - E3 }\ntrack t { play p slow 2 using b }");
    assert_eq!(played(&src), "45 .. 48 .. - - 52 ..");
}

#[test]
fn up_moves_by_degrees_of_the_scale_and_st_by_semitones() {
    // A minor: A2 up two degrees is C3, and E3 up two is G3.
    let src = song("pattern p { A2 E3 }\ntrack t { play p up 2 using b }");
    assert_eq!(played(&src), "48 55");
    let src = song("pattern p { A2 E3 }\ntrack t { play p up 5st using b }");
    assert_eq!(played(&src), "50 57");
    let src = song("pattern p { A2 E3 }\ntrack t { play p octave -1 using b }");
    assert_eq!(played(&src), "33 40");
}

#[test]
fn fast_on_drums_puts_hits_inside_the_step() {
    let src = song("pattern d { kick: x - - x }\ntrack t { play d fast 2 using kit }");
    assert_eq!(played(&src), "x <-x> x <-x>");
}

#[test]
fn a_plain_play_makes_no_new_pattern() {
    let c = compile(&song(&format!("{}\ntrack t {{ play p using b }}", LINE)));
    assert_eq!(c.patterns.len(), 1);
    assert!(c.tracks[0].play.is_none() && c.plays.is_empty());
}

/// Which pattern each of `loops` loops plays, by name.
fn loops(src: &str, loops: usize, len: usize) -> Vec<String> {
    let mut e = SongEngine::from_source(src).unwrap();
    e.start();
    let mut out = Vec::new();
    for _ in 0..loops {
        e.render_steps(1);
        out.push(e.pattern_name(e.track_pattern(0)).to_string());
        e.render_steps(len - 1);
    }
    out
}

#[test]
fn every_4_transforms_the_last_loop_of_each_four() {
    let src = song("pattern p { A2 - C3 - }\ntrack t { play p every 4 rev using b }\nscene s { track t { play p every 4 rev using b } }\narrange { s x4 }");
    let names = loops(&src, 8, 4);
    let transformed: Vec<bool> = names.iter().map(|n| n != &names[0]).collect();
    assert_eq!(transformed, vec![false, false, false, true, false, false, false, true], "{:?}", names);
}

#[test]
fn a_comma_alternates_patterns_loop_by_loop() {
    let src = song("pattern p { A2 - C3 - }\npattern q { E3 - - - }\ntrack t { play p, q using b }\nscene s { track t { play p, q using b } }\narrange { s x4 }");
    let names = loops(&src, 4, 4);
    assert!(names[0].starts_with("p ") && names[1].starts_with("q ") && names[2].starts_with("p "), "{:?}", names);
}

#[test]
fn sometimes_is_the_same_every_render() {
    let src = song("pattern p { A2 - C3 - }\ntrack t { play p sometimes 50% rev using b }\nscene s { track t { play p sometimes 50% rev using b } }\narrange { s x8 }");
    let a = loops(&src, 16, 4);
    let b = loops(&src, 16, 4);
    assert_eq!(a, b, "the draw must come from the track's own seeded stream");
    assert!(a.iter().any(|n| n != &a[0]), "half the loops should be reversed: {:?}", a);
}

#[test]
fn notes_take_a_chance() {
    let c = compile(&song("pattern p { A2?0.25 C3:0.9?0.5 - - }\ntrack t { play p using b }"));
    let probs: Vec<Option<f32>> = c.patterns[0]
        .steps
        .iter()
        .filter_map(|s| match s {
            CompiledStep::NoteOn { plock, .. } => Some(plock.probability),
            _ => None,
        })
        .collect();
    assert_eq!(probs, vec![Some(0.25), Some(0.5)]);
}

#[test]
fn a_euclidean_rhythm_spreads_its_hits() {
    let c = compile(&song("pattern d { kick: x(3,8) }\ntrack t { play d using kit }"));
    assert_eq!(shown(&c, 0), "x - - x - - x -");
}

#[test]
fn a_drum_group_puts_several_hits_in_one_step() {
    let c = compile(&song("pattern d { kick: <x o - X> - - - }\ntrack t { play d using kit }"));
    assert_eq!(shown(&c, 0), "<xx-x> - - -");
}

#[test]
fn degrade_is_heard_as_fewer_notes() {
    let plain = song("pattern d { hat: x x x x x x x x x x x x x x x x }\ntrack t { play d using kit }\nscene s { track t { play d using kit } }\narrange { s x4 }");
    let thin = plain.replace("play d using kit", "play d degrade 50% using kit");
    let energy = |src: &str| {
        let mut e = SongEngine::from_source(src).unwrap();
        let (l, r) = e.render(4);
        l.iter().chain(r.iter()).map(|x| x * x).sum::<f32>()
    };
    let (a, b) = (energy(&plain), energy(&thin));
    assert!(b < a * 0.8 && b > a * 0.2, "half the hats gone should read as about half the energy: {} -> {}", a, b);
}
