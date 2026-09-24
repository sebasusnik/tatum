//! Three warnings that each cost an afternoon, and one that cried wolf.
//!
//! Everything they check is in the text. None of them is visible by reading
//! the text, which is the whole argument for having them: the mechanism is
//! somewhere else -- in the order of a chain, in the ceiling of a knob, in
//! which side of a saturator a multiply lands on.

use tatum_core::dsl;

fn lints(src: &str) -> Vec<(String, String)> {
    let song = dsl::parse(src).unwrap_or_else(|e| panic!("{e:?}"));
    dsl::lint::lint_song(&song)
        .into_iter()
        .map(|l| (String::from(l.code), l.message))
        .collect()
}

fn codes(src: &str, code: &str) -> Vec<String> {
    lints(src).into_iter().filter(|(c, _)| c == code).map(|(_, m)| m).collect()
}

const HEAD: &str = "\
tempo 120
scale C minor
module beats kit { }
module bass low { cutoff 0.4 }
pattern beat { kick: X - - - X - - - X - - - X - - - }
pattern line { 1.1 - 1.3 - 1.5 - 1.3 - 1.1 - 1.3 - 1.5 - 1.3 - }
track drums { play beat using kit out > master }
track bass { play line using low reverb_send 0.2 out > master }
";

fn song(master: &str) -> String {
    format!("{HEAD}master {{ {master} }}\n")
}

// ── bass_into_compressor ─────────────────────────────────────────────────────

/// The bug three of nine songs had. Below 200 Hz a mix is mostly kick, so a
/// low shelf in front of the compressor feeds the detector the kick and the
/// compressor ducks the whole song once a beat. It sounds like a compressor
/// working, which is exactly why nobody goes looking.
#[test]
fn a_low_shelf_in_front_of_the_compressor_is_flagged() {
    let out = codes(&song("in > eq(low=3.5) > compressor(-8, ratio=3) > limiter > out"), "bass_into_compressor");
    assert_eq!(out.len(), 1, "{out:?}");
    assert!(out[0].contains("+3.5"), "say how much: {}", out[0]);
}

#[test]
fn the_same_shelf_after_the_compressor_is_fine() {
    assert!(codes(&song("in > compressor(-8, ratio=3) > eq(low=3.5) > limiter > out"), "bass_into_compressor").is_empty());
}

/// A gentle shelf moves the detector by less than the kick's own variation
/// between hits. Warning there would be noise, and the corpus is full of
/// shelves in the 1 to 2 dB range that sound fine.
#[test]
fn a_gentle_shelf_is_not_worth_a_warning() {
    assert!(codes(&song("in > eq(low=2.0) > compressor(-8, ratio=3) > limiter > out"), "bass_into_compressor").is_empty());
}

#[test]
fn a_shelf_with_no_compressor_after_it_is_fine() {
    assert!(codes(&song("in > eq(low=6.0) > limiter > out"), "bass_into_compressor").is_empty());
}

/// Cutting the lows is the opposite move and is never this problem.
#[test]
fn a_low_cut_is_not_a_low_boost() {
    assert!(codes(&song("in > eq(low=-4.0) > compressor(-8, ratio=3) > limiter > out"), "bass_into_compressor").is_empty());
}

// ── makeup_at_the_ceiling ────────────────────────────────────────────────────

/// `makeup` stops at 4.0 linear. Asking for more does nothing, and the
/// reflex when a mix is still quiet is to ask for more.
#[test]
fn makeup_at_its_maximum_is_flagged() {
    assert_eq!(codes(&song("in > compressor(-8, ratio=3, makeup=4.0) > limiter > out"), "makeup_at_the_ceiling").len(), 1);
}

/// The same request written in dB. It resolves to a hair under 4.0 through
/// `pow`, and a lint that fired on one spelling and not the other would be
/// worse than no lint at all.
#[test]
fn the_decibel_spelling_of_the_ceiling_counts_too() {
    let out = codes(&song("in > compressor(-8, ratio=3, makeup=12db) > limiter > out"), "makeup_at_the_ceiling");
    assert_eq!(out.len(), 1, "12 dB is 4.0 linear: {out:?}");
}

#[test]
fn ordinary_makeup_is_fine() {
    assert!(codes(&song("in > compressor(-8, ratio=3, makeup=6db) > limiter > out"), "makeup_at_the_ceiling").is_empty());
}

/// A compressor on a bus has the same ceiling.
#[test]
fn a_bus_compressor_counts() {
    let src = format!(
        "{}bus drums\ndrums {{ in > compressor(-20, ratio=8, makeup=4.0) > master }}\nmaster {{ in > limiter > out }}\n",
        HEAD.replace("out > master }\ntrack bass", "out > drums }\ntrack bass")
    );
    assert_eq!(codes(&src, "makeup_at_the_ceiling").len(), 1);
}

// ── drum_level_past_full_scale ───────────────────────────────────────────────

/// Each voice in `beats` runs its own `tanh` and only then multiplies by its
/// `*_level`, so the level is a boost on an already-limited signal rather
/// than a fader. Above 1.0 the voice leaves the module over full scale on
/// its own, before the track level or the master gets a say.
#[test]
fn a_drum_level_above_one_is_flagged() {
    let src = HEAD.replace("module beats kit { }", "module beats kit { snare_level 1.4 }")
        + "master { in > limiter > out }\n";
    let out = codes(&src, "drum_level_past_full_scale");
    assert_eq!(out.len(), 1, "{out:?}");
    assert!(out[0].contains("snare_level"), "name the knob: {}", out[0]);
}

#[test]
fn a_drum_level_at_or_below_one_is_fine() {
    let src = HEAD.replace("module beats kit { }", "module beats kit { snare_level 1.0 kick_level 0.8 }")
        + "master { in > limiter > out }\n";
    assert!(codes(&src, "drum_level_past_full_scale").is_empty());
}

/// The module's own `level` is applied to the sum, after every voice, so it
/// is a fader in the way the per-voice ones are not.
#[test]
fn the_modules_own_level_is_a_different_knob() {
    let src = HEAD.replace("module beats kit { }", "module beats kit { level 1.6 }")
        + "master { in > limiter > out }\n";
    assert!(codes(&src, "drum_level_past_full_scale").is_empty());
}

// ── unused_pattern stops crying wolf ─────────────────────────────────────────

/// A rig is a palette: eight or ten alternates waiting to be switched in by
/// hand, and a pattern nothing plays costs nothing. Naming eighteen of them
/// one line each buried the warnings that mattered.
#[test]
fn a_rig_full_of_alternates_gets_one_line_not_eighteen() {
    let src = format!(
        "{HEAD}pattern beat_b {{ kick: X - X - X - X - X - X - X - X - }}\n\
         pattern beat_c {{ kick: X X - - X X - - X X - - X X - - }}\n\
         pattern line_b {{ 1.1 - - - 1.5 - - - 1.1 - - - 1.5 - - - }}\n\
         master {{ in > limiter > out }}\n"
    );
    let out = codes(&src, "unused_pattern");
    assert_eq!(out.len(), 1, "one summary, not one per pattern: {out:?}");
    assert!(out[0].contains("3 patterns"), "{}", out[0]);
    assert!(out[0].contains("beat_b") && out[0].contains("line_b"), "still name them: {}", out[0]);
}

/// One forgotten pattern is still a forgotten pattern, and gets pointed at.
#[test]
fn a_single_unplayed_pattern_is_still_named_on_its_own() {
    let src = format!("{HEAD}pattern forgotten {{ 1.1 - - - }}\nmaster {{ in > limiter > out }}\n");
    let out = codes(&src, "unused_pattern");
    assert_eq!(out.len(), 1);
    assert!(out[0].contains("'forgotten'"), "{}", out[0]);
}

// ── chord_into_mono_voice reaches bass ───────────────────────────────────────

/// `bass` has one voice by construction, so a chord arrives as N note-ons and
/// only the last survives. The lint only ever looked at `keys`, and the one
/// real case in the corpus was a `bass`: a Sunn O)))-style drone written as
/// `E5/1`, which compiles to [E1, B1] and played B alone -- a fifth above the
/// sub, for as long as the file existed.
#[test]
fn a_chord_into_a_bass_is_flagged() {
    let src = "\
tempo 120
scale E minor
module bass drone { cutoff 0.4 }
pattern d { [E1 B1]:0.8 ..*15 }
track drone { play d using drone out > master }
master { in > limiter > out }
";
    let out = codes(src, "chord_into_mono_voice");
    assert_eq!(out.len(), 1, "{out:?}");
    assert!(out[0].contains("single voice"), "say why: {}", out[0]);
}

/// The power-chord spelling that started it, so a regression comes back with
/// the same shape it had.
#[test]
fn a_power_chord_symbol_into_a_bass_is_the_same_thing() {
    let src = "\
tempo 120
scale E minor
module bass drone { cutoff 0.4 }
pattern d { E5/1:0.8 ..*15 }
track drone { play d using drone out > master }
master { in > limiter > out }
";
    assert_eq!(codes(src, "chord_into_mono_voice").len(), 1, "`E5/1` is [E1, B1]");
}

/// With an `arp` the chord is not meant to sound at once: it is the note set
/// the arpeggiator walks, and one voice is the right shape for that. Two
/// songs in the corpus do exactly this on purpose.
#[test]
fn a_chord_feeding_an_arpeggiator_is_not_a_mistake() {
    let src = "\
tempo 120
scale E minor
module bass lead { cutoff 0.4 }
pattern d { [E1 G1 B1]:0.8 ..*15 }
track lead { play d using lead arp up rate=16 octaves=2 out > master }
master { in > limiter > out }
";
    assert!(codes(src, "chord_into_mono_voice").is_empty());
}

/// `arp off` is a track saying it does not want one, so the exemption does
/// not apply.
#[test]
fn arp_off_is_not_an_arp() {
    let src = "\
tempo 120
scale E minor
module bass lead { cutoff 0.4 }
pattern d { [E1 G1 B1]:0.8 ..*15 }
track lead { play d using lead arp off out > master }
master { in > limiter > out }
";
    assert_eq!(codes(src, "chord_into_mono_voice").len(), 1);
}

/// `fm` is eight-voice with real allocation, so a chord through it is fine.
/// Seventy-six tracks in the corpus do this and none of them is a bug.
#[test]
fn a_chord_into_an_fm_module_is_fine() {
    let src = "\
tempo 120
scale E minor
module fm stab { level 2.0 }
pattern d { [E3 G3 B3]:0.8 ..*15 }
track stab { play d using stab out > master }
master { in > limiter > out }
";
    assert!(codes(src, "chord_into_mono_voice").is_empty());
}

/// One track in eight scenes is one track. This printed the same line eight
/// times on the song it was found in.
#[test]
fn a_track_in_many_scenes_is_reported_once() {
    let src = "\
tempo 120
scale E minor
module bass drone { cutoff 0.4 }
module beats kit { }
pattern d { [E1 B1]:0.8 ..*15 }
pattern beat { kick: X - - - X - - - X - - - X - - - }
track drone { play d using drone out > master }
track drums { play beat using kit out > master }
scene a { track drone { play d using drone } track drums { play beat using kit } }
scene b { track drone { play d using drone } track drums { play beat using kit } }
scene c { track drone { play d using drone } track drums { play beat using kit } }
arrange { a x4 b x4 c x4 }
master { in > limiter > out }
";
    assert_eq!(codes(src, "chord_into_mono_voice").len(), 1);
}

/// The corpus, after the one real case was fixed.
#[test]
fn nothing_in_the_corpus_sends_a_chord_to_a_voice_that_cannot_hold_it() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/..");
    let mut bad = Vec::new();
    for dir in ["examples", "examples/rigs"] {
        let path = std::path::Path::new(root).join(dir);
        for entry in std::fs::read_dir(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())) {
            let f = entry.unwrap().path();
            if f.extension().is_none_or(|e| e != "synth") { continue }
            let src = std::fs::read_to_string(&f).unwrap();
            for m in codes(&src, "chord_into_mono_voice") {
                bad.push(format!("{}: {m}", f.file_name().unwrap().to_string_lossy()));
            }
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}
