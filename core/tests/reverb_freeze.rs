//! Long reverb and freeze: the tail can be held as a texture.

use tatum_core::song_engine::SongEngine;

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
    use tatum_core::dsl::{self, compiler};
    let ok = SONG.replace("SIZE", "0.5").replace("FREEZE", "auto reverb_freeze 0 > 1");
    assert!(compiler::compile(&dsl::parse(&ok).unwrap()).is_ok());
    let bad = SONG.replace("SIZE", "0.5").replace("FREEZE", "reverb_frezze = 1");
    let errs = match compiler::compile(&dsl::parse(&bad).unwrap()) { Err(e) => e, Ok(_) => panic!() };
    assert!(errs[0].message.contains("unknown override 'reverb_frezze'"), "{}", errs[0].message);
}

const BUS_SONG: &str = r#"
tempo 172
scale F minor
reverb size=0.8 sidechain=0.5
bus wash
wash { in > reverb(1.0) > out }
module keys pad { attack 0.5 release 0.5 }
module beats kit { kick_level 1.0 }
pattern chord { Fm9:0.7 ..*15 }
pattern beat { kick: X - - - X - - - X - - - X - - - }
track pad { play chord using pad level 0.5 reverb_send 0.5 out > wash }
track drums { play beat using kit out > master }
reverb_return { in > lowpass(1200, 0.3, lfo_bars=4, lfo_depth=800) > autopan(0.5, bars=2) > out }
scene a { track pad { play chord using pad } track drums { play beat using kit } }
arrange { a x16 }
"#;

#[test]
fn bus_reverb_node_stays_bounded_on_sustained_input() {
    let mut e = SongEngine::from_source(BUS_SONG).expect("load");
    e.start();
    let (first, _) = e.render_steps(32);
    let _ = e.render_steps(16 * 12);
    let (last, _) = e.render_steps(32);
    assert!(last.iter().all(|v| v.is_finite()), "non-finite samples");
    let peak = |x: &[f32]| x.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
    assert!(peak(&last) <= 1.0, "peak {} after 16 bars", peak(&last));
    assert!(peak(&last) < peak(&first) * 3.0 + 0.05, "reverb bus must not grow: first={} last={}", peak(&first), peak(&last));
}

#[test]
fn return_chain_and_send_sidechain_apply() {
    use tatum_core::dsl::{self, compiler};
    let ast = dsl::parse(BUS_SONG).unwrap();
    assert_eq!(ast.globals.send_reverb.sidechain, Some(0.5));
    let song = compiler::compile(&ast).expect("compile");
    assert_eq!(song.reverb_return.len(), 2, "lowpass + autopan on the reverb return");

    let plain = BUS_SONG.replace("reverb_return { in > lowpass(1200, 0.3, lfo_bars=4, lfo_depth=800) > autopan(0.5, bars=2) > out }", "");
    let mut a = SongEngine::from_source(BUS_SONG).unwrap();
    let mut b = SongEngine::from_source(&plain).unwrap();
    let (al, _) = a.render(2);
    let (bl, _) = b.render(2);
    let diff: f32 = al.iter().zip(&bl).map(|(x, y)| (x - y).abs()).sum::<f32>() / al.len() as f32;
    assert!(diff > 1e-4, "the return chain must change the output");

    let bad = BUS_SONG.replace("reverb_return {", "reverb_retrun {");
    let errs = match compiler::compile(&dsl::parse(&bad).unwrap()) { Err(e) => e, Ok(_) => panic!() };
    assert!(errs[0].message.contains("chain 'reverb_retrun' has no `bus reverb_retrun` declaration"), "{}", errs[0].message);
}
