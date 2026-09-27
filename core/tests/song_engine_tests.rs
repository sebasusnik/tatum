mod test_helpers;

use tatum_core::song_engine::SongEngine;

const EXAMPLE_SONG: &str = r#"
# ── Globals ──
tempo 128
meter 4/4

# ── Buses ──
bus reverb
bus delay
bus drumBus

# ── Instruments ──
instrument bass {
  osc saw(55) as osc1
  osc square(110) as osc2
  osc1 > mix
  osc2 > mix
  mix > ladder(900, 0.6) as filter
  filter > adsr(0.01, 0.2, 0.7, 0.3) as amp
  amp > out
}

instrument kick {
  osc sine(50) as body
  body > perc(0.001, 0.3) as bodyEnv
  bodyEnv > saturate(0.5) as bodyDist
  noise() as click
  click > highpass(5000) as clickFilt
  clickFilt > perc(0.001, 0.02) as clickEnv
  bodyDist > mix
  clickEnv > mix
  mix > out
}

instrument hat {
  noise() as src
  src > highpass(8000) as filt
  filt > perc(0.001, 0.05) as env
  env > out
}

instrument pad {
  osc saw(220) as osc1
  osc square(221) as osc2
  osc1 > mix
  osc2 > mix
  mix > lowpass(2000, 0.3) as filt
  filt > adsr(0.5, 0.3, 0.8, 1.0) as amp
  amp > out
}

# ── Patterns ──
pattern bassline {
  A1 - A1 E1
  A1 - C2 E1
}

pattern kickp {
  x - - x
  x - - x
}

pattern hatp {
  - x - x
  - x - x
}

pattern padchord {
  C4 - - -
  E4 - - -
}

# ── Bus FX chains ──
reverb {
  in > reverb(0.6) > out
}

delay {
  in > delay(1/4, 0.4) > out
}

drumBus {
  in > compressor(-6) > saturate(0.2) > master
}

# ── Tracks ──
track bass {
  play bassline
  using bass
  velocity 0.8
  out > drive(0.2) > master
}

track kick {
  play kickp
  using kick
  out > drumBus
}

track hats {
  play hatp
  using hat
  out > drumBus
}

track pads {
  play padchord
  using pad
  velocity 0.5
  out > master
}

# ── Master ──
master {
  in > eq(low=0.1, mid=-0.1) > compressor(-8) > limiter > out
}

# ── Scenes ──
scene intro {
  tempo 128
  track bass { play bassline using bass }
  track kick { play kickp using kick }
}

scene verse {
  extends intro
  track bass { play bassline using bass }
  track hats { play hatp using hat }
  track pads { play padchord using pad }
}

# ── Arrangement ──
arrange {
  intro x2
  verse x4
  intro x1
}
"#;

#[test]
fn test_song_engine_load() {
    let engine = SongEngine::from_source(EXAMPLE_SONG).expect("should load song");
    assert!((engine.tempo() - 128.0).abs() < 0.01);
    assert_eq!(engine.arrangement_bars(), 7); // 2 + 4 + 1
}

#[test]
fn test_song_engine_render_bars() {
    let mut engine = SongEngine::from_source(EXAMPLE_SONG).expect("should load song");

    // Render 2 bars
    let (out_l, out_r) = engine.render(2);

    // At 128 BPM, each bar = 4 beats = 16 sixteenth notes
    // samples_per_step = 44100 * 60 / 128 / 4 ≈ 5168
    // 2 bars = 32 steps ≈ 165375 samples
    assert!(out_l.len() > 100000, "should render substantial audio, got {} samples", out_l.len());
    assert_eq!(out_l.len(), out_r.len());

    // Should have non-zero audio
    let max_l = out_l.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
    assert!(max_l > 0.01, "output should have audible signal, max={}", max_l);

    // Write WAV for manual listening
    test_helpers::write_wav_stereo(&test_helpers::output_path("test_song_engine_2bars.wav"), &out_l, &out_r, 44100);
}

#[test]
fn test_song_engine_full_arrangement() {
    let mut engine = SongEngine::from_source(EXAMPLE_SONG).expect("should load song");

    let total_bars = engine.arrangement_bars();
    let (out_l, out_r) = engine.render(total_bars);

    assert!(out_l.len() > 200000, "full arrangement should be substantial, got {} samples", out_l.len());

    let max_l = out_l.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
    assert!(max_l > 0.01, "output should have audible signal");

    test_helpers::write_wav_stereo(&test_helpers::output_path("test_song_engine_full.wav"), &out_l, &out_r, 44100);
}

#[test]
fn test_song_engine_minimal() {
    let src = r#"
tempo 120
instrument sine {
  osc sine(440) as osc1
  osc1 > adsr(0.01, 0.1, 0.5, 0.3) as env
  env > out
}
pattern melody {
  C4 E4 G4 C5
}
track lead {
  play melody
  using sine
}
"#;

    let mut engine = SongEngine::from_source(src).expect("should load minimal song");

    // Render 1 bar
    let (out_l, out_r) = engine.render(1);

    assert!(out_l.len() > 10000, "should render audio");
    let max = out_l.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
    assert!(max > 0.01, "should have audible output, max={}", max);

    test_helpers::write_wav_stereo(&test_helpers::output_path("test_song_engine_minimal.wav"), &out_l, &out_r, 44100);
}

#[test]
#[ignore = "a whole song, rendered to listen to: cargo test --release -- --ignored"]
fn test_funk_replica_render() {
    const FUNK_SOURCE: &str = include_str!("../../examples/funk_replica.synth");
    let mut engine = SongEngine::from_source(FUNK_SOURCE).expect("should load funk_replica");

    let total_bars = engine.arrangement_bars();
    let (out_l, out_r) = engine.render(total_bars);

    let max_val = out_l.iter().chain(out_r.iter()).fold(0.0f32, |a, &b| a.max(b.abs()));
    assert!(max_val > 0.01, "funk_replica should produce audible output, max={}", max_val);

    test_helpers::write_wav_stereo(&test_helpers::output_path("funk_dsl_replica.wav"), &out_l, &out_r, 44100);
}

#[test]
#[ignore = "a whole song, rendered to listen to: cargo test --release -- --ignored"]
fn test_deephouse_render() {
    const SOURCE: &str = include_str!("../../examples/deephouse.synth");
    let mut engine = SongEngine::from_source(SOURCE).expect("should load deephouse");

    let total_bars = engine.arrangement_bars();
    let (out_l, out_r) = engine.render(total_bars);

    let max_val = out_l.iter().chain(out_r.iter()).fold(0.0f32, |a, &b| a.max(b.abs()));
    assert!(max_val > 0.01, "deephouse should produce audible output, max={}", max_val);

    test_helpers::write_wav_stereo(&test_helpers::output_path("test_deephouse.wav"), &out_l, &out_r, 44100);
}

/// Minimal bass-only test to isolate bass pattern behavior
#[test]
fn test_bass_only_dsl() {
    const BASS_ONLY: &str = r#"
tempo 112
meter 4/4

module bass funk_bass {
    cutoff 0.08
    cutoff_env 0.7
    resonance 0.75
    glide 0.15
    keytrack 0.5
    osc2_pitch 0.25
    osc1_wave saw
    attack 0.0
}

pattern funk_bass {
    A0:0.9(edepth=0.75, res=0.80)   -  -  A0:0.55(edepth=0.50, res=0.65)
    -  G0:0.7(edepth=0.65, res=0.75)   -  -
    A0:0.8(edepth=0.70, res=0.78)  -  A0:0.4(edepth=0.40, res=0.55)   -
    C1:0.5(edepth=0.55, res=0.70)  -  A0:0.65(edepth=0.60, res=0.72)  -
}

track bass {
    play funk_bass
    using funk_bass
    velocity 1.0
    level 0.5
    gate 1.0
    pan 0.0
    out > master
}

master {
    in > out
}

scene main {
    track bass { play funk_bass using funk_bass velocity 1.0 level 0.5 gate 1.0 out > master }
}

arrange {
    main x4
}
"#;

    let mut engine = SongEngine::from_source(BASS_ONLY).expect("should load bass-only");
    let (out_l, out_r) = engine.render(4);

    let max_val = out_l.iter().chain(out_r.iter()).fold(0.0f32, |a, &b| a.max(b.abs()));
    eprintln!("bass-only: max={:.4}, samples={}", max_val, out_l.len());

    // Print peak at each bar
    let samples_per_bar = out_l.len() / 4;
    for bar in 0..4 {
        let start = bar * samples_per_bar;
        let end = start + samples_per_bar;
        let bar_max = out_l[start..end].iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        eprintln!("  bar {}: peak={:.4}", bar, bar_max);
    }

    assert!(max_val > 0.01, "bass-only should produce audible output, max={}", max_val);

    test_helpers::write_wav_stereo(&test_helpers::output_path("test_bass_only_dsl_debug.wav"), &out_l, &out_r, 44100);
}

#[test]
fn test_pad_debug() {
    let src = r#"
tempo 112
meter 4/4

module keys pad {
    voice_mode 0.0
    cutoff 0.18
    detune 0.08
    chorus_mix 0.25
    attack 0.6
    decay 0.4
    sustain 0.75
    release 0.8
}

pattern pad_am {
    [A3 C4 E4]:0.5 .. .. ..
    ..              .. .. ..
    ..              .. .. ..
    ..              .. .. ..
}

track pad { play pad_am using pad velocity 1.0 level 0.5 out > master }

master { in > out }

scene main {
    track pad { play pad_am using pad velocity 1.0 level 0.5 out > master }
}
arrange { main x2 }
"#;

    let mut engine = SongEngine::from_source(src).expect("load pad test");
    let (out_l, out_r) = engine.render(2);

    let max = out_l.iter().chain(out_r.iter()).fold(0.0f32, |a, &b| a.max(b.abs()));
    let samples_per_bar = out_l.len() / 2;
    eprintln!("pad test: total={}, max={:.4}", out_l.len(), max);
    for bar in 0..2 {
        let s = bar * samples_per_bar;
        let e = s + samples_per_bar;
        let peak = out_l[s..e].iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        let rms = (out_l[s..e].iter().map(|x| x * x).sum::<f32>() / (e - s) as f32).sqrt();
        eprintln!("  bar {}: peak={:.4} rms={:.6}", bar, peak, rms);
    }

    // Verify sound is continuous (no silent gaps from failed ties)
    let total_len = out_l.len();
    let quarter = total_len / 4;
    for i in 0..4 {
        let s = i * quarter + quarter / 2; // midpoint of each quarter
        let e = s + 2000;
        let rms = (out_l[s..e.min(total_len)].iter().map(|x| x * x).sum::<f32>() / 2000.0).sqrt();
        eprintln!("  quarter {}: rms={:.6}", i, rms);
        assert!(rms > 0.01, "pad should sustain through quarter {}, rms={}", i, rms);
    }

    assert!(max > 0.01, "pad should produce sound, max={}", max);

    test_helpers::write_wav_stereo(&test_helpers::output_path("test_pad_debug.wav"), &out_l, &out_r, 44100);
}
