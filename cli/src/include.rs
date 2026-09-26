//! `use "rig.synth"`: one file pulls another in, so a performance file can be
//! a dozen lines on top of a rig tuned offline. The core is `no_std` and has
//! no filesystem, so the CLI resolves includes before parsing and hands the
//! core one source. Every combined line remembers which file and line it came
//! from, so errors point at the file the user is editing.
//!
//! A `use` line is replaced in place by the file it names, resolved relative
//! to the file that contains it. Later definitions win for globals (`tempo`,
//! `scale`), so a performance file that `use`s its rig first can override
//! them. A missing file or a cycle is an error, never a silent skip.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use tatum_core::song_engine::DslError;

pub struct Source {
    /// The combined text the core parses.
    pub text: String,
    /// Files in the order they were first opened; index 0 is the root.
    pub files: Vec<PathBuf>,
    /// For each combined line (0-based): (file index, 1-based line in it).
    lines: Vec<(usize, usize)>,
}

impl Source {
    pub fn load(path: &Path) -> Result<Source, String> {
        let mut src = Source { text: String::new(), files: Vec::new(), lines: Vec::new() };
        let mut stack = Vec::new();
        src.expand(path, &mut stack)?;
        Ok(src)
    }

    fn expand(&mut self, path: &Path, stack: &mut Vec<PathBuf>) -> Result<(), String> {
        let canonical = fs::canonicalize(path).map_err(|e| format!("cannot read '{}': {}", path.display(), e))?;
        if stack.contains(&canonical) {
            let chain: Vec<String> = stack.iter().chain(std::iter::once(&canonical))
                .map(|p| p.display().to_string()).collect();
            return Err(format!("`use` cycle: {}", chain.join(" -> ")));
        }
        let text = fs::read_to_string(&canonical).map_err(|e| format!("cannot read '{}': {}", path.display(), e))?;
        let file_idx = match self.files.iter().position(|f| *f == canonical) {
            Some(i) => i,
            None => { self.files.push(canonical.clone()); self.files.len() - 1 }
        };
        stack.push(canonical.clone());
        let dir = canonical.parent().map(Path::to_path_buf).unwrap_or_default();
        for (i, line) in text.lines().enumerate() {
            match use_target(line) {
                Some(Ok(target)) => {
                    let child = dir.join(target);
                    self.expand(&child, stack).map_err(|e| {
                        format!("{}:{}: {}", canonical.display(), i + 1, e)
                    })?;
                }
                Some(Err(msg)) => return Err(format!("{}:{}: {}", canonical.display(), i + 1, msg)),
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
    pub fn locate(&self, combined_line: usize) -> (&Path, usize) {
        match self.lines.get(combined_line.wrapping_sub(1)) {
            Some(&(f, l)) => (&self.files[f], l),
            None => (&self.files[0], combined_line),
        }
    }

    /// Newest modification time across every file involved.
    pub fn newest_mtime(&self) -> Option<SystemTime> {
        self.files.iter().filter_map(|f| fs::metadata(f).ok()?.modified().ok()).max()
    }

    /// Error location as the user should read it: the file name only when
    /// more than one file is involved.
    fn where_is(&self, line: usize) -> String {
        let (file, l) = self.locate(line);
        if self.files.len() > 1 {
            let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            format!("{}:{}", name, l)
        } else {
            format!("line {}", l)
        }
    }

    pub fn print_errors(&self, err: &DslError) {
        let root = self.files[0].display();
        match err {
            DslError::Parse(errs) => {
                eprintln!("{}: parse errors:", root);
                for e in errs { eprintln!("  {}: {}", self.where_is(e.line), e.message); }
            }
            DslError::Compile(errs) => {
                eprintln!("{}: compile errors:", root);
                for e in errs {
                    // Line 0: an error about the song as a whole, not a place in it.
                    if e.line == 0 { eprintln!("  {}", e.message) } else { eprintln!("  {}: {}", self.where_is(e.line), e.message) }
                }
            }
        }
    }
}

/// `use "file"` or `use file` on a line of its own. Anything else after the
/// keyword is an error rather than a line silently handed to the parser.
fn use_target(line: &str) -> Option<Result<String, String>> {
    let t = line.trim();
    let rest = t.strip_prefix("use")?;
    if !rest.starts_with(char::is_whitespace) { return None; }
    let rest = rest.trim();
    let rest = rest.split("//").next().unwrap_or("").trim();
    let target = rest.strip_prefix('"').and_then(|r| r.strip_suffix('"')).unwrap_or(rest);
    if target.is_empty() || target.contains(char::is_whitespace) || target.contains('"') {
        return Some(Err(format!("expected `use \"file.synth\"`, got `{}`", t)));
    }
    Some(Ok(target.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str, body: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tatum-use-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn a_use_line_is_replaced_by_the_file_and_lines_map_back() {
        tmp("rig.synth", "module beats kit { kick_level 1.0 }\nmaster { in > out }\n");
        let perf = tmp("perf.synth", "tempo 120\nuse \"rig.synth\"\npattern beat { kick: X - - - }\ntrack kick { play beat using kit out > master }\n");
        let src = Source::load(&perf).unwrap();
        assert_eq!(src.text, "tempo 120\nmodule beats kit { kick_level 1.0 }\nmaster { in > out }\npattern beat { kick: X - - - }\ntrack kick { play beat using kit out > master }\n");
        assert_eq!(src.files.len(), 2);
        assert!(src.locate(1).0.ends_with("perf.synth"));
        let (f, l) = src.locate(3);
        assert!(f.ends_with("rig.synth"));
        assert_eq!(l, 2);
        let (f, l) = src.locate(4);
        assert!(f.ends_with("perf.synth"));
        assert_eq!(l, 3);
        assert!(tatum_core::dsl::parse(&src.text).is_ok());
    }

    #[test]
    fn a_missing_file_is_an_error_with_the_including_line() {
        let perf = tmp("missing.synth", "tempo 120\nuse \"nope.synth\"\n");
        let err = Source::load(&perf).err().expect("must fail");
        assert!(err.contains("missing.synth:2"), "{}", err);
        assert!(err.contains("nope.synth"), "{}", err);
    }

    #[test]
    fn a_cycle_is_an_error() {
        tmp("a.synth", "use \"b.synth\"\n");
        let b = tmp("b.synth", "use \"a.synth\"\n");
        let err = Source::load(&b).err().expect("must fail");
        assert!(err.contains("cycle"), "{}", err);
    }

    #[test]
    fn a_malformed_use_is_an_error_not_a_line_for_the_parser() {
        let p = tmp("bad.synth", "use rig.synth extra\n");
        let err = Source::load(&p).err().expect("must fail");
        assert!(err.contains("expected `use"), "{}", err);
    }
}
