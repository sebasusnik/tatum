//! What a track plays: a graph instrument or one of the built-in modules,
//! behind one enum so the engine drives them all the same way.

use crate::graph::voice::Instrument;
use crate::modules::bass::{BassModule, BassParam};
use crate::modules::beats::BeatsModule;
use crate::modules::fm::FmModule;
use crate::modules::keys::KeysModule;
use crate::params::{self, ParamId, ModuleKind};
use crate::Module;

/// Wraps both graph-based instruments and real module instruments.
// large_enum_variant: boxing would put a pointer chase in front of every
// instrument on every sample, the engine's hottest loop, to save memory the
// engine has already reserved off-thread.
#[allow(clippy::large_enum_variant)]
pub(super) enum SongInstrument {
    Graph(Instrument),
    Bass(BassModule),
    Fm(FmModule),
    Keys(KeysModule),
    Beats(BeatsModule),
}

impl SongInstrument {
    pub(super) fn kind_str(&self) -> &'static str {
        match self {
            Self::Graph(_) => "graph",
            Self::Bass(_) => "bass",
            Self::Fm(_) => "fm",
            Self::Keys(_) => "keys",
            Self::Beats(_) => "beats",
        }
    }

    pub(super) fn note_on(&mut self, note: u8, velocity: f32) {
        match self {
            Self::Graph(inst) => inst.note_on(note, velocity),
            Self::Bass(m) => m.note_on(note, velocity),
            Self::Fm(m) => m.note_on(note, velocity),
            Self::Keys(m) => m.note_on(note, velocity),
            Self::Beats(m) => m.note_on(note, velocity),
        }
    }

    /// Glide to a new note without retriggering envelopes. Returns false when
    /// the instrument has no portamento, so the caller falls back to note_on.
    pub(super) fn slide_to(&mut self, note: u8, velocity: f32) -> bool {
        match self {
            Self::Bass(m) => { m.slide_to(note, velocity); true }
            _ => false,
        }
    }

    /// Bend every note the instrument plays by `ratio` of its frequency. A
    /// graph instrument or a drum kit has nothing to bend.
    pub(super) fn set_pitch_bend(&mut self, ratio: f32) {
        match self {
            Self::Bass(m) => m.pitch_bend_ratio = ratio,
            Self::Fm(m) => m.pitch_bend_ratio = ratio,
            Self::Keys(m) => m.pitch_bend_ratio = ratio,
            Self::Graph(_) | Self::Beats(_) => {}
        }
    }

    pub(super) fn note_off(&mut self, note: u8) {
        match self {
            Self::Graph(inst) => inst.note_off(note),
            Self::Bass(m) => m.note_off(note),
            Self::Fm(m) => m.note_off(note),
            Self::Keys(m) => m.note_off(note),
            Self::Beats(m) => m.note_off(note),
        }
    }

    fn process_block(&mut self, output: &mut [f32]) {
        match self {
            Self::Graph(inst) => inst.process_block(output),
            Self::Bass(m) => Module::process_block(m, output),
            Self::Fm(m) => Module::process_block(m, output),
            Self::Keys(m) => Module::process_block(m, output),
            Self::Beats(m) => Module::process_block(m, output),
        }
    }

    /// Process a block with stereo output. Returns true if the instrument produced native stereo.
    pub(super) fn process_block_stereo(&mut self, out_l: &mut [f32], out_r: &mut [f32]) -> bool {
        match self {
            Self::Beats(m) => { m.process_block_stereo(out_l, out_r); true }
            // Keys pans its voices (unison spreads eight of them across the
            // field) and runs the chorus in stereo. Routing it through the mono
            // `process_block` downmixed all of that and then copied one channel
            // into the other.
            Self::Keys(m) => { m.process_block_stereo(out_l, out_r); true }
            Self::Fm(m) => { m.process_block_stereo(out_l, out_r); true }
            _ => { self.process_block(out_l); false }
        }
    }

    /// True when the instrument is producing nothing and has nothing left
    /// releasing, so the engine can skip it whole.
    pub(super) fn is_idle(&self) -> bool {
        match self {
            Self::Graph(inst) => inst.is_idle(),
            Self::Bass(m) => m.is_idle(),
            Self::Fm(m) => m.is_idle(),
            Self::Keys(m) => m.is_idle(),
            Self::Beats(m) => m.is_idle(),
        }
    }

    pub(super) fn stage_plock(&mut self, cutoff: Option<f32>, env_depth: Option<f32>, resonance: Option<f32>) {
        match self {
            Self::Graph(inst) => inst.stage_plock(cutoff, env_depth, resonance),
            Self::Bass(m) => {
                // Bass p-lock values are already normalized 0-1 (matching BassParam range)
                if let Some(v) = cutoff {
                    m.set_param(BassParam::Cutoff, v);
                }
                if let Some(v) = env_depth {
                    m.set_param(BassParam::CutoffEnv, v);
                }
                if let Some(r) = resonance {
                    m.set_param(BassParam::Resonance, r);
                }
            }
            Self::Fm(_) | Self::Keys(_) | Self::Beats(_) => {}
        }
    }

    pub(super) fn reset(&mut self) {
        match self {
            Self::Graph(inst) => inst.reset(),
            Self::Bass(m) => Module::reset(m),
            Self::Fm(m) => Module::reset(m),
            Self::Keys(m) => Module::reset(m),
            Self::Beats(m) => Module::reset(m),
        }
    }

    pub(super) fn set_bpm(&mut self, bpm: f32) {
        match self {
            Self::Graph(_) => {} // Graph instruments don't have BPM
            Self::Bass(m) => m.set_bpm(bpm),
            Self::Fm(m) => m.set_bpm(bpm),
            Self::Keys(m) => m.set_bpm(bpm),
            Self::Beats(m) => m.set_bpm(bpm),
        }
    }

    /// Set a parameter by name (for automation).
    pub(super) fn set_param_by_name(&mut self, name: &str, value: f32) -> bool {
        let kind = match self.module_kind() {
            Some(k) => k,
            None => return false, // graph instruments have no named params
        };
        match params::lookup(kind, name) {
            Some(spec) => apply_param(self, spec.id, value),
            None => false,
        }
    }

    /// Registry kind for built-in modules; None for graph instruments.
    fn module_kind(&self) -> Option<ModuleKind> {
        match self {
            Self::Graph(_) => None,
            Self::Bass(_) => Some(ModuleKind::Bass),
            Self::Fm(_) => Some(ModuleKind::Fm),
            Self::Keys(_) => Some(ModuleKind::Keys),
            Self::Beats(_) => Some(ModuleKind::Beats),
        }
    }
}

/// Apply a registry-typed parameter to the right module. False when the id
/// belongs to another module kind: that is a caller bug, not a no-op.
pub(super) fn apply_param(inst: &mut SongInstrument, id: ParamId, value: f32) -> bool {
    match (inst, id) {
        (SongInstrument::Bass(m), ParamId::Bass(p)) => m.set_param(p, value),
        (SongInstrument::Fm(m), ParamId::Fm(p)) => m.set_param(p, value),
        (SongInstrument::Keys(m), ParamId::Keys(p)) => m.set_param(p, value),
        (SongInstrument::Beats(m), ParamId::Beats(p)) => m.set_param(p, value),
        _ => return false,
    }
    true
}
