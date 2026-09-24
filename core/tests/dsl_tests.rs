use tatum_core::dsl;
use tatum_core::dsl::compiler;

const EXAMPLE_SONG: &str = r#"
# ── Globals ──
tempo 128
meter 4/4
scale A minor

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

pattern bassline_intro {
  A1 - - -
  A1 - - -
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
  track bass { play bassline_intro using bass }
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
fn test_parse_full_song() {
    let song = dsl::parse(EXAMPLE_SONG).expect("should parse without errors");

    // Globals
    assert_eq!(song.globals.tempo, 128.0);
    assert_eq!(song.globals.meter, (4, 4));
    assert!(song.globals.scale.is_some());
    let scale = song.globals.scale.as_ref().unwrap();
    assert_eq!(scale.root, "A");
    assert_eq!(scale.kind, "minor");

    // Buses
    assert_eq!(song.buses.len(), 3);
    assert_eq!(song.buses[0].name, "reverb");
    assert_eq!(song.buses[1].name, "delay");
    assert_eq!(song.buses[2].name, "drumBus");

    // Instruments
    assert_eq!(song.instruments.len(), 4);
    assert_eq!(song.instruments[0].name, "bass");
    assert_eq!(song.instruments[1].name, "kick");
    assert_eq!(song.instruments[2].name, "hat");
    assert_eq!(song.instruments[3].name, "pad");

    // Bass instrument should have nodes and connections
    let bass = &song.instruments[0];
    assert!(!bass.nodes.is_empty(), "bass should have nodes");
    assert!(!bass.connections.is_empty(), "bass should have connections");

    // Patterns
    assert_eq!(song.patterns.len(), 5);
    assert_eq!(song.patterns[0].name, "bassline");
    assert_eq!(song.patterns[1].name, "bassline_intro");
    assert_eq!(song.patterns[2].name, "kickp");
    assert_eq!(song.patterns[3].name, "hatp");
    assert_eq!(song.patterns[4].name, "padchord");

    // Bassline should have 2 rows with 4 steps each
    let bassline = &song.patterns[0];
    assert_eq!(bassline.rows.len(), 2);
    assert_eq!(bassline.rows[0].len(), 4); // A1 - A1 E1
    assert_eq!(bassline.rows[1].len(), 4); // A1 - C2 E1

    // Kickp should have drum hits and rests
    let kickp = &song.patterns[2];
    assert_eq!(kickp.rows.len(), 2);

    // Bus chains
    assert_eq!(song.bus_chains.len(), 3);
    assert_eq!(song.bus_chains[0].bus_name, "reverb");
    assert_eq!(song.bus_chains[1].bus_name, "delay");
    assert_eq!(song.bus_chains[2].bus_name, "drumBus");

    // Reverb chain should have a reverb node
    assert!(!song.bus_chains[0].chain.is_empty());
    assert_eq!(song.bus_chains[0].chain[0].kind, "reverb");

    // Tracks
    assert_eq!(song.tracks.len(), 4);
    assert_eq!(song.tracks[0].name, "bass");
    assert_eq!(song.tracks[0].play, "bassline");
    assert_eq!(song.tracks[0].using_instrument, "bass");
    assert_eq!(song.tracks[0].velocity, Some(0.8));
    // Bass track should have routing: drive(0.2) > master
    assert!(!song.tracks[0].routing.is_empty());

    assert_eq!(song.tracks[1].name, "kick");
    assert_eq!(song.tracks[1].play, "kickp");
    assert_eq!(song.tracks[1].using_instrument, "kick");

    // Master
    assert!(song.master.is_some());
    let master = song.master.as_ref().unwrap();
    assert!(master.chain.len() >= 2, "master should have eq + compressor + limiter");

    // Scenes
    assert_eq!(song.scenes.len(), 2);
    assert_eq!(song.scenes[0].name, "intro");
    assert_eq!(song.scenes[1].name, "verse");
    assert!(song.scenes[1].extends.is_some());
    assert_eq!(song.scenes[1].extends.as_ref().unwrap(), "intro");

    // Arrangement
    assert_eq!(song.arrangement.len(), 3);
    assert_eq!(song.arrangement[0].scene_name, "intro");
    assert_eq!(song.arrangement[0].repeat, 2);
    assert_eq!(song.arrangement[1].scene_name, "verse");
    assert_eq!(song.arrangement[1].repeat, 4);
    assert_eq!(song.arrangement[2].scene_name, "intro");
    assert_eq!(song.arrangement[2].repeat, 1);
}

#[test]
fn test_parse_minimal() {
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
    let song = dsl::parse(src).expect("minimal song should parse");
    assert_eq!(song.globals.tempo, 120.0);
    assert_eq!(song.instruments.len(), 1);
    assert_eq!(song.patterns.len(), 1);
    assert_eq!(song.tracks.len(), 1);
}

#[test]
fn test_parse_comments_and_whitespace() {
    let src = r#"
# This is a comment
tempo 140  # inline comment

instrument test {
  # node definitions
  osc saw(55) as osc1
  osc1 > adsr(0.01, 0.1, 0.5, 0.1) as env
  env > out
}
"#;
    let song = dsl::parse(src).expect("comments should be ignored");
    assert_eq!(song.globals.tempo, 140.0);
    assert_eq!(song.instruments.len(), 1);
}

// ── Compiler tests ──

#[test]
fn test_compile_full_song() {
    let song = dsl::parse(EXAMPLE_SONG).expect("parse should succeed");
    let compiled = compiler::compile(&song).expect("compile should succeed");

    // Globals preserved
    assert_eq!(compiled.globals.tempo, 128.0);

    // 4 instruments compiled into graph templates
    assert_eq!(compiled.instruments.len(), 4);
    assert_eq!(compiled.instrument_names[0], "bass");
    assert_eq!(compiled.instrument_names[1], "kick");
    assert_eq!(compiled.instrument_names[2], "hat");
    assert_eq!(compiled.instrument_names[3], "pad");

    // Each graph instrument template has valid node count and execution order
    for (i, kind) in compiled.instruments.iter().enumerate() {
        if let Some(template) = kind.as_graph() {
            assert!(template.node_count > 0,
                "instrument {} should have nodes", compiled.instrument_names[i]);
            assert_eq!(template.exec_len, template.node_count,
                "instrument {} execution order should cover all nodes", compiled.instrument_names[i]);
        }
    }

    // Bass should have osc + env nodes
    let bass = compiled.instruments[0].as_graph().expect("bass should be graph");
    assert!(bass.osc_count > 0, "bass should have oscillators");
    assert!(bass.env_count > 0, "bass should have envelopes");

    // Kick should have osc + env nodes
    let kick = compiled.instruments[1].as_graph().expect("kick should be graph");
    assert!(kick.osc_count > 0, "kick should have oscillators");
    assert!(kick.env_count > 0, "kick should have envelopes");

    // 5 patterns compiled
    assert_eq!(compiled.patterns.len(), 5);
    assert_eq!(compiled.patterns[0].name, "bassline");
    assert_eq!(compiled.patterns[0].steps.len(), 8); // 2 rows x 4 steps

    // Check MIDI note conversion: A1 = 33
    match compiled.patterns[0].steps[0] {
        compiler::CompiledStep::NoteOn { midi_note, .. } => assert_eq!(midi_note, 33),
        _ => panic!("first step should be a note"),
    }

    // 4 tracks
    assert_eq!(compiled.tracks.len(), 4);
    assert_eq!(compiled.tracks[0].velocity, 0.8);

    // 3 buses
    assert_eq!(compiled.buses.len(), 3);
    assert!(!compiled.buses[0].fx_chain.is_empty(), "reverb bus should have FX");

    // Master chain
    assert!(!compiled.master.fx_chain.is_empty(), "master should have FX chain");

    // 2 scenes
    assert_eq!(compiled.scenes.len(), 2);
    assert_eq!(compiled.scenes[0].name, "intro");
    assert_eq!(compiled.scenes[1].name, "verse");

    // Arrangement: 3 entries
    assert_eq!(compiled.arrangement.len(), 3);
    assert_eq!(compiled.arrangement[0], (0, 2)); // intro x2
    assert_eq!(compiled.arrangement[1], (1, 4)); // verse x4
    assert_eq!(compiled.arrangement[2], (0, 1)); // intro x1
}

#[test]
fn test_compile_note_names() {
    let src = r#"
instrument test {
  osc sine(440) as osc1
  osc1 > adsr(0.01, 0.1, 0.5, 0.1) as env
  env > out
}
pattern notes {
  C4 D4 E4 F4
  G4 A4 B4 C5
}
track t { play notes using test }
"#;
    let song = dsl::parse(src).expect("parse");
    let compiled = compiler::compile(&song).expect("compile");

    let steps = &compiled.patterns[0].steps;
    let midi_notes: Vec<u8> = steps.iter().filter_map(|s| match s {
        compiler::CompiledStep::NoteOn { midi_note, .. } => Some(*midi_note),
        _ => None,
    }).collect();

    assert_eq!(midi_notes, vec![60, 62, 64, 65, 67, 69, 71, 72]);
}

#[test]
fn test_compile_instrument_graph_structure() {
    let src = r#"
instrument simple {
  osc saw(55) as osc1
  osc1 > lowpass(900) as filt
  filt > adsr(0.01, 0.2, 0.7, 0.3) as env
  env > out
}
pattern p { C4 }
track t { play p using simple }
"#;
    let song = dsl::parse(src).expect("parse");
    let compiled = compiler::compile(&song).expect("compile");

    let template = compiled.instruments[0].as_graph().expect("should be graph");
    // Should have: mix (implicit? no), osc, lowpass, adsr/env+vca?, output
    // Actually: osc, lowpass, env, out = 4 nodes minimum
    assert!(template.node_count >= 4, "simple instrument should have at least 4 nodes, got {}", template.node_count);
    assert_eq!(template.osc_count, 1, "should have 1 oscillator");
    assert_eq!(template.env_count, 1, "should have 1 envelope");
}

/// A drum lane written across several lines used to end at the first newline,
/// which turned the continuation into an unlabelled row that the compiler then
/// dropped. The pattern compiled clean and played half of what was written --
/// the same silent-wrong-output shape as the routing chains that used to
/// truncate at a newline.
#[test]
fn a_drum_lane_can_span_several_lines() {
    let src = "tempo 120\nscale C major\n\
        module beats kit { kick_level 1.0 }\n\
        pattern wide {\n\
        \x20   kick: X - - -  - - - -  - - - -  - - - -\n\
        \x20         - - - -  - - - -  X - - -  - - - -\n\
        \x20   hat:  x - x -  x - x -  x - x -  x - x -\n\
        \x20         x - x -  x - x -  x - x -  x - x -\n\
        }\n\
        track d { play wide using kit out > master }\n\
        master { in > out }\n\
        scene a { track d { play wide using kit } }\narrange { a x2 }\n";
    let ast = dsl::parse(src).unwrap_or_else(|e| panic!("{:?}", e));
    let p = ast.patterns.iter().find(|p| p.name == "wide").unwrap();
    assert_eq!(p.lane_labels, vec!["kick", "hat"], "two lanes, not four rows");
    assert_eq!(p.rows.len(), 2, "each lane is one row: {:?}", p.rows.iter().map(|r| r.len()).collect::<Vec<_>>());
    assert_eq!(p.rows[0].len(), 32, "the kick lane keeps both of its bars");
    assert_eq!(p.rows[1].len(), 32, "and so does the hat lane");
}

/// And if the counts ever disagree again it says so rather than dropping rows.
#[test]
fn a_drum_row_without_a_label_is_an_error() {
    let src = "tempo 120\nscale C major\n\
        module beats kit { kick_level 1.0 }\n\
        pattern bad {\n\
        \x20   kick: X - - -\n\
        \x20   X - - -\n\
        \x20   hat: x - x -\n\
        }\n\
        track d { play bad using kit out > master }\n\
        scene a { track d { play bad using kit } }\narrange { a x1 }\n";
    // Either the continuation joins the kick lane, or it is reported. What it
    // must never do is vanish.
    match dsl::parse(src) {
        Ok(ast) => {
            let p = ast.patterns.iter().find(|p| p.name == "bad").unwrap();
            let steps: usize = p.rows.iter().map(|r| r.len()).sum();
            assert_eq!(steps, 12, "every written step survives: {:?}", p.rows.iter().map(|r| r.len()).collect::<Vec<_>>());
        }
        Err(errs) => assert!(errs[0].message.contains("lane labels"), "{}", errs[0].message),
    }
}
