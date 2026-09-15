//! The bar counter is the clock everything above the step sequencer keys on:
//! scene changes, arrangement end, tempo and freeze automation, and the hot
//! swap. It used to advance after the last step of a bar fired instead of
//! before the first step of the next one, so `current_bar` read one sixteenth
//! early. Nothing in the suite noticed, because every test checked that
//! something sounded and none checked when. These check when.

use synth_core::song_engine::SongEngine;
use synth_core::SAMPLE_RATE;

const RIG: &str = r#"
tempo 120
scale A minor
module beats kit { kick_level 1.0 }
module keys pad { voice_mode poly attack 1ms release 5ms cutoff 5khz }
pattern beat { kick: X - - - X - - - X - - - X - - - }
pattern every { 1.3 1.3 1.3 1.3 1.3 1.3 1.3 1.3 1.3 1.3 1.3 1.3 1.3 1.3 1.3 1.3 }
track kick { play beat using kit level 0.0 out > master }
track pad  { play every using pad level 0.8 gate 0.9 out > master }
master { in > out }
"#;

/// Samples per sixteenth at 120 BPM: 44100 * 60 / 120 / 4.
const STEP: f32 = SAMPLE_RATE * 60.0 / 120.0 / 4.0;
const BAR: f32 = STEP * 16.0;

/// Drive the engine one sample at a time and record the sample index at which
/// `current_bar` changes. That is the only way to observe the counter at
/// sample resolution; blocks would hide an error smaller than a block.
fn bar_crossings(engine: &mut SongEngine, samples: usize) -> Vec<(usize, usize)> {
    let mut l = [0.0f32; 1];
    let mut r = [0.0f32; 1];
    let mut last = engine.current_bar();
    let mut out = Vec::new();
    for s in 0..samples {
        engine.process_block_stereo(&mut l, &mut r);
        let bar = engine.current_bar();
        if bar != last {
            out.push((bar, s));
            last = bar;
        }
    }
    out
}

#[test]
fn bar_counter_crosses_on_the_downbeat_sample() {
    let mut e = SongEngine::from_source(RIG).unwrap();
    e.start();
    let crossings = bar_crossings(&mut e, (BAR * 3.0) as usize + 10);
    assert_eq!(crossings.len(), 3, "three bars rendered, three crossings: {:?}", crossings);
    for (bar, sample) in crossings {
        // The step clock accumulates fractional samples (5512.5 per step), so
        // the downbeat can land one sample either side of the ideal position.
        // One sixteenth early is 5512 samples; that is what this guards.
        let expected = (bar as f32 * BAR) as isize;
        let delta = sample as isize - expected;
        assert!(delta.abs() <= 1,
            "bar {} crossed at sample {} but the downbeat is at {} (delta {})",
            bar, sample, expected, delta);
    }
}

#[test]
fn start_from_bar_does_not_count_the_first_bar_twice() {
    // start_from_bar sets both global_step and current_bar. The crossing
    // logic compares them, so the first crossing after a seek must be the
    // next bar, one full bar later, not an immediate spurious increment.
    let mut e = SongEngine::from_source(RIG).unwrap();
    e.start_from_bar(4);
    assert_eq!(e.current_bar(), 4);
    let crossings = bar_crossings(&mut e, (BAR * 1.0) as usize + 10);
    assert_eq!(crossings.len(), 1, "{:?}", crossings);
    let (bar, sample) = crossings[0];
    assert_eq!(bar, 5);
    assert!((sample as isize - BAR as isize).abs() <= 1, "bar 5 crossed at {}", sample);
}

#[test]
fn scene_change_does_not_cut_the_last_step_of_the_previous_scene() {
    // The pad plays on every step with gate 0.9. Scene a is one bar, then
    // scene b (silent). The note at step 15 of scene a must sound for 90 % of
    // its step like every other one. It used to sound for 0 %: the scene
    // change ran together with the trigger and released it on the same sample.
    let src = format!("{}\n\
        scene a {{ track pad {{ play every using pad level 0.8 gate 0.9 }} }}\n\
        scene b {{ track kick {{ play beat using kit level 0.0 }} }}\n\
        arrange {{ a x1 b x1 }}\n", RIG);
    let mut e = SongEngine::from_source(&src).unwrap();
    e.start();
    let (l, _) = e.render_steps(32);
    let audible = |step: usize| -> f32 {
        let s = (step as f32 * STEP) as usize;
        let seg = &l[s..s + STEP as usize];
        let last = seg.iter().rposition(|v| v.abs() > 0.005).unwrap_or(0);
        last as f32 / STEP
    };
    // Compare against a mid-bar step rather than the nominal gate: the
    // release and the poly voice's own envelope decide the exact figure. The
    // failure this guards is 0 % against 100 %, not a few percent.
    let reference = audible(13);
    assert!(reference > 0.85, "step 13 audible for {:.0}%", reference * 100.0);
    for step in [14usize, 15] {
        let frac = audible(step);
        assert!((frac - reference).abs() < 0.05,
            "step {} audible for {:.0}% of its length, step 13 for {:.0}%",
            step, frac * 100.0, reference * 100.0);
    }
    // And scene b really is silent: the pad did not leak past the bar line
    // beyond its 5 ms release.
    let release = (0.005 * SAMPLE_RATE) as usize + 100;
    let tail = &l[BAR as usize + release..(BAR * 2.0) as usize];
    assert!(tail.iter().all(|v| v.abs() < 0.005), "scene b is not silent");
}

#[test]
fn arranged_render_has_every_sample_of_its_last_bar() {
    // Two bars arranged. The render must stop on the downbeat of bar 2, not
    // one sixteenth before it. Compared against the engine's own arithmetic
    // so a change in samples_per_step does not make this test lie.
    let src = format!("{}\n\
        scene a {{ track pad {{ play every using pad level 0.8 gate 0.9 }} }}\n\
        scene b {{ track pad {{ play every using pad level 0.8 gate 0.9 }} }}\n\
        arrange {{ a x1 b x1 }}\n", RIG);
    let mut e = SongEngine::from_source(&src).unwrap();
    assert_eq!(e.arrangement_bars(), 2);
    let (l, _) = e.render(e.arrangement_bars());
    let expected = (BAR * 2.0) as usize;
    assert!(l.len() >= expected,
        "arranged render wrote {} samples, the arrangement is {} long (short by {}, one sixteenth is {})",
        l.len(), expected, expected - l.len(), STEP as usize);
    // The last step of the last bar is audible for its 90 % gate.
    let s = (STEP * 31.0) as usize;
    let seg = &l[s..(s + STEP as usize).min(l.len())];
    let last = seg.iter().rposition(|v| v.abs() > 0.005).unwrap_or(0) as f32 / STEP;
    assert!(last > 0.85, "last step audible for {:.0}% of its length", last * 100.0);
}
