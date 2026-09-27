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
    format!(r#"
tempo 120
scale A minor
module bass low {{ cutoff 0.5 sustain 0.9 }}
pattern line {{ {bass}:0.9 .. .. ..  .. .. .. ..  .. .. .. ..  .. .. .. .. }}
track bass {{ play line using low level 0.5 out > master }}
scene a {{ track bass {{ play line using low }} }}
arrange {{ a x2 }}
"#)
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
