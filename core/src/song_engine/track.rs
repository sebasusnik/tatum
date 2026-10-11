//! One track's playback state, and the small pieces every part of the engine
//! uses on it: its random seed, its arpeggiator, its pan law, and the glide
//! that keeps a change in its mix from arriving as a click.

use alloc::vec::Vec;

use crate::analysis::BandMeter;
use crate::dsl::compiler::{self, ArpConfig};
use crate::primitives::arp_processor::ArpProcessor;
use crate::rng::Rng;

use super::automation::InlineName;
use super::fx_chain::FxChain;

/// Per-track playback state.
/// How far a track's `level` can be pushed: +12 dB, the same ceiling a
/// compressor's `makeup` has.
///
/// It used to be 1.0, but only on the live path. A level written in the file
/// was unbounded and so was an `auto <track> level` sweep, so the corpus has
/// tracks at 1.4 and 2.0 that play fine -- until the same value arrives
/// through a live edit or a hot swap, where it silently became 1.0 and the
/// track dropped. Three paths for one number have to agree, and the one that
/// disagreed was the one nobody had written a file against.
pub const MAX_TRACK_LEVEL: f32 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct LiveRoll {
    pub(super) note: u8,
    pub(super) velocity: f32,
    /// Track and drum note.
    pub(super) kick: Option<(usize, u8)>,
}

pub(super) struct TrackPlayback {
    pub(super) instrument_idx: usize,
    pub(super) pattern_idx: usize,
    pub(super) velocity: f32,
    pub(super) level: f32, // output level 0.0-1.0
    pub(super) pan: f32,   // raw pan value -1.0 to 1.0
    pub(super) pan_l: f32, // pre-computed left gain (equal-power)
    pub(super) pan_r: f32, // pre-computed right gain (equal-power)
    pub(super) gate: f32,  // gate length as fraction of step (0.0-1.0)
    pub(super) insert_fx: FxChain,
    /// `as <name>` per insert node, for resolving automation targets.
    pub(super) fx_labels: Vec<Option<InlineName>>,
    pub(super) bus_send: Option<(usize, f32)>,
    pub(super) to_master: bool,
    pub(super) delay_send: f32,  // global delay send amount 0.0-1.0
    pub(super) reverb_send: f32, // global reverb send amount 0.0-1.0
    /// Held silent from a pad: heard at no level, whatever `level` says, so
    /// letting go puts it back exactly where the text or a fader had it.
    pub(super) muted: bool,
    /// Thrown into the delay from a pad: the whole track goes to the echo
    /// while held, and the send falls back to `delay_send` on release, with
    /// the repeats still ringing.
    pub(super) thrown: bool,
    /// A key held in a `roll` zone: the note this track rolls on the three
    /// sixteenths after each beat in place of its pattern, and the drum
    /// track whose kick it strikes on the beat, if any.
    pub(super) roll: Option<LiveRoll>,
    /// This track's own sidechain amount, or `None` to take the song's.
    /// `sidechain 0` used to mean "the song's amount" too, because the
    /// override was a float with 0 standing for unset, so a track written
    /// `sidechain 0.0` to stay still was ducked at the global amount anyway.
    pub(super) sidechain_amount: Option<f32>,
    // Step sequencer state
    pub(super) current_step: usize,
    /// Index into the engine's `plays` when the `play` line differs loop by
    /// loop; `None` loops `pattern_idx` as it always has.
    pub(super) play: Option<usize>,
    /// The step the current loop began on, and which loop it is, for a
    /// track with a `play` plan. Loops of different lengths add up, so the
    /// loop is counted rather than divided out of `current_step`.
    pub(super) loop_start: usize,
    pub(super) loop_index: u64,
    pub(super) current_notes: [u8; compiler::MAX_CHORD_NOTES], // active MIDI notes (0 = unused)
    pub(super) current_notes_count: u8,
    pub(super) gate_samples_remaining: f32,
    pub(super) active: bool,
    /// Samples left of the fade a track gets when a scene drops it. Until
    /// this ran out the track stopped the instant the scene changed, cutting
    /// its wave to zero mid-cycle, and that was the loudest click in most
    /// arranged songs: `tatum debug` found it on every section change.
    pub(super) leaving: u32,
    pub(super) stereo_src: bool, // true if instrument produces native stereo (BeatsModule)
    // Arpeggiator: the pattern supplies held notes, the arp schedules them per sample
    pub(super) arp: Option<ArpProcessor>,
    pub(super) arp_cfg: Option<ArpConfig>,
    // Metering: post-level, post-pan, pre-master. For mix reports.
    pub(super) meter_peak: f32,
    pub(super) meter_sum_sq: f64,
    pub(super) meter_samples: u64,
    /// Mid and side energy, so the report can say where in the stereo field a
    /// track sits. Two instruments in the same place cannot be told apart.
    pub(super) meter_mid_sq: f64,
    pub(super) meter_side_sq: f64,
    /// Band split of this track alone. Two tracks sitting in the same band is
    /// masking, and it used to take reasoning to notice.
    pub(super) band: BandMeter,
    /// Amount this track ducks, and the track it ducks against. `None` source
    /// means the song's global source, which is the kick unless said otherwise.
    pub(super) sc_source: Option<usize>,
    /// This track's own envelope, maintained only when something ducks against it.
    pub(super) sc_env: f32,
    pub(super) is_sc_source: bool,
    /// Level, pan and sends as they are heard, gliding to the values above.
    pub(super) heard: Heard,
    /// The sidechain amount as heard, gliding to what the scene asks for.
    pub(super) heard_duck: f32,
    /// Every random draw this track makes: velocity humanization and the
    /// probability gate on its drum hits.
    ///
    /// One shared stream used to serve the whole song, and that made a track's
    /// groove depend on its neighbours. The draws are interleaved in track
    /// order within a step, so muting a track, adding one, or reordering them
    /// shifted the numbers every other track received and the feel of the
    /// whole song moved. Seeding from the name rather than the index keeps a
    /// track's stream its own across all three.
    pub(super) rng: Rng,
}

/// A stable seed from a track's name. FNV-1a, which is four lines and spreads
/// single-character differences across the whole word -- `hat` and `hats` have
/// to land far apart or two tracks in the same song jitter in lockstep.
pub(super) fn seed_from_name(name: &str) -> u32 {
    let mut h: u32 = 2166136261;
    for b in name.as_bytes() {
        h ^= *b as u32;
        h = h.wrapping_mul(16777619);
    }
    if h == 0 {
        1
    } else {
        h
    }
}

/// Build a fresh arp processor from compiled settings at the given tempo.
pub(super) fn make_arp(cfg: &ArpConfig, tempo: f32) -> ArpProcessor {
    let mut arp = ArpProcessor::new();
    arp.set_bpm(tempo * cfg.rate_mult);
    arp.set_gate(cfg.gate);
    arp.set_pattern(cfg.pattern);
    // set_octave_range maps 0..1 → 1..4; nudge past float error so 3 stays 3.
    arp.set_octave_range((cfg.octaves.saturating_sub(1)) as f32 / 3.0 + 0.01);
    arp
}

/// Compute equal-power panning gains from a pan value (-1.0 to 1.0).
/// Returns (left_gain, right_gain).
#[inline]
pub(super) fn pan_gains(pan: f32) -> (f32, f32) {
    // Map -1..1 to 0..1 for the trig calculation
    let p = (pan.clamp(-1.0, 1.0) + 1.0) * 0.5;
    // Equal-power: cos/sin panning law
    let angle = p * crate::math::HALF_PI;
    (crate::math::cos(angle), crate::math::sin(angle))
}

impl TrackPlayback {
    /// Where the track is inside its loop, before the modulo: counted from
    /// the loop's own start when the `play` line changes loop by loop.
    pub(super) fn step_in_loop(&self) -> usize {
        if self.play.is_some() {
            self.current_step.saturating_sub(self.loop_start)
        } else {
            self.current_step
        }
    }

    /// Playing, or on its way out after a scene dropped it.
    pub(super) fn sounding(&self) -> bool {
        self.active || self.leaving > 0
    }

    /// Level, pan and sends as the track has them set.
    pub(super) fn target(&self) -> Heard {
        let level = if self.muted { 0.0 } else { self.level };
        Heard {
            left: level * self.pan_l,
            right: level * self.pan_r,
            delay: if self.thrown { 1.0 } else { self.delay_send },
            reverb: self.reverb_send,
        }
    }
}

/// How fast a mix setting follows a change: a 3 ms time constant, so it is
/// there within 15 ms. A scene that changed a track's level, pan or send, or
/// the reverb's wet level, used to jump on the first sample of the new
/// section, and the step in gain was a click `tatum debug` found on the
/// section changes of every song that rebalanced between them. Level
/// automation, written once per block, stepped the same way.
const GLIDE: f32 = 1.0 / 132.0;

/// Move `now` one sample closer to `to`.
pub(super) fn glide(now: &mut f32, to: f32) -> f32 {
    *now += (to - *now) * GLIDE;
    *now
}

/// Close enough to its target to stop gliding. Without the snap the gap
/// shrinks into denormals, which are slow.
pub(super) fn settle(now: &mut f32, to: f32) {
    if (to - *now).abs() < 1e-6 {
        *now = to;
    }
}

/// A track's gains into the mix: level times pan for each side, and its sends.
#[derive(Clone, Copy, Default)]
pub(super) struct Heard {
    pub(super) left: f32,
    pub(super) right: f32,
    pub(super) delay: f32,
    pub(super) reverb: f32,
}

impl Heard {
    pub(super) fn glide(&mut self, to: &Heard) {
        glide(&mut self.left, to.left);
        glide(&mut self.right, to.right);
        glide(&mut self.delay, to.delay);
        glide(&mut self.reverb, to.reverb);
    }

    pub(super) fn settle(&mut self, to: &Heard) {
        settle(&mut self.left, to.left);
        settle(&mut self.right, to.right);
        settle(&mut self.delay, to.delay);
        settle(&mut self.reverb, to.reverb);
    }
}
