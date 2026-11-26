//! The `sounds/` library: `sounds/<type>/<name>.synth`, listed, opened and
//! duplicated from the page. Every path it hands out is built from a type and
//! a name it has checked, inside the one directory it was given, so the page
//! cannot name a file anywhere else.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

pub struct Library {
    /// The library directory, canonical.
    pub root: PathBuf,
}

/// One sound in the library.
#[derive(Debug, PartialEq, Eq)]
pub struct Entry {
    pub kind: String,
    pub name: String,
}

/// A type or a name the library takes: `[a-z0-9_-]`, starting with a letter
/// or digit, at most 48 long. No dots, so no `..` and no extension; no
/// slashes, so no path.
pub fn is_safe(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 48
        && s.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// `type/name` as the page sends it, both checked.
pub fn split(spec: &str) -> Result<(&str, &str), String> {
    let (kind, name) = spec.trim().split_once('/').ok_or_else(|| format!("'{spec}': expected type/name"))?;
    if !is_safe(kind) || !is_safe(name) {
        return Err(format!(
            "'{spec}': a type and a name are lower-case letters, digits, _ and -, with no dots or slashes"
        ));
    }
    Ok((kind, name))
}

impl Library {
    /// The library for a sound at `file`: `explicit` when given, else the
    /// first `sounds` directory found from the file's directory upward
    /// (the directory itself, or one beside it).
    pub fn find(file: &Path, explicit: Option<&Path>) -> Option<Library> {
        if let Some(dir) = explicit {
            return fs::canonicalize(dir).ok().filter(|d| d.is_dir()).map(|root| Library { root });
        }
        let start = file.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
        // A new sound's directory may not be there yet: from the nearest one that is.
        let start = start.ancestors().find_map(|d| fs::canonicalize(d).ok())?;
        for dir in start.ancestors() {
            if dir.file_name().is_some_and(|n| n == "sounds") {
                return Some(Library { root: dir.to_path_buf() });
            }
            let beside = dir.join("sounds");
            if beside.is_dir() {
                return fs::canonicalize(beside).ok().map(|root| Library { root });
            }
        }
        None
    }

    /// Every `<type>/<name>.synth` under the library whose type and name it
    /// takes, sorted. Links are not followed.
    pub fn list(&self) -> Vec<Entry> {
        let mut out = Vec::new();
        let Ok(types) = fs::read_dir(&self.root) else { return out };
        for t in types.flatten() {
            let kind = t.file_name().to_string_lossy().to_string();
            if !is_safe(&kind) || !t.file_type().is_ok_and(|f| f.is_dir()) {
                continue;
            }
            let Ok(files) = fs::read_dir(t.path()) else { continue };
            for f in files.flatten() {
                let file = f.file_name().to_string_lossy().to_string();
                let Some(name) = file.strip_suffix(".synth") else { continue };
                if is_safe(name) && f.file_type().is_ok_and(|t| t.is_file()) {
                    out.push(Entry { kind: kind.clone(), name: name.to_string() });
                }
            }
        }
        out.sort_by(|a, b| (&a.kind, &a.name).cmp(&(&b.kind, &b.name)));
        out
    }

    /// The path of `type/name`, checked, whether or not it exists; its
    /// type directory, when it exists, must be a real directory inside the
    /// library and not a link out of it.
    pub fn path(&self, spec: &str) -> Result<PathBuf, String> {
        let (kind, name) = split(spec)?;
        let dir = self.root.join(kind);
        if let Ok(meta) = fs::symlink_metadata(&dir) {
            if !meta.is_dir() {
                return Err(format!("'{kind}' in the library is not a directory"));
            }
            let real = fs::canonicalize(&dir).map_err(|e| e.to_string())?;
            if !real.starts_with(&self.root) {
                return Err(format!("'{kind}' leads out of the library"));
            }
        }
        Ok(dir.join(format!("{name}.synth")))
    }

    /// An existing sound's path, to open: a plain file, not a link.
    pub fn existing(&self, spec: &str) -> Result<PathBuf, String> {
        let path = self.path(spec)?;
        match fs::symlink_metadata(&path) {
            Ok(m) if m.is_file() => Ok(path),
            Ok(_) => Err(format!("'{spec}' is not a plain file")),
            Err(_) => Err(format!("there is no '{spec}' in the library")),
        }
    }

    /// Write `text` as a new sound `type/name`, never over one that is
    /// there: the file is created only if nothing (not even a link) is.
    pub fn create(&self, spec: &str, text: &str) -> Result<PathBuf, String> {
        let path = self.path(spec)?;
        if fs::symlink_metadata(&path).is_ok() {
            return Err(format!("'{spec}' is already in the library; pick another name"));
        }
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        // Checked again once the directory exists: nothing in between made
        // it a link out.
        self.path(spec)?;
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| format!("cannot create '{spec}': {e}"))?;
        f.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
        Ok(path)
    }

    /// `type/name` of a path inside the library, when it is one.
    pub fn spec_of(&self, path: &Path) -> Option<String> {
        let real = fs::canonicalize(path).ok()?;
        let rel = real.strip_prefix(&self.root).ok()?;
        let mut parts = rel.iter().map(|p| p.to_string_lossy().to_string());
        let (kind, file, None) = (parts.next()?, parts.next()?, parts.next()) else { return None };
        let name = file.strip_suffix(".synth")?;
        (is_safe(&kind) && is_safe(name)).then(|| format!("{kind}/{name}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tatum-lib-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("sounds/kicks")).expect("mkdir");
        fs::write(d.join("sounds/kicks/deep.synth"), "# x\n").expect("write");
        d
    }

    #[test]
    fn names_outside_the_library_are_refused() {
        for bad in [
            "../x",
            "..",
            ".",
            "kicks/..",
            "kicks/../../etc",
            "/etc/passwd",
            "kicks//x",
            "kicks/a/b",
            "kicks/.hidden",
            "kicks/deep.synth",
            "Kicks/deep",
            "kicks/-x",
            "kicks/x y",
            "kicks/x\0",
            "kicks/~x",
            "kicks\\..\\x",
            "kicks/",
            "/x",
            "",
        ] {
            assert!(split(bad).is_err(), "{bad:?} was taken");
        }
        assert_eq!(split("kicks/deep-2_b").ok(), Some(("kicks", "deep-2_b")));
    }

    #[test]
    fn the_library_is_found_lists_and_duplicates_without_overwriting() {
        let d = temp("dup");
        let lib = Library::find(&d.join("sounds/kicks/deep.synth"), None).expect("found");
        assert_eq!(lib.root, fs::canonicalize(d.join("sounds")).unwrap());
        // A new sound in a type that is not there yet.
        let new = Library::find(&d.join("sounds/pads/new.synth"), None).expect("found for a new type");
        assert_eq!(new.root, lib.root);
        // From a song beside it, the library beside it is found too.
        let beside = Library::find(&d.join("song.synth"), None).expect("found beside");
        assert_eq!(beside.root, lib.root);
        fs::write(lib.root.join("kicks/notes.txt"), "").unwrap();
        fs::write(lib.root.join("kicks/Bad Name.synth"), "").unwrap();
        assert_eq!(lib.list(), vec![Entry { kind: "kicks".into(), name: "deep".into() }]);
        let p = lib.create("bass/acid", "# acid\n").expect("created");
        assert!(p.starts_with(&lib.root) && p.ends_with("bass/acid.synth"));
        assert!(lib.create("bass/acid", "again").unwrap_err().contains("already"));
        assert_eq!(fs::read_to_string(&p).unwrap(), "# acid\n");
        assert!(lib.create("../acid", "x").is_err());
        assert!(lib.create("bass/../../acid", "x").is_err());
        assert_eq!(lib.spec_of(&p).as_deref(), Some("bass/acid"));
        assert_eq!(lib.list().len(), 2);
        assert!(lib.existing("bass/nope").is_err());
        let _ = fs::remove_dir_all(&d);
    }

    #[cfg(unix)]
    #[test]
    fn links_out_of_the_library_are_not_followed() {
        let d = temp("link");
        let outside = d.join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("secret.synth"), "# s\n").unwrap();
        let lib = Library::find(&d.join("sounds/kicks/deep.synth"), None).expect("found");
        // A type directory that is a link out of the library.
        std::os::unix::fs::symlink(&outside, lib.root.join("evil")).unwrap();
        assert!(lib.path("evil/secret").is_err());
        assert!(lib.create("evil/new", "x").is_err());
        assert!(!outside.join("new.synth").exists());
        assert!(lib.list().iter().all(|e| e.kind != "evil"));
        // A sound that is a link to a file outside.
        std::os::unix::fs::symlink(outside.join("secret.synth"), lib.root.join("kicks/linked.synth")).unwrap();
        assert!(lib.existing("kicks/linked").is_err());
        assert!(lib.create("kicks/linked", "x").is_err(), "a link is something there");
        assert_eq!(fs::read_to_string(outside.join("secret.synth")).unwrap(), "# s\n");
        let _ = fs::remove_dir_all(&d);
    }
}
