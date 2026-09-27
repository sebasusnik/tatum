//! What `tatum render` hears while it renders is what `tatum debug` finds.

use tatum_core::song_engine::SongEngine;
use tatum_debug::Listener;

fn heard(src: &str) -> Vec<String> {
    let mut e = SongEngine::from_source(src).unwrap();
    let mut listener = Listener::new(&mut e);
    e.render_each(2, |e, l, r| listener.feed(e, l, r));
    listener.findings()
}

fn song(bass: &str) -> String {
    format!(
        r#"
tempo 120
scale A minor
module bass low {{ cutoff 0.5 sustain 0.9 }}
pattern line {{ {bass}:0.9 .. .. ..  .. .. .. ..  .. .. .. ..  .. .. .. .. }}
track bass {{ play line using low level 0.5 out > master }}
scene a {{ track bass {{ play line using low }} }}
arrange {{ a x2 }}
"#
    )
}

#[test]
fn a_clean_bass_is_heard_as_clean() {
    assert_eq!(heard(&song("A2")), Vec::<String>::new());
}

#[test]
fn a_bass_pitched_under_the_speakers_is_heard_there() {
    // E0 is 20.6 Hz.
    let h = heard(&song("E0"));
    assert!(h.iter().any(|l| l.starts_with("under 25 Hz") && l.contains("bass")), "{h:?}");
}

/// A reverb that ducks against the kick: a held low chord fills the tail and
/// the kick pulls it down on every beat. The returns used to take one duck
/// value per block, so their gain stepped every 128 samples when the kick
/// hit, and each step was a click on the tail.
#[test]
fn a_ducked_reverb_tail_does_not_click() {
    let src = r#"
tempo 122
scale F minor
sidechain 0.45 attack=1ms release=150ms
reverb size=0.88 damp=0.5 sidechain=0.8
module beats kit { kick_level 100% }
module keys pad { voice_mode poly cutoff 1.2khz attack 20ms release 1s }
pattern beat { kick: X - - - X - - - X - - - X - - - }
pattern hold { [F2 Ab2 C3]:0.8 .. .. .. .. .. .. .. .. .. .. .. .. .. .. .. }
track beat { play beat using kit out > master }
track pad  { play hold using pad level 0.2 reverb_send 1.0 sidechain 0 out > master }
scene a { track beat { play beat using kit } track pad { play hold using pad } }
arrange { a x4 }
"#;
    let mut e = SongEngine::from_source(src).unwrap();
    let mut listener = Listener::new(&mut e);
    e.render_each(4, |e, l, r| listener.feed(e, l, r));
    let h = listener.findings();
    assert!(!h.iter().any(|l| l.contains("reverb")), "{h:?}");
}
