//! A track at `level 0` used to cost exactly as much CPU as one that was
//! playing: the engine rendered every voice, ran the whole insert chain, and
//! only then multiplied by zero. Measured on three FM pads over 64 bars, the
//! muted render took 4.73 s against 1.65 s for not having the tracks at all.
//!
//! That is the difference between a live set being able to keep a rig of voices
//! loaded and not, because `level` is a fast-path edit -- it applies inside the
//! bar with no swap and no voice restart -- while removing a track from a scene
//! is structural and restarts everything.
//!
//! Muting now means the track fires no notes, and once its voices have run out
//! it is skipped whole. The same three pads now render in 1.60 s against 1.61 s.

use tatum_core::song_engine::SongEngine;

/// `pad_level` is the only thing that changes between renders.
fn song(pad_level: &str, extra_track: bool) -> String {
    let pad = if extra_track {
        format!("track pad {{ play hold using keys_pad level {pad_level} out > lowpass(2000, 0.1) > master }}")
    } else {
        String::new()
    };
    format!(r#"
tempo 120
scale C minor
gain_comp 0
humanize 0
sidechain 0

module bass low {{ cutoff 0.4 sustain 0.8 }}
module keys keys_pad {{ voice_mode poly cutoff 0.6 sustain 1.0 attack 1ms release 0.3 }}

pattern line {{ 1.1 - - -  1.1 - - -  1.1 - - -  1.1 - - - }}
pattern hold {{ [1.3 3.3 5.3] ..*15 }}

track bass {{ play line using low level 0.5 out > master }}
{pad}

master {{ in > out }}
"#)
}

fn render(src: &str, bars: u32) -> Vec<f32> {
    let mut e = SongEngine::from_source(src).unwrap_or_else(|err| panic!("{}", err));
    e.start();
    e.render(bars).0
}

/// The point of the whole change: a muted track has to be indistinguishable
/// from a track that is not in the song, sample for sample. If it were merely
/// quiet, skipping it would be an audible optimisation rather than a free one.
#[test]
fn a_muted_track_is_bit_identical_to_not_having_it() {
    let muted = render(&song("0", true), 4);
    let absent = render(&song("0", false), 4);
    assert_eq!(muted.len(), absent.len());
    let diff = muted.iter().zip(&absent).filter(|(a, b)| a != b).count();
    assert_eq!(diff, 0, "{diff} samples differ between a muted track and no track");
}

/// And it must still be a mute, not a delete: bringing the fader up has to
/// bring the part back.
#[test]
fn raising_the_fader_brings_the_part_back() {
    let loud = render(&song("0.5", true), 4);
    let muted = render(&song("0", true), 4);
    let peak = |v: &[f32]| v.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    assert!(
        peak(&loud) > peak(&muted) * 1.2,
        "unmuted {:.4} should be clearly louder than muted {:.4}",
        peak(&loud), peak(&muted)
    );
}

/// A muted track keeps counting its pattern. If it did not, unmuting would
/// drop it in wherever it happened to stop, off the grid and out of phase with
/// everything else -- the one thing a live mute must never do.
#[test]
fn a_muted_track_keeps_its_place_in_the_bar() {
    let src = r#"
tempo 120
scale C minor
gain_comp 0
humanize 0
sidechain 0
module beats kit { kick_level 1.0 }
pattern beat { kick: X - - -  - - - -  - - - -  - - - - }
track drums { play beat using kit level 0.6 out > master }
master { in > out }
"#;
    let mut e = SongEngine::from_source(src).unwrap_or_else(|err| panic!("{}", err));
    e.start();
    // Two bars muted, then two bars audible.
    e.set_track_level(0, 0.0);
    let _ = e.render(2);
    e.set_track_level(0, 0.6);
    let (l, _) = e.render(2);

    // The kick is on the downbeat of each bar. One bar at 120 BPM is 2 s.
    let sr = tatum_core::SAMPLE_RATE as usize;
    let bar = sr * 2;
    let peak_at = |start: usize| l[start..start + sr / 20].iter().fold(0.0f32, |m, x| m.max(x.abs()));
    assert!(peak_at(0) > 0.05, "the kick should land on the downbeat after unmuting, got {:.4}", peak_at(0));
    assert!(peak_at(bar) > 0.05, "and again a bar later, got {:.4}", peak_at(bar));
    // ... and nowhere else: a quarter into the bar must be quiet.
    assert!(peak_at(bar / 4) < peak_at(0) * 0.5, "the grid slipped: {:.4} a quarter in", peak_at(bar / 4));
}

/// A ghost kick -- `level 0`, feeding the sidechain so it ducks the mix without
/// being heard -- is a real idiom, and muting the fader must not mute the
/// trigger. `core/tests/sidechain_source.rs` renders one; this pins the reason.
#[test]
fn a_sidechain_source_still_runs_when_its_fader_is_down() {
    let src = |sc: &str| format!(r#"
tempo 120
scale C minor
gain_comp 0
humanize 0
sidechain {sc}
module beats kit {{ kick_level 1.0 }}
module keys pad {{ voice_mode poly cutoff 0.6 sustain 1.0 attack 1ms }}
pattern beat {{ kick: X - - -  - - - -  - - - -  - - - - }}
pattern hold {{ [1.3 3.3 5.3] ..*15 }}
track drums {{ play beat using kit level 0.0 out > master }}
track pad {{ play hold using pad level 0.5 out > master }}
master {{ in > out }}
"#);
    let ducked = render(&src("0.9"), 1);
    let flat = render(&src("0"), 1);
    let sr = tatum_core::SAMPLE_RATE as usize;
    let head = |v: &[f32]| v[..sr / 40].iter().fold(0.0f32, |m, x| m.max(x.abs()));
    assert!(
        head(&ducked) < head(&flat) * 0.85,
        "an inaudible kick must still duck: ducked {:.4} vs flat {:.4}",
        head(&ducked), head(&flat)
    );
}

/// Skipping is audio-neutral, which is the point but also means the tests
/// above pass against the old engine too: a muted track contributed zero
/// either way. This is the one that actually pins the new behaviour -- that
/// the track stops being processed at all.
#[test]
fn a_muted_track_stops_being_processed_once_its_voices_run_out() {
    let mut e = SongEngine::from_source(&song("0", true)).unwrap_or_else(|err| panic!("{}", err));
    e.start();
    let _ = e.render(4);
    assert_eq!(e.silent_track_count(), 1, "the muted pad should be skipped whole");

    let mut loud = SongEngine::from_source(&song("0.5", true)).unwrap_or_else(|err| panic!("{}", err));
    loud.start();
    let _ = loud.render(4);
    assert_eq!(loud.silent_track_count(), 0, "nothing should be skipped when the fader is up");
}

/// Muting must not cut a note that is still ringing: the track keeps being
/// processed until its release finishes, and only then falls out.
#[test]
fn muting_lets_the_tail_finish_before_the_track_is_dropped() {
    let mut e = SongEngine::from_source(&song("0.5", true)).unwrap_or_else(|err| panic!("{}", err));
    e.start();
    let _ = e.render(1);
    // The pad holds a chord with a 0.3 release. Drop the fader mid-note.
    e.set_track_level(1, 0.0);
    let sr = tatum_core::SAMPLE_RATE as usize;
    let mut out_l = vec![0.0f32; sr / 100];
    let mut out_r = vec![0.0f32; sr / 100];
    e.process_block_stereo(&mut out_l, &mut out_r);
    assert_eq!(e.silent_track_count(), 0, "the pad is still releasing, so it cannot be skipped yet");
    let _ = e.render(2);
    assert_eq!(e.silent_track_count(), 1, "once the tail is gone it should drop out");
}

/// Muting a track must not reach the tracks that are still playing, and with
/// `humanize` on it used to.
///
/// The draws came from one stream shared by the whole song, handed out in
/// track order within a step, so a track that stopped firing notes stopped
/// consuming the stream and every track after it got different numbers. The
/// mute was audible in parts that were not muted. Each track seeds from its
/// own name now, so this holds with humanization at its strongest.
#[test]
fn muting_does_not_reach_the_tracks_that_are_still_playing() {
    let head = "\
tempo 120
scale C minor
gain_comp 0
humanize 0.4 timing 0.2
sidechain 0
module bass low { cutoff 0.5 sustain 0.8 }
module keys pad { voice_mode poly cutoff 0.6 attack 1ms }
pattern line { C2:0.9 - C2:0.9 -  C2:0.9 - C2:0.9 -  C2:0.9 - C2:0.9 -  C2:0.9 - C2:0.9 - }
pattern other { [1.3 3.3 5.3]:0.8 - - -  [1.3 3.3 5.3]:0.8 - - -  - - - -  - - - - }
track bass { play line using low level 0.5 out > master }
";
    let tail = "master { in > out }\n";
    // A muted pad contributes exactly zero, so if it also leaves the bass
    // alone the render is the same to the bit as not having the track.
    let muted = format!("{head}track pad {{ play other using pad level 0 out > master }}\n{tail}");
    let absent = format!("{head}{tail}");
    assert_eq!(
        render(&muted, 2),
        render(&absent, 2),
        "muting the pad changed the bass"
    );
}

/// The other end of the fader. A `level` above 1.0 is boost, and the corpus
/// uses it -- some tracks sit at 1.4 or 2.0. It worked when the number was
/// written in the file and when an `auto` sweep reached it, but the live
/// setter clamped it to 1.0, so the same value arriving through an edit or a
/// hot swap silently dropped the track. Three paths for one number, and the
/// one that disagreed was the one nobody had written a file against.
#[test]
fn a_fader_can_be_pushed_past_unity() {
    let src = r#"
tempo 120
scale C minor
gain_comp 0
humanize 0
sidechain 0
module beats kit { kick_level 1.0 }
pattern beat { kick: X - - -  - - - -  - - - -  - - - - }
track drums { play beat using kit level 0.25 out > master }
master { in > out }
"#;
    let peak = |level: f32| {
        let mut e = SongEngine::from_source(src).unwrap_or_else(|err| panic!("{err}"));
        e.start();
        e.set_track_level(0, level);
        let (l, _) = e.render(1);
        l.iter().fold(0.0f32, |m, x| m.max(x.abs()))
    };
    let unity = peak(1.0);
    let boosted = peak(2.0);
    assert!(
        boosted > unity * 1.9,
        "level 2.0 should be about twice level 1.0, got {boosted:.4} against {unity:.4}"
    );
    // There is still a ceiling, so a typo cannot take the master out.
    let absurd = peak(40.0);
    let ceiling = peak(tatum_core::song_engine::MAX_TRACK_LEVEL);
    assert!(
        (absurd - ceiling).abs() < 1e-6,
        "level 40 should clamp to the ceiling, got {absurd:.4} against {ceiling:.4}"
    );
}
