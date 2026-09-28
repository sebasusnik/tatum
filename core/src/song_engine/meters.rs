//! What the engine reports about its own mix: per-track and per-bus meters,
//! the taps `tatum debug` listens to, and the loudness every song is brought
//! to.

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::dsl::compiler::CompiledSong;
use crate::output::Loudness;
use crate::{math, BLOCK_SIZE};

use super::SongEngine;

/// One block of one signal, both channels.
#[derive(Clone)]
pub struct TapBlock {
    pub l: [f32; BLOCK_SIZE],
    pub r: [f32; BLOCK_SIZE],
}

impl TapBlock {
    const SILENT: TapBlock = TapBlock { l: [0.0; BLOCK_SIZE], r: [0.0; BLOCK_SIZE] };

    pub(super) fn clear(&mut self, len: usize) {
        self.l[..len].fill(0.0);
        self.r[..len].fill(0.0);
    }
}

/// Every part of the mix of the last block, kept apart: what each track sends
/// on after its level and pan, the same track before its insert chain, each
/// bus after its chain, and the two send returns. The tracks that reach
/// master, the buses and the returns add up to the mix before the master
/// level and chain, so a stem is the part as it sits in the mix -- sidechain, bus compression
/// and all -- not a separate render of it alone.
///
/// For offline renders only. The buffers are allocated when the taps are
/// switched on and the block writes into them in place, but it is still work
/// the live audio thread has no use for.
pub struct Taps {
    /// Samples of the last block, the length every slice below is valid for.
    pub len: usize,
    pub tracks: Vec<TapBlock>,
    /// The instrument alone, before the track's insert chain, at the track's
    /// level and pan so it can be laid next to `tracks`.
    pub dry: Vec<TapBlock>,
    pub buses: Vec<TapBlock>,
    pub delay: TapBlock,
    pub reverb: TapBlock,
    /// Whether the track had a note held at the end of the block. Between
    /// notes a track should fall silent; this is how the report knows when.
    pub held: Vec<bool>,
}

impl SongEngine {
    // ── Metering (for mix reports) ──

    /// Peak of a track after its level and pan, before the master chain.
    pub fn track_peak(&self, idx: usize) -> f32 {
        self.tracks.get(idx).map_or(0.0, |t| t.meter_peak)
    }

    /// RMS of a track after its level and pan, before the master chain.
    pub fn track_rms(&self, idx: usize) -> f32 {
        self.tracks.get(idx).map_or(0.0, |t| {
            if t.meter_samples == 0 {
                0.0
            } else {
                math::sqrt((t.meter_sum_sq / t.meter_samples as f64) as f32)
            }
        })
    }

    pub fn reset_meters(&mut self) {
        for t in self.tracks.iter_mut() {
            t.meter_peak = 0.0;
            t.meter_sum_sq = 0.0;
            t.meter_samples = 0;
            t.meter_mid_sq = 0.0;
            t.meter_side_sq = 0.0;
            t.band.reset();
        }
        for b in self.buses.iter_mut() {
            b.meter_peak = 0.0;
            b.meter_sum_sq = 0.0;
            b.meter_samples = 0;
        }
        self.master_in_peak = 0.0;
        self.master_in_sum_sq = 0.0;
        self.master_in_samples = 0;
    }

    /// Keep every part of each block apart, for `tatum debug`. See [`Taps`].
    pub fn set_taps(&mut self, on: bool) {
        self.taps = on.then(|| {
            Box::new(Taps {
                len: 0,
                tracks: vec![TapBlock::SILENT; self.tracks.len()],
                dry: vec![TapBlock::SILENT; self.tracks.len()],
                buses: vec![TapBlock::SILENT; self.buses.len()],
                delay: TapBlock::SILENT,
                reverb: TapBlock::SILENT,
                held: vec![false; self.tracks.len()],
            })
        });
    }

    /// The parts of the last block, if [`set_taps`](Self::set_taps) is on.
    pub fn taps(&self) -> Option<&Taps> {
        self.taps.as_deref()
    }

    /// Whether a track goes straight to master, rather than only into a bus.
    pub fn track_to_master(&self, idx: usize) -> bool {
        self.tracks.get(idx).is_some_and(|t| t.to_master)
    }

    /// Turn on per-track band analysis. Off by default: it costs nine one-pole
    /// sections per track per sample, which a live audio thread should not pay.
    pub fn set_band_metering(&mut self, on: bool) {
        self.band_metering = on;
    }

    /// Where this track sits in the stereo field: 0 is dead centre, 0.5 is hard
    /// to one side.
    pub fn track_width(&self, idx: usize) -> f32 {
        self.tracks.get(idx).map_or(0.0, |t| {
            let total = t.meter_mid_sq + t.meter_side_sq;
            if total <= 0.0 {
                0.0
            } else {
                (t.meter_side_sq / total) as f32
            }
        })
    }

    /// Share of this track's energy in each of `analysis::BAND_NAMES`, in percent.
    pub fn track_bands(&self, idx: usize) -> [f32; 5] {
        self.tracks.get(idx).map_or([0.0; 5], |t| t.band.percentages())
    }

    /// Energy in each of `analysis::BAND_NAMES` since the meters were last
    /// reset, and how many samples it was summed over, so a live display can
    /// turn it into a level per band rather than a share.
    pub fn track_band_energy(&self, idx: usize) -> ([f64; 5], u64) {
        self.tracks.get(idx).map_or(([0.0; 5], 0), |t| (t.band.energy(), t.meter_samples / 2))
    }

    /// Index into `analysis::BAND_NAMES` of the band this track mostly occupies.
    pub fn track_dominant_band(&self, idx: usize) -> Option<usize> {
        self.tracks.get(idx).and_then(|t| t.band.dominant())
    }

    pub fn bus_count(&self) -> usize {
        self.buses.len()
    }

    pub fn bus_name(&self, idx: usize) -> &str {
        self.buses.get(idx).map_or("", |b| b.name.as_str())
    }

    /// Peak of a bus after its own chain, before it reaches master.
    pub fn bus_peak(&self, idx: usize) -> f32 {
        self.buses.get(idx).map_or(0.0, |b| b.meter_peak)
    }

    pub fn bus_rms(&self, idx: usize) -> f32 {
        self.buses.get(idx).map_or(0.0, |b| {
            if b.meter_samples == 0 {
                0.0
            } else {
                math::sqrt((b.meter_sum_sq / b.meter_samples as f64) as f32)
            }
        })
    }

    /// Peak and RMS entering the master chain, so the caller can compare the
    /// crest factor before and after and see what the limiter took.
    pub fn master_input_peak_rms(&self) -> (f32, f32) {
        let rms = if self.master_in_samples == 0 {
            0.0
        } else {
            math::sqrt((self.master_in_sum_sq / self.master_in_samples as f64) as f32)
        };
        (self.master_in_peak, rms)
    }

    /// An engine for `song` at the loudness every song plays at. It renders
    /// the song once first to measure it, which takes a fraction of its length.
    pub fn normalized(song: CompiledSong) -> Self {
        let lufs = Self::loudness(&song);
        let mut engine = Self::from_compiled(song);
        engine.output.gain = crate::output::gain_for(lufs);
        engine
    }

    /// Integrated loudness of `song` as it leaves its master chain, in LUFS:
    /// the whole arrangement, or four bars of a song without one. `None` when
    /// it is silent.
    pub fn loudness(song: &CompiledSong) -> Option<f32> {
        let mut engine = Self::from_compiled(song.clone());
        engine.output.bypass = true;
        let bars = match engine.arrangement_bars() {
            0 => 4,
            n => n,
        };
        let total = (bars as f32 * engine.steps_per_bar as f32 * engine.samples_per_step) as usize;
        engine.start();
        let mut meter = Loudness::new();
        let (mut l, mut r) = ([0.0f32; BLOCK_SIZE], [0.0f32; BLOCK_SIZE]);
        let mut done = 0;
        while done < total && engine.running {
            let n = (total - done).min(BLOCK_SIZE);
            engine.process_block_stereo(&mut l[..n], &mut r[..n]);
            for i in 0..n {
                meter.push(l[i], r[i]);
            }
            done += n;
        }
        meter.lufs()
    }

    /// The gain the output stage puts on the song, linear. 1.0 until the
    /// engine is normalized.
    pub fn output_gain(&self) -> f32 {
        self.output.gain
    }

    /// Put the output stage at a gain measured elsewhere: a live session keeps
    /// the one it measured when the song was loaded.
    pub fn set_output_gain(&mut self, gain: f32) {
        if gain.is_finite() {
            self.output.gain = gain;
        }
    }

    /// How many tracks the last rendered block skipped whole: muted, nothing
    /// left ringing, and not feeding the sidechain. Skipping is audio-neutral
    /// by construction -- a muted track contributed zero before too -- so this
    /// is the only way to see the difference from outside without a stopwatch.
    pub fn silent_track_count(&self) -> usize {
        self.track_silent.iter().filter(|s| **s).count()
    }
}
