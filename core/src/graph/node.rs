use crate::primitives::oscillator::{Oscillator, Waveform};
use crate::primitives::envelope::Envelope;
use crate::primitives::filter::{BiquadFilter, FilterType, LadderFilter};
use crate::primitives::lfo::Lfo;
use crate::primitives::noise::NoiseGen;
use crate::effects::saturator::Saturator;
use crate::effects::chorus::Chorus;
use crate::effects::phaser::Phaser;
use crate::effects::formant::Formant;
use crate::effects::bitcrusher::Bitcrusher;
use crate::effects::compressor::Compressor;
use crate::effects::limiter::Limiter;
use crate::effects::eq::{TiltEq, ThreeBandEq};
use crate::effects::delay::Delay;
use crate::effects::reverb::DattorroReverb;
use crate::effects::capture::Capture;
use crate::SAMPLE_RATE;

/// Maximum number of inputs a single node can accept.
pub const MAX_NODE_INPUTS: usize = 8;

// ── NodeSpec: lightweight description for creating nodes ──

/// Describes how to create a node. Stored in GraphTemplate.
/// Small, Copy-able, no heap allocations.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NodeSpec {
    Osc { waveform: Waveform, freq: f32, drift_seed: u32, fixed: bool, pitch_semitones: f32 },
    /// Oscillator with built-in pitch envelope (exponential sweep from start→end).
    /// Used for drum body synthesis (kick, tom). Not MIDI-note tracked.
    PitchOsc { waveform: Waveform, start_freq: f32, end_freq: f32, decay: f32 },
    Noise { seed: u32 },
    Lfo { rate: f32, depth: f32 },
    Env { a: f32, d: f32, s: f32, r: f32 },
    Biquad {
        filter_type: FilterType, cutoff: f32, resonance: f32,
        env_attack: f32, env_decay: f32, env_sustain: f32, env_release: f32,
        env_depth: f32,
        lfo: FilterLfo,
    },
    Ladder {
        cutoff: f32, resonance: f32,
        env_attack: f32, env_decay: f32, env_sustain: f32, env_release: f32,
        env_depth: f32,
        lfo: FilterLfo,
    },
    /// Stereo auto-panner: slow LFO moves the signal between L and R.
    AutoPan { hz: f32, bars: f32, depth: f32 },
    /// Swept allpass phaser.
    Phaser { mix: f32, hz: f32, bars: f32, stages: u8, feedback: f32, depth: f32 },
    /// Vowel formant filter morphing between two vowels.
    Vowel { from: u8, to: u8, hz: f32, bars: f32, mix: f32 },
    Mix,
    Gain { amount: f32 },
    Vca,
    Saturator { drive: f32 },
    Chorus { mix: f32 },
    Bitcrusher { bits: f32, rate: f32 },
    Compressor { threshold_db: f32, ratio: f32, attack_ms: f32, release_ms: f32, makeup: f32 },
    Limiter { threshold: f32 },
    TiltEq { amount: f32 },
    ThreeBandEq { low: f32, mid: f32, high: f32 },
    /// Delay (bus/master chain only, not per-voice).
    Delay { sync_div: f32, feedback: f32 },
    /// Reverb (bus/master chain only, not per-voice).
    Reverb { room_size: f32 },
    /// Record a window of the chain's output and loop it back. The window is
    /// already in samples: the compiler resolves `bars` against the song's
    /// slowest tempo so the buffer can be allocated when the chain is built.
    Capture { samples: u32, start_samples: u32, speed: f32, reverse: bool, mix: f32 },
    Output,
}

/// LFO settings on a filter node. `bars` > 0 syncs one cycle to that many
/// bars (needs the chain's tempo); otherwise `hz` is used. `depth` in Hz.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FilterLfo {
    pub hz: f32,
    pub bars: f32,
    pub depth: f32,
}

fn make_lfo(hz: f32) -> Lfo {
    let mut lfo = Lfo::new(SAMPLE_RATE);
    lfo.set_rate(if hz > 0.0 { hz } else { 0.25 });
    lfo.set_depth(1.0);
    lfo.set_waveform(crate::primitives::lfo::LfoWaveform::Sine);
    lfo
}

impl NodeSpec {
    /// Convenience: ladder with no filter envelope (backward compat).
    pub fn ladder(cutoff: f32, resonance: f32) -> Self {
        NodeSpec::Ladder {
            cutoff, resonance,
            env_attack: 0.005, env_decay: 0.2, env_sustain: 0.0, env_release: 0.1,
            env_depth: 0.0,
            lfo: FilterLfo::default(),
        }
    }

    /// Convenience: biquad with no filter envelope (backward compat).
    pub fn biquad(filter_type: FilterType, cutoff: f32, resonance: f32) -> Self {
        NodeSpec::Biquad {
            filter_type, cutoff, resonance,
            env_attack: 0.005, env_decay: 0.2, env_sustain: 0.0, env_release: 0.1,
            env_depth: 0.0,
            lfo: FilterLfo::default(),
        }
    }

    /// Create a fresh NodeKind from this spec.
    pub fn instantiate(&self) -> NodeKind {
        match *self {
            NodeSpec::Osc { waveform, freq, drift_seed, .. } => {
                let mut osc = Oscillator::new(waveform, SAMPLE_RATE);
                osc.set_frequency(freq);
                osc.set_drift_seed(drift_seed);
                NodeKind::Osc(osc)
            }
            NodeSpec::PitchOsc { waveform, start_freq, end_freq, decay } => {
                let mut osc = Oscillator::new(waveform, SAMPLE_RATE);
                osc.set_frequency(start_freq);
                osc.set_drift_enabled(false); // drums don't need drift
                NodeKind::PitchOsc(PitchOscState {
                    osc,
                    current_freq: start_freq,
                    end_freq,
                    decay,
                })
            }
            NodeSpec::Noise { seed } => NodeKind::Noise(NoiseGen::new(seed)),
            NodeSpec::Lfo { rate, depth } => {
                let mut lfo = Lfo::new(SAMPLE_RATE);
                lfo.set_rate(rate);
                lfo.set_depth(depth);
                NodeKind::Lfo(lfo)
            }
            NodeSpec::Env { a, d, s, r } => {
                let mut env = Envelope::new(SAMPLE_RATE);
                env.set_adsr(a, d, s, r);
                NodeKind::Env(env)
            }
            NodeSpec::Biquad { filter_type, cutoff, resonance,
                               env_attack, env_decay, env_sustain, env_release, env_depth, lfo } => {
                let mut f = BiquadFilter::new(SAMPLE_RATE);
                f.set_params(filter_type, cutoff, resonance);
                let mut env = Envelope::new(SAMPLE_RATE);
                env.set_adsr(env_attack, env_decay, env_sustain, env_release);
                NodeKind::Biquad(ModBiquad {
                    filter: f, env, base_cutoff: cutoff, env_depth, velocity: 1.0,
                    lfo: make_lfo(lfo.hz), lfo_depth: lfo.depth, lfo_bars: lfo.bars, lfo_tick: 0,
                })
            }
            NodeSpec::AutoPan { hz, bars, depth } => {
                NodeKind::AutoPan { lfo: make_lfo(hz), bars, depth }
            }
            NodeSpec::Phaser { mix, hz, bars, stages, feedback, depth } => {
                NodeKind::Phaser(Phaser::new(mix, hz, bars, stages as usize, feedback, depth))
            }
            NodeSpec::Capture { samples, start_samples, speed, reverse, mix } => {
                NodeKind::Capture(Capture::new(samples, start_samples, speed, reverse, mix))
            }
            NodeSpec::Vowel { from, to, hz, bars, mix } => {
                NodeKind::Vowel(Formant::new(from, to, hz, bars, mix))
            }
            NodeSpec::Ladder { cutoff, resonance,
                               env_attack, env_decay, env_sustain, env_release, env_depth, lfo } => {
                let mut f = LadderFilter::new(SAMPLE_RATE);
                f.set_params(cutoff, resonance);
                let mut env = Envelope::new(SAMPLE_RATE);
                env.set_adsr(env_attack, env_decay, env_sustain, env_release);
                NodeKind::Ladder(ModLadder {
                    filter: f, env, base_cutoff: cutoff, env_depth, velocity: 1.0,
                    lfo: make_lfo(lfo.hz), lfo_depth: lfo.depth, lfo_bars: lfo.bars, lfo_tick: 0,
                })
            }
            NodeSpec::Mix => NodeKind::Mix,
            NodeSpec::Gain { amount } => NodeKind::Gain(amount),
            NodeSpec::Vca => NodeKind::Vca,
            NodeSpec::Saturator { drive } => NodeKind::Saturator(Saturator::new(drive)),
            NodeSpec::Chorus { mix } => {
                let mut c = Chorus::new();
                c.set_mix(mix);
                NodeKind::Chorus(c)
            }
            NodeSpec::Bitcrusher { bits, rate } => {
                let mut bc = Bitcrusher::new();
                bc.set_bit_depth(bits);
                bc.set_rate_reduce(rate);
                NodeKind::Bitcrusher(bc)
            }
            NodeSpec::Compressor { threshold_db, ratio, attack_ms, release_ms, makeup } => {
                let mut c = Compressor::new(SAMPLE_RATE);
                c.set_threshold(threshold_db);
                c.set_ratio(ratio);
                c.set_attack(attack_ms);
                c.set_release(release_ms);
                c.set_makeup(makeup);
                NodeKind::Compressor(c)
            }
            NodeSpec::Limiter { threshold } => {
                let mut l = Limiter::new(SAMPLE_RATE);
                l.set_threshold(threshold);
                NodeKind::Limiter(l)
            }
            NodeSpec::TiltEq { amount } => {
                let mut eq = TiltEq::new(SAMPLE_RATE);
                eq.set_tilt(amount);
                NodeKind::TiltEq(eq)
            }
            NodeSpec::ThreeBandEq { low, mid, high } => {
                let mut eq = ThreeBandEq::new(SAMPLE_RATE);
                eq.set_low(low);
                eq.set_mid(mid);
                eq.set_high(high);
                NodeKind::ThreeBandEq(eq)
            }
            NodeSpec::Delay { feedback, .. } => {
                let mut d = Delay::new(SAMPLE_RATE, 2.0);
                d.set_feedback(feedback);
                d.set_mix(1.0); // wet-only for sends
                NodeKind::Delay(d)
            }
            NodeSpec::Reverb { room_size } => {
                let mut r = DattorroReverb::new(SAMPLE_RATE);
                r.set_room_size(room_size);
                r.set_mix(1.0); // wet-only for sends
                NodeKind::Reverb(r)
            }
            NodeSpec::Output => NodeKind::Output,
        }
    }

    pub fn is_oscillator(&self) -> bool {
        matches!(self, NodeSpec::Osc { fixed: false, .. })
    }

    pub fn is_pitch_osc(&self) -> bool {
        matches!(self, NodeSpec::PitchOsc { .. })
    }

    pub fn is_envelope(&self) -> bool {
        matches!(self, NodeSpec::Env { .. })
    }

    pub fn is_filter(&self) -> bool {
        matches!(self, NodeSpec::Ladder { .. } | NodeSpec::Biquad { .. })
    }

    pub fn has_filter_env(&self) -> bool {
        match self {
            NodeSpec::Ladder { env_depth, .. } | NodeSpec::Biquad { env_depth, .. } => *env_depth > 0.0,
            _ => false,
        }
    }
}

// ── Modulated filter wrappers ──

/// Ladder filter with built-in filter envelope for per-note cutoff sweeps.
pub struct ModLadder {
    pub filter: LadderFilter,
    pub env: Envelope,
    pub base_cutoff: f32,
    pub env_depth: f32,
    pub velocity: f32,
    pub lfo: Lfo,
    pub lfo_depth: f32,
    pub lfo_bars: f32,
    pub lfo_tick: u32,
}

impl ModLadder {
    pub fn gate_on(&mut self) {
        if self.env_depth > 0.0 { self.env.gate_on(); }
    }

    pub fn gate_off(&mut self) {
        if self.env_depth > 0.0 { self.env.gate_off(); }
    }

    pub fn set_velocity(&mut self, vel: f32) {
        self.velocity = vel;
    }

    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let has_env = self.env_depth > 0.0;
        let has_lfo = self.lfo_depth > 0.0;
        if has_env || has_lfo {
            let mut cutoff = self.base_cutoff;
            if has_env {
                cutoff += self.env.next_sample() * self.env_depth;
            }
            if has_lfo {
                // Cheap: advance the LFO every sample, retune the filter every 8.
                let v = self.lfo.next_sample();
                self.lfo_tick = self.lfo_tick.wrapping_add(1);
                if self.lfo_tick % 8 != 0 && !has_env {
                    return self.filter.process(input);
                }
                cutoff += v * self.lfo_depth;
            }
            self.filter.set_cutoff(cutoff);
        }
        self.filter.process(input)
    }

    /// Retune a bar-synced LFO to the song tempo (4/4).
    pub fn set_bpm(&mut self, bpm: f32) {
        if self.lfo_bars > 0.0 {
            self.lfo.set_rate(bpm / 60.0 / 4.0 / self.lfo_bars);
        }
    }

    pub fn reset(&mut self) {
        self.filter.reset();
        self.env.reset();
    }
}

/// Biquad filter with built-in filter envelope for per-note cutoff sweeps.
pub struct ModBiquad {
    pub filter: BiquadFilter,
    pub env: Envelope,
    pub base_cutoff: f32,
    pub env_depth: f32,
    pub velocity: f32,
    pub lfo: Lfo,
    pub lfo_depth: f32,
    pub lfo_bars: f32,
    pub lfo_tick: u32,
}

impl ModBiquad {
    pub fn gate_on(&mut self) {
        if self.env_depth > 0.0 { self.env.gate_on(); }
    }

    pub fn gate_off(&mut self) {
        if self.env_depth > 0.0 { self.env.gate_off(); }
    }

    pub fn set_velocity(&mut self, vel: f32) {
        self.velocity = vel;
    }

    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let has_env = self.env_depth > 0.0;
        let has_lfo = self.lfo_depth > 0.0;
        if has_env || has_lfo {
            let mut cutoff = self.base_cutoff;
            if has_env {
                cutoff += self.env.next_sample() * self.env_depth;
            }
            if has_lfo {
                // Cheap: advance the LFO every sample, retune the filter every 8.
                let v = self.lfo.next_sample();
                self.lfo_tick = self.lfo_tick.wrapping_add(1);
                if self.lfo_tick % 8 != 0 && !has_env {
                    return self.filter.process(input);
                }
                cutoff += v * self.lfo_depth;
            }
            self.filter.set_cutoff(cutoff);
        }
        self.filter.process(input)
    }

    /// Retune a bar-synced LFO to the song tempo (4/4).
    pub fn set_bpm(&mut self, bpm: f32) {
        if self.lfo_bars > 0.0 {
            self.lfo.set_rate(bpm / 60.0 / 4.0 / self.lfo_bars);
        }
    }

    pub fn reset(&mut self) {
        self.filter.reset();
        self.env.reset();
    }
}

// ── Pitch-envelope oscillator state ──

/// Oscillator with built-in exponential pitch sweep (start_freq → end_freq).
/// Used for drum body synthesis. Fresh instance per note (re-triggers from start_freq).
pub struct PitchOscState {
    pub osc: Oscillator,
    pub current_freq: f32,
    pub end_freq: f32,
    pub decay: f32,
}

impl PitchOscState {
    #[inline]
    pub fn next_sample(&mut self) -> f32 {
        let out = self.osc.next_sample();
        // Exponential pitch sweep toward end_freq
        self.current_freq = self.end_freq + (self.current_freq - self.end_freq) * self.decay;
        self.osc.set_frequency(self.current_freq);
        out
    }

    pub fn reset(&mut self) {
        self.osc.reset();
    }
}

// ── NodeKind: runtime DSP node with full state ──

/// Every DSP primitive and effect as a first-class node.
/// No trait objects, no dynamic dispatch.
pub enum NodeKind {
    // ── Sources ──
    Osc(Oscillator),
    PitchOsc(PitchOscState),
    Noise(NoiseGen),
    Lfo(Lfo),

    // ── Envelopes ──
    Env(Envelope),

    // ── Filters (with optional modulation envelope) ──
    Biquad(ModBiquad),
    Ladder(ModLadder),

    // ── Mixing / routing ──
    Mix,
    Gain(f32),
    Vca,

    // ── Effects ──
    AutoPan { lfo: Lfo, bars: f32, depth: f32 },
    Phaser(Phaser),
    Vowel(Formant),
    Saturator(Saturator),
    Chorus(Chorus),
    Bitcrusher(Bitcrusher),
    Compressor(Compressor),
    Limiter(Limiter),
    TiltEq(TiltEq),
    ThreeBandEq(ThreeBandEq),
    Delay(Delay),
    Reverb(DattorroReverb),
    Capture(Capture),

    // ── Output ──
    Output,
}

impl NodeKind {
    /// Process one sample. Reads from `inputs` (gathered from connected nodes).
    #[inline]
    pub fn process(&mut self, inputs: &[f32; MAX_NODE_INPUTS], input_count: u8) -> f32 {
        match self {
            NodeKind::Osc(osc) => osc.next_sample(),
            NodeKind::PitchOsc(po) => po.next_sample(),
            NodeKind::Noise(noise) => noise.next_sample(),
            NodeKind::Lfo(lfo) => lfo.next_sample(),
            NodeKind::Capture(c) => c.process(inputs[0]),
            NodeKind::Env(env) => env.next_sample(),
            NodeKind::Biquad(m) => m.process(inputs[0]),
            NodeKind::Ladder(m) => m.process(inputs[0]),
            NodeKind::AutoPan { lfo, .. } => { let _ = lfo.next_sample(); inputs[0] }
            NodeKind::Phaser(p) => p.process(inputs[0]),
            NodeKind::Vowel(v) => v.process(inputs[0]),

            NodeKind::Mix => {
                let n = input_count as usize;
                let mut sum = 0.0;
                for i in 0..n {
                    sum += inputs[i];
                }
                sum
            }

            NodeKind::Gain(g) => inputs[0] * *g,
            NodeKind::Vca => inputs[0] * inputs[1],

            NodeKind::Saturator(sat) => sat.process(inputs[0]),
            NodeKind::Chorus(chorus) => chorus.process(inputs[0]),
            NodeKind::Bitcrusher(bc) => {
                let (l, _) = bc.process_stereo(inputs[0], inputs[0]);
                l
            }
            NodeKind::Compressor(comp) => {
                let (l, _) = comp.process_stereo(inputs[0], inputs[0]);
                l
            }
            NodeKind::Limiter(lim) => {
                let (l, _) = lim.process_stereo(inputs[0], inputs[0]);
                l
            }
            NodeKind::TiltEq(eq) => {
                let (l, _) = eq.process_stereo(inputs[0], inputs[0]);
                l
            }
            NodeKind::ThreeBandEq(eq) => {
                let (l, _) = eq.process_stereo(inputs[0], inputs[0]);
                l
            }
            NodeKind::Delay(d) => d.process(inputs[0]),
            NodeKind::Reverb(r) => r.process(inputs[0]),

            NodeKind::Output => inputs[0],
        }
    }

    /// Process a stereo pair through this node.
    /// Effects with native stereo support use it; others fall back to dual-mono.
    #[inline]
    pub fn process_stereo(&mut self, l: f32, r: f32) -> (f32, f32) {
        match self {
            NodeKind::Compressor(comp) => comp.process_stereo(l, r),
            NodeKind::Limiter(lim) => lim.process_stereo(l, r),
            NodeKind::TiltEq(eq) => eq.process_stereo(l, r),
            NodeKind::ThreeBandEq(eq) => eq.process_stereo(l, r),
            NodeKind::Bitcrusher(bc) => bc.process_stereo(l, r),
            NodeKind::Saturator(sat) => (sat.process(l), sat.process(r)),
            NodeKind::Gain(g) => (l * *g, r * *g),
            NodeKind::Phaser(p) => p.process_stereo(l, r),
            // Explicit, because the dual-mono fallback would step the capture's
            // read position twice per stereo sample.
            NodeKind::Capture(c) => c.process_stereo(l, r),
            // Same reason, and it matters more here: calling a delay line twice
            // per stereo sample double-clocks it and runs both channels through
            // one state. A bus reverb came out as wind and rain.
            NodeKind::Delay(d) => d.process_stereo(l, r),
            NodeKind::Reverb(rv) => rv.process_stereo_in(l, r),
            NodeKind::Vowel(v) => v.process_stereo(l, r),
            NodeKind::AutoPan { lfo, depth, .. } => {
                // Equal-power pan driven by the LFO: p in -1..1
                let p = lfo.next_sample() * *depth;
                let angle = (p + 1.0) * 0.25 * crate::math::PI; // 0..pi/2
                let gl = crate::math::cos(angle) * 1.4142;
                let gr = crate::math::sin(angle) * 1.4142;
                let mono = (l + r) * 0.5;
                // Keep the source stereo image, scaled by the pan law
                (l * gl * 0.5 + mono * gl * 0.5, r * gr * 0.5 + mono * gr * 0.5)
            }
            // Dual-mono fallback for everything else
            _ => {
                let mut inp = [0.0f32; MAX_NODE_INPUTS];
                inp[0] = l;
                let ol = self.process(&inp, 1);
                inp[0] = r;
                let or = self.process(&inp, 1);
                (ol, or)
            }
        }
    }

    /// Runtime automation of effect parameters by name (master / bus chains).
    /// Returns false when this node has no such parameter.
    pub fn set_named(&mut self, name: &str, value: f32) -> bool {
        match (self, name) {
            (NodeKind::TiltEq(eq), "tilt") => { eq.set_tilt(value); true }
            (NodeKind::ThreeBandEq(eq), "eq_low") => { eq.set_low(value); true }
            (NodeKind::ThreeBandEq(eq), "eq_mid") => { eq.set_mid(value); true }
            (NodeKind::ThreeBandEq(eq), "eq_high") => { eq.set_high(value); true }
            (NodeKind::Saturator(s), "drive") => { s.set_drive(value); true }
            (NodeKind::Gain(g), "gain") => { *g = value; true }
            (NodeKind::Capture(c), n) => c.set_named(n, value),
            (NodeKind::Limiter(l), "limiter") => { l.set_threshold(value); true }
            (NodeKind::Compressor(c), "comp_threshold") => { c.set_threshold(value); true }
            (NodeKind::Biquad(m), "cutoff") => { m.base_cutoff = value; m.filter.set_cutoff(value); true }
            (NodeKind::Ladder(m), "cutoff") => { m.base_cutoff = value; m.filter.set_cutoff(value); true }
            _ => false,
        }
    }

    /// Retune bar-synced modulation to the song tempo.
    pub fn set_bpm(&mut self, bpm: f32) {
        match self {
            NodeKind::Biquad(m) => m.set_bpm(bpm),
            NodeKind::Ladder(m) => m.set_bpm(bpm),
            NodeKind::AutoPan { lfo, bars, .. } => {
                if *bars > 0.0 { lfo.set_rate(bpm / 60.0 / 4.0 / *bars); }
            }
            NodeKind::Phaser(p) => p.set_bpm(bpm),
            NodeKind::Vowel(v) => v.set_bpm(bpm),
            _ => {}
        }
    }

    pub fn gate_on(&mut self) {
        match self {
            NodeKind::Env(env) => env.gate_on(),
            NodeKind::Ladder(m) => m.gate_on(),
            NodeKind::Biquad(m) => m.gate_on(),
            _ => {}
        }
    }

    pub fn gate_off(&mut self) {
        match self {
            NodeKind::Env(env) => env.gate_off(),
            NodeKind::Ladder(m) => m.gate_off(),
            NodeKind::Biquad(m) => m.gate_off(),
            _ => {}
        }
    }

    pub fn set_frequency(&mut self, freq: f32) {
        if let NodeKind::Osc(osc) = self {
            osc.set_frequency(freq);
        }
    }

    pub fn set_velocity(&mut self, vel: f32) {
        match self {
            NodeKind::Ladder(m) => m.set_velocity(vel),
            NodeKind::Biquad(m) => m.set_velocity(vel),
            _ => {}
        }
    }

    pub fn is_idle(&self) -> bool {
        match self {
            NodeKind::Env(env) => env.is_idle(),
            _ => false,
        }
    }

    pub fn reset(&mut self) {
        match self {
            NodeKind::Osc(osc) => osc.reset(),
            NodeKind::PitchOsc(po) => po.reset(),
            NodeKind::Noise(_) => {}
            NodeKind::Lfo(lfo) => lfo.reset(),
            NodeKind::AutoPan { lfo, .. } => lfo.reset(),
            NodeKind::Capture(c) => c.reset(),
            NodeKind::Phaser(p) => p.reset(),
            NodeKind::Vowel(v) => v.reset(),
            NodeKind::Env(env) => env.reset(),
            NodeKind::Biquad(m) => m.reset(),
            NodeKind::Ladder(m) => m.reset(),
            NodeKind::Mix | NodeKind::Gain(_) | NodeKind::Vca | NodeKind::Output => {}
            NodeKind::Saturator(_) => {}
            NodeKind::Chorus(c) => c.reset(),
            NodeKind::Bitcrusher(bc) => bc.reset(),
            NodeKind::Compressor(comp) => comp.reset(),
            NodeKind::Limiter(lim) => lim.reset(),
            NodeKind::TiltEq(eq) => eq.reset(),
            NodeKind::ThreeBandEq(eq) => eq.reset(),
            NodeKind::Delay(d) => d.reset(),
            NodeKind::Reverb(r) => r.reset(),
        }
    }
}
