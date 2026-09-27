//! FM voice allocation.

use tatum_core::modules::fm::FmModule;
use tatum_core::{Module, BLOCK_SIZE};

fn energy(m: &mut FmModule, blocks: usize) -> f32 {
    let mut buf = [0.0f32; BLOCK_SIZE];
    let mut e = 0.0;
    for _ in 0..blocks {
        m.process_block(&mut buf);
        e += buf.iter().map(|v| v * v).sum::<f32>();
    }
    e
}

/// A note struck again while its previous strike is still releasing has two
/// voices on it. The note-off has to reach the new one: releasing the first
/// match used to release the fading voice a second time and leave the new
/// one sustaining forever, a drone under everything that came after.
#[test]
fn a_note_struck_again_while_it_fades_is_released() {
    let mut m = FmModule::new();
    m.note_on(48, 1.0);
    energy(&mut m, 40);
    m.note_off(48);
    energy(&mut m, 4); // the first strike is releasing, not yet idle
    m.note_on(48, 1.0);
    energy(&mut m, 40);
    m.note_off(48);

    // Two seconds later nothing should be left sounding.
    energy(&mut m, (2 * 44_100) / BLOCK_SIZE);
    let tail = energy(&mut m, 40);
    assert!(tail < 1e-6, "a voice is still sounding after both note-offs: energy {tail}");
}
