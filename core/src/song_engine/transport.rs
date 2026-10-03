//! Starting, stopping and rendering: the engine's clock put at the top of
//! the song, or at a bar, and run for as long as asked.

use alloc::vec::Vec;

use crate::rng::Rng;
use crate::BLOCK_SIZE;

use super::SongEngine;

impl SongEngine {
    pub fn start(&mut self) {
        self.running = true;
        self.timing_rng = Rng::new(7919); // reset for deterministic humanization
        self.reseed_tracks();
        self.current_step_duration = self.effective_step_samples(0);
        self.sample_counter = self.current_step_duration; // trigger first step immediately
        self.arrangement_idx = 0;
        self.arrangement_bar_count = 0;
        self.current_bar = 0;
        self.global_step = 0;
        self.reverb_wet_level = 1.0;
        self.delay_wet_level = 1.0;
        self.active_automations.clear();
        self.scene_step = 0;
        self.scene_total_steps = 0;

        // Set BPM on all module instruments
        for inst in self.instruments.iter_mut() {
            inst.set_bpm(self.tempo);
        }

        // If arrangement exists, apply first scene; without one, the
        // top-level lanes start on bar 0.
        if !self.arrangement.is_empty() {
            let (scene_idx, _) = self.arrangement[0];
            self.apply_scene(scene_idx);
        } else {
            self.lane_origin = 0;
            self.start_lanes();
        }
        self.reassert_held();
        self.snap_mix();

        for track in self.tracks.iter_mut() {
            track.current_step = 0;
            track.current_notes_count = 0;
            track.gate_samples_remaining = 0.0;
        }
    }

    /// Render the entire song (or specified number of bars) to stereo buffers.
    pub fn render(&mut self, bars: u32) -> (Vec<f32>, Vec<f32>) {
        self.render_each(bars, |_, _, _| {})
    }

    /// [`Self::render`], handing each block to `each` as it is made, with
    /// the engine so its taps can be read: how `tatum render` listens for
    /// clicks and noise in the same pass that writes the file.
    pub fn render_each(&mut self, bars: u32, mut each: impl FnMut(&Self, &[f32], &[f32])) -> (Vec<f32>, Vec<f32>) {
        let total_samples = (bars as f32 * self.steps_per_bar as f32 * self.samples_per_step) as usize;
        let mut out_l = Vec::with_capacity(total_samples + BLOCK_SIZE);
        let mut out_r = Vec::with_capacity(total_samples + BLOCK_SIZE);

        self.start();

        let mut rendered = 0;
        while rendered < total_samples && self.running {
            let chunk = (total_samples - rendered).min(BLOCK_SIZE);
            let mut bl = [0.0f32; BLOCK_SIZE];
            let mut br = [0.0f32; BLOCK_SIZE];
            self.process_block_stereo(&mut bl[..chunk], &mut br[..chunk]);
            each(self, &bl[..chunk], &br[..chunk]);
            out_l.extend_from_slice(&bl[..chunk]);
            out_r.extend_from_slice(&br[..chunk]);
            rendered += chunk;
        }

        (out_l, out_r)
    }

    /// Calculate total bars from arrangement.
    /// Render `steps` sequencer steps from the current position without
    /// restarting. Call `start()` first. Useful for tests and previews.
    pub fn render_steps(&mut self, steps: usize) -> (Vec<f32>, Vec<f32>) {
        let total = (steps as f32 * self.samples_per_step) as usize;
        let mut out_l = vec![0.0f32; total];
        let mut out_r = vec![0.0f32; total];
        let mut pos = 0;
        while pos < total {
            let chunk = BLOCK_SIZE.min(total - pos);
            self.process_block_stereo(&mut out_l[pos..pos + chunk], &mut out_r[pos..pos + chunk]);
            pos += chunk;
        }
        (out_l, out_r)
    }

    pub fn tempo(&self) -> f32 {
        self.tempo
    }
    /// Sixteenths in a bar: 16 in 4/4, 12 in 3/4. A set counts its steps in
    /// bars and a step can change the tempo or the meter, so the length of a
    /// bar has to be asked of the engine that is about to play it.
    pub fn steps_per_bar(&self) -> usize {
        self.steps_per_bar
    }

    pub fn reset(&mut self) {
        for track in self.tracks.iter_mut() {
            if let Some(arp) = track.arp.as_mut() {
                arp.reset();
            }
        }
        self.running = false;
        self.sample_counter = 0.0;
        self.current_step_duration = self.samples_per_step;
        self.timing_rng = Rng::new(7919);
        self.reseed_tracks();
        self.arrangement_idx = 0;
        self.arrangement_bar_count = 0;
        self.current_bar = 0;
        self.global_step = 0;
        self.sc_envelope = 0.0;
        self.reverb_wet_level = 1.0;
        self.delay_wet_level = 1.0;
        self.active_automations.clear();
        self.scene_step = 0;
        self.scene_total_steps = 0;

        for inst in self.instruments.iter_mut() {
            inst.reset();
        }
        for bus in self.buses.iter_mut() {
            bus.reset();
        }
        self.send_delay.reset();
        self.send_reverb.reset();
        self.master_fx.reset();
        self.output.reset();
        self.reverb_return.reset();
        self.delay_return.reset();
        for track in self.tracks.iter_mut() {
            track.current_step = 0;
            track.current_notes_count = 0;
            track.gate_samples_remaining = 0.0;
        }
        self.snap_mix();
    }

    /// Put every glide where it is going. Starting, or jumping to a bar, is
    /// not a change anyone should hear arrive.
    fn snap_mix(&mut self) {
        for t in self.tracks.iter_mut() {
            t.heard = t.target();
            t.heard_duck = t.sidechain_amount.unwrap_or(self.sidechain_amount);
        }
        self.reverb_wet_heard = self.reverb_wet_level;
        self.delay_wet_heard = self.delay_wet_level;
    }

    pub fn global_step(&self) -> usize {
        self.global_step
    }

    /// Where playback is, in bars from the top, to the sample: the steps
    /// that have sounded and how far into the current one the clock is. A
    /// scripted performance times its events on this.
    pub fn position_bars(&self) -> f64 {
        if self.global_step == 0 || self.steps_per_bar == 0 {
            return 0.0;
        }
        let within = if self.current_step_duration > 0.0 && self.current_step_duration < f32::MAX {
            (self.sample_counter / self.current_step_duration).clamp(0.0, 1.0) as f64
        } else {
            0.0
        };
        ((self.global_step - 1) as f64 + within) / self.steps_per_bar as f64
    }
    pub fn running(&self) -> bool {
        self.running
    }
    pub fn current_bar(&self) -> usize {
        self.current_bar
    }

    /// Start playback from a specific bar (for hot-swap continuity).
    /// Fast-forwards through the arrangement to land on the right scene.
    pub fn start_from_bar(&mut self, bar: usize) {
        self.running = true;
        self.timing_rng = Rng::new(7919);
        self.reseed_tracks();
        self.current_step_duration = self.effective_step_samples(0);
        self.sample_counter = self.current_step_duration;
        self.reverb_wet_level = 1.0;
        self.delay_wet_level = 1.0;
        self.active_automations.clear();
        self.scene_step = 0;
        self.scene_total_steps = 0;

        for inst in self.instruments.iter_mut() {
            inst.set_bpm(self.tempo);
        }

        // Fast-forward arrangement to the target bar
        self.arrangement_idx = 0;
        self.arrangement_bar_count = 0;
        self.current_bar = 0;
        self.global_step = 0;

        if !self.arrangement.is_empty() {
            let mut bars_remaining = bar;
            while self.arrangement_idx < self.arrangement.len() {
                let (_, repeat) = self.arrangement[self.arrangement_idx];
                let scene_bars = repeat as usize;
                if bars_remaining < scene_bars {
                    self.arrangement_bar_count = bars_remaining as u32;
                    break;
                }
                bars_remaining -= scene_bars;
                self.arrangement_idx += 1;
            }
            // Clamp to last scene if past the end
            if self.arrangement_idx >= self.arrangement.len() {
                self.arrangement_idx = self.arrangement.len() - 1;
                self.arrangement_bar_count = 0;
            }
            let (scene_idx, _) = self.arrangement[self.arrangement_idx];
            self.apply_scene(scene_idx);
            // `apply_scene` starts the scene clock at zero. Landing mid-scene
            // must not restart its automation: a sweep that was three bars in
            // stays three bars in.
            self.scene_step = self.arrangement_bar_count as usize * self.steps_per_bar;
        } else {
            // Top-level lanes count from the bar the engine starts on: the
            // first bar this text plays.
            self.lane_origin = bar * self.steps_per_bar;
            self.start_lanes();
        }

        self.current_bar = bar;
        self.global_step = bar * self.steps_per_bar;
        self.reassert_held();
        self.snap_mix();

        for track in self.tracks.iter_mut() {
            track.current_step = 0;
            track.current_notes_count = 0;
            track.gate_samples_remaining = 0.0;
        }
    }
}
