//! Every song comes out at the same loudness, under the same ceiling, however
//! loud it was mixed: the engine's output stage sets both, and a song cannot.

use tatum_core::output::{Loudness, CEILING_DB, TARGET_LUFS};
use tatum_core::song_engine::SongEngine;

fn song(level: &str) -> String {
    format!(r#"
tempo 120
scale A minor
module beats kit {{ kick_level 1.0 }}
module keys pad {{ voice_mode poly attack 10ms release 300ms cutoff 3khz }}
pattern beat {{ kick: X - - - X - - - X - - - X - - - }}
pattern hold {{ [1.3 3.3 5.3]:0.8 ..*15 }}
track kick {{ play beat using kit level {level} out > master }}
track pad  {{ play hold using pad level {level} out > master }}
scene a {{ track kick {{ play beat using kit }} track pad {{ play hold using pad }} }}
arrange {{ a x4 }}
"#)
}

fn lufs(l: &[f32], r: &[f32]) -> f32 {
    let mut m = Loudness::new();
    for (a, b) in l.iter().zip(r) {
        m.push(*a, *b);
    }
    m.lufs().unwrap()
}

#[test]
fn a_quiet_mix_and_a_loud_one_come_out_equally_loud() {
    for level in ["0.1", "0.5", "3.0"] {
        let compiled = tatum_core::dsl::isolate::compile(&song(level), &Default::default()).unwrap();
        let mut e = SongEngine::normalized(compiled);
        let (l, r) = e.render(4);
        let got = lufs(&l, &r);
        // The loud one is pushed into the limiter, which takes a little off.
        assert!((got - TARGET_LUFS).abs() < 0.5, "level {level}: {got:.2} LUFS");
        let peak = l.iter().chain(&r).fold(0.0f32, |a, v| a.max(v.abs()));
        assert!(20.0 * peak.log10() <= CEILING_DB, "level {level}: sample peak {:.2} dB", 20.0 * peak.log10());
    }
}

#[test]
fn a_limiter_on_the_master_is_left_out() {
    let plain = song("0.5");
    let limited = plain.replace("arrange", "master { in > limiter(0.3) > out }\narrange");
    let a = SongEngine::from_source(&plain).unwrap().render(1);
    let b = SongEngine::from_source(&limited).unwrap().render(1);
    assert_eq!(a, b);
}
