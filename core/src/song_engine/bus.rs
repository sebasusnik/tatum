//! Named buses: tracks sent into one share its effect chain before master.

use alloc::string::String;

use crate::graph::node::ChainStep;
use crate::BLOCK_SIZE;

use super::fx_chain::FxChain;

/// Named bus with FX chain.
pub(super) struct SongBus {
    pub(super) name: String,
    pub(super) fx_chain: FxChain,
    /// Stereo. A bus used to sum L and R and return the result down the middle,
    /// so any track routed into one lost its position in the stereo field --
    /// which is most of what tells instruments apart.
    pub(super) buffer: [f32; BLOCK_SIZE],
    pub(super) buffer_r: [f32; BLOCK_SIZE],
    // Metered after the bus chain: a track can read fine on its own meter and
    // still arrive at master 12 dB down because of what the bus does to it.
    pub(super) meter_peak: f32,
    pub(super) meter_sum_sq: f64,
    pub(super) meter_samples: u64,
}

impl SongBus {
    pub(super) fn new(name: String, specs: &[ChainStep], slowest_bpm: f32) -> Self {
        Self {
            name,
            fx_chain: FxChain::new(specs, slowest_bpm),
            buffer: [0.0; BLOCK_SIZE],
            buffer_r: [0.0; BLOCK_SIZE],
            meter_peak: 0.0,
            meter_sum_sq: 0.0,
            meter_samples: 0,
        }
    }

    pub(super) fn clear(&mut self, len: usize) {
        for i in 0..len {
            self.buffer[i] = 0.0;
            self.buffer_r[i] = 0.0;
        }
    }

    pub(super) fn reset(&mut self) {
        self.buffer = [0.0; BLOCK_SIZE];
        self.fx_chain.reset();
    }
}
