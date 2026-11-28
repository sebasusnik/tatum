//! A track that leaves a scene while a note is releasing, and comes back.
//!
//! A track the scene does not name is not rendered. Its notes were released
//! on the bar line and it faded over 10 ms, and then its instrument and its
//! insert chain stayed exactly as they were: voices mid-release, filters and
//! delays mid-ring. The scene that brought the track back opened with the end
//! of a note from the last time it played, bars before (`tatum render` on
//! `oberheim_dark`: "not quiet between notes: maw at bar 26.9").
//!
//! What the fade leaves is cleared when it ends. The fade itself, and the
//! tails of the shared reverb and delay ringing on over the change, are what
//! they were.

use tatum_core::song_engine::SongEngine;
use tatum_core::{BLOCK_SIZE, SAMPLE_RATE};

const TEMPO: f32 = 120.0;

fn bar() -> usize {
    (SAMPLE_RATE * 60.0 / TEMPO * 4.0) as usize
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| (v * v) as f64).sum::<f64>() / x.len().max(1) as f64).sqrt() as f32
}

fn db(a: f32, b: f32) -> f32 {
    20.0 * (a.max(1e-12) / b.max(1e-12)).log10()
}

/// One bar with the voice, two without it, one with it again. The voice
/// rests for half a bar and then holds a note to the bar line, so the scene
/// change finds it sounding, and its return opens with half a bar of rest.
fn song(voice: &str, pattern: &str, track: &str) -> String {
    format!(
        "tempo {TEMPO}\nscale C major\n{voice}\n\
         module beats kit {{ kick_level 1.0 }}\n\
         pattern p {{ {pattern} }}\n\
         pattern none {{ kick: -*16 }}\n\
         track t {{ play p using v {track} }}\n\
         track d {{ play none using kit out > master }}\n\
         master {{ in > out }}\n\
         scene with {{ track t {{ play p using v }} track d {{ play none using kit }} }}\n\
         scene without {{ track d {{ play none using kit }} }}\n\
         arrange {{ with x1 without x2 with x1 }}\n"
    )
}

const HELD: &str = "-*8 C3 ..*7";

fn render_in(src: &str, block: usize) -> Vec<f32> {
    let mut e = SongEngine::from_source(src).unwrap_or_else(|e| panic!("{e}\n{src}"));
    e.start();
    let total = bar() * 4;
    let (mut l, mut r) = (vec![0.0f32; total], vec![0.0f32; total]);
    let mut pos = 0;
    while pos < total {
        let n = block.min(total - pos);
        e.process_block_stereo(&mut l[pos..pos + n], &mut r[pos..pos + n]);
        pos += n;
    }
    l
}

fn render(src: &str) -> Vec<f32> {
    render_in(src, BLOCK_SIZE)
}

/// Every kind of instrument, each ringing for a second or more when the
/// scene drops it, and two short voices whose insert chain rings instead.
fn voices() -> Vec<(&'static str, String)> {
    let dry = "out > master";
    vec![
        (
            "graph",
            song(
                "instrument v { osc saw > lowpass(900, 0.2) as f\n f > adsr(0.005, 0.1, 0.8, 1.5) as a\n a > out }",
                HELD,
                dry,
            ),
        ),
        ("keys", song("module keys v { voice_mode poly attack 5ms release 1.5s chorus_mix 0.5 }", HELD, dry)),
        (
            "fm",
            song(
                "module fm v { op0_envelope 0.005 0.1 0.8 1.5 op1_envelope 0.005 0.1 0.8 1.5 chorus_mix 0.5 }",
                HELD,
                dry,
            ),
        ),
        ("bass", song("module bass v { release 1.5s }", HELD, dry)),
        ("beats", song("module beats v { }", "crash: -*15 X", dry)),
        (
            "reverb in the chain",
            song(
                "instrument v { osc saw > adsr(0.005, 0.1, 0.8, 0.02) as a\n a > out }",
                HELD,
                "out > reverb(0.8) > master",
            ),
        ),
        (
            "delay in the chain",
            song(
                "instrument v { osc saw > adsr(0.005, 0.1, 0.8, 0.02) as a\n a > out }",
                HELD,
                "out > delay(1/8, 0.6) > master",
            ),
        ),
        (
            "chorus in the chain",
            song(
                "instrument v { osc saw > adsr(0.005, 0.1, 0.8, 0.02) as a\n a > out }",
                HELD,
                "out > chorus(0.8) > master",
            ),
        ),
    ]
}

#[test]
fn a_track_that_returns_brings_nothing_back() {
    // Every voice is measured before any is judged, so a failure names all
    // the kinds that bring something back, not the first.
    let mut stale = Vec::new();
    for (name, src) in voices() {
        let x = render(&src);
        let b = bar();
        let note = rms(&x[b / 2..b]);
        assert!(note > 0.005, "{name}: the voice plays, rms {note}");
        // Out of the scene: gone once the 10 ms fade is over.
        let gone = peak(&x[b + b / 50..3 * b]);
        assert!(gone == 0.0, "{name}: {gone} while the scene does not name the track");
        // Back in the scene: half a bar of rest before its first note.
        let back = peak(&x[3 * b..3 * b + b / 2 - 64]);
        if db(back, note) >= -90.0 {
            stale.push(format!("{name} ({:.1} dB)", db(back, note)));
        }
    }
    assert!(stale.is_empty(), "came back with what they were ringing when they left: {}", stale.join(", "));
}

#[test]
fn a_track_that_returns_plays_as_it_did() {
    for (name, src) in voices() {
        let x = render(&src);
        let b = bar();
        let (first, again) = (rms(&x[b / 2..b]), rms(&x[3 * b + b / 2..4 * b]));
        // Its chorus and its LFOs are somewhere else in their sweep, no more.
        assert!(db(again, first).abs() < 3.0, "{name}: the note is {:.1} dB off on the way back", db(again, first));
    }
}

#[test]
fn a_dropped_track_still_fades_and_the_sends_still_ring() {
    // The promise in the manual: dropped, a track fades over 10 ms. What it
    // sent to the shared reverb rings on over the change, as it always has.
    let dry = song("module keys v { voice_mode poly attack 5ms release 1.5s }", HELD, "out > master");
    let x = render(&dry);
    let b = bar();
    let end = x[..2 * b].iter().rposition(|v| v.abs() > 1e-6).expect("some sound");
    assert!(end > b + 300 && end < b + 600, "faded from {b} to {end}");

    let wet = song("module keys v { voice_mode poly attack 5ms release 1.5s }", HELD, "reverb_send 0.6 out > master")
        .replace("scale C major", "scale C major\nreverb size=0.85 damp=0.3");
    let x = render(&wet);
    let note = rms(&x[b / 2..b]);
    let tail = rms(&x[b + b / 4..b + b / 2]);
    assert!(
        db(tail, note) > -40.0,
        "the reverb tail is {:.1} dB under the note a beat after the change",
        db(tail, note)
    );
}

#[test]
fn a_track_that_changes_instrument_and_back_brings_nothing_back() {
    let src = format!(
        "tempo {TEMPO}\nscale C major\n\
         instrument v {{ osc saw > adsr(0.005, 0.1, 0.8, 1.5) as a\n a > out }}\n\
         instrument w {{ osc sine > adsr(0.005, 0.1, 0.8, 0.02) as a\n a > out }}\n\
         pattern p {{ {HELD} }}\n\
         pattern rest {{ -*16 }}\n\
         track t {{ play p using v out > master }}\n\
         master {{ in > out }}\n\
         scene one {{ track t {{ play p using v }} }}\n\
         scene other {{ track t {{ play rest using w }} }}\n\
         arrange {{ one x1 other x2 one x1 }}\n"
    );
    let x = render(&src);
    let b = bar();
    let note = rms(&x[b / 2..b]);
    assert!(note > 0.005);
    let back = peak(&x[3 * b..3 * b + b / 2 - 64]);
    assert!(db(back, note) < -90.0, "the first instrument came back with {:.1} dB of its old note", db(back, note));
    let again = rms(&x[3 * b + b / 2..4 * b]);
    assert!(db(again, note).abs() < 3.0, "the note is {:.1} dB off on the way back", db(again, note));
}

#[test]
fn it_is_the_same_at_any_block_size() {
    for (name, src) in voices() {
        // The same render twice is the same samples.
        assert!(render(&src) == render(&src), "{name}: two renders differ");
        // In blocks of any size the track is gone while it is out, comes
        // back clean, and plays the same note.
        let whole = render(&src);
        let b = bar();
        for block in [3usize, 17, 100] {
            let x = render_in(&src, block);
            let note = rms(&x[b / 2..b]);
            assert!(peak(&x[b + b / 50..3 * b]) == 0.0, "{name}, blocks of {block}: not silent out of the scene");
            let back = peak(&x[3 * b..3 * b + b / 2 - 64]);
            assert!(db(back, note) < -90.0, "{name}, blocks of {block}: came back with {:.1} dB", db(back, note));
            let again = rms(&x[3 * b + b / 2..4 * b]);
            assert!(
                db(again, rms(&whole[3 * b + b / 2..4 * b])).abs() < 0.5,
                "{name}, blocks of {block}: the returning note is not the one blocks of {BLOCK_SIZE} play"
            );
        }
    }
}

/// The case reported: `oberheim_dark`'s `maw` plays through `hunt` and
/// `kill`, sits out the two bars of `stalk` and comes back with `kill`, whose
/// first step is a rest. That rest opened with 15 ms of the note it was
/// releasing when it left, at -36 dBFS.
#[test]
fn oberheim_dark_is_quiet_between_notes_when_maw_returns() {
    let src = include_str!("../../examples/oberheim_dark.synth");
    let mut e = SongEngine::from_source(src).expect("compiles");
    let maw = (0..e.track_count()).find(|&i| e.track_name(i) == "maw").expect("a maw track");
    e.set_taps(true);
    e.start();
    // dread x4 stalk x4 hunt x8 kill x8 stalk x2, then kill again.
    let bar = SAMPLE_RATE * 60.0 / 132.0 * 4.0;
    let (left, back) = ((24.0 * bar) as usize, (26.0 * bar) as usize);
    let (mut l, mut r) = ([0.0f32; BLOCK_SIZE], [0.0f32; BLOCK_SIZE]);
    let (mut pos, mut out, mut rest, mut played) = (0usize, 0.0f32, 0.0f32, 0.0f32);
    let mut first_note = None;
    while pos < back + 2 * BLOCK_SIZE || first_note.is_none() {
        e.process_block_stereo(&mut l, &mut r);
        let taps = e.taps().expect("taps on");
        let p = peak(&taps.tracks[maw].l[..taps.len]).max(peak(&taps.tracks[maw].r[..taps.len]));
        let held = taps.held[maw];
        if pos >= left - 8 * BLOCK_SIZE && pos < left {
            played = played.max(p);
        }
        // Past its 10 ms fade (a block late at most) and up to the return.
        if pos >= left + (SAMPLE_RATE / 50.0) as usize && pos + BLOCK_SIZE <= back {
            out = out.max(p);
        }
        if pos >= back && first_note.is_none() {
            if held {
                first_note = Some(pos);
            } else {
                rest = rest.max(p);
            }
        }
        pos += taps.len;
        assert!(pos < back + bar as usize, "maw never played again");
    }
    let first = first_note.unwrap();
    assert!(first - back > 1000, "the first step of kill is a rest: {} samples", first - back);
    assert!(played > 0.01, "maw was playing when stalk dropped it: peak {played}");
    assert!(out == 0.0, "maw out of the scene: peak {out}");
    assert!(
        db(rest, played) < -90.0,
        "maw came back with {:.1} dB of its old note in the rest before its first one",
        db(rest, played)
    );
}

/// A track the song's first scene does not name has never sounded: it fades
/// from nothing at the start, as it always did, and there is nothing of it to
/// clear. It is left as it was, and costs nothing, until it first plays; once
/// it has, its first exit clears it, and an exit after a scene in which it
/// stayed silent does not again.
#[test]
fn a_track_that_never_sounded_is_left_as_it_was() {
    let voice = "module bass v { release 1.5s }";
    let src = format!(
        "tempo {TEMPO}\nscale C major\n{voice}\n\
         module beats kit {{ kick_level 1.0 }}\n\
         pattern p {{ {HELD} }}\n\
         pattern rest {{ -*16 }}\n\
         pattern none {{ kick: -*16 }}\n\
         track t {{ play p using v out > delay(1/8, 0.6) > master }}\n\
         track d {{ play none using kit out > master }}\n\
         master {{ in > out }}\n\
         scene without {{ track d {{ play none using kit }} }}\n\
         scene with {{ track t {{ play p using v }} track d {{ play none using kit }} }}\n\
         scene hush {{ track t {{ play rest using v }} track d {{ play none using kit }} }}\n\
         arrange {{ without x2 with x1 without x1 hush x1 without x1 }}\n"
    );
    let mut e = SongEngine::from_source(&src).unwrap();
    e.start();
    let b = bar();
    let (mut l, mut r) = (vec![0.0f32; BLOCK_SIZE], vec![0.0f32; BLOCK_SIZE]);
    let mut pos = 0;
    let mut at = |e: &mut SongEngine, until: usize| {
        while pos < until {
            e.process_block_stereo(&mut l, &mut r);
            pos += BLOCK_SIZE;
        }
        e.cleared_track_count()
    };
    // Dropped by the first scene, and by the second, before it ever played.
    assert_eq!(at(&mut e, 2 * b + b / 4), 0, "a track that never sounded was cleared");
    // Played a bar, dropped: cleared once.
    assert_eq!(at(&mut e, 3 * b + b / 4), 1, "the track that played was not cleared when dropped");
    // Back for a bar of rests, dropped again: nothing new to clear.
    assert_eq!(at(&mut e, 6 * b), 1, "a track that stayed silent was cleared again");
}
