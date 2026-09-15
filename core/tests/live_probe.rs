//! Temporary measurement probe for the livecoding analysis. Mirrors the
//! WASM hot-swap logic (wasm/src/lib.rs) natively so the swap can be measured.
use synth_core::song_engine::SongEngine;
use synth_core::BLOCK_SIZE;
use std::time::Instant;

const SONG: &str = r#"
tempo 120
scale A minor
reverb size=1.0 damp=0.3 predelay=10
delay sync=dotted_eighth feedback=0.6

module beats kit { kick_level 1.0 }
module keys pad { voice_mode poly attack 10ms release 400ms cutoff 2khz }

pattern beat { kick: X - - - X - - - X - - - X - - - }
pattern hold { [1.3 3.3 5.3]:0.8 ..*15 }

track kick { play beat using kit level 0.8 out > master }
track pad  { play hold using pad level 0.5 reverb_send 0.7 delay_send 0.5 out > master }
master { in > limiter > out }
"#;

// Structurally different (one more unused pattern), identical audio.
fn song_b() -> String { format!("{}\npattern extra {{ 1.1 - - - }}\n", SONG) }

fn render_straight(src: &str, bars: usize) -> (Vec<f32>, Vec<f32>) {
    let mut e = SongEngine::from_source(src).unwrap();
    e.start();
    let steps = bars * 16;
    e.render_steps(steps)
}

/// Mirrors wasm/src/lib.rs: run engine A, when the bar counter changes past
/// `swap_bar` swap to engine B via start_from_bar with a 512-sample crossfade.
fn render_with_swap(src_a: &str, src_b: &str, bars: usize, swap_bar: usize) -> (Vec<f32>, Vec<f32>) {
    let mut a = SongEngine::from_source(src_a).unwrap();
    a.start();
    let total = (bars as f32 * 16.0 * a_spb(&a)) as usize;
    let mut pending = Some(SongEngine::from_source(src_b).unwrap());
    let mut out_l = vec![0.0f32; total];
    let mut out_r = vec![0.0f32; total];
    let mut pos = 0;
    let mut last_bar = 0;
    let mut xf: Option<(Vec<f32>, Vec<f32>, usize)> = None;
    while pos < total {
        let chunk = BLOCK_SIZE.min(total - pos);
        let (l, r) = (&mut out_l[pos..pos + chunk], &mut out_r[pos..pos + chunk]);
        a.process_block_stereo(l, r);
        if let Some((ol, or, xpos)) = xf.as_mut() {
            let n = chunk.min(512 - *xpos);
            for i in 0..n {
                let t = (*xpos + i) as f32 / 512.0;
                l[i] = l[i] * t + ol[*xpos + i] * (1.0 - t);
                r[i] = r[i] * t + or[*xpos + i] * (1.0 - t);
            }
            *xpos += n;
            if *xpos >= 512 { xf = None; }
        }
        let cur = a.current_bar();
        if pending.is_some() && cur != last_bar && cur >= swap_bar {
            let mut ol = vec![0.0f32; 512];
            let mut or = vec![0.0f32; 512];
            let mut off = 0;
            while off < 512 {
                let c = (512 - off).min(BLOCK_SIZE);
                a.process_block_stereo(&mut ol[off..off + c], &mut or[off..off + c]);
                off += c;
            }
            xf = Some((ol, or, 0));
            let mut b = pending.take().unwrap();
            b.start_from_bar(cur);
            a = b;
        }
        last_bar = cur;
        pos += chunk;
    }
    (out_l, out_r)
}

fn a_spb(e: &SongEngine) -> f32 { 44100.0 * 60.0 / e.tempo() / 4.0 }

fn rms(x: &[f32]) -> f32 { (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt() }
fn db(x: f32) -> f32 { 20.0 * x.max(1e-9).log10() }

#[test]
fn probe_swap_tail_loss() {
    let bars = 8;
    let swap_bar = 4;
    let (ref_l, _) = render_straight(SONG, bars);
    let (sw_l, _) = render_with_swap(SONG, &song_b(), bars, swap_bar);
    assert_eq!(ref_l.len(), sw_l.len());
    let spb = 44100.0 * 60.0 / 120.0 / 4.0;
    let bar_samples = (spb * 16.0) as usize;
    let swap_at = swap_bar * bar_samples;
    // Before the swap the two must be identical.
    let pre_diff: f32 = ref_l[..swap_at - 256].iter().zip(&sw_l[..swap_at - 256]).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
    println!("max |diff| before swap: {:.6}", pre_diff);
    // Window: the pad is held (tie) so the pad+reverb+delay should be continuous.
    // Measure RMS in 100 ms windows after the swap, reference vs swapped.
    let win = 4410;
    for k in 0..10 {
        let s = swap_at + k * win;
        let r = rms(&ref_l[s..s + win]);
        let w = rms(&sw_l[s..s + win]);
        println!("t+{:>4}ms  ref {:>6.1} dB  swapped {:>6.1} dB  delta {:>+5.1} dB", k * 100, db(r), db(w), db(w) - db(r));
    }
    // Sample-level worst-case discontinuity around the swap point.
    let mut max_jump = 0.0f32;
    for i in swap_at.saturating_sub(600)..swap_at + 600 {
        max_jump = max_jump.max((sw_l[i + 1] - sw_l[i]).abs());
    }
    let mut ref_jump = 0.0f32;
    for i in swap_at.saturating_sub(600)..swap_at + 600 {
        ref_jump = ref_jump.max((ref_l[i + 1] - ref_l[i]).abs());
    }
    println!("max sample jump around swap: swapped {:.4} ref {:.4}", max_jump, ref_jump);
}

#[test]
fn probe_swap_double_trigger() {
    // Count kick onsets in the 30 ms after the swap bar: the reference has one.
    let bars = 8;
    let swap_bar = 4;
    let src = SONG.replace("level 0.5 reverb_send 0.7 delay_send 0.5", "level 0.0");
    let srcb = format!("{}\npattern extra {{ 1.1 - - - }}\n", src);
    let (ref_l, _) = render_straight(&src, bars);
    let (sw_l, _) = render_with_swap(&src, &srcb, bars, swap_bar);
    let spb = 44100.0 * 60.0 / 120.0 / 4.0;
    let swap_at = swap_bar * (spb * 16.0) as usize;
    let onsets = |x: &[f32]| -> Vec<usize> {
        // envelope rise detector: |x| crossing 0.05 after being below for 1ms
        let mut out = vec![];
        let mut below = 0usize;
        for i in swap_at.saturating_sub(300)..swap_at + 2000 {
            if x[i].abs() < 0.02 { below += 1; }
            else { if below > 44 { out.push(i as isize as usize); } below = 0; }
        }
        out
    };
    let ro = onsets(&ref_l); let so = onsets(&sw_l);
    println!("kick onsets near swap bar (samples rel. to bar start): ref {:?} swapped {:?}",
        ro.iter().map(|i| *i as isize - swap_at as isize).collect::<Vec<_>>(),
        so.iter().map(|i| *i as isize - swap_at as isize).collect::<Vec<_>>());
}

#[test]
fn probe_compile_latency_and_rt_headroom() {
    for path in ["../dogfood/psytech_goa.synth", "../dogfood/dub_techno.synth", "../examples/hitech_psy.synth"] {
        let src = std::fs::read_to_string(path).unwrap();
        let n = 20;
        let t0 = Instant::now();
        let mut ast = None;
        for _ in 0..n { ast = Some(synth_core::dsl::parse(&src).unwrap()); }
        let t_parse = t0.elapsed() / n;
        let ast = ast.unwrap();
        let t1 = Instant::now();
        let mut compiled = None;
        for _ in 0..n { compiled = Some(synth_core::dsl::compiler::compile(&ast).unwrap()); }
        let t_compile = t1.elapsed() / n;
        let t2 = Instant::now();
        let mut engine = None;
        for _ in 0..n { engine = Some(SongEngine::from_compiled(synth_core::dsl::compiler::compile(&ast).unwrap())); }
        let t_build = t2.elapsed() / n - t_compile;
        let mut e = engine.unwrap();
        e.start();
        let blocks = 4000; // ~11.6 s of audio
        let mut l = [0.0f32; BLOCK_SIZE]; let mut r = [0.0f32; BLOCK_SIZE];
        let mut worst = std::time::Duration::ZERO;
        let t3 = Instant::now();
        for _ in 0..blocks {
            let b0 = Instant::now();
            e.process_block_stereo(&mut l, &mut r);
            worst = worst.max(b0.elapsed());
        }
        let total = t3.elapsed();
        let audio_secs = blocks as f32 * BLOCK_SIZE as f32 / 44100.0;
        let block_budget_us = BLOCK_SIZE as f32 / 44100.0 * 1e6;
        println!("{}: parse {:?}  compile {:?}  build {:?}  | render {:.1}x realtime, mean block {:.0}us worst {:.0}us (budget {:.0}us)",
            path, t_parse, t_compile, t_build,
            audio_secs / total.as_secs_f32(),
            total.as_secs_f32() * 1e6 / blocks as f32, worst.as_secs_f32() * 1e6, block_budget_us);
    }
}

#[test]
fn probe_first_divergence() {
    let src = SONG.replace("level 0.5 reverb_send 0.7 delay_send 0.5", "level 0.0");
    let srcb = format!("{}\npattern extra {{ 1.1 - - - }}\n", src);
    let (ref_l, _) = render_straight(&src, 8);
    let (sw_l, _) = render_with_swap(&src, &srcb, 8, 4);
    let first = ref_l.iter().zip(&sw_l).position(|(a, b)| (a - b).abs() > 1e-6);
    println!("first divergence at sample {:?} (swap bar starts at 23520)", first);
    let swap_at = 23520;
    println!("rms ref after swap {:.4}  swapped after swap {:.4}", rms(&ref_l[swap_at..swap_at+5880]), rms(&sw_l[swap_at..swap_at+5880]));
    println!("rms ref before swap {:.4}  swapped before swap {:.4}", rms(&ref_l[..swap_at]), rms(&sw_l[..swap_at]));
    // Straight run of B alone, started with start_from_bar(4), first bar.
    let mut b = SongEngine::from_source(&srcb).unwrap();
    b.start_from_bar(4);
    let (bl, _) = b.render_steps(16);
    println!("B alone via start_from_bar(4): rms {:.4}, running {}", rms(&bl), b.running());
    let mut c = SongEngine::from_source(&srcb).unwrap();
    c.start();
    let (cl, _) = c.render_steps(16);
    println!("B alone via start(): rms {:.4}", rms(&cl));
}

#[test]
fn probe_bar_counter_timing() {
    // Track current_bar() sample by sample. Bar 1 should begin at sample 88200 (120 BPM).
    let src = SONG.replace("level 0.5 reverb_send 0.7 delay_send 0.5", "level 0.0");
    let mut e = SongEngine::from_source(&src).unwrap();
    e.start();
    let mut l = [0.0f32; 1]; let mut r = [0.0f32; 1];
    let mut last = e.current_bar();
    for s in 0..200_000usize {
        e.process_block_stereo(&mut l, &mut r);
        let b = e.current_bar();
        if b != last { println!("current_bar -> {} at sample {} (expected {})", b, s, b * 88200); last = b; }
    }
    // Same for a two-scene arrangement: when does scene b's kick land?
    let arranged = format!("{}\nscene a {{ track kick {{ play beat using kit }} }}\nscene b {{ track pad {{ play hold using pad level 0.0 }} }}\narrange {{ a x1 b x1 }}\n", src);
    let mut e = SongEngine::from_source(&arranged).unwrap();
    e.start();
    let (ol, _) = e.render_steps(32);
    // Bar 0 has kicks at steps 0,4,8,12 (samples 0, 22050, 44100, 66150). Bar 1 is silent.
    // If the scene switches a step early, the kick at step 12 still sounds but step 15 area is where the switch happens; the observable is: silence begins where?
    let mut last_loud = 0;
    for (i, v) in ol.iter().enumerate() { if v.abs() > 0.01 { last_loud = i; } }
    println!("arranged: last sample above 0.01 at {} (bar 1 starts at 88200)", last_loud);
}

#[test]
fn probe_scene_change_cuts_last_step() {
    // Pad plays a note on every step with gate 0.9. Scene a -> scene b (kick only).
    // The note at step 15 of the last bar of scene a should last 0.9 * 5512 samples.
    let src = r#"
tempo 120
scale A minor
module beats kit { kick_level 1.0 }
module keys pad { voice_mode poly attack 1ms release 5ms cutoff 5khz }
pattern beat { kick: X - - - X - - - X - - - X - - - }
pattern every { 1.3 1.3 1.3 1.3 1.3 1.3 1.3 1.3 1.3 1.3 1.3 1.3 1.3 1.3 1.3 1.3 }
track kick { play beat using kit level 0.0 out > master }
track pad  { play every using pad level 0.8 gate 0.9 out > master }
master { in > out }
scene a { track pad { play every using pad level 0.8 gate 0.9 } }
scene b { track kick { play beat using kit level 0.0 } }
arrange { a x1 b x1 }
"#;
    let mut e = SongEngine::from_source(src).unwrap();
    e.start();
    let (l, _) = e.render_steps(32);
    let step = 5512.5f32;
    for k in [13usize, 14, 15] {
        let s = (k as f32 * step) as usize;
        let seg = &l[s..s + step as usize];
        let mut last = 0;
        for (i, v) in seg.iter().enumerate() { if v.abs() > 0.005 { last = i; } }
        println!("step {}: note audible for {} of {} samples ({:.0}%)", k, last, step as usize, last as f32 / step * 100.0);
    }
}
