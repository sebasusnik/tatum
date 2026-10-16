//! `tatum compose` on a disk: the loop itself is `tatum_core::compose`;
//! this reads the library and the session file in, writes out what an
//! action says, and runs the live session on the full screen with the
//! loop's panel over it.

use std::fs;
use std::path::{Path, PathBuf};

use tatum_core::compose::{Library, Outcome, Session, GROOVE_ROLE};

/// The session file, from a session path or a directory holding
/// `session.synth`.
fn session_path(path: &Path) -> PathBuf {
    if path.is_dir() {
        path.join("session.synth")
    } else {
        path.to_path_buf()
    }
}

/// Every `.synth` in `dir`, sorted.
fn files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|e| format!("cannot read {}: {}", dir.display(), e))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "synth"))
        .collect();
    out.sort();
    Ok(out)
}

/// The library beside a session file: `grooves/*.synth` and
/// `voices/<role>/*.synth`. A file that is not a voice is skipped, and said.
fn scan(root: &Path) -> Result<Library, String> {
    let mut entries: Vec<(String, String, String)> = Vec::new();
    for p in files(&root.join("grooves"))? {
        let text = fs::read_to_string(&p).map_err(|e| format!("cannot read {}: {}", p.display(), e))?;
        entries.push((GROOVE_ROLE.into(), p.to_string_lossy().into_owned(), text));
    }
    let voices_dir = root.join("voices");
    let mut roles: Vec<PathBuf> = fs::read_dir(&voices_dir)
        .map_err(|e| format!("cannot read {}: {}", voices_dir.display(), e))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    roles.sort();
    for role_dir in roles {
        let role = role_dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        for p in files(&role_dir)? {
            let text = fs::read_to_string(&p).map_err(|e| format!("cannot read {}: {}", p.display(), e))?;
            entries.push((role.clone(), p.to_string_lossy().into_owned(), text));
        }
    }
    let (library, skipped) = Library::from_files(entries.iter().map(|(r, p, t)| (r.as_str(), p.as_str(), t.as_str())))
        .map_err(|e| format!("{} in {}", e, root.join("grooves").display()))?;
    for s in skipped {
        eprintln!("skipped: {}", s);
    }
    Ok(library)
}

/// Open `path` -- a session file, or a directory holding one as
/// `session.synth` -- and the library beside it. A session that does not
/// exist yet is written with the first groove and the first bass.
pub fn open(path: &Path, knob_ccs: Vec<u8>) -> Result<Session, String> {
    let path = session_path(path);
    let root = path.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    let library = scan(&root)?;
    let name = path.to_string_lossy().into_owned();
    if path.exists() {
        let text = fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
        Ok(Session::from_text(library, &name, knob_ccs, &text))
    } else {
        let session = Session::new(library, &name, knob_ccs);
        fs::write(&path, session.text()).map_err(|e| format!("cannot write {}: {}", path.display(), e))?;
        Ok(session)
    }
}

/// Write what an action asked for. The session file among them is what
/// the watcher picks up as a save.
pub fn commit(outcome: &Outcome) -> Result<(), String> {
    for (path, text) in &outcome.writes {
        fs::write(path, text).map_err(|e| format!("cannot write {}: {}", path, e))?;
    }
    Ok(())
}

const USAGE: &str = "usage: tatum compose [<dir> | <session.synth>] [--knobs 74,71,76,77,93,18,19,16] [--device <name>] [--rate <hz>] [--midi <name>] [--buffer <frames>] [--glass]";

pub fn cmd(args: &[String]) {
    let mut path: Option<String> = None;
    let mut rest: Vec<String> = Vec::new();
    let mut knob_ccs = tatum_core::compose::DEFAULT_KNOB_CCS.to_vec();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--knobs" => {
                i += 1;
                let list = args
                    .get(i)
                    .and_then(|v| v.split(',').map(|n| n.trim().parse::<u8>().ok()).collect::<Option<Vec<u8>>>());
                match list {
                    Some(l) if !l.is_empty() && l.iter().all(|&c| c < 128) => knob_ccs = l,
                    _ => {
                        eprintln!("error: --knobs takes controller numbers separated by commas, like 74,71,76,77");
                        std::process::exit(1);
                    }
                }
            }
            "--device" | "-d" | "--rate" | "--midi" | "--buffer" => {
                rest.push(args[i].clone());
                i += 1;
                if let Some(v) = args.get(i) {
                    rest.push(v.clone());
                }
            }
            "--glass" | "--list-devices" | "--list-midi" => rest.push(args[i].clone()),
            "--help" | "-h" => {
                eprintln!("{}", USAGE);
                return;
            }
            other if other.starts_with('-') => {
                eprintln!("error: unknown flag '{}'\n{}", other, USAGE);
                std::process::exit(1);
            }
            other => path = Some(other.to_string()),
        }
        i += 1;
    }
    let path = path.unwrap_or_else(|| "compose".to_string());
    let session = match open(Path::new(&path), knob_ccs) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {}", e);
            std::process::exit(1);
        }
    };
    let session_path = session.path.clone();
    crate::live::cmd_with(&session_path, &rest, Some(session));
}

#[cfg(test)]
mod tests {
    use super::*;
    use tatum_core::compose::Action;
    use tatum_core::live::LivePlanner;

    /// The repo's own library, copied where a test can write.
    fn library_copy(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tatum-compose-{}-{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../compose");
        for sub in ["grooves", "voices/bass", "voices/lead", "voices/pad", "voices/perc"] {
            fs::create_dir_all(dir.join(sub)).unwrap();
            for f in fs::read_dir(src.join(sub)).unwrap().flatten() {
                fs::copy(f.path(), dir.join(sub).join(f.file_name())).unwrap();
            }
        }
        dir
    }

    #[test]
    fn the_repos_library_opens_and_a_save_lands_on_disk() {
        let dir = library_copy("disk");
        let mut session = open(&dir, vec![74]).unwrap();
        let text = fs::read_to_string(dir.join("session.synth")).unwrap();
        assert!(text.contains("use \"grooves/dark.synth\""), "{text}");
        assert!(text.contains("use \"voices/bass/acid.synth\""), "{text}");
        assert_eq!(session.slots.iter().map(|s| s.role.as_str()).collect::<Vec<_>>(), ["bass", "lead", "pad", "perc"]);
        assert_eq!(session.library.grooves.len(), 4);

        let source = crate::include::Source::load(&dir.join("session.synth")).unwrap();
        let mut planner = LivePlanner::new();
        planner.plan(&source.text, 0).unwrap();
        session.act(Action::Turn { row: 1, knob: 0, value: 127 }, &mut planner, 0).unwrap();
        let out = session.act(Action::Save { row: 1, name: "acid_open".into() }, &mut planner, 0).unwrap();
        commit(&out).unwrap();
        let saved = fs::read_to_string(dir.join("voices/bass/acid_open.synth")).unwrap();
        assert!(saved.contains("    cutoff 20000hz\n"), "{saved}");
        let text = fs::read_to_string(dir.join("session.synth")).unwrap();
        assert!(text.contains("use \"voices/bass/acid_open.synth\""), "{text}");
        // Reopened, the file on disk is what the session is.
        let again = open(&dir, vec![74]).unwrap();
        assert_eq!(again.slots[0].voice.as_deref(), Some("acid_open"));
        assert_eq!(again.library.of("bass").len(), 6);
        let _ = fs::remove_dir_all(&dir);
    }
}
