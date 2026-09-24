//! Does it survive a live callback? Rendering faster than real time on
//! average says nothing: the audio device asks for one small block at a time,
//! and a single late block is an audible click.
//!
//! So the honest thing to measure is the worst block — and on a shared CI
//! runner you cannot. The worst block there is the hypervisor taking the CPU
//! away, not the engine. `live_set` failed this at 94% of the budget on one
//! block out of 5170, while its mean was 4% and its 99th percentile 6%: the
//! engine was nowhere near late, the machine went to sleep for 2.7 ms.
//!
//! The 99th percentile is not safe either: on a laptop with a build running,
//! liquid_dnb's p99 went from 268 us to 1443 us — half the budget — while its
//! median barely moved. Fifty blocks out of 5170 is not many blocks.
//!
//! So this asserts the **median**, which is the one statistic a busy machine
//! cannot ruin, against half the budget. That is not the claim about clicks;
//! it is the claim that the engine has a large margin, and it is the one that
//! catches what this test is actually for — an engine that got several times
//! slower. Everything else is printed, because on a shared runner those
//! numbers describe the runner.

use std::time::Instant;
use tatum_core::song_engine::SongEngine;
use tatum_core::BLOCK_SIZE;

fn report(name: &str, path: &str) -> f64 {
    let src = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let mut e = SongEngine::from_source(&src).unwrap_or_else(|err| panic!("{path}: {err}"));
    e.start();
    let mut l = [0.0f32; BLOCK_SIZE];
    let mut r = [0.0f32; BLOCK_SIZE];
    let budget_us = BLOCK_SIZE as f64 / tatum_core::SAMPLE_RATE as f64 * 1e6;
    let blocks = (tatum_core::SAMPLE_RATE as f64 * 60.0 / BLOCK_SIZE as f64) as usize;

    let mut times = Vec::with_capacity(blocks);
    for _ in 0..blocks {
        let t = Instant::now();
        e.process_block_stereo(&mut l, &mut r);
        times.push(t.elapsed().as_secs_f64() * 1e6);
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let pick = |q: f64| times[((times.len() - 1) as f64 * q) as usize];
    let worst = *times.last().unwrap();
    let median = pick(0.5);
    println!(
        "{name:<18} presupuesto {budget_us:.0} us | mediana {median:6.1} = {:4.1}% | p99 {:6.1} | peor {worst:7.1} us  (p99 y peor son la máquina)",
        median / budget_us * 100.0,
        pick(0.99),
    );
    median / budget_us
}

/// Release only. A debug build is about five times slower than the one you
/// would actually play with (`cargo run --release -- play`), so under
/// `cargo test` this measures a machine nobody uses. Ignoring it in debug is
/// not weakening the check, it is pointing it at the build the claim is
/// about. CI runs it with `--release`.
#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "release only: debug is ~5x slower than the build you would play with"
)]
fn the_engine_fits_in_a_callback_with_room_to_spare() {
    println!();
    for (n, p) in [
        ("liquid_dnb", "../examples/liquid_dnb.synth"),
        ("psytech_goa", "../examples/psytech_goa.synth"),
        ("primus_mud", "../examples/primus_mud.synth"),
        ("live_rig (apagado)", "../examples/rigs/live_rig.synth"),
        ("live_set", "../examples/live_set.synth"),
    ] {
        let used = report(n, p);
        // Half the budget at the median. The corpus sits between 2% and 8%,
        // so this is a six-fold margin: it is here to catch an engine that
        // got several times slower, not to police a runner's scheduler.
        assert!(
            used < 0.5,
            "{n}: the median block took {:.0}% of the callback budget",
            used * 100.0
        );
    }
}
