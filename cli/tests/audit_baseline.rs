//! A net under the engine: if a DSP change starts adding grit to everything,
//! something has to go red.
//!
//! Nothing else in the suite would notice. The bit-identity tests compare a
//! render against another render of the same code, the callback-budget test
//! measures time, and the lints read text. A filter that begins ringing, a
//! saturator that gets hotter, an interpolation that starts aliasing -- all
//! of that lands in the audit's number and nowhere else.
//!
//! Three songs, chosen to span what the measurement can see rather than to
//! cover the catalogue:
//!
//!   - `neon_arterial` has the widest range in one file: a pure FM bell at
//!     -50 dB, a saturated pad at -4, and five voices in between.
//!   - `liquid_dnb` carries a real chorus finding (`glass`, 11.6 dB), so the
//!     detector's own output is pinned and not just the timbres.
//!   - `detroit` is a different genre at a different tempo, with the
//!     cleanest voice in the corpus.
//!
//! Three and not thirty on purpose. Thirty would take twenty-five minutes and
//! would make regenerating the file routine, which is how a baseline stops
//! being read.
//!
//! Release only: the audit renders every track twice and a debug build is
//! five times slower. CI runs it in the `ci` profile (release without fat
//! LTO).

use std::collections::BTreeMap;
use std::process::{Command, Stdio};

const SONGS: &[&str] = &[
    "examples/detroit.synth",
    "examples/liquid_dnb.synth",
    "examples/neon_arterial.synth",
];

/// How far a number may move before it is a regression. Far above any
/// platform difference in the last bits of an FFT, far below the 16.7 dB a
/// real one cost.
const TOLERANCE_DB: f64 = 1.5;

const BASELINE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/audit_baseline.txt");

/// `song  track  median_db  [effect db]...`
type Key = (String, String);
#[derive(Debug, PartialEq)]
struct Entry {
    median: f64,
    blame: Vec<(String, f64)>,
}

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").canonicalize().unwrap()
}

/// Read one number out of a `"key": value` pair. The audit's JSON is written
/// by hand and read by hand: a dependency for six fields either side would be
/// the tail wagging the dog.
fn num(line: &str, key: &str) -> Option<f64> {
    let at = line.find(&format!("\"{key}\":"))? + key.len() + 3;
    line[at..].trim_start().split([',', '}', ']']).next()?.trim().parse().ok()
}

fn text(line: &str, key: &str) -> Option<String> {
    let at = line.find(&format!("\"{key}\":"))? + key.len() + 3;
    let rest = line[at..].trim_start().strip_prefix('"')?;
    Some(rest[..rest.find('"')?].to_string())
}

fn measure(songs: &[&str]) -> BTreeMap<Key, Entry> {
    let mut out = BTreeMap::new();
    // All at once: one after the other they took as long as the rest of the
    // suite together. Each audit also renders its tracks side by side, so
    // this keeps every core busy until the last song is done.
    let running: Vec<_> = songs
        .iter()
        .map(|song| {
            Command::new(env!("CARGO_BIN_EXE_tatum"))
                .current_dir(root())
                .args(["audit", song, "--json"])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("run tatum audit")
        })
        .collect();
    for (song, child) in songs.iter().zip(running) {
        let res = child.wait_with_output().expect("wait for tatum audit");
        let stdout = String::from_utf8_lossy(&res.stdout);
        assert!(
            stdout.contains("\"tracks\""),
            "{song}: audit produced no tracks\n{}",
            String::from_utf8_lossy(&res.stderr)
        );
        for line in stdout.lines().filter(|l| l.contains("\"median_db\"")) {
            let Some(track) = text(line, "track") else { continue };
            let median = num(line, "median_db").expect("median_db");
            // `"blame": [{"what": "chorus", "db": 11.63}]`
            let mut blame = Vec::new();
            if let Some(rest) = line.split("\"blame\":").nth(1) {
                for part in rest.split("{\"what\":").skip(1) {
                    let what = part.trim_start().trim_start_matches('"');
                    let what = what[..what.find('"').unwrap()].to_string();
                    if let Some(db) = num(part, "db") {
                        blame.push((what, db));
                    }
                }
            }
            out.insert(((*song).to_string(), track), Entry { median, blame });
        }
    }
    out
}

fn write_baseline(m: &BTreeMap<Key, Entry>) -> String {
    let mut s = String::from(
        "# The audit's fingerprint of three songs. A number here moving by more than\n\
         # 1.5 dB means the engine changed how a voice sounds, which is either the point\n\
         # of the commit or a bug in it.\n\
         #\n\
         # These are NOT comparable to each other: a saturated pad reads dirtier than a\n\
         # bell however clean both are. Each one is only comparable to itself.\n\
         #\n\
         # Regenerate, and then READ THE DIFF:\n\
         #   UPDATE_AUDIT_BASELINE=1 cargo test --release -p tatum-cli --test audit_baseline\n\
         #\n\
         # song  track  median_db  [effect db]...\n",
    );
    for ((song, track), e) in m {
        s.push_str(&format!("{song}\t{track}\t{:.2}", e.median));
        for (what, db) in &e.blame {
            s.push_str(&format!("\t{what} {db:.2}"));
        }
        s.push('\n');
    }
    s
}

fn read_baseline(text: &str) -> BTreeMap<Key, Entry> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') { continue }
        let mut cols = line.split('\t');
        let song = cols.next().unwrap().to_string();
        let track = cols.next().expect("track column").to_string();
        let median = cols.next().expect("median column").trim().parse().expect("a number");
        let blame = cols
            .map(|c| {
                let (w, d) = c.trim().split_once(' ').expect("`effect db`");
                (w.to_string(), d.parse().expect("a number"))
            })
            .collect();
        out.insert((song, track), Entry { median, blame });
    }
    out
}

/// One song against its lines in the baseline. A test per song so that CI
/// can give each its own runner; locally they run side by side anyway.
fn check(song: &str) {
    // Regenerating is `baseline_is_rewritten_when_asked`'s job: it needs all
    // three songs to write one file.
    if std::env::var("UPDATE_AUDIT_BASELINE").is_ok() { return }
    let now = measure(&[song]);
    let mut was = read_baseline(&std::fs::read_to_string(BASELINE).expect("audit_baseline.txt"));
    was.retain(|(s, _), _| s == song);
    let mut moved = Vec::new();

    for (key, old) in &was {
        match now.get(key) {
            None => moved.push(format!("{} / {}: gone from the audit entirely", key.0, key.1)),
            Some(new) => {
                if (new.median - old.median).abs() > TOLERANCE_DB {
                    moved.push(format!(
                        "{} / {}: median {:.2} -> {:.2} dB ({:+.2})",
                        key.0, key.1, old.median, new.median, new.median - old.median
                    ));
                }
                for (what, db) in &old.blame {
                    match new.blame.iter().find(|(w, _)| w == what) {
                        None => moved.push(format!(
                            "{} / {}: the {what} no longer accounts for anything (was {db:.2} dB)",
                            key.0, key.1
                        )),
                        Some((_, now_db)) if (now_db - db).abs() > TOLERANCE_DB => {
                            moved.push(format!(
                                "{} / {}: the {what} accounts for {db:.2} -> {now_db:.2} dB",
                                key.0, key.1
                            ));
                        }
                        Some(_) => {}
                    }
                }
                for (what, db) in &new.blame {
                    if !old.blame.iter().any(|(w, _)| w == what) {
                        moved.push(format!(
                            "{} / {}: the {what} now accounts for {db:.2} dB and did not before",
                            key.0, key.1
                        ));
                    }
                }
            }
        }
    }
    for key in now.keys() {
        if !was.contains_key(key) {
            moved.push(format!("{} / {}: new, not in the baseline", key.0, key.1));
        }
    }

    assert!(
        moved.is_empty(),
        "the audit measures these differently now:\n  {}\n\n\
         If the commit meant to change how they sound, regenerate and read the diff:\n  \
         UPDATE_AUDIT_BASELINE=1 cargo test --release -p tatum-cli --test audit_baseline",
        moved.join("\n  ")
    );
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "release only: the audit renders every track twice and debug is ~5x slower"
)]
fn detroit_still_measures_the_way_it_did() {
    check("examples/detroit.synth");
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "release only: the audit renders every track twice and debug is ~5x slower"
)]
fn liquid_dnb_still_measures_the_way_it_did() {
    check("examples/liquid_dnb.synth");
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "release only: the audit renders every track twice and debug is ~5x slower"
)]
fn neon_arterial_still_measures_the_way_it_did() {
    check("examples/neon_arterial.synth");
}

/// Does nothing unless `UPDATE_AUDIT_BASELINE` is set.
#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "release only: the audit renders every track twice and debug is ~5x slower"
)]
fn baseline_is_rewritten_when_asked() {
    if std::env::var("UPDATE_AUDIT_BASELINE").is_err() { return }
    std::fs::write(BASELINE, write_baseline(&measure(SONGS))).unwrap();
    eprintln!("wrote {BASELINE} -- read the diff before committing it");
}
