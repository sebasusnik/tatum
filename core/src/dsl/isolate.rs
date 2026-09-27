//! `--solo` and `--mute`: hear one part of a song without editing the file.
//!
//! Done on the AST, before compiling, so render, play, watch and debug all
//! isolate the same way and the live planner sees a song that simply has
//! those tracks at `level 0`. That is also what keeps the rest of the song
//! honest: a muted kick still feeds the sidechain -- the engine keeps a
//! `level 0` sidechain source running -- so a soloed bass still breathes the
//! way it does in the mix, and the scenes, the arrangement and the automatic
//! gain compensation all see the same tracks as before.

use alloc::string::String;
use alloc::vec::Vec;

use super::ast::Song;
use super::compiler::{self, CompiledSong};
use super::error::CompileError;
use crate::song_engine::DslError;

/// Which tracks to take out. Empty `solo` means "all of them stay".
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Isolation {
    pub solo: Vec<String>,
    pub mute: Vec<String>,
}

impl Isolation {
    pub fn is_empty(&self) -> bool {
        self.solo.is_empty() && self.mute.is_empty()
    }

    fn silences(&self, track: &str) -> bool {
        self.mute.iter().any(|m| m == track) || (!self.solo.is_empty() && !self.solo.iter().any(|s| s == track))
    }

    /// Names given that are not a track of `song`, so a typo is an error
    /// rather than a solo that silences everything.
    pub fn unknown(&self, song: &Song) -> Vec<String> {
        let names = track_names(song);
        self.solo.iter().chain(self.mute.iter()).filter(|n| !names.contains(n)).cloned().collect()
    }

    /// Pull every track this silences down to `level 0`, everywhere it is
    /// defined, and drop the level automations that would bring it back.
    pub fn apply(&self, song: &mut Song) {
        if self.is_empty() {
            return;
        }
        for t in song.tracks.iter_mut().chain(song.scenes.iter_mut().flat_map(|s| s.tracks.iter_mut())) {
            if self.silences(&t.name) {
                t.level = Some(0.0);
            }
        }
        for sc in song.scenes.iter_mut() {
            sc.automations.retain(|a| !a.target.strip_suffix(".level").is_some_and(|t| self.silences(t)));
        }
    }
}

/// Parse, isolate and compile. A name that is not a track is a compile error
/// naming the tracks there are.
pub fn compile(source: &str, isolation: &Isolation) -> Result<CompiledSong, DslError> {
    let mut ast = super::parse(source).map_err(DslError::Parse)?;
    let unknown = isolation.unknown(&ast);
    if !unknown.is_empty() {
        return Err(DslError::Compile(vec![CompileError::new(format!(
            "no track named {} (tracks: {})",
            unknown.join(", "),
            track_names(&ast).join(", "),
        ))]));
    }
    isolation.apply(&mut ast);
    compiler::compile(&ast).map_err(DslError::Compile)
}

/// The output gain for `source`, measured on the whole song whatever is
/// isolated from it: a soloed track plays as loud as it sits in the mix, not
/// levelled up to where the whole song would be.
pub fn output_gain(source: &str) -> Result<f32, DslError> {
    let full = compile(source, &Isolation::default())?;
    Ok(crate::output::gain_for(crate::song_engine::SongEngine::loudness(&full)))
}

/// Every track name in the song, top level and scenes, first appearance order.
pub fn track_names(song: &Song) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for t in song.tracks.iter().chain(song.scenes.iter().flat_map(|s| s.tracks.iter())) {
        if !names.contains(&t.name) {
            names.push(t.name.clone());
        }
    }
    names
}
