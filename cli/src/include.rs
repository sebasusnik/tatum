//! `use "rig.synth"` on a disk: the core resolves includes over a
//! [`Files`] (`tatum_core::include`), and this is that over `std::fs`, with
//! the paths back as `PathBuf`s for the watcher and the editors.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use tatum_core::include::Files;
use tatum_core::song_engine::DslError;

/// The disk, as the core reads files.
pub struct Disk;

impl Files for Disk {
    fn read(&self, path: &str) -> Option<String> {
        fs::read_to_string(path).ok()
    }
}

pub struct Source {
    /// The combined text the core parses.
    pub text: String,
    /// Files in the order they were first opened; index 0 is the root.
    pub files: Vec<PathBuf>,
    inner: tatum_core::include::Source,
}

impl Source {
    pub fn load(path: &Path) -> Result<Source, String> {
        let inner = tatum_core::include::Source::load(&path.to_string_lossy(), &Disk)?;
        Ok(Source { text: inner.text.clone(), files: inner.files.iter().map(PathBuf::from).collect(), inner })
    }

    /// Where a 1-based line of the combined text came from.
    pub fn locate(&self, combined_line: usize) -> (&Path, usize) {
        let (file, line) = self.inner.locate(combined_line);
        let i = self.inner.files.iter().position(|f| f == file).unwrap_or(0);
        (&self.files[i], line)
    }

    /// Newest modification time across every file involved.
    pub fn newest_mtime(&self) -> Option<SystemTime> {
        self.files.iter().filter_map(|f| fs::metadata(f).ok()?.modified().ok()).max()
    }

    pub fn print_errors(&self, err: &DslError) {
        for line in self.error_lines(err) {
            eprintln!("{}", line);
        }
    }

    /// The errors as `print_errors` writes them, one line each.
    pub fn error_lines(&self, err: &DslError) -> Vec<String> {
        self.inner.error_lines(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_on_disk_is_read_through_its_uses() {
        let dir = std::env::temp_dir().join(format!("tatum-use-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("rig.synth"), "module beats kit { kick_level 1.0 }\n").unwrap();
        let perf = dir.join("perf.synth");
        fs::write(&perf, "tempo 120\nuse \"rig.synth\"\n").unwrap();
        let src = Source::load(&perf).unwrap();
        assert_eq!(src.text, "tempo 120\nmodule beats kit { kick_level 1.0 }\n");
        assert_eq!(src.files.len(), 2);
        assert!(src.locate(2).0.ends_with("rig.synth"));
        assert!(src.newest_mtime().is_some());
        let err = Source::load(&dir.join("nope.synth")).err().expect("must fail");
        assert!(err.starts_with("cannot read"), "{}", err);
    }
}
