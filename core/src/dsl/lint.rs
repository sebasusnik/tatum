//! Design lints: a song can compile and still sound flat. These checks run on
//! the AST after a successful compile and return warnings, never errors.
//! They exist so an AI author gets "this pad never moves" as feedback instead
//! of only "this parses".

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;

use super::ast::{Song, Step, TrackDef, ModuleDef, NoteRef};

/// One design warning with a stable code, what was found and what to do.
#[derive(Debug, Clone, PartialEq)]
pub struct Lint {
    pub code: &'static str,
    pub message: String,
    pub hint: String,
}

fn lint(code: &'static str, message: String, hint: &str) -> Lint {
    Lint { code, message, hint: String::from(hint) }
}

/// Longest run of ties after a note or chord, in steps.
fn longest_hold(song: &Song, pattern: &str) -> usize {
    let Some(pat) = song.patterns.iter().find(|p| p.name == pattern) else { return 0 };
    if !pat.lane_labels.is_empty() {
        return 0;
    }
    let mut best = 0;
    let mut run = 0;
    let mut holding = false;
    for step in pat.rows.iter().flatten() {
        match step {
            Step::Note(_) | Step::Chord(_) => {
                holding = true;
                run = 0;
            }
            Step::Tie if holding => {
                run += 1;
                best = best.max(run);
            }
            _ => {
                holding = false;
                run = 0;
            }
        }
    }
    best
}

fn param(m: &ModuleDef, name: &str) -> Option<f32> {
    m.params.iter().find(|p| p.name == name).map(|p| p.value)
}

/// Intrinsic movement: LFO, vibrato, or an arp on the top-level track.
fn has_intrinsic_modulation(m: &ModuleDef, track: &TrackDef) -> bool {
    param(m, "lfo_depth").unwrap_or(0.0) > 0.0
        || param(m, "vibrato_depth").unwrap_or(0.0) > 0.0
        || track.arp.as_ref().is_some_and(|a| a.mode != "off")
}

/// Scenes in which the track holds long notes with nothing moving the module.
fn static_scenes(song: &Song, m: &ModuleDef, track: &TrackDef) -> Vec<String> {
    let prefix = format!("{}.", m.name);
    let mut out = Vec::new();
    for scene in &song.scenes {
        let Some(st) = scene.tracks.iter().find(|t| t.name == track.name) else { continue };
        if longest_hold(song, &st.play) < 8 {
            continue;
        }
        let arped = st.arp.as_ref().is_some_and(|a| a.mode != "off");
        let automated =
            scene.automations.iter().any(|a| a.target.starts_with(&prefix) && !a.target.ends_with(".level"));
        if !arped && !automated {
            out.push(scene.name.clone());
        }
    }
    // No scenes at all: judge the top-level track
    if song.scenes.is_empty() && longest_hold(song, &track.play) >= 8 {
        out.push(String::from("(top level)"));
    }
    out
}

/// The lowest and highest fundamentals a pattern asks for, in Hz, with the
/// name of the top note. Scale degrees are skipped: without the key they are
/// not a pitch, and the notes that get a chorus into trouble are absolute.
///
/// Both ends matter. A chorus only reads as roughness on a voice that is
/// ENTIRELY up high, like a bell: when the same high note is the top of a
/// chord whose root is two octaves down, the beat is buried under everything
/// below it, and warning about it would just be noise.
fn note_span_hz(song: &Song, pattern: &str) -> Option<(f32, f32, String)> {
    // Scale degrees resolve too. Skipping them left a real hole: the repo's
    // own arpeggiator example writes its chord as `[1.4 3.4 5.4 7.4]`, which
    // in C minor is C5 Eb5 G5 Bb5 through a 40% chorus -- exactly this bug,
    // and the lint walked straight past it.
    let (intervals, root) = super::compiler::scale_context(song);
    let pat = song.patterns.iter().find(|p| p.name == pattern)?;
    if !pat.lane_labels.is_empty() {
        return None;
    }
    let mut hi: Option<(u8, String)> = None;
    let mut lo: Option<u8> = None;
    let mut consider = |n: &NoteRef| {
        if matches!(n, NoteRef::Absolute(_) | NoteRef::Degree(..) | NoteRef::Midi(_)) {
            let midi = super::compiler::resolve_note(n, &intervals, root);
            let name = match n {
                NoteRef::Absolute(s) => s.clone(),
                _ => midi_name(midi),
            };
            if hi.as_ref().is_none_or(|(m, _)| midi > *m) {
                hi = Some((midi, name));
            }
            if lo.is_none_or(|m| midi < m) {
                lo = Some(midi);
            }
        }
    };
    for step in pat.rows.iter().flatten() {
        match step {
            Step::Note(ns) => consider(&ns.note),
            Step::Chord(cs) => cs.notes.iter().for_each(|ns| consider(&ns.note)),
            Step::Subdiv(subs) => subs.iter().for_each(|ns| consider(&ns.note)),
            _ => {}
        }
    }
    let (himidi, name) = hi?;
    let hz = |m: u8| 440.0 * libm_pow2((m as f32 - 69.0) / 12.0);
    Some((hz(lo?), hz(himidi), name))
}

fn libm_pow2(x: f32) -> f32 {
    crate::math::pow2(x)
}

/// A MIDI number as a name, so a warning about a scale degree still says
/// which pitch it is worried about.
fn midi_name(m: u8) -> String {
    const N: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    format!("{}{}", N[(m % 12) as usize], m as i16 / 12 - 1)
}

/// How much chorus reaches a track: the module's own, plus any `chorus()` in
/// its chain. `keys` in a non-poly voice mode is exempt — its chorus is inert.
fn chorus_amount(song: &Song, track: &TrackDef, m: &ModuleDef) -> f32 {
    let mode = m.params.iter().find(|p| p.name == "voice_mode");
    let inert = m.module_type == "keys" && mode.is_some_and(|p| p.value > 0.01);
    let own = if inert { 0.0 } else { param(m, "chorus_mix").unwrap_or(0.0) };
    let in_chain = song
        .tracks
        .iter()
        .find(|t| t.name == track.name)
        .into_iter()
        .flat_map(|t| t.routing.iter())
        .filter(|r| r.kind == "chorus")
        .filter_map(|r| match r.params.first() {
            Some(super::ast::Param::Float(v)) => Some(*v),
            Some(super::ast::Param::Named(n, v)) if n == "mix" => Some(*v),
            _ => None,
        })
        .fold(0.0f32, f32::max);
    own.max(in_chain)
}

/// A named argument of a chain node, after the parser has resolved its unit:
/// `makeup=6db` reads back as the linear 1.995 it compiles to, and
/// `eq(low=3.5)` as 3.5 dB, because that is what each argument is declared in.
fn chain_arg(params: &[super::ast::Param], name: &str) -> Option<f32> {
    params.iter().find_map(|p| match p {
        super::ast::Param::Named(n, v) if n == name => Some(*v),
        _ => None,
    })
}

/// Run every lint on a song that already compiled.
pub fn lint_song(song: &Song) -> Vec<Lint> {
    let mut out = Vec::new();

    // Tracks as they appear anywhere (top level and scenes), keyed by name.
    let all_tracks: Vec<&TrackDef> =
        song.tracks.iter().chain(song.scenes.iter().flat_map(|s| s.tracks.iter())).collect();

    // ── static_pad: sustained keys/fm notes with nothing moving them ──
    for track in &song.tracks {
        let Some(m) = song.module_defs.iter().find(|m| m.name == track.using_instrument) else { continue };
        if m.module_type != "keys" && m.module_type != "fm" {
            continue;
        }
        if has_intrinsic_modulation(m, track) {
            continue;
        }
        let scenes = static_scenes(song, m, track);
        if !scenes.is_empty() {
            out.push(lint(
                "static_pad",
                format!("track '{}' holds long notes on module '{}' with nothing moving it in: {}", track.name, m.name, scenes.join(", ")),
                "Sustained sounds need movement: add `lfo_target cutoff` with `lfo_depth`, `vibrato_depth`, a per-scene `auto <module> cutoff a > b`, or an `arp`.",
            ));
        }
    }

    // ── chorus_beats: a chorus on a high note is roughness, not width ──
    //
    // This one came from a real session: 18% chorus on an FM bell playing C#6
    // was heard as distortion on the first note and took a long search to
    // pin down, because averaged over the whole song the track measured
    // perfectly clean. The rule is arithmetic, so it costs nothing to check.
    let detune = crate::effects::chorus::max_detune_ratio();
    let floor_hz = crate::effects::chorus::roughness_above_hz();
    for track in &song.tracks {
        let Some(m) = song.module_defs.iter().find(|m| m.name == track.using_instrument) else { continue };
        let mix = chorus_amount(song, track, m);
        if mix < 0.08 {
            continue;
        }
        // every pattern this track plays, top level and in scenes
        let played = core::iter::once(track.play.clone()).chain(
            song.scenes.iter().filter_map(|sc| sc.tracks.iter().find(|t| t.name == track.name).map(|t| t.play.clone())),
        );
        let mut worst: Option<(f32, f32, String)> = None;
        for pat in played {
            if let Some((lo, hi, name)) = note_span_hz(song, &pat) {
                if worst.as_ref().is_none_or(|(_, h, _)| hi > *h) {
                    worst = Some((lo, hi, name));
                }
            }
        }
        let Some((lo_hz, mut hz, mut name)) = worst else { continue };
        // An arp lifts the held notes by `octaves - 1` and plays them one at
        // a time. Both halves of that matter: the top note is higher than the
        // pattern says, and because nothing sounds underneath it, the
        // chord-top exemption below does not apply -- there is no chord.
        let arp =
            song.tracks.iter().find(|t| t.name == track.name).and_then(|t| t.arp.as_ref()).filter(|a| a.mode != "off");
        if let Some(a) = arp {
            let oct = a.octaves.unwrap_or(1.0).max(1.0) as i32 - 1;
            if oct > 0 {
                hz *= libm_pow2(oct as f32);
                name = format!("{} lifted {} octave(s) by its arp", name, oct);
            }
        }
        if hz <= floor_hz {
            continue;
        }
        // A high note on top of a low chord is buried by the notes under it.
        // Only exempt it when the notes really do sound together.
        if arp.is_none() && lo_hz <= floor_hz {
            continue;
        }
        out.push(lint(
            "chorus_beats",
            format!(
                "track '{}' plays {} ({:.0} Hz) through chorus_mix {:.0}% on module '{}': \
                 the chorus detunes its copy by {:.1}%, so it beats against the dry signal at {:.0} Hz",
                track.name,
                name,
                hz,
                mix * 100.0,
                m.name,
                detune * 100.0,
                hz * detune
            ),
            "Above ~20 Hz a beat stops sounding like two tones and starts sounding like roughness, \
             which on a clean tone reads as distortion. Drop chorus_mix on this voice and get its \
             width from `pan`, `autopan` or the sends, none of which move the pitch.",
        ));
    }

    // ── dry_mix: nothing goes to the sends or a bus ──
    let any_send = all_tracks.iter().any(|t| {
        t.delay_send.unwrap_or(0.0) > 0.0
            || t.reverb_send.unwrap_or(0.0) > 0.0
            || t.routing.iter().any(|r| song.buses.iter().any(|b| b.name == r.kind))
    });
    if !any_send && !song.tracks.is_empty() {
        out.push(lint(
            "dry_mix",
            String::from("no track uses reverb_send, delay_send or a bus"),
            "Give pads and stabs some space: `reverb_send 0.3`, `delay_send 0.2`, or route into a bus with its own chain.",
        ));
    }

    // ── no_sidechain: drums plus a bass or pad, but nothing ducks ──
    let has_drums = song
        .tracks
        .iter()
        .any(|t| song.module_defs.iter().any(|m| m.name == t.using_instrument && m.module_type == "beats"));
    let has_tonal = song
        .tracks
        .iter()
        .any(|t| song.module_defs.iter().any(|m| m.name == t.using_instrument && m.module_type != "beats"));
    let any_sidechain = song.globals.sidechain > 0.0 || all_tracks.iter().any(|t| t.sidechain.unwrap_or(0.0) > 0.0);
    if has_drums && has_tonal && !any_sidechain {
        out.push(lint(
            "no_sidechain",
            String::from("drums and tonal tracks with no sidechain"),
            "Let the kick breathe through the mix: global `sidechain 0.4`, or per track `sidechain 0.5` on bass and pads.",
        ));
    }

    // ── sidechain_without_kick: ducking set, but this scene has no beats track ──
    let is_beats = |name: &str| song.module_defs.iter().any(|m| m.name == name && m.module_type == "beats");
    for scene in &song.scenes {
        if scene.tracks.iter().any(|t| is_beats(&t.using_instrument)) {
            continue;
        }
        let ducked: Vec<&str> = scene
            .tracks
            .iter()
            .filter(|t| {
                let global = song.tracks.iter().find(|g| g.name == t.name);
                let amount = t.sidechain.or(global.and_then(|g| g.sidechain)).unwrap_or(song.globals.sidechain);
                amount > 0.0
            })
            .map(|t| t.name.as_str())
            .collect();
        if !ducked.is_empty() {
            out.push(lint(
                "sidechain_without_kick",
                format!("scene '{}' has no drums, so sidechain does nothing there for: {}", scene.name, ducked.join(", ")),
                "Those tracks play at full, unducked level in this scene. Lower their level or velocity here so a breakdown does not end up louder than the drop.",
            ));
        }
    }

    // ── single_scene: long song with no arrangement shape ──
    let total_bars: u32 = song.arrangement.iter().map(|a| a.repeat).sum();
    if song.scenes.len() <= 1 && total_bars > 8 {
        out.push(lint(
            "single_scene",
            format!("{} bars in a single scene", total_bars),
            "Shape the energy with scenes: intro with fewer tracks, a build with an `auto` sweep, a drop with everything, a breakdown without kick.",
        ));
    }

    // ── master_limiter: a limiter the engine leaves out ──
    let has_limiter = song.master.as_ref().is_some_and(|m| m.chain.iter().any(|n| n.kind == "limiter"));
    if has_limiter {
        out.push(lint(
            "master_limiter",
            String::from("the master chain has a limiter, which is left out"),
            "Every song ends in the engine's own limiter, after the gain that brings it to the same loudness as every other song, and it keeps the true peak under -1 dB. Take `limiter` off the master; a compressor or saturation there still shapes the sound.",
        ));
    }

    // ── bare_number: a knob position where a quantity was meant ──
    let mut bare = Vec::new();
    for m in &song.module_defs {
        let Some(kind) = crate::params::ModuleKind::from_str(&m.module_type) else { continue };
        for p in m.params.iter().filter(|p| p.bare) {
            let Some(spec) = crate::params::lookup(kind, &p.name) else { continue };
            let Some(units) = spec.write_in_units(p.value) else { continue };
            bare.push(format!(
                "`{} {}` is `{} {}` (module '{}', line {})",
                p.name, p.value, p.name, units, m.name, p.line
            ));
        }
    }
    if !bare.is_empty() {
        let more = if bare.len() > 3 { format!(", and {} more", bare.len() - 3) } else { String::new() };
        out.push(lint(
            "bare_number",
            format!("{} parameters are written as knob positions: {}{}", bare.len(), bare[..bare.len().min(3)].join("; "), more),
            "A plain number on a parameter with a unit is where the knob sits, not what it does: `cutoff 0.1` says nothing a reader can hear. `tatum fmt --units <file>` rewrites every one of them in its unit, and the song sounds the same.",
        ));
    }

    // ── gain_comp_ignored: a setting the engine dropped ──
    if song.globals.gain_comp.is_some() {
        out.push(lint(
            "gain_comp_ignored",
            String::from("`gain_comp` does nothing any more"),
            "The engine no longer scales each scene by how many tracks it has: the level you write is the level you hear, in every scene, and the whole song is levelled at the end. Delete the line.",
        ));
    }

    // ── level_used_as_fader: a sustained element ridden up and down per scene ──
    //
    // A continuous drone or pad is a bed, not a fader. Moving its level between
    // scenes to hit a loudness target is audible as the bed changing volume
    // under everything else, which is exactly the complaint this came from.
    for track in &song.tracks {
        let Some(m) = song.module_defs.iter().find(|m| m.name == track.using_instrument) else { continue };
        if m.module_type == "beats" {
            continue;
        }
        let mut levels: Vec<(String, f32)> = Vec::new();
        for scene in &song.scenes {
            let Some(st) = scene.tracks.iter().find(|t| t.name == track.name) else { continue };
            // Only sustained material: a staccato part may well be a fader.
            if longest_hold(song, &st.play) < 8 {
                continue;
            }
            let level = st.level.or(track.level).unwrap_or(0.8);
            levels.push((scene.name.clone(), level));
        }
        if levels.len() < 2 {
            continue;
        }
        let lo = levels.iter().fold(f32::MAX, |a, (_, v)| a.min(*v));
        let hi = levels.iter().fold(0.0f32, |a, (_, v)| a.max(*v));
        if lo <= 0.0 {
            continue;
        }
        // 3 dB is where a level move stops reading as arrangement and starts
        // reading as someone touching the fader.
        let spread_db = 20.0 * crate::math::log10(hi / lo);
        if spread_db > 3.0 {
            out.push(lint(
                "level_used_as_fader",
                format!(
                    "track '{}' holds long notes and its level moves {:.1} dB across scenes ({:.2} to {:.2})",
                    track.name, spread_db, lo, hi
                ),
                "A sustained bed should keep one level. Change what plays over it instead, or move it with a filter or a send so the change reads as texture rather than volume.",
            ));
        }
    }

    // ── chord_into_mono_voice: a chord played by a voice mode that cannot hold it ──
    //
    // `unison` stacks all eight voices on one note, which is what the hardware
    // it copies does, so a five-note chord arrives as five note-ons and only the
    // last survives. Both the docs and the MCP instructions used to recommend
    // unison for pads, and an agent spent two thirds of a session wondering why
    // its harmony was thin. Rendering two different chords through it gives
    // bit-identical output.
    //
    // `bass` is the other half of this and was never checked: it is one voice
    // by construction, so a chord reaching it loses every note but the last
    // the same way. Both are exempt when the track has an `arp`, because
    // there the chord is not meant to sound at once -- it is the note set the
    // arpeggiator walks, and one voice is the right shape for it.
    // A track that appears in eight scenes is one track; `all_tracks` carries
    // a copy per scene and this used to print the same line eight times.
    let mut reported: Vec<&str> = Vec::new();
    for track in &all_tracks {
        if reported.contains(&track.name.as_str()) {
            continue;
        }
        let Some(m) = song.module_defs.iter().find(|m| m.name == track.using_instrument) else { continue };
        let why = match m.module_type.as_str() {
            // poly is index 0 of VOICE_MODES; every other mode collapses voices.
            "keys" if param(m, "voice_mode").unwrap_or(0.0) >= 0.01 => "whose voice_mode is not poly",
            "bass" => "which is a single voice",
            _ => continue,
        };
        if track.arp.as_ref().is_some_and(|a| a.mode != "off") {
            continue;
        }
        let Some(pat) = song.patterns.iter().find(|p| p.name == track.play) else { continue };
        let widest = pat
            .rows
            .iter()
            .flatten()
            .filter_map(|s| match s {
                Step::Chord(c) => Some(c.notes.len()),
                _ => None,
            })
            .max()
            .unwrap_or(0);
        if widest > 1 {
            reported.push(&track.name);
            out.push(lint(
                "chord_into_mono_voice",
                format!(
                    "track '{}' plays a {}-note chord through module '{}', {}",
                    track.name, widest, m.name, why
                ),
                "Only the last note of the chord sounds: unison, octave, fifth and ringmod stack their voices on one note, and `bass` has one to begin with. Use a `keys` module with `voice_mode poly`, or an `fm` module, which is eight-voice. Add `arp up` if you meant the chord as material for an arpeggio.",
            ));
        }
    }

    // ── plock_ignored: filter locks on a module that drops them ──
    //
    // `(cutoff=, edepth=, res=)` reach a `bass` module and instrument graphs,
    // and `keys` takes cutoff and resonance per voice; `fm` has no filter and
    // a kit none per drum, so they play the note and drop the lock without a
    // word, as `keys` does with `edepth` (it has no filter envelope).
    // A dark techno hook moved from `bass` to `fm` kept twenty of them in its
    // patterns, written as if they shaped every note, and none did anything.
    // `gate=` is the sequencer's and works everywhere, so it is not counted.
    // One warning per track: a track that plays four patterns through
    // its scenes gets one line listing them, not four copies of the hint.
    let mut tracks_done: Vec<&str> = Vec::new();
    for track in &all_tracks {
        if tracks_done.contains(&track.name.as_str()) {
            continue;
        }
        tracks_done.push(&track.name);
        let (mut total, mut cutoff, mut edepth, mut res) = (0usize, false, false, false);
        let mut per_pattern: Vec<(&str, usize)> = Vec::new();
        let mut module: Option<&ModuleDef> = None;
        for t in all_tracks.iter().filter(|t| t.name == track.name) {
            let Some(m) = song.module_defs.iter().find(|m| m.name == t.using_instrument) else { continue };
            // `keys` takes cutoff and resonance per voice but has no filter
            // envelope, so only `edepth` is lost on it.
            let keys = m.module_type == "keys";
            if !matches!(m.module_type.as_str(), "fm" | "keys" | "beats") {
                continue;
            }
            if per_pattern.iter().any(|(n, _)| *n == t.play) {
                continue;
            }
            let Some(pat) = song.patterns.iter().find(|p| p.name == t.play) else { continue };
            let mut count = 0usize;
            let mut note = |p: &super::ast::PLock| {
                let (c, e, r) = (p.cutoff.is_some() && !keys, p.env_depth.is_some(), p.resonance.is_some() && !keys);
                if c || e || r {
                    count += 1;
                    cutoff |= c;
                    edepth |= e;
                    res |= r;
                }
            };
            for step in pat.rows.iter().flatten() {
                match step {
                    Step::Note(n) => note(&n.plock),
                    Step::Chord(c) => note(&c.plock),
                    Step::DrumHit(d) => note(&d.plock),
                    Step::Subdiv(ns) => ns.iter().for_each(|n| note(&n.plock)),
                    Step::DrumSub(_) | Step::Rest | Step::Tie => {}
                }
            }
            if count > 0 {
                per_pattern.push((&t.play, count));
                total += count;
                module = Some(m);
            }
        }
        let Some(m) = module else { continue };
        let which: Vec<&str> = [(cutoff, "cutoff"), (edepth, "edepth"), (res, "res")]
            .into_iter()
            .filter(|(on, _)| *on)
            .map(|(_, n)| n)
            .collect();
        let where_: Vec<String> = per_pattern.iter().map(|(n, c)| format!("'{n}' ({c})")).collect();
        out.push(lint(
            "plock_ignored",
            format!(
                "track '{}' has {} filter lock{} ({}) in {} through module '{}', a `{}` module, which ignores them",
                track.name,
                total,
                if total == 1 { "" } else { "s" },
                which.join(", "),
                where_.join(", "),
                m.name,
                m.module_type
            ),
            "Filter locks reach `bass`, `keys` (cutoff and res, not edepth: it has no filter envelope) and instrument graphs; `fm` and `beats` drop them without a sound. `gate=` works everywhere. On `fm` the note's velocity already sets its brightness; for a sweep use `auto <module> mod_index a > b`, or play the part on a `bass` or `keys` module.",
        ));
    }

    // ── mono_mix: everything in the same place in the stereo field ──
    //
    // Twenty-one of the twenty-five songs in this repo measured under 2% wide.
    // Instruments that sit on top of each other cannot be told apart however
    // well they are balanced, and nothing in the tooling looked at it.
    {
        let tonal: Vec<&&TrackDef> = all_tracks
            .iter()
            .filter(|t| song.module_defs.iter().any(|m| m.name == t.using_instrument && m.module_type != "beats"))
            .collect();
        let panned = tonal.iter().filter(|t| t.pan.is_some_and(|p| crate::math::abs(p) > 0.15)).count();
        if tonal.len() >= 4 && panned * 3 < tonal.len() {
            out.push(lint(
                "mono_mix",
                format!("{} of {} tonal tracks sit within 0.15 of centre", tonal.len() - panned, tonal.len()),
                "Give each instrument its own place: `pan -0.4` and `pan 0.35` on the parts that share a frequency band, or `autopan(0.4, bars=8)` and `chorus_mix` for width that moves. The kick, sub and lead stay centred.",
            ));
        }
    }

    // ── the master chain ──
    //
    // Three warnings that each came out of an afternoon. They are cheap
    // because everything they need is in the text; they exist because none
    // of them is visible by reading the text.
    if let Some(master) = &song.master {
        // A low shelf before the compressor feeds the compressor's detector,
        // and what dominates a mix below 200 Hz is the kick. The compressor
        // then ducks the whole song once per kick -- not the kick, the song.
        // It reads as "the mix breathes", which is what a compressor is
        // supposed to do, so nobody looks at it. Three of nine songs fixed in
        // one afternoon had exactly this, and moving the shelf after the
        // compressor fixed all three.
        let comp_at = master.chain.iter().position(|n| n.kind == "compressor");
        if let Some(ci) = comp_at {
            for node in master.chain.iter().take(ci).filter(|n| n.kind == "eq") {
                let low = chain_arg(&node.params, "low").unwrap_or(0.0);
                if low >= 3.0 {
                    out.push(lint(
                        "bass_into_compressor",
                        format!("master: eq boosts the low shelf {low:+.1} dB before the compressor"),
                        "The kick owns that band, so the compressor now ducks the whole mix on every kick. Move the `eq` after the `compressor`, or keep the boost under 3 dB and make up the weight on the bass track instead.",
                    ));
                }
            }
        }
        // `makeup` maxes out at 4.0 linear, which is +12 dB. Reaching for it
        // means the mix arrives too quiet and the compressor is being asked
        // to be a fader. It cannot: past its ceiling the knob silently stops
        // moving, and the reflex is to turn it further.
        for node in master.chain.iter().chain(song.bus_chains.iter().flat_map(|b| b.chain.iter())) {
            if node.kind != "compressor" {
                continue;
            }
            let makeup = chain_arg(&node.params, "makeup").unwrap_or(1.0);
            // Within 5% of the registry's maximum, not at it: `makeup=12db`
            // comes back a hair under 4.0 through `pow`. Reading the ceiling
            // from the registry also means the check follows if the range
            // ever moves.
            let ceiling = crate::nodes::arg_at("compressor", Some("makeup"), 0).map_or(4.0, |a| a.max);
            if makeup >= ceiling * 0.95 {
                out.push(lint(
                    "makeup_at_the_ceiling",
                    format!("a compressor asks for makeup={:.0}db, which is the maximum", 20.0 * crate::math::log10(makeup)),
                    "The knob stops here, so turning it further does nothing. Raise the track levels feeding it instead -- `level` goes to 4.0 too -- and leave makeup for matching the level the compression took away.",
                ));
            }
        }
    }

    // ── drum levels are applied after the saturator ──
    //
    // Every voice in `beats` runs its own `tanh`, which bounds it at +/-1, and
    // then multiplies by its `*_level`. So the level is not a fader between
    // silence and the voice: above 1.0 it takes an already-limited signal past
    // full scale, before the track level or the master has had a say. It reads
    // like a mix knob and behaves like a boost, which is how one song arrived
    // at a snare that clipped on its own.
    for m in song.module_defs.iter().filter(|m| m.module_type == "beats") {
        for mp in &m.params {
            if !mp.name.ends_with("_level") || mp.name == "level" {
                continue;
            }
            if mp.value > 1.0 {
                out.push(lint(
                    "drum_level_past_full_scale",
                    format!("module '{}': {} {:.2} is applied after the voice's saturator", m.name, mp.name, mp.value),
                    "Above 1.0 this pushes a signal the saturator already bounded at 1.0 past full scale. To make a drum louder relative to the others, bring the others down, or raise the track `level`, which is after the mix and has headroom to give.",
                ));
            }
        }
    }

    // ── unused definitions ──
    for m in &song.module_defs {
        if !all_tracks.iter().any(|t| t.using_instrument == m.name) {
            out.push(lint(
                "unused_module",
                format!("module '{}' is never used by a track", m.name),
                "Remove it or add a track that plays it.",
            ));
        }
    }
    // An unused pattern is usually a pattern someone forgot to wire up. In a
    // rig it is the opposite: a rig is a palette of alternates waiting to be
    // switched in by hand, and naming all eighteen of them one per line
    // buried the warnings that mattered under a wall of ones that did not.
    // A file with no arrangement is not a song, and several unused patterns
    // in one is a palette; a single one is still worth pointing at by name.
    let unplayed: Vec<&str> = song
        .patterns
        .iter()
        .filter(|p| !all_tracks.iter().any(|t| t.play == p.name))
        .map(|p| p.name.as_str())
        .collect();
    let is_rig = song.arrangement.is_empty();
    if is_rig && unplayed.len() >= 3 {
        out.push(lint(
            "unused_pattern",
            format!("{} patterns are never played: {}", unplayed.len(), unplayed.join(", ")),
            "In a rig that is the palette, and this is a note rather than a problem: a pattern nothing plays costs nothing. In a song it means a part that was written and never wired up.",
        ));
    } else {
        for name in unplayed {
            out.push(lint(
                "unused_pattern",
                format!("pattern '{name}' is never played"),
                "Remove it or play it from a track or scene.",
            ));
        }
    }
    // A declared bus nothing routes into processes silence. Easy to write when
    // a track means to use `reverb_send` and the author also declares a bus
    // called `reverb`, which is how two of the examples ended up with one.
    for b in &song.buses {
        if !all_tracks.iter().any(|t| t.routing.iter().any(|r| r.kind == b.name)) {
            out.push(lint(
                "unused_bus",
                format!("bus '{}' has no track routed into it", b.name),
                "Route a track with `out > <bus>`, or remove it. `reverb_send` and `delay_send` feed the global sends, not a bus of that name.",
            ));
        }
    }

    out
}
