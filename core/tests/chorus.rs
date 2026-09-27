//! The chorus reads its delay line at a moving, fractional position. The
//! read has to move smoothly: it used to interpolate towards the newer
//! sample while the delay grew towards the older one, so it jumped two
//! samples every time the delay crossed a whole one, hundreds of times a
//! second, and a bright pad through it sounded bitcrushed.

use tatum_core::effects::chorus::Chorus;
use tatum_core::SAMPLE_RATE;

#[test]
fn the_wet_copy_moves_no_faster_than_the_input() {
    let mut chorus = Chorus::new();
    chorus.set_mix(1.0);
    let hz = 3000.0;
    let step = 2.0 * core::f32::consts::PI * hz / SAMPLE_RATE;
    let input: Vec<f32> = (0..SAMPLE_RATE as usize * 2).map(|n| (n as f32 * step).sin()).collect();
    let out: Vec<f32> = input.iter().map(|&x| chorus.process(x)).collect();
    let steepest = |x: &[f32]| x.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
    // Skip the first 50 ms, while the delay line fills.
    let (i, o) = (steepest(&input[2205..]), steepest(&out[2205..]));
    // The sweep detunes the copy by up to 4%, so it may move that much
    // faster; a two-sample jump moves it almost twice as fast.
    assert!(o < i * 1.1, "wet copy steepest step {o:.3} against the input's {i:.3}");
}

#[test]
fn both_channels_read_smoothly() {
    let mut chorus = Chorus::new();
    chorus.set_mix(1.0);
    let step = 2.0 * core::f32::consts::PI * 3000.0 / SAMPLE_RATE;
    let mut prev = (0.0f32, 0.0f32);
    let mut worst = 0.0f32;
    for n in 0..SAMPLE_RATE as usize * 2 {
        let x = (n as f32 * step).sin();
        let (l, r) = chorus.process_stereo(x, x);
        if n > 2205 {
            worst = worst.max((l - prev.0).abs()).max((r - prev.1).abs());
        }
        prev = (l, r);
    }
    assert!(worst < step * 1.1, "steepest step {worst:.3} against the input's {step:.3}");
}
