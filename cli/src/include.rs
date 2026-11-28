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
            let chain: Vec<String> =
                stack.iter().chain(std::iter::once(&canonical)).map(|p| p.display().to_string()).collect();
            return Err(format!("`use` cycle: {}", chain.join(" -> ")));
        }
        let text = fs::read_to_string(&canonical).map_err(|e| format!("cannot read '{}': {}", path.display(), e))?;
        let file_idx = match self.files.iter().position(|f| *f == canonical) {
            Some(i) => i,
            None => {
                self.files.push(canonical.clone());
                self.files.len() - 1
            }
        };
        stack.push(canonical.clone());
        let dir = canonical.parent().map(Path::to_path_buf).unwrap_or_default();
        for (i, line) in text.lines().enumerate() {
            match use_target(line) {
                Some(Ok(target)) => {
                    let child = dir.join(target);
                    self.expand(&child, stack).map_err(|e| format!("{}:{}: {}", canonical.display(), i + 1, e))?;
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
        for line in self.error_lines(err) {
            eprintln!("{}", line);
        }
    }

    /// The errors as `print_errors` writes them, one line each.
    pub fn error_lines(&self, err: &DslError) -> Vec<String> {
        let root = self.files[0].display();
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
fn use_target(line: &str) -> Option<Result<String, String>> {
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

#[cfg(test)]
mod sounds_tests {
    use super::Source;
    use std::path::Path;

    /// `sounds/vintage_kit.synth` plays the sounds in the spirit of classic
    /// machines together, through `use`: every one of them is read from its
    /// own file, plays on its own track at a level of its own, and the five
    /// together stay under full scale on their way into the master.
    #[test]
    fn the_vintage_kit_plays_its_sounds_together() {
        let src = Source::load(Path::new("../sounds/vintage_kit.synth")).expect("every use resolves");
        let mut e = tatum_core::song_engine::SongEngine::from_source(&src.text).expect("compiles");
        let (l, r) = e.render(2);
        assert!(l.iter().chain(&r).all(|v| v.is_finite()));
        // Each track's meter is that sound alone, after its level and before
        // the master. The floors are a third of what each measures at
        // 44.1 kHz (kick 0.141, hat 0.0150, open 0.0094, bass 0.076, pad
        // 0.065) and a tenth for the hats, which a build at 22.05 kHz plays
        // 9.5 dB down: a sound gone silent fails here whatever the other
        // four do.
        let sounds = [
            ("kick", "boom", "kicks/boom.synth", 0.045),
            ("hat", "metal_closed", "hats/metal_closed.synth", 0.0015),
            ("open", "metal_open", "hats/metal_open.synth", 0.001),
            ("bass", "rubber", "bass/rubber.synth", 0.025),
            ("pad", "chorus_pad", "keys/chorus_pad.synth", 0.02),
        ];
        assert_eq!(e.track_count(), sounds.len());
        for (track, sound, file, floor) in sounds {
            assert!(src.files.iter().any(|f| f.ends_with(file)), "{file} is read");
            let t = (0..e.track_count()).find(|&t| e.track_name(t) == track).expect(track);
            let inst = e.track_instrument(t).map(|i| e.instrument_name(i));
            assert_eq!(inst, Some(sound), "track {track} plays {sound}");
            let (peak, rms) = (e.track_peak(t), e.track_rms(t));
            assert!(rms > floor, "{sound} plays: rms {rms}, floor {floor}");
            assert!(peak < 1.0, "{sound} alone stays under full scale: peak {peak}");
        }
        let (peak, _) = e.master_input_peak_rms();
        assert!(peak < 1.0, "the five together do not clip the mix: peak {peak} into the master");
    }

    /// `sounds/node_kit.synth` plays the kit made of one node a piece: the
    /// kick, the hat, the snare, the clap and the tom are each read from
    /// their own file and each is heard on its own track. The floors are a
    /// third of what each measures at 44.1 kHz over four bars (kick 0.139,
    /// hat 0.0052, snare 0.0173, clap 0.0088, tom 0.0317), the hat's too now
    /// that a build at 22.05 kHz plays it as loud: a piece gone silent fails
    /// here whatever the other four do.
    #[test]
    fn the_node_kit_plays_every_piece() {
        let src = Source::load(Path::new("../sounds/node_kit.synth")).expect("every use resolves");
        let mut e = tatum_core::song_engine::SongEngine::from_source(&src.text).expect("compiles");
        let (l, r) = e.render(4);
        assert!(l.iter().chain(&r).all(|v| v.is_finite()));
        let sounds = [
            ("kick", "deep_kick", "kicks/deep.synth", 0.045),
            ("hat", "tick", "hats/tick.synth", 0.0017),
            ("snare", "crack", "snares/crack.synth", 0.0055),
            ("clap", "room", "claps/room.synth", 0.0029),
            ("tom", "low_tom", "toms/low.synth", 0.0105),
        ];
        assert_eq!(e.track_count(), sounds.len());
        for (track, sound, file, floor) in sounds {
            assert!(src.files.iter().any(|f| f.ends_with(file)), "{file} is read");
            let t = (0..e.track_count()).find(|&t| e.track_name(t) == track).expect(track);
            let inst = e.track_instrument(t).map(|i| e.instrument_name(i));
            assert_eq!(inst, Some(sound), "track {track} plays {sound}");
            let (peak, rms) = (e.track_peak(t), e.track_rms(t));
            assert!(rms > floor, "{sound} plays: rms {rms}, floor {floor}");
            assert!(peak < 1.0, "{sound} alone stays under full scale: peak {peak}");
            // Each file is one node and nothing else.
            let text = std::fs::read_to_string(Path::new("../sounds").join(file)).expect("reads");
            let nodes = text.lines().filter(|l| !l.starts_with('#') && l.contains(" as ")).count();
            assert_eq!(nodes, 1, "{file} is one node");
        }
        let (peak, _) = e.master_input_peak_rms();
        assert!(peak < 1.0, "the five together do not clip the mix: peak {peak} into the master");
    }

    /// The open hat rings its 320 ms whatever the length of the note: on a
    /// one-step note (123 ms at 122 BPM) it is still there 200 ms in, and
    /// gone by 400.
    #[test]
    fn the_open_hat_rings_past_a_short_note() {
        let sound = std::fs::read_to_string("../sounds/hats/metal_open.synth").expect("reads");
        let song = format!(
            "tempo 122\n{sound}pattern p {{ C3 - - -  - - - -  - - - -  - - - - }}\ntrack t {{ play p using metal_open }}\n"
        );
        let mut e = tatum_core::song_engine::SongEngine::from_source(&song).expect("compiles");
        let (l, _) = e.render(1);
        let peak_in = |from_ms: f32, to_ms: f32| {
            let at = |ms: f32| (ms * 0.001 * tatum_core::SAMPLE_RATE) as usize;
            l[at(from_ms)..at(to_ms)].iter().fold(0.0f32, |m, v| m.max(v.abs()))
        };
        let peak = peak_in(0.0, 100.0);
        assert!(peak > 0.05, "it plays: peak {peak}");
        // 38 dB down at 200 ms; a note-off that cut it would leave nothing.
        let ringing = peak_in(200.0, 250.0);
        assert!(ringing > peak * 0.003, "still ringing after the note: {ringing} of {peak}");
        let after = peak_in(400.0, 1000.0);
        assert!(after < peak * 0.001, "and over by 400 ms: {after} of {peak}");
    }
}
