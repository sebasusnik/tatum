//! `--solo` / `--mute` and the engine's taps, the two halves of `tatum debug`.
//!
//! What matters about a soloed track is that it sounds the way it does in the
//! mix. A pad that ducks against the kick has to keep ducking with the kick
//! muted, or the solo shows a different sound from the one being chased.

use tatum_core::dsl::isolate::{self, Isolation};
use tatum_core::song_engine::SongEngine;
use tatum_core::output::OUTPUT_DELAY;
use tatum_core::BLOCK_SIZE;

const SONG: &str = r#"
tempo 120
scale C major

module beats kit { kick_level 1.0 }
module bass low { cutoff 0.4 sustain 0.9 }
module keys pad { cutoff 0.6 sustain 1.0 attack 1ms }

pattern beat { kick: X - - -  - - - -  X - - -  - - - - }
pattern bassline { - - - -  1.1 .. .. ..  - - - -  1.1 .. .. .. }
pattern hold { [1.3 3.3 5.3] .. .. ..  .. .. .. ..  .. .. .. ..  .. .. .. .. }

bus low_end
low_end { in > compressor(-12, ratio=3, attack=5, release=80) > master }

track drums { play beat using kit out > master }
track bass  { play bassline using low out > low_end }
track pad   { play hold using pad sidechain 0.8 reverb_send 0.4 out > master }

scene a {
    track drums { play beat using kit }
    track bass  { play bassline using low }
    track pad   { play hold using pad }
    auto drums level 1.0 > 0.5
}
arrange { a x2 }
"#;

fn iso(solo: &[&str], mute: &[&str]) -> Isolation {
    Isolation {
        solo: solo.iter().map(|s| s.to_string()).collect(),
        mute: mute.iter().map(|s| s.to_string()).collect(),
    }
}

/// Every part of the render, block by block: per track, per bus, the two
/// returns, and the output.
struct Parts {
    tracks: Vec<Vec<f32>>,
    buses: Vec<Vec<f32>>,
    returns: Vec<f32>,
    out: Vec<f32>,
    names: Vec<String>,
}

fn render(isolation: &Isolation, bars: usize) -> Parts {
    let song = isolate::compile(SONG, isolation).unwrap_or_else(|e| panic!("{}", e.to_json()));
    let mut e = SongEngine::from_compiled(song);
    e.set_taps(true);
    e.start();
    let names = (0..e.track_count()).map(|i| e.track_name(i).to_string()).collect();
    let mut p = Parts {
        tracks: vec![Vec::new(); e.track_count()],
        buses: vec![Vec::new(); e.bus_count()],
        returns: Vec::new(),
        out: Vec::new(),
        names,
    };
    let blocks = bars * 2 * 44100 / BLOCK_SIZE; // 120 bpm: two seconds a bar
    for _ in 0..blocks {
        let (mut l, mut r) = ([0.0f32; BLOCK_SIZE], [0.0f32; BLOCK_SIZE]);
        e.process_block_stereo(&mut l, &mut r);
        let t = e.taps().unwrap();
        for (i, tr) in t.tracks.iter().enumerate() { p.tracks[i].extend_from_slice(&tr.l); }
        for (i, b) in t.buses.iter().enumerate() { p.buses[i].extend_from_slice(&b.l); }
        p.returns.extend((0..BLOCK_SIZE).map(|s| t.delay.l[s] + t.reverb.l[s]));
        p.out.extend_from_slice(&l);
    }
    p
}

impl Parts {
    fn track(&self, name: &str) -> &[f32] {
        &self.tracks[self.names.iter().position(|n| n == name).unwrap()]
    }
}

fn peak(x: &[f32]) -> f32 { x.iter().fold(0.0, |a, v| a.max(v.abs())) }

#[test]
fn the_parts_add_up_to_the_mix() {
    // No master chain: what enters master comes out,
    // at the engine's fixed master level, OUTPUT_DELAY samples later (this
    // song stays under the output limiter's ceiling).
    const MASTER_LEVEL: f32 = 0.8;
    let p = render(&Isolation::default(), 1);
    let mut worst = 0.0f32;
    for s in 0..p.out.len() - OUTPUT_DELAY {
        // drums and pad go to master; bass only through its bus
        let sum = p.track("drums")[s] + p.track("pad")[s] + p.buses[0][s] + p.returns[s];
        worst = worst.max((sum * MASTER_LEVEL - p.out[s + OUTPUT_DELAY]).abs());
    }
    assert!(peak(&p.out) > 0.05);
    assert!(worst < 1e-5, "the parts miss the mix by {worst}");
}

#[test]
fn a_soloed_track_sounds_as_it_does_in_the_mix() {
    let full = render(&Isolation::default(), 2);
    let solo = render(&iso(&["pad"], &[]), 2);
    assert_eq!(full.track("pad"), solo.track("pad"),
        "the pad changed when soloed: the muted kick must still duck it");
    assert_eq!(peak(solo.track("drums")), 0.0);
    assert_eq!(peak(solo.track("bass")), 0.0);
    assert_eq!(peak(&solo.buses[0]), 0.0);
}

#[test]
fn a_muted_track_stays_muted_through_its_level_automation() {
    let muted = render(&iso(&[], &["drums"]), 2);
    assert_eq!(peak(muted.track("drums")), 0.0, "`auto drums level` brought the muted drums back");
    assert!(peak(muted.track("bass")) > 0.01);
    assert!(peak(muted.track("pad")) > 0.01);
}

#[test]
fn a_name_that_is_not_a_track_is_an_error_that_lists_the_tracks() {
    let err = isolate::compile(SONG, &iso(&["pda"], &[])).err().expect("should fail").to_json();
    assert!(err.contains("pda") && err.contains("drums, bass, pad"), "{err}");
}
