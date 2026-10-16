//! `use "rig.synth"`: one file pulls another in, so a performance file can be
//! a dozen lines on top of a rig tuned offline. The core has no filesystem:
//! whoever calls it hands over a [`Files`], and this module resolves the
//! includes before parsing and keeps, for every combined line, which file
//! and line it came from, so errors point at the file the user is editing.
//!
//! A `use` line is replaced in place by the file it names, resolved relative
//! to the file that contains it. Later definitions win for globals (`tempo`,
//! `scale`), so a performance file that `use`s its rig first can override
//! them. A missing file or a cycle is an error, never a silent skip.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::song_engine::DslError;

/// Where the text of a file comes from: a disk, a map of strings in a
/// browser, a test. Paths are as `use` lines and callers write them, joined
/// and normalized by [`join`] before they get here.
pub trait Files {
    /// The text of `path`, or `None` when there is no such file.
    fn read(&self, path: &str) -> Option<String>;
}

pub struct Source {
    /// The combined text the core parses.
    pub text: String,
    /// Files in the order they were first opened; index 0 is the root.
    pub files: Vec<String>,
    /// For each combined line (0-based): (file index, 1-based line in it).
    lines: Vec<(usize, usize)>,
}

impl Source {
    pub fn load(path: &str, files: &dyn Files) -> Result<Source, String> {
        let mut src = Source { text: String::new(), files: Vec::new(), lines: Vec::new() };
        let mut stack = Vec::new();
        src.expand(&normalize(path), files, &mut stack)?;
        Ok(src)
    }

    fn expand(&mut self, path: &str, files: &dyn Files, stack: &mut Vec<String>) -> Result<(), String> {
        if stack.iter().any(|p| p == path) {
            let chain: Vec<&str> = stack.iter().map(String::as_str).chain(core::iter::once(path)).collect();
            return Err(format!("`use` cycle: {}", chain.join(" -> ")));
        }
        let text = files.read(path).ok_or_else(|| format!("cannot read '{}'", path))?;
        let file_idx = match self.files.iter().position(|f| f == path) {
            Some(i) => i,
            None => {
                self.files.push(path.to_string());
                self.files.len() - 1
            }
        };
        stack.push(path.to_string());
        for (i, line) in text.lines().enumerate() {
            match use_target(line) {
                Some(Ok(target)) => {
                    let child = join(path, &target);
                    self.expand(&child, files, stack).map_err(|e| format!("{}:{}: {}", path, i + 1, e))?;
                }
                Some(Err(msg)) => return Err(format!("{}:{}: {}", path, i + 1, msg)),
                None => {
                    self.text.push_str(line);
                    self.text.push('\n');
                    self.lines.push((file_idx, i + 1));
                }
            }
        }
        stack.pop();
        Ok(())
    }

    /// Where a 1-based line of the combined text came from.
    pub fn locate(&self, combined_line: usize) -> (&str, usize) {
        match self.lines.get(combined_line.wrapping_sub(1)) {
            Some(&(f, l)) => (&self.files[f], l),
            None => (&self.files[0], combined_line),
        }
    }

    /// Error location as the user should read it: the file name only when
    /// more than one file is involved.
    fn where_is(&self, line: usize) -> String {
        let (file, l) = self.locate(line);
        if self.files.len() > 1 {
            format!("{}:{}", file_name(file), l)
        } else {
            format!("line {}", l)
        }
    }

    /// The errors as a terminal prints them, one line each.
    pub fn error_lines(&self, err: &DslError) -> Vec<String> {
        let root = &self.files[0];
        let mut out = Vec::new();
        match err {
            DslError::Parse(errs) => {
                out.push(format!("{}: parse errors:", root));
                for e in errs {
                    out.push(format!("  {}: {}", self.where_is(e.line), e.message));
                }
            }
            DslError::Compile(errs) => {
                out.push(format!("{}: compile errors:", root));
                for e in errs {
                    // Line 0: an error about the song as a whole, not a place in it.
                    if e.line == 0 {
                        out.push(format!("  {}", e.message))
                    } else {
                        out.push(format!("  {}: {}", self.where_is(e.line), e.message))
                    }
                }
            }
        }
        out
    }
}

/// `use "file"` or `use file` on a line of its own. Anything else after the
/// keyword is an error rather than a line silently handed to the parser.
pub fn use_target(line: &str) -> Option<Result<String, String>> {
    let t = line.trim();
    let rest = t.strip_prefix("use")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let rest = rest.trim();
    let rest = rest.split("//").next().unwrap_or("").trim();
    let target = rest.strip_prefix('"').and_then(|r| r.strip_suffix('"')).unwrap_or(rest);
    if target.is_empty() || target.contains(char::is_whitespace) || target.contains('"') {
        return Some(Err(format!("expected `use \"file.synth\"`, got `{}`", t)));
    }
    Some(Ok(target.to_string()))
}

/// The last part of a path: `perf.synth` of `sets/a/perf.synth`.
pub fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// The directory of a path, without its trailing slash; empty for a bare name.
pub fn dir_of(path: &str) -> &str {
    path.rfind('/').map(|i| &path[..i]).unwrap_or("")
}

/// `target` as a `use` line in `from` names it: beside that file, unless it
/// is absolute, with `.` and `..` folded.
pub fn join(from: &str, target: &str) -> String {
    if target.starts_with('/') {
        return normalize(target);
    }
    let dir = dir_of(from);
    if dir.is_empty() {
        normalize(target)
    } else {
        normalize(&format!("{}/{}", dir, target))
    }
}

/// A path with `.` and `..` folded and doubled slashes removed, so the same
/// file reached two ways is seen once (and a cycle through `..` is caught).
pub fn normalize(path: &str) -> String {
    let absolute = path.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for p in path.split('/') {
        match p {
            "" | "." => {}
            ".." => match parts.last() {
                Some(&last) if last != ".." => {
                    parts.pop();
                }
                _ if absolute => {}
                _ => parts.push(".."),
            },
            p => parts.push(p),
        }
    }
    let joined = parts.join("/");
    if absolute {
        format!("/{}", joined)
    } else if joined.is_empty() {
        String::from(".")
    } else {
        joined
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    struct Map(Vec<(&'static str, &'static str)>);

    impl Files for Map {
        fn read(&self, path: &str) -> Option<String> {
            self.0.iter().find(|(p, _)| *p == path).map(|(_, t)| t.to_string())
        }
    }

    #[test]
    fn a_use_line_is_replaced_by_the_file_and_lines_map_back() {
        let files = Map(vec![
            ("sets/rig.synth", "module beats kit { kick_level 1.0 }\nmaster { in > out }\n"),
            (
                "sets/perf.synth",
                "tempo 120\nuse \"rig.synth\"\npattern beat { kick: X - - - }\ntrack kick { play beat using kit out > master }\n",
            ),
        ]);
        let src = Source::load("sets/perf.synth", &files).unwrap();
        assert_eq!(src.text, "tempo 120\nmodule beats kit { kick_level 1.0 }\nmaster { in > out }\npattern beat { kick: X - - - }\ntrack kick { play beat using kit out > master }\n");
        assert_eq!(src.files, ["sets/perf.synth", "sets/rig.synth"]);
        assert_eq!(src.locate(1), ("sets/perf.synth", 1));
        assert_eq!(src.locate(3), ("sets/rig.synth", 2));
        assert_eq!(src.locate(4), ("sets/perf.synth", 3));
        assert!(crate::dsl::parse(&src.text).is_ok());
    }

    #[test]
    fn a_missing_file_is_an_error_with_the_including_line() {
        let files = Map(vec![("missing.synth", "tempo 120\nuse \"nope.synth\"\n")]);
        let err = Source::load("missing.synth", &files).err().expect("must fail");
        assert!(err.contains("missing.synth:2"), "{}", err);
        assert!(err.contains("nope.synth"), "{}", err);
    }

    #[test]
    fn a_cycle_is_an_error_even_through_dot_dot() {
        let files = Map(vec![("x/a.synth", "use \"../x/b.synth\"\n"), ("x/b.synth", "use \"a.synth\"\n")]);
        let err = Source::load("x/b.synth", &files).err().expect("must fail");
        assert!(err.contains("cycle"), "{}", err);
    }

    #[test]
    fn a_malformed_use_is_an_error_not_a_line_for_the_parser() {
        let files = Map(vec![("bad.synth", "use rig.synth extra\n")]);
        let err = Source::load("bad.synth", &files).err().expect("must fail");
        assert!(err.contains("expected `use"), "{}", err);
    }

    #[test]
    fn paths_join_beside_the_including_file() {
        assert_eq!(join("sets/viaje/01.synth", "_base.synth"), "sets/viaje/_base.synth");
        assert_eq!(join("sets/viaje/01.synth", "../rigs/live.synth"), "sets/rigs/live.synth");
        assert_eq!(join("song.synth", "rig.synth"), "rig.synth");
        assert_eq!(join("/abs/song.synth", "./rig.synth"), "/abs/rig.synth");
        assert_eq!(join("a/b.synth", "/abs/rig.synth"), "/abs/rig.synth");
        assert_eq!(normalize("./a//b/../c.synth"), "a/c.synth");
        assert_eq!(file_name("sets/a/perf.synth"), "perf.synth");
        assert_eq!(dir_of("perf.synth"), "");
    }
}
