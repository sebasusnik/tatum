//! `chorus_beats`: a chorus on a high voice is roughness, not width.
//!
//! This lint exists because of one bug that cost an hour. An FM bell with 18%
//! chorus playing C#6 was heard as a small distortion on its first note.
//! Every whole-song measurement said the track was clean — averaged over 222
//! seconds it was the tidiest voice in the mix — and the search went through
//! clipping, aliasing, quantisation, interpolation and block-rate modulation
//! before landing on it.
//!
//! The mechanism is arithmetic, which is why it can be caught for free:
//!
//! A chorus reads a delay line whose length is moving, and reading a moving
//! delay resamples — the copy comes out pitch-shifted by the rate of change.
//! With this engine's constants that is about ±3.8%. The copy then beats
//! against the dry signal at `f × 0.038`, so the beat rate scales with the
//! note: 3 Hz on a low E, 42 Hz on a C#6. Somewhere around 20 Hz the ear
//! stops hearing two tones and starts hearing roughness, and on a clean tone
//! roughness is indistinguishable from distortion.

use synth_core::dsl;
use synth_core::effects::chorus;

fn lints(src: &str) -> Vec<String> {
    let song = dsl::parse(src).unwrap_or_else(|e| panic!("{e:?}"));
    dsl::lint::lint_song(&song)
        .into_iter()
        .filter(|l| l.code == "chorus_beats")
        .map(|l| l.message)
        .collect()
}

fn song(module: &str, pattern: &str) -> String {
    format!(
        "tempo 120\nscale C minor\n\
         module fm bell {{ level 2.0 {module} }}\n\
         pattern p {{ {pattern} }}\n\
         track bell {{ play p using bell out > master }}\n\
         master {{ in > limiter(0.95) > out }}\n"
    )
}

// ── The physics the threshold comes from ─────────────────────────────────────

/// If someone retunes the chorus, the lint has to follow rather than keep
/// checking a number that used to be true.
#[test]
fn the_threshold_is_derived_from_the_chorus_itself() {
    let detune = chorus::max_detune_ratio();
    assert!(
        (0.03..0.05).contains(&detune),
        "the chorus detunes its copy by {:.3}, which is not what the lint assumes",
        detune
    );
    let floor = chorus::roughness_above_hz();
    assert!(
        (450.0..650.0).contains(&floor),
        "roughness floor came out at {floor:.0} Hz"
    );
    // the floor is exactly where the beat reaches 20 Hz
    assert!((floor * detune - 20.0).abs() < 0.01);
}

// ── What it catches ──────────────────────────────────────────────────────────

/// The bug, as it was written.
#[test]
fn a_chorused_bell_up_at_c_sharp_6_is_flagged() {
    let out = lints(&song("chorus_mix 18%", "C#6:0.5 - - - A5:0.5 - - -"));
    assert_eq!(out.len(), 1, "expected exactly one warning, got {out:?}");
    let m = &out[0];
    assert!(m.contains("C#6"), "name the note: {m}");
    assert!(m.contains("42 Hz"), "give the beat rate: {m}");
}

#[test]
fn a_chorus_in_the_track_chain_counts_too() {
    let src = "tempo 120\nscale C minor\n\
               module fm bell { level 2.0 }\n\
               pattern p { C#6:0.5 - A5:0.5 - }\n\
               track bell { play p using bell out > chorus(0.5) > master }\n\
               master { in > limiter(0.95) > out }\n";
    assert_eq!(lints(src).len(), 1, "a chorus node is the same mechanism");
}

// ── What it leaves alone ─────────────────────────────────────────────────────

/// The same chorus an octave and a half down beats at 8 Hz, which is width.
#[test]
fn the_same_chorus_on_a_low_voice_is_fine() {
    assert!(lints(&song("chorus_mix 18%", "C3:0.5 - - - G2:0.5 - - -")).is_empty());
}

#[test]
fn a_high_voice_without_chorus_is_fine() {
    assert!(lints(&song("chorus_mix 0%", "C#6:0.5 - - - A5:0.5 - - -")).is_empty());
}

/// A top note two octaves above the root is a chord voicing, not a high
/// voice: the beat is buried under everything below it. Warning here would
/// be noise, and a lint that cries wolf gets switched off.
#[test]
fn a_high_note_on_top_of_a_low_chord_is_not_flagged() {
    let out = lints(&song("chorus_mix 18%", "[C3 G3 C#6]:0.5 - - -"));
    assert!(out.is_empty(), "chord tops must not trip it: {out:?}");
}

/// `keys` outside poly stacks every voice on one note and its chorus never
/// runs, so warning about it would send someone to fix a line that does
/// nothing.
#[test]
fn chorus_on_a_unison_keys_voice_is_inert_and_not_flagged() {
    let src = "tempo 120\nscale C minor\n\
               module keys lead { voice_mode unison chorus_mix 50% }\n\
               pattern p { C#6:0.5 - A5:0.5 - }\n\
               track lead { play p using lead out > master }\n\
               master { in > limiter(0.95) > out }\n";
    assert!(lints(src).is_empty());
}

#[test]
fn a_trace_of_chorus_is_not_worth_a_warning() {
    assert!(lints(&song("chorus_mix 4%", "C#6:0.5 - - - A5:0.5 - - -")).is_empty());
}

// ── Scale degrees and the arpeggiator ────────────────────────────────────────

/// Degrees are pitches too. The lint read only absolute note names at first
/// and walked straight past the repo's own arpeggiator example, which writes
/// its chord as `[1.4 3.4 5.4 7.4]`.
#[test]
fn a_pattern_written_in_scale_degrees_is_resolved() {
    let src = "tempo 120\nscale C minor\n\
               module fm bell { level 2.0 chorus_mix 30% }\n\
               pattern p { [5.5 7.5]:0.5 - - - }\n\
               track bell { play p using bell out > master }\n\
               master { in > limiter(0.95) > out }\n";
    let out = lints(src);
    assert_eq!(out.len(), 1, "degrees must resolve: {out:?}");
    assert!(out[0].contains("Hz"), "{}", out[0]);
}

/// An arp lifts the held notes by `octaves - 1`, so the voice reaches higher
/// than the pattern reads. This is what arp_keys.synth does.
#[test]
fn the_arp_octave_lift_counts_toward_the_pitch() {
    let base = "tempo 120\nscale C minor\n\
                module keys pad { voice_mode poly chorus_mix 40% }\n\
                pattern p { [1.4 3.4 5.4 7.4]:0.7 ..*15 }\n";
    let tail = "master { in > limiter(0.95) > out }\n";
    let no_arp = format!("{base}track keys {{ play p using pad out > master }}\n{tail}");
    let arped = format!(
        "{base}track keys {{ play p using pad arp up rate=16 octaves=2 out > master }}\n{tail}"
    );
    assert!(lints(&no_arp).is_empty(), "held low, it is a chord: {:?}", lints(&no_arp));
    let out = lints(&arped);
    assert_eq!(out.len(), 1, "the arp lifts it over the line: {out:?}");
    assert!(out[0].contains("arp"), "say why it is higher: {}", out[0]);
}

/// The chord-top exemption only makes sense when the notes sound together.
/// An arp plays them one at a time, so there is nothing to bury the beat.
#[test]
fn an_arp_gets_no_chord_top_exemption() {
    let src = "tempo 120\nscale C minor\n\
               module keys pad { voice_mode poly chorus_mix 40% }\n\
               pattern p { [C3 G3 C#6]:0.6 ..*15 }\n\
               track keys { play p using pad arp up rate=16 octaves=1 out > master }\n\
               master { in > limiter(0.95) > out }\n";
    assert_eq!(lints(src).len(), 1, "arpeggiated, the top note is exposed");
}

// ── The corpus ───────────────────────────────────────────────────────────────

/// The song the bug came from, after the fix.
#[test]
fn neon_arterial_is_clean() {
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../examples/neon_arterial.synth"
    ))
    .expect("neon_arterial.synth");
    assert!(lints(&src).is_empty(), "{:?}", lints(&src));
}
