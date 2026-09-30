//! The screen's transform keys write the `play` line in the file, and the
//! save goes the same way as one from the editor: the code always says what
//! plays, and there is only one way a change reaches the engine.
//!
//! The line is found in the text and only its `play` clause is rewritten, so
//! the rest of the file -- spacing, comments, the other options on the line
//! -- is left as it was. A track the file does not define (a step of a set
//! that plays its rig's track) gets a one-line redefinition at the end, which
//! only says what it changes.

use tatum_core::dsl::ast::Transform;
use tatum_core::dsl::compiler::transform::describe;

/// What a key does to the selected track's `play` line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Op {
    /// On if it is off, off if it is on.
    Toggle(Toggle),
    /// `shift` one step more or less; at 0 it goes away.
    Shift(i32),
    /// `up` one degree more or less; at 0 it goes away.
    Up(i32),
    /// `degrade`: off, 25%, 50%, off.
    Degrade,
    /// Every transform off; what the line plays stays.
    Clear,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Toggle {
    Rev,
    Fast,
    Slow,
    EveryRev,
}

impl Toggle {
    fn transform(self) -> Transform {
        match self {
            Toggle::Rev => Transform::Rev,
            Toggle::Fast => Transform::Fast(2),
            Toggle::Slow => Transform::Slow(2),
            Toggle::EveryRev => Transform::Every(4, Box::new(Transform::Rev)),
        }
    }
}

/// The words a track's `play` clause says: the patterns and the transforms.
#[derive(Debug, Clone, PartialEq)]
pub struct Clause {
    pub patterns: Vec<String>,
    pub transforms: Vec<Transform>,
}

impl Clause {
    /// Read a clause the way the language does, by parsing it.
    pub fn parse(text: &str) -> Option<Clause> {
        let song = tatum_core::dsl::parse(&format!("track t {{ play {} }}", text)).ok()?;
        let t = song.tracks.first()?;
        let mut patterns = vec![t.play.clone()];
        patterns.extend(t.play_also.iter().cloned());
        Some(Clause { patterns, transforms: t.transforms.clone() })
    }

    pub fn text(&self) -> String {
        describe(&self.patterns[0], &self.patterns[1..], &self.transforms)
    }

    pub fn apply(&mut self, op: Op) {
        let t = &mut self.transforms;
        match op {
            Op::Toggle(which) => {
                let want = which.transform();
                match t.iter().position(|x| *x == want) {
                    Some(i) => {
                        t.remove(i);
                    }
                    None => t.push(want),
                }
            }
            Op::Shift(d) => bump(
                t,
                d,
                |x| matches!(x, Transform::Shift(_)),
                |x| match x {
                    Transform::Shift(n) => Some(*n),
                    _ => None,
                },
                Transform::Shift,
            ),
            Op::Up(d) => bump(
                t,
                d,
                |x| matches!(x, Transform::Up { semitones: false, .. }),
                |x| match x {
                    Transform::Up { amount, semitones: false } => Some(*amount),
                    _ => None,
                },
                |n| Transform::Up { amount: n, semitones: false },
            ),
            Op::Degrade => {
                let at = t.iter().position(|x| matches!(x, Transform::Degrade(_)));
                match at.map(|i| (i, t[i].clone())) {
                    None => t.push(Transform::Degrade(0.25)),
                    Some((i, Transform::Degrade(p))) if p < 0.4 => t[i] = Transform::Degrade(0.5),
                    Some((i, _)) => {
                        t.remove(i);
                    }
                }
            }
            Op::Clear => t.clear(),
        }
    }
}

/// Move a numbered transform by `d`, adding it at `d` or removing it at 0.
fn bump(
    t: &mut Vec<Transform>,
    d: i32,
    is: impl Fn(&Transform) -> bool,
    value: impl Fn(&Transform) -> Option<i32>,
    make: impl Fn(i32) -> Transform,
) {
    match t.iter().position(is) {
        Some(i) => {
            let n = value(&t[i]).unwrap_or(0) + d;
            if n == 0 {
                t.remove(i);
            } else {
                t[i] = make(n);
            }
        }
        None => t.push(make(d)),
    }
}

/// Words that end a `play` clause: the track's other options.
const OPTIONS: [&str; 10] =
    ["using", "level", "pan", "gate", "velocity", "delay_send", "reverb_send", "sidechain", "arp", "out"];

/// `text` with `track`'s `play` clause set to `clause`. `current` is what the
/// track plays now, for a file that does not define the track itself.
pub fn rewrite(text: &str, track: &str, clause: &Clause) -> Result<String, String> {
    // A scene restates what its tracks play, so the top-level line would
    // change nothing that is heard.
    if in_a_scene(text, track) {
        return Err(format!("'{}' plays from its scenes in this file: write the transform on the scene's line", track));
    }
    if let Some((start, end)) = top_level_block(text, track) {
        let block = &text[start..end];
        let new_block = match play_span(block) {
            Some((a, b)) => format!("{}{}{}", &block[..a], clause.text(), &block[b..]),
            None => {
                let open = block.find('{').ok_or("the track block has no `{`")?;
                format!("{} play {}{}", &block[..=open], clause.text(), &block[open + 1..])
            }
        };
        return Ok(format!("{}{}{}", &text[..start], new_block, &text[end..]));
    }
    let mut out = text.to_string();
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out += &format!("track {} {{ play {} }}\n", track, clause.text());
    Ok(out)
}

/// Byte range of `track NAME { ... }` outside any scene, braces matched.
fn top_level_block(text: &str, track: &str) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut depth = 0i32;
    let mut i = 0;
    let mut found = None;
    while i < bytes.len() {
        match bytes[i] {
            b'#' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'{' => depth += 1,
            b'}' => depth -= 1,
            _ => {}
        }
        if depth == 0 && text[i..].starts_with("track") && (i == 0 || !is_word(bytes[i - 1])) {
            let rest = &text[i + 5..];
            let name_start = rest.len() - rest.trim_start().len();
            let name: String = rest.trim_start().chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            let after = rest.trim_start()[name.len()..].trim_start();
            if name == track && after.starts_with('{') && name_start > 0 {
                found = Some(i);
            }
        }
        i += 1;
    }
    let start = found?;
    let open = start + text[start..].find('{')?;
    let mut depth = 0;
    for (k, c) in text[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((start, open + k + 1));
                }
            }
            _ => {}
        }
    }
    None
}

/// Where the words after `play` run, inside a block: up to the next option,
/// the end of the line, a comment or the block's `}`.
fn play_span(block: &str) -> Option<(usize, usize)> {
    let bytes = block.as_bytes();
    let (at, _) = block.match_indices("play").find(|(i, _)| {
        (*i == 0 || !is_word(bytes[i - 1])) && bytes.get(i + 4).is_some_and(|b| *b == b' ' || *b == b'\t')
    })?;
    let blank = |b: u8| b == b' ' || b == b'\t';
    let mut i = at + 4;
    while i < bytes.len() && blank(bytes[i]) {
        i += 1;
    }
    let first = i;
    let mut end = first;
    while i < bytes.len() && !matches!(bytes[i], b'\n' | b'}' | b'#') {
        let word_start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'}' {
            i += 1;
        }
        if OPTIONS.contains(&&block[word_start..i]) {
            break;
        }
        end = i;
        while i < bytes.len() && blank(bytes[i]) {
            i += 1;
        }
    }
    Some((first, end))
}

/// Whether a `scene` block in the text names the track.
fn in_a_scene(text: &str, track: &str) -> bool {
    let mut rest = text;
    while let Some(at) = rest.find("scene ") {
        let body = &rest[at..];
        let Some(open) = body.find('{') else { break };
        let mut depth = 0;
        let mut end = body.len();
        for (k, c) in body[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = open + k + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        let scene = &body[..end];
        let words: Vec<&str> =
            scene.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|w| !w.is_empty()).collect();
        if words.windows(2).any(|w| w[0] == "track" && w[1] == track) {
            return true;
        }
        rest = &rest[at + end.max(6)..];
    }
    false
}

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clause(s: &str) -> Clause {
        Clause::parse(s).unwrap()
    }

    #[test]
    fn keys_toggle_and_bump_transforms() {
        let mut c = clause("acid");
        c.apply(Op::Toggle(Toggle::Rev));
        c.apply(Op::Toggle(Toggle::EveryRev));
        c.apply(Op::Shift(1));
        c.apply(Op::Shift(1));
        c.apply(Op::Up(-1));
        assert_eq!(c.text(), "acid rev every 4 rev shift 2 up -1");
        c.apply(Op::Toggle(Toggle::Rev));
        c.apply(Op::Up(1));
        c.apply(Op::Degrade);
        assert_eq!(c.text(), "acid every 4 rev shift 2 degrade 25%");
        c.apply(Op::Degrade);
        c.apply(Op::Degrade);
        c.apply(Op::Clear);
        assert_eq!(c.text(), "acid");
    }

    #[test]
    fn only_the_play_clause_of_the_line_changes() {
        let text =
            "use \"_rig.synth\"\n# the acid\ntrack acid   { play acid_riff16 level 0.6 }\ntrack hats { play h }\n";
        let out = rewrite(text, "acid", &clause("acid_riff16 every 4 rev")).unwrap();
        assert_eq!(
            out,
            "use \"_rig.synth\"\n# the acid\ntrack acid   { play acid_riff16 every 4 rev level 0.6 }\ntrack hats { play h }\n"
        );
        let back = rewrite(&out, "acid", &clause("acid_riff16")).unwrap();
        assert_eq!(back, text);
    }

    #[test]
    fn a_play_at_the_end_of_a_block_keeps_its_brace() {
        let text = "track t { level 0.5 play p}\n";
        assert_eq!(rewrite(text, "t", &clause("p rev")).unwrap(), "track t { level 0.5 play p rev}\n");
    }

    #[test]
    fn a_track_the_file_does_not_define_gets_a_line_of_its_own() {
        let text = "use \"_rig.synth\"\ntempo 132\n";
        let out = rewrite(text, "roll", &clause("roll_move fast 2")).unwrap();
        assert_eq!(out, "use \"_rig.synth\"\ntempo 132\ntrack roll { play roll_move fast 2 }\n");
    }

    #[test]
    fn a_block_without_play_gets_one() {
        let text = "track t {\n    level 0.5\n}\n";
        let out = rewrite(text, "t", &clause("p shift 1")).unwrap();
        assert!(out.starts_with("track t { play p shift 1\n"), "{}", out);
        assert!(tatum_core::dsl::parse(&format!(
            "pattern p {{ A2 }}\nmodule bass b {{}}\n{}",
            out.replace("level", "using b level")
        ))
        .is_ok());
    }

    #[test]
    fn a_track_its_scenes_play_is_not_rewritten() {
        let text = "track t { play p }\nscene s { track t { play p } }\n";
        let err = rewrite(text, "t", &clause("p rev")).unwrap_err();
        assert!(err.contains("scene"), "{}", err);
        // Another track of the same file, outside every scene, still can be.
        let text = "track t { play p }\ntrack u { play q }\nscene s { track t { play p } }\n";
        assert!(rewrite(text, "u", &clause("q rev")).is_ok());
    }
}

#[cfg(test)]
mod demo {
    use super::*;

    /// The demo defines each track twice: the rig, and the live line under
    /// it. The keys change the live line.
    #[test]
    fn the_live_line_is_the_one_that_changes() {
        let text =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../examples/transform_demo.synth")).unwrap();
        let mut c = Clause::parse("rest").unwrap();
        c.apply(Op::Toggle(Toggle::EveryRev));
        let out = rewrite(&text, "acid", &c).unwrap();
        assert!(out.contains("track acid  { play rest every 4 rev }"), "the live line: {}", &out[out.len() - 400..]);
        assert!(out.contains("track acid  { play rest using acid level 0.5"), "the rig's line is untouched");
        assert!(tatum_core::dsl::parse(&out).is_ok());
    }
}
