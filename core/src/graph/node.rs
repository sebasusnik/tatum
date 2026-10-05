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
extern crate alloc;
use alloc::boxed::Box;

/// Maximum number of inputs a single node can accept.
pub const MAX_NODE_INPUTS: usize = 8;

/// One step of an FX chain: what to build, and how much of it to hear.
///
/// `wet` is the universal bypass. At 0 the engine skips the node's processing
/// entirely rather than multiplying its output by zero, so a chain of effects
/// that are switched off costs nothing -- the same reasoning as a muted track.
/// At 1 the node replaces the signal; in between it is blended against the dry.
/// It is deliberately separate from the `mix` some nodes declare, which is
/// those nodes' own internal wet amount and has nothing to do with bypassing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChainStep {
    pub spec: NodeSpec,
    pub wet: f32,
}

impl ChainStep {
    pub fn full(spec: NodeSpec) -> Self {
        Self { spec, wet: 1.0 }
    }
}

// ── NodeSpec: lightweight description for creating nodes ──

/// Describes how to create a node. Stored in GraphTemplate.
/// Small, Copy-able, no heap allocations.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NodeSpec {
    Osc {
        waveform: Waveform,
        freq: f32,
        drift_seed: u32,
        fixed: bool,
        pitch_semitones: f32,
    },
    /// Oscillator with built-in pitch envelope (exponential sweep from start→end).
    /// Used for drum body synthesis (kick, tom). Not MIDI-note tracked.
    PitchOsc {
        waveform: Waveform,
        start_freq: f32,
        end_freq: f32,
        decay: f32,
    },
    Noise {
        seed: u32,
    },
    Lfo {
        rate: f32,
        depth: f32,
    },
    Env {
        a: f32,
        d: f32,
        s: f32,
        r: f32,
    },
    Biquad {
        filter_type: FilterType,
        cutoff: f32,
        resonance: f32,
        env_attack: f32,
        env_decay: f32,
        env_sustain: f32,
        env_release: f32,
        env_depth: f32,
        lfo: FilterLfo,
    },
    Ladder {
        cutoff: f32,
        resonance: f32,
        env_attack: f32,
        env_decay: f32,
        env_sustain: f32,
        env_release: f32,
        env_depth: f32,
        lfo: FilterLfo,
    },
    /// Stereo auto-panner: slow LFO moves the signal between L and R.
    AutoPan {
        hz: f32,
        bars: f32,
        depth: f32,
    },
    /// Stereo panner driven by the signal's own envelope rather than an LFO.
    PanEnv {
        depth: f32,
        attack_ms: f32,
        release_ms: f32,
    },
    /// Envelope filter: a resonant filter whose cutoff the signal's own
    /// envelope sweeps. `mode` is 0 lowpass, 1 bandpass, 2 highpass.
    AutoWah {
        sens: f32,
        base: f32,
        range: f32,
        q: f32,
        attack_ms: f32,
        release_ms: f32,
        mode: u8,
        down: bool,
        wobble: f32,
        wobble_hz: f32,
    },
    /// Swept allpass phaser.
    Phaser {
        mix: f32,
        hz: f32,
        bars: f32,
        stages: u8,
        feedback: f32,
        depth: f32,
    },
    /// Vowel formant filter morphing between two vowels.
    Vowel {
        from: u8,
        to: u8,
        hz: f32,
        bars: f32,
        mix: f32,
    },
    Mix,
    Gain {
        amount: f32,
    },
    Vca,
    Saturator {
        drive: f32,
    },
    Chorus {
        mix: f32,
    },
    Bitcrusher {
        bits: f32,
        rate: f32,
    },
    Compressor {
        threshold_db: f32,
        ratio: f32,
        attack_ms: f32,
        release_ms: f32,
        makeup: f32,
    },
    Limiter {
        threshold: f32,
    },
    TiltEq {
        amount: f32,
    },
    ThreeBandEq {
        low: f32,
        mid: f32,
        high: f32,
    },
    /// Delay (bus/master chain only, not per-voice).
    Delay {
        sync_div: f32,
        feedback: f32,
    },
    /// Reverb (bus/master chain only, not per-voice).
    Reverb {
        room_size: f32,
    },
    /// Record a window of the chain's output and loop it back. The window is
    /// already in samples: the compiler resolves `bars` against the song's
    /// slowest tempo so the buffer can be allocated when the chain is built.
    Capture {
        samples: u32,
        start_samples: u32,
        speed: f32,
        reverse: bool,
        mix: f32,
    },
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

/// How fast the peak reference forgets, per sample: about four seconds, long
/// enough to span the gap between two chords.
const PANENV_PEAK_FALL: f32 = crate::per_sample(0.999996);
/// Below this the follower is dividing noise by noise; hold the last position.
const PANENV_FLOOR: f32 = 1.0e-4;
/// One-pole on the pan position itself, about 30 ms, so it can never jump.
const PANENV_SLEW: f32 = 0.0007;

/// One-pole coefficient for a time constant in milliseconds.
fn coeff(ms: f32) -> f32 {
    let t = (ms * 0.001 * SAMPLE_RATE).max(1.0);
    1.0 - crate::math::exp(-1.0 / t)
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
            cutoff,
            resonance,
            env_attack: 0.005,
            env_decay: 0.2,
            env_sustain: 0.0,
            env_release: 0.1,
            env_depth: 0.0,
            lfo: FilterLfo::default(),
        }
    }

    /// Convenience: biquad with no filter envelope (backward compat).
    pub fn biquad(filter_type: FilterType, cutoff: f32, resonance: f32) -> Self {
        NodeSpec::Biquad {
            filter_type,
            cutoff,
            resonance,
            env_attack: 0.005,
            env_decay: 0.2,
            env_sustain: 0.0,
            env_release: 0.1,
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
                NodeKind::PitchOsc(PitchOscState { osc, current_freq: start_freq, end_freq, decay })
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
            NodeSpec::Biquad {
                filter_type,
                cutoff,
                resonance,
                env_attack,
                env_decay,
                env_sustain,
                env_release,
                env_depth,
                lfo,
            } => {
                let mut f = BiquadFilter::new(SAMPLE_RATE);
                f.set_params(filter_type, cutoff, resonance);
                let mut fr = BiquadFilter::new(SAMPLE_RATE);
                fr.set_params(filter_type, cutoff, resonance);
                let mut env = Envelope::new(SAMPLE_RATE);
                env.set_adsr(env_attack, env_decay, env_sustain, env_release);
                NodeKind::Biquad(ModBiquad {
                    filter: f,
                    filter_r: fr,
                    env,
                    base_cutoff: cutoff,
                    env_depth,
                    velocity: 1.0,
                    lfo: make_lfo(lfo.hz),
                    lfo_depth: lfo.depth,
                    lfo_bars: lfo.bars,
                    lfo_tick: 0,
                    glide_to: 0.0,
                })
            }
            NodeSpec::AutoPan { hz, bars, depth } => NodeKind::AutoPan { lfo: make_lfo(hz), bars, depth },
            NodeSpec::AutoWah { sens, base, range, q, attack_ms, release_ms, mode, down, wobble, wobble_hz } => {
                let ft = match mode {
                    1 => FilterType::BandPass,
                    2 => FilterType::HighPass,
                    _ => FilterType::LowPass,
                };
                let mut fl = BiquadFilter::new(SAMPLE_RATE);
                fl.set_params(ft, base, q);
                let mut fr = BiquadFilter::new(SAMPLE_RATE);
                fr.set_params(ft, base, q);
                NodeKind::AutoWah {
                    left: fl,
                    right: fr,
                    sens,
                    base,
                    range,
                    q,
                    atk: coeff(attack_ms),
                    rel: coeff(release_ms),
                    down,
                    env: 0.0,
                    peak: 0.0,
                    cutoff: base,
                    tick: 0,
                    wobble,
                    wob_inc: wobble_hz / SAMPLE_RATE,
                    wob_phase: 0.0,
                }
            }
            NodeSpec::PanEnv { depth, attack_ms, release_ms } => {
                NodeKind::PanEnv { depth, atk: coeff(attack_ms), rel: coeff(release_ms), env: 0.0, peak: 0.0, pos: 0.0 }
            }
            NodeSpec::Phaser { mix, hz, bars, stages, feedback, depth } => {
                NodeKind::Phaser(Phaser::new(mix, hz, bars, stages as usize, feedback, depth))
            }
            NodeSpec::Capture { samples, start_samples, speed, reverse, mix } => {
                NodeKind::Capture(Capture::new(samples, start_samples, speed, reverse, mix))
            }
            NodeSpec::Vowel { from, to, hz, bars, mix } => NodeKind::Vowel(Formant::new(from, to, hz, bars, mix)),
            NodeSpec::Ladder { cutoff, resonance, env_attack, env_decay, env_sustain, env_release, env_depth, lfo } => {
                let mut f = LadderFilter::new(SAMPLE_RATE);
                f.set_params(cutoff, resonance);
                let mut fr = LadderFilter::new(SAMPLE_RATE);
                fr.set_params(cutoff, resonance);
                let mut env = Envelope::new(SAMPLE_RATE);
                env.set_adsr(env_attack, env_decay, env_sustain, env_release);
                NodeKind::Ladder(ModLadder {
                    filter: f,
                    filter_r: fr,
                    env,
                    base_cutoff: cutoff,
                    env_depth,
                    velocity: 1.0,
                    lfo: make_lfo(lfo.hz),
                    lfo_depth: lfo.depth,
                    lfo_bars: lfo.bars,
                    lfo_tick: 0,
                    glide_to: 0.0,
                })
            }
            NodeSpec::Mix => NodeKind::Mix,
            NodeSpec::Gain { amount } => NodeKind::Gain(amount),
            NodeSpec::Vca => NodeKind::Vca,
            NodeSpec::Saturator { drive } => NodeKind::Saturator(Saturator::new(drive)),
            NodeSpec::Chorus { mix } => {
                let mut c = Box::new(Chorus::new());
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
                let mut r = Box::new(DattorroReverb::new(SAMPLE_RATE));
                r.set_room_size(room_size);
                r.set_mix(1.0); // wet-only for sends
                NodeKind::Reverb(r)
            }
            NodeSpec::Output => NodeKind::Output,
        }
    }

    /// As `instantiate`, for a chain in a song whose slowest tempo is
    /// `slowest_bpm`: a delay line holds the longest echo it can be asked for
    /// there, rather than two seconds. Today a chain delay plays 300 ms
    /// whatever its division says (the division is not applied yet), so the
    /// line covers that too.
    pub fn instantiate_in_chain(&self, slowest_bpm: f32) -> NodeKind {
        match *self {
            NodeSpec::Delay { sync_div, feedback } => {
                let synced = 60.0 / slowest_bpm.max(20.0) * 4.0 * sync_div;
                let mut d = Delay::new(SAMPLE_RATE, (synced.max(0.3) * 1.2).min(2.0));
                d.set_feedback(feedback);
                d.set_mix(1.0);
                NodeKind::Delay(d)
            }
            _ => self.instantiate(),
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
///
/// Two filters, one per channel. A single one processed twice per stereo frame
/// runs at twice its design rate — a `lowpass(3400)` measured its -3 dB corner
/// at 7681 Hz — and mixes the channels through one set of state registers.
pub struct ModLadder {
    pub filter: LadderFilter,
    pub filter_r: LadderFilter,
    pub env: Envelope,
    pub base_cutoff: f32,
    pub env_depth: f32,
    pub velocity: f32,
    pub lfo: Lfo,
    pub lfo_depth: f32,
    pub lfo_bars: f32,
    pub lfo_tick: u32,
    /// Where a knob is taking the cutoff, while it gets there; 0 when still.
    /// See [`glide_cutoff`].
    pub glide_to: f32,
}

impl ModLadder {
    pub fn gate_on(&mut self) {
        if self.env_depth > 0.0 {
            self.env.gate_on();
        }
    }

    pub fn gate_off(&mut self) {
        if self.env_depth > 0.0 {
            self.env.gate_off();
        }
    }

    pub fn set_velocity(&mut self, vel: f32) {
        self.velocity = vel;
    }

    /// Advance the envelope and LFO one frame and retune both filters. Called
    /// once per frame, mono or stereo, so the modulation runs at the same rate
    /// either way.
    #[inline]
    fn tick_cutoff(&mut self) {
        let has_env = self.env_depth > 0.0;
        let has_lfo = self.lfo_depth > 0.0;
        let gliding = self.glide_to > 0.0;
        if !(has_env || has_lfo || gliding) {
            return;
        }
        if gliding {
            self.lfo_tick = self.lfo_tick.wrapping_add(if has_lfo { 0 } else { 1 });
            if !has_lfo && !has_env && !self.lfo_tick.is_multiple_of(8) {
                return;
            }
            if self.lfo_tick.is_multiple_of(8) {
                glide_cutoff(&mut self.base_cutoff, &mut self.glide_to);
            }
        }
        let mut cutoff = self.base_cutoff;
        if has_env {
            cutoff += self.env.next_sample() * self.env_depth;
        }
        if has_lfo {
            // Cheap: advance the LFO every sample, retune the filter every 8.
            let v = self.lfo.next_sample();
            self.lfo_tick = self.lfo_tick.wrapping_add(1);
            if !self.lfo_tick.is_multiple_of(8) && !has_env {
                return;
            }
            cutoff += v * self.lfo_depth;
        }
        self.filter.set_cutoff(cutoff);
        self.filter_r.set_cutoff(cutoff);
    }

    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        self.tick_cutoff();
        self.filter.process(input)
    }

    /// One filter per channel: see the note on the struct.
    #[inline]
    pub fn process_stereo(&mut self, l: f32, r: f32) -> (f32, f32) {
        self.tick_cutoff();
        (self.filter.process(l), self.filter_r.process(r))
    }

    /// Retune a bar-synced LFO to the song tempo (4/4).
    pub fn set_bpm(&mut self, bpm: f32) {
        if self.lfo_bars > 0.0 {
            self.lfo.set_rate(bpm / 60.0 / 4.0 / self.lfo_bars);
        }
    }

    pub fn reset(&mut self) {
        self.filter.reset();
        self.filter_r.reset();
        self.env.reset();
    }
}

/// How far a gliding cutoff moves every 8 samples: a 10 ms time constant.
const CUTOFF_GLIDE: f32 = 8.0 / (0.010 * SAMPLE_RATE);

/// Move `base` a step towards `to`, along the octaves rather than the Hz, and
/// stop there. A knob sends a few dozen values across a sweep; set straight,
/// each one retunes the filter at once, and the steps are heard as a crackle
/// on anything bright, like something being snapped.
fn glide_cutoff(base: &mut f32, to: &mut f32) {
    let ratio = *to / base.max(1.0);
    if crate::math::abs(ratio - 1.0) < 0.001 {
        *base = *to;
        *to = 0.0;
    } else {
        *base *= crate::math::pow(ratio, CUTOFF_GLIDE);
    }
}

/// Biquad filter with built-in filter envelope for per-note cutoff sweeps.
pub struct ModBiquad {
    pub filter: BiquadFilter,
    pub filter_r: BiquadFilter,
    pub env: Envelope,
    pub base_cutoff: f32,
    pub env_depth: f32,
    pub velocity: f32,
    pub lfo: Lfo,
    pub lfo_depth: f32,
    pub lfo_bars: f32,
    pub lfo_tick: u32,
    /// Where a knob is taking the cutoff, while it gets there; 0 when still.
    /// See [`glide_cutoff`].
    pub glide_to: f32,
}

impl ModBiquad {
    pub fn gate_on(&mut self) {
        if self.env_depth > 0.0 {
            self.env.gate_on();
        }
    }

    pub fn gate_off(&mut self) {
        if self.env_depth > 0.0 {
            self.env.gate_off();
        }
    }

    pub fn set_velocity(&mut self, vel: f32) {
        self.velocity = vel;
    }

    /// Advance the envelope and LFO one frame and retune both filters. Called
    /// once per frame, mono or stereo, so the modulation runs at the same rate
    /// either way.
    #[inline]
    fn tick_cutoff(&mut self) {
        let has_env = self.env_depth > 0.0;
        let has_lfo = self.lfo_depth > 0.0;
        let gliding = self.glide_to > 0.0;
        if !(has_env || has_lfo || gliding) {
            return;
        }
        if gliding {
            self.lfo_tick = self.lfo_tick.wrapping_add(if has_lfo { 0 } else { 1 });
            if !has_lfo && !has_env && !self.lfo_tick.is_multiple_of(8) {
                return;
            }
            if self.lfo_tick.is_multiple_of(8) {
                glide_cutoff(&mut self.base_cutoff, &mut self.glide_to);
            }
        }
        let mut cutoff = self.base_cutoff;
        if has_env {
            cutoff += self.env.next_sample() * self.env_depth;
        }
        if has_lfo {
            // Cheap: advance the LFO every sample, retune the filter every 8.
            let v = self.lfo.next_sample();
            self.lfo_tick = self.lfo_tick.wrapping_add(1);
            if !self.lfo_tick.is_multiple_of(8) && !has_env {
                return;
            }
            cutoff += v * self.lfo_depth;
        }
        self.filter.set_cutoff(cutoff);
        self.filter_r.set_cutoff(cutoff);
    }

    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        self.tick_cutoff();
        self.filter.process(input)
    }

    /// One filter per channel: see the note on the struct.
    #[inline]
    pub fn process_stereo(&mut self, l: f32, r: f32) -> (f32, f32) {
        self.tick_cutoff();
        (self.filter.process(l), self.filter_r.process(r))
    }

    /// Retune a bar-synced LFO to the song tempo (4/4).
    pub fn set_bpm(&mut self, bpm: f32) {
        if self.lfo_bars > 0.0 {
            self.lfo.set_rate(bpm / 60.0 / 4.0 / self.lfo_bars);
        }
    }

    pub fn reset(&mut self) {
        self.filter.reset();
        self.filter_r.reset();
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
// large_enum_variant: boxing the big variant would allocate per voice on every
// note-on, which is the audio thread. Voices hold these inline by design.
#[allow(clippy::large_enum_variant)]
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
    AutoPan {
        lfo: Lfo,
        bars: f32,
        depth: f32,
    },
    /// `env` follows the input, `peak` is a slowly falling reference so the
    /// follower reads 0..1 whatever the level going in.
    PanEnv {
        depth: f32,
        atk: f32,
        rel: f32,
        env: f32,
        peak: f32,
        pos: f32,
    },
    /// A resonant filter the signal sweeps by its own envelope. At high `q` the
    /// filter rings near self-oscillation and every attack excites it, so what
    /// you hear is a narrow resonant peak sliding -- which is what a thin sheet
    /// does when you flex it and its modes shift.
    AutoWah {
        left: BiquadFilter,
        right: BiquadFilter,
        sens: f32,
        base: f32,
        range: f32,
        q: f32,
        atk: f32,
        rel: f32,
        down: bool,
        env: f32,
        peak: f32,
        cutoff: f32,
        tick: u32,
        wobble: f32,
        wob_inc: f32,
        wob_phase: f32,
    },
    Phaser(Phaser),
    Vowel(Formant),
    Saturator(Saturator),
    /// Boxed, with the reverb: their buffers live inline (32 KB, 18 KB), and
    /// every node is as large as the largest kind. Boxed, a node is a few
    /// hundred bytes, which is what lets a voice keep its nodes allocated.
    Chorus(Box<Chorus>),
    Bitcrusher(Bitcrusher),
    Compressor(Compressor),
    Limiter(Limiter),
    TiltEq(TiltEq),
    ThreeBandEq(ThreeBandEq),
    Delay(Delay),
    Reverb(Box<DattorroReverb>),
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
            NodeKind::AutoPan { lfo, .. } => {
                let _ = lfo.next_sample();
                inputs[0]
            }
            // Panning is a stereo gesture; in mono the signal passes.
            NodeKind::PanEnv { .. } => inputs[0],
            NodeKind::AutoWah { left, .. } => left.process(inputs[0]),
            NodeKind::Phaser(p) => p.process(inputs[0]),
            NodeKind::Vowel(v) => v.process(inputs[0]),

            NodeKind::Mix => inputs[..input_count as usize].iter().sum(),

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
            // Filters and the chorus each hold a delay line or state registers.
            // The dual-mono fallback below clocks them twice per stereo frame
            // and runs both channels through one copy of that state.
            NodeKind::Biquad(m) => m.process_stereo(l, r),
            NodeKind::Ladder(m) => m.process_stereo(l, r),
            NodeKind::Chorus(c) => c.process_stereo(l, r),
            // Explicit, because the dual-mono fallback would step the capture's
            // read position twice per stereo sample.
            NodeKind::Capture(c) => c.process_stereo(l, r),
            // Same reason, and it matters more here: calling a delay line twice
            // per stereo sample double-clocks it and runs both channels through
            // one state. A bus reverb came out as wind and rain.
            NodeKind::Delay(d) => d.process_stereo(l, r),
            NodeKind::Reverb(rv) => rv.process_stereo_in(l, r),
            NodeKind::Vowel(v) => v.process_stereo(l, r),
            // approx_constant: the pan law is tuned at this precision; SQRT_2 is a
            // different f32 and would shift every auto-panned gain.
            #[allow(clippy::approx_constant)]
            NodeKind::AutoPan { lfo, depth, .. } => {
                // Equal-power pan driven by the LFO: p in -1..1
                let p = lfo.next_sample() * *depth;
                let angle = (p + 1.0) * 0.25 * crate::math::PI; // 0..pi/2
                let gl = crate::math::cos(angle) * 1.4142;
                let gr = crate::math::sin(angle) * 1.4142;
                // Pan the channels where they are. Blending half the mono sum
                // back in fed a quarter of each channel into the other, which on
                // a reverb return — the one place the signal is already wide —
                // took the correlation of a 100% wet bus from 0.37 to 0.77. With
                // a mono source the two forms are the same expression.
                (l * gl, r * gr)
            }
            NodeKind::AutoWah {
                left,
                right,
                sens,
                base,
                range,
                atk,
                rel,
                down,
                env,
                peak,
                cutoff,
                tick,
                wobble,
                wob_inc,
                wob_phase,
                ..
            } => {
                let level = crate::math::abs(l) + crate::math::abs(r);
                let c = if level > *env { *atk } else { *rel };
                *env += (level - *env) * c;
                // The pedal's Gain knob: how much signal reaches the detector,
                // against a fixed threshold. Normalising against a rolling peak
                // instead leaves the filter open for as long as a chord rings,
                // and the sweep stops following the playing.
                *peak = *sens * 40.0 + 0.5;
                // Retune every 8 samples and glide there gently. A biquad at
                // Q 18 whose coefficients jump is a zipper: the ring is what we
                // want, the grit on the way is not.
                *tick = tick.wrapping_add(1);
                if *tick % 8 == 0 {
                    let norm = (*env * *peak).min(1.0);
                    let mut target = if *down { *base + *range * (1.0 - norm) } else { *base + *range * norm };
                    // Bending a sheet and letting go does not move the resonance
                    // once: it rings back and forth and settles. The wobble
                    // rides on the envelope, so a fresh hit swings wide and the
                    // swing dies with the note.
                    if *wobble > 0.0 {
                        *wob_phase += *wob_inc * 8.0;
                        if *wob_phase >= 1.0 {
                            *wob_phase -= 1.0;
                        }
                        target += *wobble * norm * crate::math::sin(*wob_phase * crate::math::TWO_PI);
                    }
                    // Asymmetric on purpose: the sweep opens with the attack
                    // and closes slowly. Gliding up as gently as it comes down
                    // puts the bright part of the hit after the transient
                    // instead of on it, which reads as a late attack.
                    let slew = if target > *cutoff { 0.55 } else { 0.06 };
                    *cutoff += (target - *cutoff) * slew;
                    let hz = crate::math::clamp(*cutoff, 30.0, 16000.0);
                    left.set_cutoff(hz);
                    right.set_cutoff(hz);
                }
                (left.process(l), right.process(r))
            }
            NodeKind::PanEnv { depth, atk, rel, env, peak, pos } => {
                // Follow the input, then place it by how loud it is against its
                // own recent peak: the hit lands to one side and the decay walks
                // back across. A struck chord sweeps as it rings out.
                let level = crate::math::abs(l) + crate::math::abs(r);
                let c = if level > *env { *atk } else { *rel };
                *env += (level - *env) * c;
                *peak = if *env > *peak { *env } else { *peak * PANENV_PEAK_FALL };
                // Below the floor the ratio is two small numbers dividing each
                // other, which chatters at sample rate and reads as a crushed,
                // grainy tail. There, hold the last position instead.
                if *peak > PANENV_FLOOR {
                    let norm = (*env / *peak).min(1.0);
                    let target = *depth * (2.0 * norm - 1.0);
                    *pos += (target - *pos) * PANENV_SLEW;
                }
                // A balance law, not a pan law: one channel holds at unity and
                // the other gives way. Constant-power panning boosts a channel
                // by 3 dB at the extremes, and this node is reached for on
                // signals that are already near the ceiling.
                let p = *pos;
                let gl = if p > 0.0 { 1.0 - p } else { 1.0 };
                let gr = if p < 0.0 { 1.0 + p } else { 1.0 };
                (l * gl, r * gr)
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
            (NodeKind::TiltEq(eq), "tilt") => {
                eq.set_tilt(value);
                true
            }
            (NodeKind::ThreeBandEq(eq), "eq_low") => {
                eq.set_low(value);
                true
            }
            (NodeKind::ThreeBandEq(eq), "eq_mid") => {
                eq.set_mid(value);
                true
            }
            (NodeKind::ThreeBandEq(eq), "eq_high") => {
                eq.set_high(value);
                true
            }
            (NodeKind::Saturator(s), "drive") => {
                s.set_drive(value);
                true
            }
            (NodeKind::Gain(g), "gain") => {
                *g = value;
                true
            }
            (NodeKind::Capture(c), n) => c.set_named(n, value),
            (NodeKind::Limiter(l), "limiter") => {
                l.set_threshold(value);
                true
            }
            (NodeKind::Compressor(c), "comp_threshold") => {
                c.set_threshold(value);
                true
            }
            (NodeKind::Biquad(m), "cutoff") => {
                m.base_cutoff = value;
                m.filter.set_cutoff(value);
                m.filter_r.set_cutoff(value);
                true
            }
            (NodeKind::Ladder(m), "cutoff") => {
                m.base_cutoff = value;
                m.filter.set_cutoff(value);
                m.filter_r.set_cutoff(value);
                true
            }
            _ => false,
        }
    }

    /// Like [`set_named`](Self::set_named), for a value arriving from a knob:
    /// a cutoff glides there over a few milliseconds instead of jumping, so a
    /// turn is heard as a sweep rather than a run of steps. Anything else is
    /// set at once.
    pub fn glide_named(&mut self, name: &str, value: f32) -> bool {
        match (self, name) {
            (NodeKind::Biquad(m), "cutoff") => {
                m.glide_to = value.max(1.0);
                true
            }
            (NodeKind::Ladder(m), "cutoff") => {
                m.glide_to = value.max(1.0);
                true
            }
            (node, _) => node.set_named(name, value),
        }
    }

    /// Retune bar-synced modulation to the song tempo.
    pub fn set_bpm(&mut self, bpm: f32) {
        match self {
            NodeKind::Biquad(m) => m.set_bpm(bpm),
            NodeKind::Ladder(m) => m.set_bpm(bpm),
            NodeKind::AutoPan { lfo, bars, .. } => {
                if *bars > 0.0 {
                    lfo.set_rate(bpm / 60.0 / 4.0 / *bars);
                }
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

    /// Become what `spec` instantiates, reusing this node's memory: a voice
    /// does this on every note. The kinds that own a buffer (a delay line, a
    /// capture window, the chorus) are cleared and set again in place, which
    /// plays exactly as a fresh one does; the rest are small and simply made
    /// again. The reverb is made again too: its `reset` leaves state behind,
    /// and it lives on buses and the master, never in a voice.
    pub fn renew(&mut self, spec: &NodeSpec) {
        match (&mut *self, spec) {
            (NodeKind::Delay(d), NodeSpec::Delay { feedback, .. }) => {
                d.reset();
                d.set_feedback(*feedback);
                d.set_mix(1.0);
            }
            (NodeKind::Capture(c), NodeSpec::Capture { .. }) => c.reset(),
            (NodeKind::Chorus(c), NodeSpec::Chorus { mix }) => {
                c.reset();
                c.set_mix(*mix);
            }
            _ => *self = spec.instantiate(),
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
            NodeKind::PanEnv { env, peak, pos, .. } => {
                *env = 0.0;
                *peak = 0.0;
                *pos = 0.0;
            }
            NodeKind::AutoWah { left, right, env, peak, cutoff, base, wob_phase, .. } => {
                left.reset();
                right.reset();
                *env = 0.0;
                *peak = 0.0;
                *cutoff = *base;
                *wob_phase = 0.0;
            }
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

#[cfg(test)]
mod renew_tests {
    use super::*;

    /// Feed a node a burst, renew it, and it must then play exactly what a
    /// freshly instantiated one plays: no tail, no state left over.
    fn renews_like_new(spec: NodeSpec) {
        let input = |i: usize| if i % 97 < 40 { ((i as f32) * 0.37).sin() * 0.8 } else { 0.0 };
        let mut used = spec.instantiate();
        for i in 0..20_000 {
            let mut ins = [0.0; MAX_NODE_INPUTS];
            ins[0] = input(i);
            used.process(&ins, 1);
        }
        used.renew(&spec);
        let mut fresh = spec.instantiate();
        for i in 0..20_000 {
            let mut ins = [0.0; MAX_NODE_INPUTS];
            ins[0] = input(i);
            let (a, b) = (used.process(&ins, 1), fresh.process(&ins, 1));
            assert_eq!(a.to_bits(), b.to_bits(), "{spec:?} differs at sample {i}");
        }
    }

    #[test]
    fn a_renewed_node_plays_like_a_new_one() {
        renews_like_new(NodeSpec::Delay { sync_div: 0.25, feedback: 0.6 });
        renews_like_new(NodeSpec::Capture { samples: 4_000, start_samples: 0, speed: 0.5, reverse: true, mix: 1.0 });
        renews_like_new(NodeSpec::Chorus { mix: 0.5 });
        renews_like_new(NodeSpec::Saturator { drive: 2.0 });
    }

    #[test]
    fn a_node_is_small_enough_to_keep_per_voice() {
        assert!(core::mem::size_of::<NodeKind>() <= 512, "NodeKind is {} bytes", core::mem::size_of::<NodeKind>());
    }
}
