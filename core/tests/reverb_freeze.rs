//! Long reverb and freeze: the tail can be held as a texture.

use synth_core::song_engine::SongEngine;

const SONG: &str = r#"
tempo 120
scale F minor
reverb size=SIZE damp=0.2
module keys pad { attack 0.2 release 0.3 }
pattern chord { Fm9:0.7 ..*3 -*12 }
pattern silence { -*16 }
track pad { play chord using pad level 0.5 reverb_send 0.8 out > master }
scene bloom { track pad { play chord using pad } }
scene tail { FREEZE track pad { play silence using pad } }
arrange { bloom x1 tail x2 }
"#;

fn rms(x: &[f32]) -> f32 { (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt() }

fn tail_ratio(size: &str, freeze: &str) -> f32 {
    let src = SONG.replace("SIZE", size).replace("FREEZE", freeze);
    let mut e = SongEngine::from_source(&src).expect("load");
    e.start();
    let _ = e.render_steps(16);            // scene bloom
    let (early, _) = e.render_steps(4);    // start of the tail
    let _ = e.render_steps(20);
    let (late, _) = e.render_steps(4);     // 1.5 bars later
    rms(&late) / rms(&early).max(1e-9)
}

#[test]
fn bigger_size_means_longer_tail() {
    let small = tail_ratio("0.3", "");
    let huge = tail_ratio("1.0", "");
    assert!(huge > small * 2.0, "size 1.0 should decay much slower: small={} huge={}", small, huge);
}

#[test]
fn freeze_holds_the_tail() {
    let free = tail_ratio("0.5", "");
    let frozen = tail_ratio("0.5", "reverb_freeze = 1");
    assert!(free < 0.5, "an unfrozen tail decays: {}", free);
    assert!(frozen > 0.7, "a frozen tail holds its level: {}", frozen);
}

#[test]
fn freeze_lane_and_override_validate() {
    use synth_core::dsl::{self, compiler};
    let ok = SONG.replace("SIZE", "0.5").replace("FREEZE", "auto reverb_freeze 0 > 1");
    assert!(compiler::compile(&dsl::parse(&ok).unwrap()).is_ok());
    let bad = SONG.replace("SIZE", "0.5").replace("FREEZE", "reverb_frezze = 1");
    let errs = match compiler::compile(&dsl::parse(&bad).unwrap()) { Err(e) => e, Ok(_) => panic!() };
    assert!(errs[0].message.contains("unknown override 'reverb_frezze'"), "{}", errs[0].message);
}
