//! What `synth render` prints about a mix, beyond the levels.
//!
//! The levels were already there, and they answer "is anything buried". Three
//! things that decide whether a song sounds good were not, and all three were
//! computed by hand, repeatedly, during mixing sessions:
//!
//!   - **Width.** Everything in the middle is why instruments cannot be told
//!     apart however well their levels are set.
//!   - **Who shares a band.** The same problem in frequency. A column reading
//!     `mid` five times does not make it jump out.
//!   - **The arc.** A drop that measures the same as the breakdown before it
//!     is flat however good the parts are, and a table that averages the whole
//!     render into one row per track cannot show that.
//!
//! None of this is what `synth audit` measures. The audit asks whether a voice
//! is dirty; a song can be perfectly clean and still sound wrong.

use std::process::Command;

static NTH: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn render(src: &str) -> String {
    let dir = std::env::temp_dir().join(format!("synth-mix-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let nth = NTH.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let song = dir.join(format!("{nth}.synth"));
    let wav = dir.join(format!("{nth}.wav"));
    std::fs::write(&song, src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_synth"))
        .arg("render").arg(&song).arg("-o").arg(&wav)
        .output()
        .expect("run synth render");
    let _ = std::fs::remove_file(&song);
    let _ = std::fs::remove_file(&wav);
    // The report goes to stderr; stdout is for things a pipe would want.
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// `quiet` and `loud` differ only in the levels the scenes set.
fn two_scenes(quiet: f32, loud: f32) -> String {
    format!(
        "tempo 120\nscale C minor\ngain_comp 0\nhumanize 0\nsidechain 0\n\
         module beats kit {{ }}\n\
         module keys pad {{ voice_mode poly attack 5ms release 200ms }}\n\
         pattern beat {{ kick: X - - - X - - - X - - - X - - - }}\n\
         pattern hold {{ [C4 Eb4 G4]:0.8 ..*15 }}\n\
         track drums {{ play beat using kit level 0.6 out > master }}\n\
         track pad {{ play hold using pad level 0.5 out > master }}\n\
         scene soft {{ track drums {{ play beat using kit level {quiet} }} track pad {{ play hold using pad level {quiet} }} }}\n\
         scene hard {{ track drums {{ play beat using kit level {loud} }} track pad {{ play hold using pad level {loud} }} }}\n\
         arrange {{ soft x4 hard x4 }}\n\
         master {{ in > limiter(0.95) > out }}\n"
    )
}

// ── the arc ──────────────────────────────────────────────────────────────────

#[test]
fn every_section_is_reported_with_its_level() {
    let out = render(&two_scenes(0.15, 0.7));
    assert!(out.contains("sections:"), "{out}");
    assert!(out.contains("soft"), "name the sections: {out}");
    assert!(out.contains("hard"), "{out}");
}

#[test]
fn a_song_that_goes_somewhere_reports_an_arc() {
    let out = render(&two_scenes(0.15, 0.7));
    let arc = out.lines().find(|l| l.contains("arc:")).unwrap_or_else(|| panic!("{out}"));
    let db: f32 = arc.split("arc:").nth(1).unwrap().trim().split(' ').next().unwrap().parse().unwrap();
    assert!(db > 6.0, "0.15 against 0.7 is 13 dB of level; the arc read {db}: {arc}");
    assert!(!arc.contains("flat"), "{arc}");
}

/// The case worth catching. Two sections at the same level is a song with no
/// shape, and it looks fine in every other number.
#[test]
fn a_song_that_does_not_move_is_called_flat() {
    let out = render(&two_scenes(0.5, 0.5));
    let arc = out.lines().find(|l| l.contains("arc:")).unwrap_or_else(|| panic!("{out}"));
    assert!(arc.contains("that is flat"), "{arc}");
}

// ── who shares a band ────────────────────────────────────────────────────────

#[test]
fn two_tracks_in_one_band_at_the_same_level_are_called_out() {
    let src = "\
tempo 120
scale C minor
gain_comp 0
humanize 0
sidechain 0
module keys one { voice_mode poly attack 5ms release 200ms }
module keys two { voice_mode poly attack 5ms release 200ms }
pattern a { [C4 Eb4 G4]:0.8 ..*15 }
pattern b { [C4 F4 Ab4]:0.8 ..*15 }
track first { play a using one level 0.5 out > master }
track second { play b using two level 0.5 out > master }
master { in > limiter(0.95) > out }
";
    let out = render(src);
    let line = out.lines().find(|l| l.trim_start().starts_with("mid")).unwrap_or_else(|| panic!("{out}"));
    assert!(line.contains("first") && line.contains("second"), "{line}");
    assert!(line.contains("within 6 dB"), "same band, same level: {line}");
}

/// One clearly behind the other is a mix decision, not a problem.
#[test]
fn a_track_well_behind_another_is_listed_but_not_flagged() {
    let src = "\
tempo 120
scale C minor
gain_comp 0
humanize 0
sidechain 0
module keys one { voice_mode poly attack 5ms release 200ms }
module keys two { voice_mode poly attack 5ms release 200ms }
pattern a { [C4 Eb4 G4]:0.8 ..*15 }
pattern b { [C4 F4 Ab4]:0.8 ..*15 }
track first { play a using one level 0.8 out > master }
track second { play b using two level 0.05 out > master }
master { in > limiter(0.95) > out }
";
    let out = render(src);
    let line = out.lines().find(|l| l.trim_start().starts_with("mid")).unwrap_or_else(|| panic!("{out}"));
    assert!(line.contains("first") && line.contains("second"), "{line}");
    assert!(!line.contains("within 6 dB"), "24 dB apart is not crowding: {line}");
}

// ── width ────────────────────────────────────────────────────────────────────

#[test]
fn autopan_reads_wider_than_the_middle() {
    let voice = |chain: &str| {
        format!(
            "tempo 120\nscale C minor\ngain_comp 0\nhumanize 0\nsidechain 0\n\
             module keys pad {{ voice_mode poly attack 5ms release 200ms }}\n\
             pattern hold {{ [C4 Eb4 G4]:0.8 ..*15 }}\n\
             track pad {{ play hold using pad level 0.6 out {chain}> master }}\n\
             master {{ in > limiter(0.95) > out }}\n"
        )
    };
    let width = |src: &str| -> f32 {
        let out = render(src);
        let row = out.lines().find(|l| l.trim_start().starts_with("pad ")).unwrap_or_else(|| panic!("{out}"));
        row.split('%').next().unwrap().rsplit(' ').next().unwrap().parse().unwrap_or_else(|e| panic!("{row}: {e}"))
    };
    let centred = width(&voice(""));
    let moving = width(&voice("> autopan(0.8, bars=1) "));
    // Not near zero: a poly `keys` spreads its own voices, which is the
    // point of it. What has to hold is that moving the thing across the
    // field reads as much wider than leaving it where it was.
    assert!(centred < 15.0, "a pad with no panning should not read wide, got {centred}%");
    assert!(
        moving > centred * 2.5,
        "autopan read {moving}% against {centred}% still -- the meter is not seeing it"
    );
}
