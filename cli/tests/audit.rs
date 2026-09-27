//! `tatum audit` has to be able to say a voice is dirty, not only that it got
//! dirtier than it was yesterday.
//!
//! The bug it is built around: an FM bell with 18% chorus, heard as a small
//! distortion on its first note. Every average over the whole song said the
//! track was the cleanest voice in the mix. Two measurements here have a
//! chance of catching that and one does not:
//!
//!   - **An absolute number per voice cannot work.** A saturated saw has most
//!     of its energy off the harmonics by design and a bell has almost none,
//!     so any threshold that catches the bell calls every saw broken.
//!   - **A note against the other notes of the same voice** catches one note
//!     going wrong. It did not catch this one: the chorus raised every note
//!     of the bell equally.
//!   - **The same voice with and without one effect** catches exactly this.
//!     Both renders are the same timbre, so the difference is caused by the
//!     thing that was removed, and it needs no threshold that travels.

use std::process::Command;

/// Tests run in parallel and three of them audit the same source, so the
/// file name cannot come from the source: two threads would write and delete
/// the same path and one would find it missing.
static NTH: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn audit(src: &str) -> String {
    let dir = std::env::temp_dir().join(format!("tatum-audit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let nth = NTH.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = dir.join(format!("{nth}.synth"));
    std::fs::write(&path, src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_tatum")).arg("audit").arg(&path).output().expect("run tatum audit");
    let _ = std::fs::remove_file(&path);
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A bell: a pure FM tone, a few notes an octave and a half up, nothing else.
fn bell(chorus: &str) -> String {
    format!(
        "tempo 120\n\
         scale C minor\n\
         \
         humanize 0\n\
         module fm bell {{ level 2.0 algorithm dual_pairs mod_index 20% {chorus} }}\n\
         module beats kit {{ }}\n\
         pattern beat {{ kick: X - - - X - - - X - - - X - - - }}\n\
         pattern b {{ C6:0.7 .. .. ..  G5:0.7 .. .. ..  Eb6:0.7 .. .. ..  G5:0.7 .. .. .. }}\n\
         track kick {{ play beat using kit level 0.6 out > master }}\n\
         track bell {{ play b using bell level 0.6 out > master }}\n\
         scene a {{ track kick {{ play beat using kit level 0.6 }} track bell {{ play b using bell level 0.6 }} }}\n\
         arrange {{ a x8 }}\n\
         master {{ in > limiter(0.95) > out }}\n"
    )
}

#[test]
fn the_chorus_that_dirtied_a_bell_is_named() {
    let out = audit(&bell("chorus_mix 18%"));
    assert!(out.contains("the chorus on `bell` accounts for"), "the audit did not name the chorus:\n{out}");
}

#[test]
fn the_same_bell_without_it_is_left_alone() {
    let out = audit(&bell("chorus_mix 0%"));
    assert!(!out.contains("the chorus on `bell`"), "blamed a chorus that is not there:\n{out}");
    assert!(out.contains("bell"), "the bell should still get a row:\n{out}");
}

/// The blame is a difference between two renders of the same voice, so it has
/// to grow with the amount of the thing being blamed. A number that did not
/// would mean the tool is measuring the render and not the effect.
#[test]
fn more_chorus_is_more_blame() {
    let grab = |src: &str| -> f64 {
        let out = audit(src);
        let line = out
            .lines()
            .find(|l| l.contains("the chorus on `bell`"))
            .unwrap_or_else(|| panic!("no blame line in:\n{out}"));
        line.split("accounts for").nth(1).unwrap().trim().split(' ').next().unwrap().parse().unwrap()
    };
    let small = grab(&bell("chorus_mix 12%"));
    let large = grab(&bell("chorus_mix 40%"));
    assert!(large > small, "40% chorus blamed {large:.1} dB, 12% blamed {small:.1}");
}

/// A voice with nothing switched on gets no blame line at all, and the tool
/// does not spend a render finding that out.
#[test]
fn a_voice_with_no_effects_is_not_rendered_twice() {
    let out = audit(&bell("chorus_mix 0%"));
    assert!(!out.contains("Switched off one at a time"), "{out}");
}

/// Rows are alphabetical, because ranking voices against each other by a
/// number that is not comparable between them points at the wrong one.
#[test]
fn the_report_says_what_the_number_is_not_good_for() {
    let out = audit(&bell("chorus_mix 0%"));
    assert!(out.contains("not comparable"), "{out}");
}

// ── what the per-note check refuses to judge ─────────────────────────────────
//
// Run across the corpus without these, it reported 30 "dirty" notes on a sub,
// 24 on a low organ and 8 on an arpeggio -- and always the same handful of
// pitches, over and over. A note that goes wrong goes wrong once; the same
// pitch reading badly every time it appears is the measurement being wrong
// about a pitch.

fn low(pattern: &str, extra_track: &str) -> String {
    format!(
        "tempo 120\n\
         scale C minor\n\
         \
         humanize 0\n\
         module bass sub {{ cutoff 0.3 sustain 0.9 }}\n\
         module beats kit {{ }}\n\
         pattern beat {{ kick: X - - - X - - - X - - - X - - - }}\n\
         pattern b {{ {pattern} }}\n\
         track kick {{ play beat using kit level 0.6 out > master }}\n\
         track sub {{ play b using sub level 0.6 {extra_track} out > master }}\n\
         scene a {{ track kick {{ play beat using kit level 0.6 }} track sub {{ play b using sub level 0.6 {extra_track} }} }}\n\
         arrange {{ a x8 }}\n\
         master {{ in > limiter(0.95) > out }}\n"
    )
}

/// A sub at D1 has its harmonics 37 Hz apart, and they are measured in bands
/// 45 Hz wide, so the bands overlap and the number is arithmetic rather than
/// sound. A window short enough to sit inside one step cannot resolve better,
/// so this is a property of the method and not a setting to tune.
#[test]
fn notes_too_low_to_resolve_are_not_called_dirty() {
    let out = audit(&low("D1:0.8 .. .. ..  C1:0.8 .. .. ..  D1:0.8 .. .. ..  A0:0.8 .. .. ..", ""));
    assert!(!out.contains("stand out from their own voice"), "a sub cannot be judged note by note:\n{out}");
}

/// An arp runs below the step, so a window that fits inside one step still
/// holds several of its notes and every one of them is "not at a harmonic"
/// of the others.
#[test]
fn an_arpeggio_is_measured_but_not_checked_note_by_note() {
    let src = format!(
        "tempo 120\nscale C minor\nhumanize 0\n\
         module keys plink {{ voice_mode poly attack 1ms decay 0.2 sustain 0.0 }}\n\
         module beats kit {{ }}\n\
         pattern beat {{ kick: X - - - X - - - X - - - X - - - }}\n\
         pattern c {{ [C4 Eb4 G4 Bb4]:0.8 ..*15 }}\n\
         track kick {{ play beat using kit level 0.6 out > master }}\n\
         track plink {{ play c using plink level 0.6 arp up rate=16 octaves=2 out > master }}\n\
         scene a {{ track kick {{ play beat using kit level 0.6 }} track plink {{ play c using plink level 0.6 arp up rate=16 octaves=2 }} }}\n\
         arrange {{ a x8 }}\n\
         master {{ in > limiter(0.95) > out }}\n"
    );
    let out = audit(&src);
    assert!(out.contains("its arp puts several notes in every window"), "it should say why it stepped back:\n{out}");
    assert!(!out.contains("stand out from their own voice"), "{out}");
}

/// Stepping back from the per-note check must not stop the tool measuring the
/// voice: the with-and-without comparison is the same voice both times, so
/// none of the reasons above touch it. `glass` in liquid_dnb is an arp track
/// and its chorus is found anyway.
#[test]
fn an_arpeggio_is_still_compared_with_and_without_its_chorus() {
    let src = format!(
        "tempo 120\nscale C minor\nhumanize 0\n\
         module fm plink {{ level 2.0 algorithm dual_pairs mod_index 20% chorus_mix 35% }}\n\
         module beats kit {{ }}\n\
         pattern beat {{ kick: X - - - X - - - X - - - X - - - }}\n\
         pattern c {{ [C5 Eb5 G5 Bb5]:0.8 ..*15 }}\n\
         track kick {{ play beat using kit level 0.6 out > master }}\n\
         track plink {{ play c using plink level 0.6 arp up rate=8 out > master }}\n\
         scene a {{ track kick {{ play beat using kit level 0.6 }} track plink {{ play c using plink level 0.6 arp up rate=8 }} }}\n\
         arrange {{ a x8 }}\n\
         master {{ in > limiter(0.95) > out }}\n"
    );
    let out = audit(&src);
    assert!(out.contains("the chorus on `plink` accounts for"), "an arp still gets the comparison:\n{out}");
}
