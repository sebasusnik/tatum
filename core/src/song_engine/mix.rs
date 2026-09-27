//! The render loop: one block of the whole song, from the step clock through
//! instruments, inserts, sidechain, buses and sends to the master chain.

use crate::primitives::arp_processor::ArpEvent;
use crate::{math, BLOCK_SIZE};

use super::instrument::SongInstrument;
use super::scene::SCENE_FADE;
use super::sequencer::NO_RELEASE;
use super::track::{glide, settle};
use super::SongEngine;

impl SongEngine {
    /// Process one stereo block.
    pub fn process_block_stereo(&mut self, output_l: &mut [f32], output_r: &mut [f32]) {
        let len = output_l.len().min(BLOCK_SIZE);

        // Clear output
        for i in 0..len {
            output_l[i] = 0.0;
            output_r[i] = 0.0;
        }

        if !self.running {
            return;
        }
        let mut taps = self.taps.take();
        if let Some(t) = taps.as_deref_mut() {
            t.len = len;
            for b in t.tracks.iter_mut().chain(t.dry.iter_mut()).chain(t.buses.iter_mut()) {
                b.clear(len);
            }
            t.delay.clear(len);
            t.reverb.clear(len);
        }

        // Per-track instrument render buffers (L and R), borrowed from the engine so
        // the block allocates nothing. Instruments write the whole slice they are
        // given, so no clearing is needed between blocks.
        let track_count = self.tracks.len();
        let mut track_bufs_l = core::mem::take(&mut self.track_bufs_l);
        let mut track_bufs_r = core::mem::take(&mut self.track_bufs_r);

        // Step sequencer: process sample-by-sample for accurate timing
        for _s in 0..len {
            self.sample_counter += 1.0;

            // Step advance FIRST — so Tie can extend gate before gate-off check
            if self.sample_counter >= self.current_step_duration {
                self.sample_counter -= self.current_step_duration;
                self.advance_step();
                // Pre-compute next step's duration
                self.current_step_duration = self.effective_step_samples(self.global_step);
                // Apply timing humanization as micro-offset on step duration
                if self.humanize_timing > 0.0 {
                    let max_offset = self.samples_per_step * 0.04; // max ±4% of step
                    let offset = self.timing_rng.next_bipolar() * self.humanize_timing * max_offset;
                    self.current_step_duration += offset;
                }
            }

            // Process pending nudge triggers (delayed drum hits from groove blocks)
            let mut i = 0;
            while i < self.pending_triggers.len() {
                self.pending_triggers[i].samples -= 1.0;
                if self.pending_triggers[i].samples <= 0.0 {
                    let t = self.pending_triggers.swap_remove(i);
                    if t.inst_idx < self.instruments.len() {
                        if t.slide && self.instruments[t.inst_idx].slide_to(t.midi_note, t.velocity) {
                            // glided, nothing to release
                        } else {
                            if t.release != NO_RELEASE {
                                self.instruments[t.inst_idx].note_off(t.release);
                            }
                            self.instruments[t.inst_idx].note_on(t.midi_note, t.velocity);
                        }
                    }
                } else {
                    i += 1;
                }
            }

            // Gate-off handling (after step advance, so Tie extends before expiry)
            for ti in 0..track_count {
                if !self.tracks[ti].active {
                    continue;
                }
                if self.tracks[ti].gate_samples_remaining > 0.0 {
                    self.tracks[ti].gate_samples_remaining -= 1.0;
                    if self.tracks[ti].gate_samples_remaining <= 0.0 {
                        let inst_idx = self.tracks[ti].instrument_idx;
                        if self.tracks[ti].arp.is_some() {
                            Self::release_track_notes(&mut self.tracks[ti], &mut self.instruments);
                            continue;
                        }
                        let count = self.tracks[ti].current_notes_count as usize;
                        if count > 0 && inst_idx < self.instruments.len() {
                            for ni in 0..count {
                                self.instruments[inst_idx].note_off(self.tracks[ti].current_notes[ni]);
                            }
                            self.tracks[ti].current_notes_count = 0;
                        }
                    }
                }
            }

            // Arpeggiators: one tick per sample, events go straight to the instrument
            for ti in 0..track_count {
                if !self.tracks[ti].active {
                    continue;
                }
                let inst_idx = self.trigger_instrument(ti);
                if inst_idx >= self.instruments.len() {
                    continue;
                }
                if let Some(arp) = self.tracks[ti].arp.as_mut() {
                    match arp.tick() {
                        Some(ArpEvent::NoteOn(n, v)) => self.instruments[inst_idx].note_on(n, v),
                        Some(ArpEvent::NoteOff(n)) => self.instruments[inst_idx].note_off(n),
                        None => {}
                    }
                }
            }
        }

        // A muted track with nothing left ringing costs nothing: no voices, no
        // insert chain, no sidechain read, no mix. `level` is a fast-path edit
        // while scene membership is a structural one, so this is what lets a
        // live set keep a rig of voices loaded and bring them in without a swap.
        for ti in 0..track_count {
            // A ghost kick -- `level 0` but feeding the sidechain so it ducks the
            // mix without being heard -- is a real idiom, and it has to keep
            // running. Muting the fader does not mute the trigger.
            let muted = self.tracks[ti].active && self.tracks[ti].level <= 0.0 && !self.tracks[ti].is_sc_source;
            // Pulling the fader releases what is held, the way a mute does.
            // Without this a tied note never ends: the tie branch extends the
            // gate every step whether or not the track fired the note, so a
            // held chord would keep one voice alive forever and the track
            // would never become idle enough to skip.
            if muted && self.tracks[ti].current_notes_count > 0 {
                Self::release_track_notes(&mut self.tracks[ti], &mut self.instruments);
            }
            let silent = muted && self.instruments.get(self.tracks[ti].instrument_idx).is_some_and(|i| i.is_idle());
            // Clear the chain on the way in, so a delay or reverb sitting in it
            // does not come back stale when the track is brought back.
            if silent && !self.track_silent[ti] {
                self.tracks[ti].insert_fx.reset();
            }
            self.track_silent[ti] = silent;
        }

        // Render each track's instrument (stereo-aware)
        for ti in 0..track_count {
            if !self.tracks[ti].sounding() || self.track_silent[ti] {
                continue;
            }
            let inst_idx = self.tracks[ti].instrument_idx;
            if inst_idx < self.instruments.len() {
                let is_stereo = self.instruments[inst_idx]
                    .process_block_stereo(&mut track_bufs_l[ti][..len], &mut track_bufs_r[ti][..len]);
                if !is_stereo {
                    // Mono instrument rendered into L; copy to R
                    track_bufs_r[ti][..len].copy_from_slice(&track_bufs_l[ti][..len]);
                }
                if let Some(t) = taps.as_deref_mut() {
                    t.dry[ti].l[..len].copy_from_slice(&track_bufs_l[ti][..len]);
                    t.dry[ti].r[..len].copy_from_slice(&track_bufs_r[ti][..len]);
                }
            }
        }

        // Apply insert FX per track (mono processing applied to both channels)
        for ti in 0..track_count {
            if !self.tracks[ti].sounding() || self.track_silent[ti] {
                continue;
            }
            if self.tracks[ti].insert_fx.nodes.is_empty() {
                continue;
            }
            for s in 0..len {
                let (fl, fr) = self.tracks[ti].insert_fx.process_stereo(track_bufs_l[ti][s], track_bufs_r[ti][s]);
                track_bufs_l[ti][s] = fl;
                track_bufs_r[ti][s] = fr;
            }
        }

        // Sidechain ducking: use kick track to duck other tracks
        // A track's own amount overrides the song's, zero included.
        let has_any_sidechain = self.sidechain_amount > 0.0
            || self.reverb_sidechain > 0.0
            || self.delay_sidechain > 0.0
            || self
                .tracks
                .iter()
                .any(|t| t.active && (t.sidechain_amount.is_some_and(|a| a > 0.0) || t.heard_duck > 0.0));
        let (sc_attack, sc_release) = (self.sc_attack_coeff, self.sc_release_coeff);
        if has_any_sidechain {
            for s in 0..len {
                // Each source keeps its own envelope, so `sidechain from=bass`
                // breathes with the bass while the drums still duck the pads.
                for (si, buf_l) in track_bufs_l[..track_count].iter().enumerate() {
                    if !self.tracks[si].is_sc_source || !self.tracks[si].active {
                        continue;
                    }
                    // A Beats track uses its kick envelope rather than the raw
                    // signal, so hats and snares do not duck the mix.
                    let level = if let SongInstrument::Beats(ref m) = self.instruments[self.tracks[si].instrument_idx] {
                        if s < m.kick_env.len() {
                            m.kick_env[s]
                        } else {
                            0.0
                        }
                    } else {
                        math::abs(buf_l[s])
                    };
                    let env = self.tracks[si].sc_env;
                    self.tracks[si].sc_env =
                        if level > env { env + (level - env) * sc_attack } else { sc_release * env };
                }
                // The sends follow the kick, or the song's named source.
                self.sc_envelope = self.global_sc_idx.map_or(0.0, |si| self.tracks[si].sc_env);
                self.sc_curve[s] = self.sc_envelope;
                for ti in 0..track_count {
                    if !self.tracks[ti].active {
                        continue;
                    }
                    let Some(si) = self.tracks[ti].sc_source else { continue };
                    let target = self.tracks[ti].sidechain_amount.unwrap_or(self.sidechain_amount);
                    let amount = glide(&mut self.tracks[ti].heard_duck, target);
                    if amount > 0.0 {
                        let duck = 1.0 - amount * self.tracks[si].sc_env;
                        track_bufs_l[ti][s] *= duck;
                        track_bufs_r[ti][s] *= duck;
                    }
                }
            }
            for t in self.tracks.iter_mut() {
                let target = t.sidechain_amount.unwrap_or(self.sidechain_amount);
                if t.sc_source.is_none() {
                    t.heard_duck = target
                } else {
                    settle(&mut t.heard_duck, target)
                }
            }
        } else {
            self.sc_curve[..len].fill(self.sc_envelope);
        }

        // Clear bus buffers
        for bus in self.buses.iter_mut() {
            bus.clear(len);
        }

        // Send effect accumulation buffers
        let mut delay_in_l = [0.0f32; BLOCK_SIZE];
        let mut delay_in_r = [0.0f32; BLOCK_SIZE];
        let mut reverb_in_l = [0.0f32; BLOCK_SIZE];
        let mut reverb_in_r = [0.0f32; BLOCK_SIZE];

        // Mix tracks into master + bus sends + global sends with level and panning
        // NOTE: velocity is already baked into the instrument output (via voice.velocity
        // which = step_vel * track_vel, set in note_on). Only apply track level here.
        self.apply_automation();

        let band_metering = self.band_metering;
        for ti in 0..track_count {
            if !self.tracks[ti].sounding() || self.track_silent[ti] {
                continue;
            }
            // A leaving track fades over the block from where it is; the
            // fade is linear, which over ten milliseconds is inaudible as a
            // shape and only removes the corner.
            let (fade_from, fade_step) = match self.tracks[ti].leaving {
                0 => (1.0, 0.0),
                left => {
                    let n = left.min(len as u32);
                    self.tracks[ti].leaving -= n;
                    (left as f32 / SCENE_FADE as f32, 1.0 / SCENE_FADE as f32)
                }
            };
            if fade_step > 0.0 {
                for s in 0..len {
                    let g = (fade_from - s as f32 * fade_step).max(0.0);
                    track_bufs_l[ti][s] *= g;
                    track_bufs_r[ti][s] *= g;
                }
            }
            let target = self.tracks[ti].target();
            let mut heard = self.tracks[ti].heard;
            let mut tap = taps.as_deref_mut().map(|t| (&mut t.dry[ti], &mut t.tracks[ti]));
            for s in 0..len {
                heard.glide(&target);
                let (gain_l, gain_r, d_send, r_send) = (heard.left, heard.right, heard.delay, heard.reverb);
                if let Some((dry, wet)) = tap.as_mut() {
                    dry.l[s] *= gain_l;
                    dry.r[s] *= gain_r;
                    wet.l[s] = track_bufs_l[ti][s] * gain_l;
                    wet.r[s] = track_bufs_r[ti][s] * gain_r;
                }
                // Both channels always: mono sources were copied to R before the
                // insert chain, and stereo inserts (autopan, chorus) rely on R.
                let (sample_l, sample_r) = (track_bufs_l[ti][s] * gain_l, track_bufs_r[ti][s] * gain_r);
                let t = &mut self.tracks[ti];
                t.meter_peak = t.meter_peak.max(sample_l.abs()).max(sample_r.abs());
                t.meter_sum_sq += (sample_l * sample_l + sample_r * sample_r) as f64;
                t.meter_samples += 2;
                let (mid, side) = ((sample_l + sample_r) as f64, (sample_l - sample_r) as f64);
                t.meter_mid_sq += mid * mid;
                t.meter_side_sq += side * side;
                if band_metering {
                    t.band.push_stereo(sample_l, sample_r);
                }

                // Bus send (mono sum to bus)
                if let Some((bus_idx, amount)) = self.tracks[ti].bus_send {
                    if bus_idx < self.buses.len() {
                        self.buses[bus_idx].buffer[s] += sample_l * amount;
                        self.buses[bus_idx].buffer_r[s] += sample_r * amount;
                    }
                }

                // Global delay/reverb sends (post-pan)
                if d_send > 0.0 {
                    delay_in_l[s] += sample_l * d_send;
                    delay_in_r[s] += sample_r * d_send;
                }
                if r_send > 0.0 {
                    reverb_in_l[s] += sample_l * r_send;
                    reverb_in_r[s] += sample_r * r_send;
                }

                // Direct to master with stereo panning
                if self.tracks[ti].to_master {
                    output_l[s] += sample_l;
                    output_r[s] += sample_r;
                }
            }
            heard.settle(&target);
            self.tracks[ti].heard = heard;
        }

        // Process bus FX chains and mix into master
        // Always, even on silence. Gating on a non-zero input truncated the
        // tail of any reverb or delay on a bus the instant its tracks stopped,
        // and made `capture(start=N)` count processed samples rather than bars,
        // so its window landed on the wrong music. The global sends have always
        // been ticked unconditionally for the same reason.
        for (bi, bus) in self.buses.iter_mut().enumerate() {
            let mut tap = taps.as_deref_mut().and_then(|t| t.buses.get_mut(bi));
            for s in 0..len {
                let (pl, pr) = bus.fx_chain.process_stereo(bus.buffer[s], bus.buffer_r[s]);
                if let Some(b) = tap.as_deref_mut() {
                    b.l[s] = pl;
                    b.r[s] = pr;
                }
                bus.meter_peak = bus.meter_peak.max(math::abs(pl)).max(math::abs(pr));
                bus.meter_sum_sq += ((pl * pl + pr * pr) * 0.5) as f64;
                bus.meter_samples += 1;
                output_l[s] += pl;
                output_r[s] += pr;
            }
        }

        // Process global send effects (wet-only returns, scaled by wet levels)
        let (dwet_to, rwet_to) = (self.delay_wet_level, self.reverb_wet_level);
        // Always tick the sends: their tails must ring out (and freeze must
        // hold) after every track has gone silent.
        let duck_delay = self.delay_sidechain;
        let duck_reverb = self.reverb_sidechain;
        for s in 0..len {
            let sc = self.sc_curve[s];
            let dwet = glide(&mut self.delay_wet_heard, dwet_to);
            let rwet = glide(&mut self.reverb_wet_heard, rwet_to);
            let (mut dl, mut dr) = self.send_delay.process_stereo_wet(delay_in_l[s], delay_in_r[s]);
            if !self.delay_return.nodes.is_empty() {
                (dl, dr) = self.delay_return.process_stereo(dl, dr);
            }
            if duck_delay > 0.0 {
                let g = 1.0 - duck_delay * sc;
                dl *= g;
                dr *= g;
            }
            output_l[s] += dl * dwet;
            output_r[s] += dr * dwet;
            if let Some(t) = taps.as_deref_mut() {
                t.delay.l[s] = dl * dwet;
                t.delay.r[s] = dr * dwet;
            }
            let (mut rl, mut rr) = self.send_reverb.process_stereo_in_wet(reverb_in_l[s], reverb_in_r[s]);
            if !self.reverb_return.nodes.is_empty() {
                (rl, rr) = self.reverb_return.process_stereo(rl, rr);
            }
            if duck_reverb > 0.0 {
                let g = 1.0 - duck_reverb * sc;
                rl *= g;
                rr *= g;
            }
            output_l[s] += rl * rwet;
            output_r[s] += rr * rwet;
            if let Some(t) = taps.as_deref_mut() {
                t.reverb.l[s] = rl * rwet;
                t.reverb.r[s] = rr * rwet;
            }
        }
        settle(&mut self.delay_wet_heard, dwet_to);
        settle(&mut self.reverb_wet_heard, rwet_to);

        // Master level and the master chain. There used to be a gain here too
        // that scaled each scene by 1/sqrt(its active tracks), so a breakdown
        // of two tracks played 4 dB over a drop of five at the same levels.
        // It kept full scenes from clipping before the engine levelled and
        // limited every song; after that, all it did was bend the shape a
        // song wrote, and make `level` mean something different per scene.
        let g = self.master_level;
        let chain = !self.master_fx.nodes.is_empty();
        for s in 0..len {
            let (in_l, in_r) = (output_l[s] * g, output_r[s] * g);
            self.master_in_peak = self.master_in_peak.max(math::abs(in_l)).max(math::abs(in_r));
            self.master_in_sum_sq += (in_l * in_l + in_r * in_r) as f64;
            self.master_in_samples += 2;
            (output_l[s], output_r[s]) = if chain { self.master_fx.process_stereo(in_l, in_r) } else { (in_l, in_r) };
        }

        for s in 0..len {
            (output_l[s], output_r[s]) = self.output.process(output_l[s], output_r[s]);
        }

        self.track_bufs_l = track_bufs_l;
        self.track_bufs_r = track_bufs_r;
        if let Some(t) = taps.as_deref_mut() {
            for (h, tr) in t.held.iter_mut().zip(self.tracks.iter()) {
                *h = tr.active && (tr.current_notes_count > 0 || tr.gate_samples_remaining > 0.0);
            }
        }
        self.taps = taps;
    }
}
