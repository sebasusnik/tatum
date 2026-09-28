//! What the audio thread shows the screen, without a lock and without
//! allocating: the mix goes into a ring of samples the screen reads the newest
//! of, and each track's peak and energy per band go into atomics the screen
//! takes and clears once a frame.
//!
//! Everything is lossy on purpose. A frame the screen misses is a frame not
//! drawn, never a callback that waited.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use tatum_core::song_engine::SongEngine;

/// Samples of mix kept: enough for the longest analysis window the screen
/// takes, with room for the callback to write while the screen reads.
pub const RING: usize = 1 << 14;
/// Tracks shown. A song with more shows its first ones.
pub const MAX_TRACKS: usize = 64;
pub const BANDS: usize = 5;

pub struct Telemetry {
    ring: Box<[AtomicU32]>,
    written: AtomicUsize,
    /// Per track, the loudest sample since the screen last looked.
    peak: Box<[AtomicU32]>,
    /// Per track and band, the loudest mean power over one callback since the
    /// screen last looked.
    band: Box<[AtomicU32]>,
    /// Per track, whether this loop is one its `play` line transforms.
    transformed: Box<[AtomicBool]>,
    /// Per track, the pattern playing and the step in it.
    pattern: Box<[AtomicU32]>,
    step: Box<[AtomicU32]>,
    tracks: AtomicUsize,
}

impl Telemetry {
    pub fn new() -> Self {
        let atomics = |n: usize| (0..n).map(|_| AtomicU32::new(0)).collect::<Vec<_>>().into_boxed_slice();
        Self {
            ring: atomics(RING),
            written: AtomicUsize::new(0),
            peak: atomics(MAX_TRACKS),
            band: atomics(MAX_TRACKS * BANDS),
            transformed: (0..MAX_TRACKS).map(|_| AtomicBool::new(false)).collect::<Vec<_>>().into_boxed_slice(),
            pattern: atomics(MAX_TRACKS),
            step: atomics(MAX_TRACKS),
            tracks: AtomicUsize::new(0),
        }
    }

    // ── Audio thread ──

    /// One block of the mix, as the engine rendered it.
    pub fn push(&self, l: &[f32], r: &[f32]) {
        let mut w = self.written.load(Ordering::Relaxed);
        for (a, b) in l.iter().zip(r) {
            self.ring[w % RING].store(((a + b) * 0.5).to_bits(), Ordering::Relaxed);
            w = w.wrapping_add(1);
        }
        self.written.store(w, Ordering::Release);
    }

    /// Take the engine's meters for this callback and clear them. Positive
    /// floats order the same way as their bits, so `fetch_max` on the bits
    /// keeps the largest value.
    pub fn meter(&self, engine: &mut SongEngine) {
        let n = engine.track_count().min(MAX_TRACKS);
        self.tracks.store(n, Ordering::Relaxed);
        for i in 0..n {
            self.transformed[i].store(engine.track_transformed(i), Ordering::Relaxed);
            self.pattern[i].store(engine.track_pattern(i) as u32, Ordering::Relaxed);
            self.step[i].store(engine.track_step(i) as u32, Ordering::Relaxed);
            self.peak[i].fetch_max(engine.track_peak(i).abs().to_bits(), Ordering::Relaxed);
            let (energy, samples) = engine.track_band_energy(i);
            if samples > 0 {
                for (b, e) in energy.iter().enumerate() {
                    let power = (*e / samples as f64) as f32;
                    self.band[i * BANDS + b].fetch_max(power.max(0.0).to_bits(), Ordering::Relaxed);
                }
            }
        }
        engine.reset_meters();
    }

    // ── Screen thread ──

    /// The newest `out.len()` samples of the mix, oldest first.
    pub fn latest(&self, out: &mut [f32]) {
        let w = self.written.load(Ordering::Acquire);
        let n = out.len().min(RING);
        let start = w.wrapping_sub(n);
        for (i, o) in out.iter_mut().enumerate().take(n) {
            *o = f32::from_bits(self.ring[start.wrapping_add(i) % RING].load(Ordering::Relaxed));
        }
    }

    /// Whether track `i` is on a loop its `play` line transforms.
    pub fn transformed(&self, i: usize) -> bool {
        self.transformed.get(i).is_some_and(|t| t.load(Ordering::Relaxed))
    }

    /// The pattern track `i` plays and the step it is on.
    pub fn position(&self, i: usize) -> (usize, usize) {
        match (self.pattern.get(i), self.step.get(i)) {
            (Some(p), Some(s)) => (p.load(Ordering::Relaxed) as usize, s.load(Ordering::Relaxed) as usize),
            _ => (0, 0),
        }
    }

    pub fn tracks(&self) -> usize {
        self.tracks.load(Ordering::Relaxed)
    }

    /// Peak and power per band of one track since the last call, clearing them.
    pub fn take(&self, track: usize) -> (f32, [f32; BANDS]) {
        let peak = f32::from_bits(self.peak[track].swap(0, Ordering::Relaxed));
        let mut bands = [0.0; BANDS];
        for (b, v) in bands.iter_mut().enumerate() {
            *v = f32::from_bits(self.band[track * BANDS + b].swap(0, Ordering::Relaxed));
        }
        (peak, bands)
    }
}
