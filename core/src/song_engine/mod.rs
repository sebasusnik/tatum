//! The song engine: a compiled song, played.
//!
//! [`SongEngine`] owns everything a song needs to sound -- its instruments,
//! one copy per track that names them, the tracks that sequence them, the
//! buses, the two global sends and the master chain -- and renders it block
//! by block. The struct lives here; what it does is split by job across the
//! files below, each adding its own `impl SongEngine` block, so the render
//! loop, the sequencer and the live swap can each be read without the rest.

extern crate alloc;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use crate::dsl::compiler::{CompiledPattern, CompiledScene};
pub use crate::dsl::error::DslError;
use crate::effects::delay::Delay;
use crate::effects::reverb::Reverb;
use crate::output::OutputStage;
use crate::rng::Rng;
use crate::BLOCK_SIZE;

mod automation;
mod build;
mod bus;
mod control;
mod fx_chain;
mod held;
mod handover;
mod instrument;
mod meters;
mod mix;
mod scene;
mod sequencer;
mod track;
mod transport;

pub use automation::INLINE_NAME_CAP;
pub use meters::{TapBlock, Taps};
pub use track::MAX_TRACK_LEVEL;

use automation::ActiveAutomation;
use bus::SongBus;
use fx_chain::FxChain;
use instrument::SongInstrument;
use sequencer::PendingTrigger;
use track::TrackPlayback;

/// Self-contained engine for rendering DSL songs.
///
/// Uses graph-based and module-based instruments, named buses, pattern sequencing,
/// and arrangement playback with scene automation.
pub struct SongEngine {
    // Compiled data
    instruments: Vec<SongInstrument>,
    instrument_names: Vec<String>,
    /// Which live instrument a track plays for each compiled module it may
    /// name, flat: `inst_for[track * n_instruments + module]`. Every pair
    /// gets its own instance so no two tracks drive one set of voices.
    inst_for: Vec<usize>,
    n_instruments: usize,
    patterns: Vec<CompiledPattern>,

    // Playback tracks
    tracks: Vec<TrackPlayback>,
    track_names: Vec<String>,

    // Buses
    buses: Vec<SongBus>,

    // Global send effects (delay + reverb)
    send_delay: Delay,
    send_reverb: Reverb,

    // Master FX chain
    master_fx: FxChain,
    /// Loudness and the true-peak ceiling, after the master chain: see `output`.
    output: OutputStage,
    reverb_return: FxChain,
    delay_return: FxChain,
    reverb_sidechain: f32,
    delay_sidechain: f32,

    // Timing
    tempo: f32,
    samples_per_step: f32,
    sample_counter: f32,
    current_step_duration: f32, // effective duration of current step (with swing)
    steps_per_bar: usize,

    // Groove / humanization
    swing: f32,             // 0.5 = straight, 0.67 = triplet feel (range 0.5-0.75)
    humanize_velocity: f32, // velocity jitter amount 0.0-1.0
    humanize_timing: f32,   // timing jitter amount 0.0-1.0
    /// Jitter on the step clock, which is one clock for the whole song. Every
    /// other random draw belongs to a track and lives on the track, so that
    /// this stream is not perturbed by how many tracks the song happens to
    /// have. See `TrackPlayback::rng`.
    timing_rng: Rng,

    // Master level (applied before master FX, matching reference Engine's 0.8)
    master_level: f32,

    // Automatic gain compensation for track summing

    // Sidechain compression
    sidechain_amount: f32,
    sc_envelope: f32,
    /// The send envelope at every sample of the block being rendered. The
    /// returns duck by it sample by sample; taking one value per block made
    /// their gain step every 128 samples when the kick hit, and that step
    /// is a click on a reverb tail.
    sc_curve: [f32; BLOCK_SIZE],
    kick_track_idx: Option<usize>,
    /// Song-wide `sidechain ... from=`; `None` falls back to the kick track.
    global_sc_source: Option<String>,
    /// One-pole coefficients for the source envelope. The release is what makes
    /// a duck read as a pump: too fast and the bass snaps back inside the kick,
    /// too slow and it never comes back.
    sc_attack_coeff: f32,
    sc_release_coeff: f32,
    /// Resolved once per scene: the audio path must not search by name.
    global_sc_idx: Option<usize>,

    // Scene effect overrides
    reverb_wet_level: f32,
    delay_wet_level: f32,
    /// The wet levels as heard, gliding like a track's level does.
    reverb_wet_heard: f32,
    delay_wet_heard: f32,

    // Automation state
    active_automations: Vec<ActiveAutomation>,
    /// The reverb's freeze as the text, a scene, `auto` or a knob set it,
    /// and as a pad holds it: frozen while either is.
    freeze_set: bool,
    freeze_pad: bool,
    /// What the hands hold: see `held.rs`.
    held: [crate::live::FastOp; held::MAX_HELD],
    held_count: usize,
    scene_step: usize,
    scene_total_steps: usize,

    // Arrangement
    scenes: Vec<CompiledScene>,
    arrangement: Vec<(usize, u32)>,
    arrangement_idx: usize,
    arrangement_bar_count: u32,
    current_bar: usize,
    global_step: usize,

    // Pending nudge triggers: (samples_remaining, instrument_idx, midi_note, velocity)
    // Capacity is reserved up front; see `push_trigger` for what happens when it fills.
    pending_triggers: Vec<PendingTrigger>,

    // Band metering costs nine one-poles per track per sample, so it is off
    // unless a render report asks for it.
    band_metering: bool,
    master_in_peak: f32,
    master_in_sum_sq: f64,
    master_in_samples: u64,

    // Per-track render buffers, allocated once. The audio path takes them with
    // `mem::take` and puts them back, so a block never touches the allocator.
    track_bufs_l: Vec<[f32; BLOCK_SIZE]>,
    track_bufs_r: Vec<[f32; BLOCK_SIZE]>,
    /// Recomputed every block: a track that is muted and has nothing left
    /// ringing is skipped whole. Allocated once, like the buffers above.
    track_silent: Vec<bool>,

    /// What `tatum debug` listens to: every part of the last block on its own.
    /// `None` unless asked for; see [`Taps`].
    taps: Option<Box<Taps>>,

    running: bool,
}
