//! Design lints: a song can compile and still sound flat. These checks run on
//! the AST after a successful compile and return warnings, never errors.
//! They exist so an AI author gets "this pad never moves" as feedback instead
//! of only "this parses".

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;

use super::ast::{Song, Step, TrackDef, ModuleDef};

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
    if !pat.lane_labels.is_empty() { return 0; }
    let mut best = 0;
    let mut run = 0;
    let mut holding = false;
    for step in pat.rows.iter().flatten() {
        match step {
            Step::Note(_) | Step::Chord(_) => { holding = true; run = 0; }
            Step::Tie if holding => { run += 1; best = best.max(run); }
            _ => { holding = false; run = 0; }
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
        || track.arp.as_ref().map_or(false, |a| a.mode != "off")
}

/// Scenes in which the track holds long notes with nothing moving the module.
fn static_scenes(song: &Song, m: &ModuleDef, track: &TrackDef) -> Vec<String> {
    let prefix = format!("{}.", m.name);
    let mut out = Vec::new();
    for scene in &song.scenes {
        let Some(st) = scene.tracks.iter().find(|t| t.name == track.name) else { continue };
        if longest_hold(song, &st.play) < 8 { continue; }
        let arped = st.arp.as_ref().map_or(false, |a| a.mode != "off");
        let automated = scene.automations.iter().any(|a| a.target.starts_with(&prefix) && !a.target.ends_with(".level"));
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

/// Run every lint on a song that already compiled.
pub fn lint_song(song: &Song) -> Vec<Lint> {
    let mut out = Vec::new();

    // Tracks as they appear anywhere (top level and scenes), keyed by name.
    let all_tracks: Vec<&TrackDef> = song.tracks.iter()
        .chain(song.scenes.iter().flat_map(|s| s.tracks.iter()))
        .collect();

    // ── static_pad: sustained keys/fm notes with nothing moving them ──
    for track in &song.tracks {
        let Some(m) = song.module_defs.iter().find(|m| m.name == track.using_instrument) else { continue };
        if m.module_type != "keys" && m.module_type != "fm" { continue; }
        if has_intrinsic_modulation(m, track) { continue; }
        let scenes = static_scenes(song, m, track);
        if !scenes.is_empty() {
            out.push(lint(
                "static_pad",
                format!("track '{}' holds long notes on module '{}' with nothing moving it in: {}", track.name, m.name, scenes.join(", ")),
                "Sustained sounds need movement: add `lfo_target cutoff` with `lfo_depth`, `vibrato_depth`, a per-scene `auto <module> cutoff a > b`, or an `arp`.",
            ));
        }
    }

    // ── dry_mix: nothing goes to the sends or a bus ──
    let any_send = all_tracks.iter().any(|t| {
        t.delay_send.unwrap_or(0.0) > 0.0 || t.reverb_send.unwrap_or(0.0) > 0.0
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
    let has_drums = song.tracks.iter().any(|t| song.module_defs.iter().any(|m| m.name == t.using_instrument && m.module_type == "beats"));
    let has_tonal = song.tracks.iter().any(|t| song.module_defs.iter().any(|m| m.name == t.using_instrument && m.module_type != "beats"));
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
        if scene.tracks.iter().any(|t| is_beats(&t.using_instrument)) { continue; }
        let ducked: Vec<&str> = scene.tracks.iter()
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

    // ── no_limiter: master chain without a limiter ──
    let has_limiter = song.master.as_ref().map_or(false, |m| m.chain.iter().any(|n| n.kind == "limiter"));
    if !has_limiter {
        out.push(lint(
            "no_limiter",
            String::from("master chain has no limiter"),
            "End the master chain with `limiter` to catch peaks: `master { in > eq(...) > compressor(...) > limiter > out }`.",
        ));
    }

    // ── level_used_as_fader: a sustained element ridden up and down per scene ──
    //
    // A continuous drone or pad is a bed, not a fader. Moving its level between
    // scenes to hit a loudness target is audible as the bed changing volume
    // under everything else, which is exactly the complaint this came from.
    for track in &song.tracks {
        let Some(m) = song.module_defs.iter().find(|m| m.name == track.using_instrument) else { continue };
        if m.module_type == "beats" { continue; }
        let mut levels: Vec<(String, f32)> = Vec::new();
        for scene in &song.scenes {
            let Some(st) = scene.tracks.iter().find(|t| t.name == track.name) else { continue };
            // Only sustained material: a staccato part may well be a fader.
            if longest_hold(song, &st.play) < 8 { continue; }
            let level = st.level.or(track.level).unwrap_or(0.8);
            levels.push((scene.name.clone(), level));
        }
        if levels.len() < 2 { continue; }
        let lo = levels.iter().fold(f32::MAX, |a, (_, v)| a.min(*v));
        let hi = levels.iter().fold(0.0f32, |a, (_, v)| a.max(*v));
        if lo <= 0.0 { continue; }
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

    // ── unused definitions ──
    for m in &song.module_defs {
        if !all_tracks.iter().any(|t| t.using_instrument == m.name) {
            out.push(lint("unused_module", format!("module '{}' is never used by a track", m.name), "Remove it or add a track that plays it."));
        }
    }
    for p in &song.patterns {
        if !all_tracks.iter().any(|t| t.play == p.name) {
            out.push(lint("unused_pattern", format!("pattern '{}' is never played", p.name), "Remove it or play it from a track or scene."));
        }
    }

    out
}
