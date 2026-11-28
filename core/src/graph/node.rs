use crate::primitives::oscillator::{Oscillator, Waveform};
use crate::primitives::envelope::Envelope;
use crate::primitives::filter::{BiquadFilter, FilterType, LadderFilter, SvfFilter};
use crate::primitives::lfo::Lfo;
use crate::primitives::noise::NoiseGen;
use crate::effects::saturator::Saturator;
use crate::effects::chorus::Chorus;
use crate::effects::phaser::Phaser;
use crate::effects::formant::Formant;
use crate::effects::bitcrusher::Bitcrusher;
use crate::effects::compressor::Compressor;
use crate::effects::limiter::Limiter;
use crate::effects::clipper::Clipper;
use crate::effects::eq::{Band, ParamEq, TiltEq, ThreeBandEq};
use crate::effects::transient::Transient;
use crate::effects::delay::Delay;
use crate::effects::reverb::DattorroReverb;
use crate::effects::capture::Capture;
use crate::SAMPLE_RATE;
use super::drums::{ClapState, HatState, SnareState};
extern crate alloc;
use alloc::boxed::Box;

/// Maximum number of inputs a single node can accept.
pub const MAX_NODE_INPUTS: usize = 8;

/// Where a connection lands on the node it goes into. Most carry audio, and a
/// node reads those as its inputs in the order they were wired. The rest are
/// modulation: `lfo(0.3, 0.2) > a.width` does not feed `a` a signal to
/// process, it moves one of its knobs, sample by sample, by the value coming
/// in. Several connections into the same port add up.
pub mod port {
    pub const AUDIO: u8 = 0;
    /// Linear FM on an oscillator: 1.0 swings it a whole carrier frequency
    /// either way, the Oberheim's X-mod. It stops at 0 Hz rather than running
    /// backwards through it.
    pub const FM: u8 = 1;
    /// Exponential pitch on an oscillator, in octaves: 1.0 is an octave up,
    /// 1/12 a semitone. Vibrato, or an envelope bending the note.
    pub const PITCH: u8 = 2;
    /// Hard sync: each time the signal coming in crosses zero going up, the
    /// oscillator starts its cycle again.
    pub const SYNC: u8 = 3;
    /// Pulse width, added to the pulse's own `width=`: PWM.
    pub const WIDTH: u8 = 4;
    /// A filter's cutoff, in octaves on top of wherever it is: 1.0 is an
    /// octave up. An oscillator in here is the cross-mod into the filter.
    pub const CUTOFF: u8 = 5;
    /// The `svf` mode knob, added to its `morph=`.
    pub const MORPH: u8 = 6;
    pub const COUNT: usize = 7;

    pub const NAMES: [(&str, u8); 6] =
        [("fm", FM), ("pitch", PITCH), ("sync", SYNC), ("width", WIDTH), ("cutoff", CUTOFF), ("morph", MORPH)];

    pub fn from_name(name: &str) -> Option<u8> {
        NAMES.iter().find(|(n, _)| *n == name).map(|&(_, p)| p)
    }

    pub const fn bit(port: u8) -> u8 {
        1 << port
    }
}

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
        /// Hz added to the note at its start, falling away with `pitch_decay`:
        /// the click of pitch a plucked bass or a lead has on its attack.
        pitch_env: f32,
        /// How long that fall lasts, in seconds, timed as an envelope stage is.
        pitch_decay: f32,
    },
    /// Oscillator with built-in pitch envelope (exponential sweep from start→end).
    /// Used for drum body synthesis (kick, tom). Not MIDI-note tracked.
    /// With `mid_freq` under `start_freq` the sweep has two stages: a fast
    /// drop from start to mid at `fast`, laid over the slow one from mid to
    /// end at `decay` -- the snap of a kick's hit, then its body falling.
    PitchOsc {
        waveform: Waveform,
        start_freq: f32,
        end_freq: f32,
        decay: f32,
        mid_freq: f32,
        fast: f32,
    },
    /// A whole kick: a two-stage sweep onto `note`, a level held and then
    /// released, a click and a saturator, timed in milliseconds. Not
    /// MIDI-note tracked, and deaf to the note's end: every hit plays whole.
    Kick {
        /// Hz the body settles on.
        note: f32,
        /// Hz at the hit.
        start: f32,
        /// Hz where the snap hands over to the body.
        knee: f32,
        /// ms the snap takes to fall to 10% of its span.
        snap_ms: f32,
        /// Hz of a third, fastest stage over the start: the tick of pitch a
        /// hit has in its first milliseconds. Equal to `start`, none.
        chirp: f32,
        /// ms the chirp takes to fall to 10% of its span.
        chirp_ms: f32,
        /// ms until the pitch is within 10% of `note`.
        body_ms: f32,
        attack_ms: f32,
        hold_ms: f32,
        /// dB the level sinks across the hold (0 or negative): a top that
        /// leans before the release takes it.
        sag_db: f32,
        /// ms from the end of the hold to -60 dB.
        release_ms: f32,
        /// Where the sine starts, in cycles: 0 at the zero crossing, 0.25 at the top.
        phase: f32,
        /// Linear level of the click against the body; 0 is none.
        click: f32,
        /// Hz the click is centred on.
        tone: f32,
        /// 0 clean to 1 heavy.
        drive: f32,
        /// Share the hold and the release sway by, hit to hit; 0, none.
        vary: f32,
        /// Share the pitch sways by, hit to hit; 0, none.
        vary_pitch: f32,
    },
    /// A whole hat: six inharmonic square partials and noise through a band
    /// and a highpass, falling 60 dB in `decay_ms`. Deaf to the note and to
    /// its end, like the kick.
    Hat {
        /// ms from the hit to -60 dB.
        decay_ms: f32,
        /// Hz the band is centred on.
        band: f32,
        /// Hz under which it is cut.
        floor: f32,
        /// Share of noise against the metal, 0..1.
        noise: f32,
        /// Hz of the lowest of the six partials.
        pitch: f32,
        /// What brings the hat these arguments make to the level every hat
        /// has, found when it is compiled (`drums::hat_gain`); 1, as it is.
        gain: f32,
        /// Share the decay sways by, hit to hit; 0, none.
        vary: f32,
    },
    /// A whole snare: a body of two modes dropping onto `note`, and noise
    /// through a band for the wires, each with its own decay.
    Snare {
        /// Hz the body's lower mode settles on.
        note: f32,
        /// ms the drop takes to come within 10% of the note.
        snap_ms: f32,
        /// ms the body takes to fall 60 dB.
        body_ms: f32,
        /// ms the wires take to fall 60 dB.
        wires_ms: f32,
        /// Hz the wires' band is centred on.
        band: f32,
        /// Share of wires against body, 0..1.
        balance: f32,
        /// 0 clean to 1 heavy.
        drive: f32,
        /// Share the two decays sway by, hit to hit; 0, none.
        vary: f32,
    },
    /// A whole clap: `bursts` short bursts of noise in a band, `spacing_ms`
    /// apart, the last one falling 60 dB in `tail_ms`.
    Clap {
        /// How many bursts, the one that rings out included; a whole number.
        bursts: f32,
        spacing_ms: f32,
        tail_ms: f32,
        /// Hz the band is centred on.
        band: f32,
        /// Share the spacing and the tail sway by, hit to hit; 0, none.
        vary: f32,
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
    /// SEM-style state-variable filter: `morph` 0 lowpass, 0.5 notch, 1
    /// highpass, or the bandpass when `bandpass` is set.
    Svf {
        cutoff: f32,
        resonance: f32,
        morph: f32,
        bandpass: bool,
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
    /// One parametric band: a bell or a shelf at any frequency.
    ParamEq {
        band: Band,
        freq: f32,
        gain_db: f32,
        q: f32,
    },
    /// Antialiased hard clipper.
    Clip {
        drive: f32,
        ceiling: f32,
    },
    /// Transient shaper: attack and sustain in dB.
    Transient {
        attack_db: f32,
        sustain_db: f32,
    },
    /// Delay (bus/master chain only, not per-voice).
    Delay {
        sync_div: f32,
        feedback: f32,
        /// Slow drift of the delay time, 0..1.
        wow: f32,
        /// Fast, irregular tremble of the delay time, 0..1.
        flutter: f32,
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
            NodeSpec::Osc { waveform, freq, drift_seed, pitch_env, pitch_decay, .. } => {
                let mut osc = Oscillator::new(waveform, SAMPLE_RATE);
                osc.set_drift_seed(drift_seed);
                let mut note_osc = NoteOsc {
                    osc,
                    base: freq,
                    env: pitch_env,
                    env_coeff: crate::primitives::envelope::stage_coeff(pitch_decay.max(1e-4), SAMPLE_RATE),
                    duty: if let Waveform::Pulse(d) = waveform { d } else { 0.5 },
                    mods: OscMods::default(),
                };
                note_osc.set_frequency(freq);
                NodeKind::Osc(note_osc)
            }
            NodeSpec::PitchOsc { waveform, start_freq, end_freq, decay, mid_freq, fast } => {
                let mut osc = Oscillator::new(waveform, SAMPLE_RATE);
                osc.set_frequency(start_freq);
                osc.set_drift_enabled(false); // drums don't need drift
                NodeKind::PitchOsc(PitchOscState {
                    osc,
                    current_freq: mid_freq,
                    end_freq,
                    // Written per sample at 44.1 kHz, as the docs say: the
                    // same sweep at any rate.
                    decay: crate::per_sample(decay),
                    fast_part: start_freq - mid_freq,
                    fast: crate::per_sample(fast),
                })
            }
            NodeSpec::Kick { .. } => NodeKind::Kick(KickState::new(self)),
            NodeSpec::Hat { .. } => NodeKind::Hat(HatState::new(self)),
            NodeSpec::Snare { .. } => NodeKind::Snare(SnareState::new(self)),
            NodeSpec::Clap { .. } => NodeKind::Clap(ClapState::new(self)),
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
                    cut_mod: None,
                })
            }
            NodeSpec::Svf {
                cutoff,
                resonance,
                morph,
                bandpass,
                env_attack,
                env_decay,
                env_sustain,
                env_release,
                env_depth,
                lfo,
            } => {
                let mut f = SvfFilter::new(SAMPLE_RATE);
                f.set_params(cutoff, resonance);
                let mut fr = SvfFilter::new(SAMPLE_RATE);
                fr.set_params(cutoff, resonance);
                let mut env = Envelope::new(SAMPLE_RATE);
                env.set_adsr(env_attack, env_decay, env_sustain, env_release);
                NodeKind::Svf(ModSvf {
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
                    cut_mod: None,
                    morph,
                    morph_mod: 0.0,
                    bandpass,
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
            NodeSpec::ParamEq { band, freq, gain_db, q } => {
                NodeKind::ParamEq(ParamEq::new(band, freq, gain_db, q, SAMPLE_RATE))
            }
            NodeSpec::Clip { drive, ceiling } => NodeKind::Clip(Clipper::new(drive, ceiling)),
            NodeSpec::Transient { attack_db, sustain_db } => {
                NodeKind::Transient(Transient::new(attack_db, sustain_db, SAMPLE_RATE))
            }
            NodeSpec::Delay { feedback, wow, flutter, .. } => {
                let mut d = Delay::new(SAMPLE_RATE, 2.0);
                d.set_feedback(feedback);
                d.set_wow(wow);
                d.set_flutter(flutter);
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
            NodeSpec::Delay { sync_div, feedback, wow, flutter } => {
                let synced = 60.0 / slowest_bpm.max(20.0) * 4.0 * sync_div;
                let mut d = Delay::new(SAMPLE_RATE, (synced.max(0.3) * 1.2).min(2.0));
                d.set_feedback(feedback);
                d.set_wow(wow);
                d.set_flutter(flutter);
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

    /// A node the voice waits on: it ends when every one of these is idle.
    /// A kick carries its own level, so it is one, though it brings no VCA;
    /// so do a hat, a snare and a clap.
    pub fn is_envelope(&self) -> bool {
        matches!(
            self,
            NodeSpec::Env { .. }
                | NodeSpec::Kick { .. }
                | NodeSpec::Hat { .. }
                | NodeSpec::Snare { .. }
                | NodeSpec::Clap { .. }
        )
    }

    /// A drum node with `vary=`: it is made again for each hit by the hit's
    /// number ([`NodeKind::renew_hit`]).
    pub fn sways(&self) -> bool {
        match *self {
            NodeSpec::Kick { vary, vary_pitch, .. } => vary > 0.0 || vary_pitch > 0.0,
            NodeSpec::Hat { vary, .. } | NodeSpec::Snare { vary, .. } | NodeSpec::Clap { vary, .. } => vary > 0.0,
            _ => false,
        }
    }

    pub fn is_filter(&self) -> bool {
        matches!(self, NodeSpec::Ladder { .. } | NodeSpec::Biquad { .. } | NodeSpec::Svf { .. })
    }

    pub fn has_filter_env(&self) -> bool {
        match self {
            NodeSpec::Ladder { env_depth, .. }
            | NodeSpec::Biquad { env_depth, .. }
            | NodeSpec::Svf { env_depth, .. } => *env_depth > 0.0,
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
    /// Octaves the `cutoff` port moves it by this sample, when something is
    /// wired there.
    pub cut_mod: Option<f32>,
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
        if let Some(cutoff) = next_cutoff(
            &mut self.base_cutoff,
            &mut self.glide_to,
            &mut self.lfo_tick,
            &mut self.env,
            self.env_depth,
            &mut self.lfo,
            self.lfo_depth,
            self.cut_mod,
        ) {
            self.filter.set_cutoff(cutoff);
            self.filter_r.set_cutoff(cutoff);
        }
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

/// One frame of a filter's cutoff modulation: the envelope, the LFO and a
/// knob gliding, shared by every filter node. Returns the cutoff to retune to,
/// or `None` when it stays where it is.
///
/// With nothing wired into its `cutoff` port a filter retunes every 8 samples
/// for an LFO alone, which is plenty for a sweep. With something wired there
/// it retunes every sample, because what comes in may be an oscillator: the
/// cross-mod into the filter is a sound at audio rate, not a sweep.
#[allow(clippy::too_many_arguments)]
#[inline]
fn next_cutoff(
    base: &mut f32,
    glide_to: &mut f32,
    tick: &mut u32,
    env: &mut Envelope,
    env_depth: f32,
    lfo: &mut Lfo,
    lfo_depth: f32,
    cut_mod: Option<f32>,
) -> Option<f32> {
    let has_env = env_depth > 0.0;
    let has_lfo = lfo_depth > 0.0;
    let gliding = *glide_to > 0.0;
    if let Some(octaves) = cut_mod {
        if gliding {
            *tick = tick.wrapping_add(1);
            if tick.is_multiple_of(8) {
                glide_cutoff(base, glide_to);
            }
        }
        let mut cutoff = *base;
        if has_env {
            cutoff += env.next_sample() * env_depth;
        }
        if has_lfo {
            cutoff += lfo.next_sample() * lfo_depth;
        }
        return Some(cutoff * crate::math::pow2(octaves));
    }
    if !(has_env || has_lfo || gliding) {
        return None;
    }
    if gliding {
        *tick = tick.wrapping_add(if has_lfo { 0 } else { 1 });
        if !has_lfo && !has_env && !tick.is_multiple_of(8) {
            return None;
        }
        if tick.is_multiple_of(8) {
            glide_cutoff(base, glide_to);
        }
    }
    let mut cutoff = *base;
    if has_env {
        cutoff += env.next_sample() * env_depth;
    }
    if has_lfo {
        // Cheap: advance the LFO every sample, retune the filter every 8.
        let v = lfo.next_sample();
        *tick = tick.wrapping_add(1);
        if !tick.is_multiple_of(8) && !has_env {
            return None;
        }
        cutoff += v * lfo_depth;
    }
    Some(cutoff)
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
        if let Some(cutoff) = next_cutoff(
            &mut self.base_cutoff,
            &mut self.glide_to,
            &mut self.lfo_tick,
            &mut self.env,
            self.env_depth,
            &mut self.lfo,
            self.lfo_depth,
            // No `cutoff` port on a biquad: retuned every sample its state
            // no longer fits its coefficients, and it blows up.
            None,
        ) {
            self.filter.set_cutoff(cutoff);
            self.filter_r.set_cutoff(cutoff);
        }
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

/// The SEM-style state-variable filter with the same envelope, LFO and knob
/// glide as the other filters, plus its mode knob.
pub struct ModSvf {
    pub filter: SvfFilter,
    pub filter_r: SvfFilter,
    pub env: Envelope,
    pub base_cutoff: f32,
    pub env_depth: f32,
    pub velocity: f32,
    pub lfo: Lfo,
    pub lfo_depth: f32,
    pub lfo_bars: f32,
    pub lfo_tick: u32,
    pub glide_to: f32,
    pub cut_mod: Option<f32>,
    /// 0 lowpass, 0.5 notch, 1 highpass.
    pub morph: f32,
    /// What the `morph` port adds this sample.
    pub morph_mod: f32,
    pub bandpass: bool,
}

impl ModSvf {
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

    #[inline]
    fn tick_cutoff(&mut self) {
        if let Some(cutoff) = next_cutoff(
            &mut self.base_cutoff,
            &mut self.glide_to,
            &mut self.lfo_tick,
            &mut self.env,
            self.env_depth,
            &mut self.lfo,
            self.lfo_depth,
            self.cut_mod,
        ) {
            self.filter.set_cutoff(cutoff);
            self.filter_r.set_cutoff(cutoff);
        }
    }

    #[inline]
    fn output(&self, filter: &SvfFilter, out: crate::primitives::filter::SvfOut) -> f32 {
        if self.bandpass {
            filter.band(out)
        } else {
            SvfFilter::morph(out, self.morph + self.morph_mod)
        }
    }

    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        self.tick_cutoff();
        let out = self.filter.process(input);
        self.output(&self.filter, out)
    }

    #[inline]
    pub fn process_stereo(&mut self, l: f32, r: f32) -> (f32, f32) {
        self.tick_cutoff();
        let ol = self.filter.process(l);
        let or = self.filter_r.process(r);
        (self.output(&self.filter, ol), self.output(&self.filter_r, or))
    }

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

// ── Note oscillator state ──

/// An oscillator that follows the played note, with an optional pitch
/// envelope: `env` Hz on top of the note at its start, multiplied down every
/// sample until it no longer moves the pitch.
pub struct NoteOsc {
    pub osc: Oscillator,
    base: f32,
    env: f32,
    env_coeff: f32,
    /// The pulse's own `width=`, which the `width` port moves around.
    duty: f32,
    mods: OscMods,
}

/// What is wired into an oscillator's ports, as of this sample. Left at its
/// default the oscillator plays exactly as it did before there were ports.
#[derive(Default)]
struct OscMods {
    /// Something is wired into a port: take the modulated path.
    on: bool,
    fm: f32,
    pitch: f32,
    width: Option<f32>,
    sync: Option<f32>,
    /// The sync input one sample ago, to see it cross zero.
    sync_prev: f32,
    /// A DC blocker on the way out. A synced cycle cut short, or a carrier
    /// sped up and slowed down in step with its own phase, no longer averages
    /// to zero; an analog synth's output is AC-coupled, and this is that.
    /// Only `fm` and `sync` switch it on: it tilts a low square and lifts
    /// its peak, which vibrato or PWM on a bass has no reason to pay for.
    dc_block: bool,
    dc_x1: f32,
    dc_y1: f32,
    /// A synced oscillator runs one sample late: the sample it returns is the
    /// one before, so that when a reset lands between the two it can smooth
    /// the jump on both sides of it.
    pending: f32,
}

/// Below this many Hz the envelope is left out, and the oscillator plays the
/// note it was given as a plain one.
const PITCH_ENV_FLOOR: f32 = 0.01;

/// As high as its ports can push an oscillator, at whatever rate the engine
/// runs: just under half of it.
const MOD_FREQ_TOP: f32 = 0.45 * SAMPLE_RATE;

impl NoteOsc {
    /// Take this sample's values from the ports. `mask` says which are wired.
    #[inline]
    pub fn modulate(&mut self, mods: &[f32; port::COUNT], mask: u8) {
        let m = &mut self.mods;
        m.on = true;
        m.dc_block = mask & (port::bit(port::FM) | port::bit(port::SYNC)) != 0;
        m.fm = mods[port::FM as usize];
        m.pitch = mods[port::PITCH as usize];
        if mask & port::bit(port::WIDTH) != 0 {
            m.width = Some(mods[port::WIDTH as usize]);
        }
        if mask & port::bit(port::SYNC) != 0 {
            m.sync = Some(mods[port::SYNC as usize]);
        }
    }

    #[inline]
    pub fn next_sample(&mut self) -> f32 {
        if self.mods.on {
            return self.next_modulated();
        }
        if self.env != 0.0 {
            self.env *= self.env_coeff;
            if self.env.abs() < PITCH_ENV_FLOOR {
                self.env = 0.0;
            }
            self.osc.set_frequency((self.base + self.env).max(1.0));
        }
        self.osc.next_sample()
    }

    pub fn set_frequency(&mut self, freq: f32) {
        self.base = freq;
        self.osc.set_frequency((freq + self.env).max(1.0));
    }

    fn next_modulated(&mut self) -> f32 {
        if self.env != 0.0 {
            self.env *= self.env_coeff;
            if self.env.abs() < PITCH_ENV_FLOOR {
                self.env = 0.0;
            }
        }
        let m = &self.mods;
        let mut freq = (self.base + self.env).max(1.0);
        if m.pitch != 0.0 {
            freq *= crate::math::pow2(m.pitch);
        }
        // Linear FM, as the analog cross-mod is: the modulator adds and takes
        // away Hz in proportion to the carrier. Through-zero would run the
        // phase backwards, which the band-limiting here does not follow, so
        // it stops at standing still.
        freq *= (1.0 + m.fm).max(0.0);
        // Nothing wired into a port takes it past the top: over half the
        // sample rate the phase moves more than a cycle a sample and the
        // band-limiting stops meaning anything. `min` also drops a NaN.
        self.osc.set_frequency(freq.min(MOD_FREQ_TOP));
        if let Some(w) = m.width {
            self.osc.set_waveform(Waveform::Pulse(crate::math::clamp(self.duty + w, 0.05, 0.95)));
        }
        let out = match m.sync {
            Some(sync) => self.next_synced(sync),
            None => self.osc.next_sample(),
        };
        if !self.mods.dc_block {
            return out;
        }
        // One pole at about 5 Hz, as the ladder's.
        const R: f32 = crate::per_sample(0.99929);
        let y = out - self.mods.dc_x1 + R * self.mods.dc_y1;
        self.mods.dc_x1 = out;
        self.mods.dc_y1 = y;
        y
    }

    /// Hard sync, band-limited. When the master crosses zero going up between
    /// the last sample and this one, the phase restarts at the moment it
    /// crossed, which falls between two samples. The waveform jumps there by
    /// however far it was from the start of its cycle, and a jump left as it
    /// is aliases into a whine under every note; it is smoothed the way the
    /// oscillator smooths its own edges, a polyBLEP spread over the sample
    /// before and the sample after, which is why this runs one sample late.
    fn next_synced(&mut self, sync: f32) -> f32 {
        let prev = self.mods.sync_prev;
        self.mods.sync_prev = sync;
        let mut before = self.mods.pending;
        let now = if prev < 0.0 && sync >= 0.0 {
            // Samples since the crossing, 0..1.
            let after = sync / (sync - prev);
            let inc = self.osc.phase_inc();
            let at = self.osc.phase() - after * inc;
            let value = self.osc.naive_at(at - crate::math::floor(at));
            // The sample before never saw a wrap coming, so it takes the
            // whole jump. The sample after starts a cycle, and the oscillator
            // already smooths that as the jump of its own wrap, from the end
            // of a cycle; what is left there is the difference between that
            // jump and this one.
            let jump = self.osc.naive_at(0.0) - value;
            let rest = self.osc.naive_at(0.999_999) - value;
            self.osc.set_phase(after * inc);
            before += 0.5 * jump * after * after;
            let left = 1.0 - after;
            self.osc.next_sample() - 0.5 * rest * left * left
        } else {
            self.osc.next_sample()
        };
        self.mods.pending = now;
        before
    }

    pub fn reset(&mut self) {
        self.osc.reset();
    }
}

// ── Pitch-envelope oscillator state ──

/// Oscillator with built-in exponential pitch sweep (start_freq → end_freq).
/// Used for drum body synthesis. Fresh instance per note (re-triggers from start_freq).
pub struct PitchOscState {
    pub osc: Oscillator,
    /// The slow stage: from the mid frequency (the start, with one stage)
    /// towards `end_freq`.
    pub current_freq: f32,
    pub end_freq: f32,
    pub decay: f32,
    /// The fast stage, on top: start minus mid, falling at `fast`. Zero with
    /// one stage, and adding zero leaves the sweep as it always was.
    pub fast_part: f32,
    pub fast: f32,
}

impl PitchOscState {
    #[inline]
    pub fn next_sample(&mut self) -> f32 {
        let out = self.osc.next_sample();
        // Exponential pitch sweep toward end_freq
        self.current_freq = self.end_freq + (self.current_freq - self.end_freq) * self.decay;
        self.fast_part *= self.fast;
        self.osc.set_frequency(self.current_freq + self.fast_part);
        out
    }

    pub fn reset(&mut self) {
        self.osc.reset();
    }
}

// ── Kick state ──

/// Below this the release has reached -60 dB and the kick is over.
const KICK_FLOOR: f32 = 1.0e-3;
/// The click lasts this long to -60 dB: a beater, not a tick and not a hiss.
const KICK_CLICK_MS: f32 = 4.0;
/// The noise is seeded the same on every hit, so every click is the same.
const KICK_CLICK_SEED: u32 = 0x4b49_434b;
/// The highest the kick's pitch goes, as a share of the sample rate.
const KICK_TOP: f32 = 0.45;

/// Per-sample factor that takes a level down by `db` (a negative number) in
/// `ms` milliseconds.
pub(super) fn fall_per_sample(db: f32, ms: f32) -> f32 {
    let samples = (ms * 0.001 * SAMPLE_RATE).max(1.0);
    crate::math::exp(db * (core::f32::consts::LN_10 / 20.0) / samples)
}

/// The `kick` node at run time. The pitch is the note plus two decaying
/// parts, the body (knee to note) and the snap (start to knee), each falling
/// on its own time constant; the level is attack, hold, then an exponential
/// release to -60 dB. Everything is set when the hit starts, so two hits of
/// the same kick are the same samples.
pub struct KickState {
    /// Where the sine is, in cycles.
    phase: f32,
    note: f32,
    body_part: f32,
    body_mul: f32,
    snap_part: f32,
    snap_mul: f32,
    chirp_part: f32,
    chirp_mul: f32,
    /// Samples of attack left, and how much each one adds.
    attack_left: u32,
    attack_step: f32,
    hold_left: u32,
    /// Where the hold has sunk to, and how much more each sample of it.
    held: f32,
    sag_mul: f32,
    release_mul: f32,
    level: f32,
    drive_gain: f32,
    /// 1 / tanh(drive_gain): the saturated peak stays where the clean one was.
    drive_norm: f32,
    click: f32,
    click_mul: f32,
    click_filter: BiquadFilter,
    rng: crate::rng::Rng,
    done: bool,
}

impl KickState {
    fn new(spec: &NodeSpec) -> Self {
        let NodeSpec::Kick {
            note,
            start,
            knee,
            snap_ms,
            chirp,
            chirp_ms,
            body_ms,
            attack_ms,
            hold_ms,
            sag_db,
            release_ms,
            phase,
            click,
            tone,
            drive,
            ..
        } = *spec
        else {
            unreachable!("KickState::new takes a kick")
        };
        let ms = |t: f32| t * 0.001 * SAMPLE_RATE;
        // A sine over half the rate folds back down: at 22.05 kHz a 12 kHz
        // chirp would play as a 10 kHz one. The top of the sweep is held
        // under 0.45 of the rate, which at 44.1 kHz is over anything the
        // arguments allow, so only a smaller machine hears a difference.
        let top = KICK_TOP * SAMPLE_RATE;
        let chirp = chirp.min(top);
        let start = start.min(chirp);
        let knee = knee.min(start);
        // The body is within 10% of the note when its part is down to a tenth
        // of the note: from knee - note to 0.1 * note in `body_ms`, which is
        // how `tatum analyze` times a sweep. The compiler keeps the knee over
        // 1.1 times the note, so the ratio is over 1.
        let body_part = knee - note;
        let body_ratio = (body_part / (0.1 * note)).max(1.0001);
        let body_tau = ms(body_ms).max(1.0) / crate::math::ln(body_ratio);
        // The snap falls by 10x in `snap_ms`.
        let snap_tau = ms(snap_ms).max(1.0) / core::f32::consts::LN_10;
        let chirp_tau = ms(chirp_ms).max(1.0) / core::f32::consts::LN_10;
        let hold = ms(hold_ms) as u32;
        let attack_left = ms(attack_ms) as u32;
        let drive_gain = 1.0 + 5.0 * drive;
        let mut click_filter = BiquadFilter::new(SAMPLE_RATE);
        click_filter.set_params(FilterType::BandPass, tone, 0.01);
        Self {
            phase,
            note,
            body_part,
            body_mul: crate::math::exp(-1.0 / body_tau),
            snap_part: start - knee,
            snap_mul: crate::math::exp(-1.0 / snap_tau),
            chirp_part: chirp - start,
            chirp_mul: crate::math::exp(-1.0 / chirp_tau),
            attack_left,
            attack_step: 1.0 / (attack_left.max(1) as f32),
            hold_left: hold,
            held: 1.0,
            // 0 dB is exactly 1: an unsagged hold stays at full level.
            sag_mul: if sag_db < 0.0 && hold > 0 { fall_per_sample(sag_db, hold_ms) } else { 1.0 },
            release_mul: fall_per_sample(-60.0, release_ms),
            level: if attack_left > 0 { 0.0 } else { 1.0 },
            drive_gain,
            drive_norm: 1.0 / crate::math::tanh(drive_gain),
            click,
            click_mul: fall_per_sample(-60.0, KICK_CLICK_MS),
            click_filter,
            rng: crate::rng::Rng::new(KICK_CLICK_SEED),
            done: false,
        }
    }

    #[inline]
    pub fn next_sample(&mut self) -> f32 {
        if self.done {
            return 0.0;
        }
        self.step_level();
        let body = crate::math::sin(self.phase * crate::math::TWO_PI) * self.level;
        // Saturated with its level, so the hit drives harder than the tail.
        let body =
            if self.drive_gain > 1.0 { crate::math::tanh(body * self.drive_gain) * self.drive_norm } else { body };
        let click = if self.click > 0.0 {
            let c = self.click_filter.process(self.rng.next_bipolar()) * self.click;
            self.click *= self.click_mul;
            if self.click < KICK_FLOOR * KICK_FLOOR {
                self.click = 0.0;
            }
            c
        } else {
            0.0
        };

        let freq = self.freq();
        self.phase += freq / SAMPLE_RATE;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        self.step_pitch();

        if self.level < KICK_FLOOR && self.attack_left == 0 && self.hold_left == 0 && self.click == 0.0 {
            self.done = true;
        }
        body + click
    }

    pub fn is_idle(&self) -> bool {
        self.done
    }

    /// The kick for hit number `hit` of its instrument: with `vary=` or
    /// `vary_pitch=`, its hold and release, or its whole sweep, swayed by a
    /// share that drifts slowly from hit to hit -- two slow sines of periods
    /// that do not divide, 37 and 13.7 hits, so the pulse breathes over bars
    /// and does not repeat soon. The same hit number is always the same kick.
    pub fn new_hit(spec: &NodeSpec, hit: u32) -> Self {
        let NodeSpec::Kick { vary, vary_pitch, .. } = *spec else { unreachable!("KickState::new_hit takes a kick") };
        if vary <= 0.0 && vary_pitch <= 0.0 {
            return Self::new(spec);
        }
        let mut s = *spec;
        let n = hit as f32;
        let sway = |a: f32, pa: f32, b: f32, pb: f32| {
            0.6 * crate::math::sin(crate::math::TWO_PI * n / a + pa)
                + 0.4 * crate::math::sin(crate::math::TWO_PI * n / b + pb)
        };
        if let NodeSpec::Kick { note, start, knee, chirp, hold_ms, release_ms, .. } = &mut s {
            let decay = 1.0 + vary * sway(37.0, 0.3, 13.7, 1.1);
            *hold_ms *= decay;
            *release_ms *= decay;
            let pitch = 1.0 + vary_pitch * sway(29.0, 2.0, 11.1, 0.5);
            *note *= pitch;
            *start *= pitch;
            *knee *= pitch;
            *chirp *= pitch;
        }
        Self::new(&s)
    }

    /// The level: attack, then hold, then release.
    #[inline]
    fn step_level(&mut self) {
        if self.attack_left > 0 {
            self.attack_left -= 1;
            self.level = (self.level + self.attack_step).min(1.0);
        } else if self.hold_left > 0 {
            self.hold_left -= 1;
            self.level = self.held;
            self.held *= self.sag_mul;
        } else {
            self.level *= self.release_mul;
        }
    }

    /// The pitch parts alone, to look one sample ahead.
    fn shape(&self) -> KickPitch {
        KickPitch {
            note: self.note,
            body_part: self.body_part,
            body_mul: self.body_mul,
            snap_part: self.snap_part,
            snap_mul: self.snap_mul,
            chirp_part: self.chirp_part,
            chirp_mul: self.chirp_mul,
        }
    }

    #[inline]
    fn freq(&self) -> f32 {
        self.note + self.body_part + self.snap_part + self.chirp_part
    }

    /// The peak of a cycle at this level once `drive` has saturated it.
    fn sounding_level(&self) -> f32 {
        if self.drive_gain > 1.0 {
            crate::math::tanh(self.level * self.drive_gain) * self.drive_norm
        } else {
            self.level
        }
    }

    #[inline]
    fn step_pitch(&mut self) {
        self.body_part *= self.body_mul;
        self.snap_part *= self.snap_mul;
        self.chirp_part *= self.chirp_mul;
    }
}

/// A kick's pitch, without its level or its sound.
struct KickPitch {
    note: f32,
    body_part: f32,
    body_mul: f32,
    snap_part: f32,
    snap_mul: f32,
    chirp_part: f32,
    chirp_mul: f32,
}

impl KickPitch {
    fn step(&mut self) {
        self.body_part *= self.body_mul;
        self.snap_part *= self.snap_mul;
        self.chirp_part *= self.chirp_mul;
    }

    fn freq(&self) -> f32 {
        self.note + self.body_part + self.snap_part + self.chirp_part
    }
}

/// From here a kick's sweep is heard as pitch; before it, as the click of
/// the hit.
pub const HEARD_FROM_MS: f32 = 5.0;
/// Under this, against the hit's top, it no longer counts.
pub const HEARD_DOWN_DB: f32 = -30.0;
/// Semitones per millisecond at which a falling pitch counts half: the ear
/// needs a few steady cycles to hear a pitch, and a sweep moving faster than
/// this is heard more as a fall than as a note.
pub const HEARD_STEADY_ST_PER_MS: f32 = 0.1;

/// How much a moment counts towards the note heard: its energy, times how
/// steady its pitch is.
#[inline]
pub fn heard_weight(level: f32, st_per_ms: f32) -> f32 {
    let r = st_per_ms / HEARD_STEADY_ST_PER_MS;
    level * level / (1.0 + r * r)
}

/// The note a kick is heard as, in Hz. A sweep that is still falling while
/// the kick is loud is heard over the note it lands on: a kick tuned to A1
/// with a slow body reads as a C or a D. This is a model of that, not a
/// law of the ear: the mean of the pitch on a log scale, each moment weighted
/// by its energy and by how steady its pitch is ([`heard_weight`]), from
/// [`HEARD_FROM_MS`] until the level is [`HEARD_DOWN_DB`] under its top.
/// The level is the one that sounds, after `drive`: saturation flattens the
/// top and lifts the tail against it, so a driven kick is heard nearer the
/// note it lands on. `tatum analyze` reads a record's kick the same way, from
/// its averaged hit.
pub fn kick_heard_hz(spec: &NodeSpec) -> f32 {
    let NodeSpec::Kick { note, .. } = *spec else {
        return 0.0;
    };
    let mut k = KickState::new(spec);
    let from = (HEARD_FROM_MS * 0.001 * SAMPLE_RATE) as u32;
    let floor = crate::math::pow(10.0, HEARD_DOWN_DB / 20.0);
    let (mut sum, mut weight) = (0.0f32, 0.0f32);
    let mut top = 0.0f32;
    // Until it is `HEARD_DOWN_DB` down, which the release sees to; eight
    // seconds at most, over the longest attack, hold and release together.
    for i in 0..(8 * SAMPLE_RATE as u32) {
        k.step_level();
        let level = k.sounding_level();
        top = top.max(level);
        if i >= from {
            if level < floor * top && k.attack_left == 0 && k.hold_left == 0 {
                break;
            }
            let f = k.freq();
            let mut next = k.shape();
            next.step();
            // Semitones the pitch falls in this sample, per ms.
            let st_per_ms = 12.0 / core::f32::consts::LN_2 * crate::math::ln(f / next.freq()) * SAMPLE_RATE * 0.001;
            let w = heard_weight(level, crate::math::abs(st_per_ms));
            sum += w * crate::math::ln(f);
            weight += w;
        }
        k.step_pitch();
    }
    if weight > 0.0 {
        crate::math::exp(sum / weight)
    } else {
        note
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
    Osc(NoteOsc),
    PitchOsc(PitchOscState),
    Kick(KickState),
    Hat(HatState),
    Snare(SnareState),
    Clap(ClapState),
    Noise(NoiseGen),
    Lfo(Lfo),

    // ── Envelopes ──
    Env(Envelope),

    // ── Filters (with optional modulation envelope) ──
    Biquad(ModBiquad),
    Ladder(ModLadder),
    Svf(ModSvf),

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
    ParamEq(ParamEq),
    Clip(Clipper),
    Transient(Transient),
    Delay(Delay),
    Reverb(Box<DattorroReverb>),
    Capture(Capture),

    // ── Output ──
    Output,
}

impl AsMut<NodeKind> for NodeKind {
    fn as_mut(&mut self) -> &mut NodeKind {
        self
    }
}

impl NodeKind {
    /// Process one sample. Reads from `inputs` (gathered from connected nodes).
    #[inline]
    pub fn process(&mut self, inputs: &[f32; MAX_NODE_INPUTS], input_count: u8) -> f32 {
        match self {
            NodeKind::Osc(osc) => osc.next_sample(),
            NodeKind::PitchOsc(po) => po.next_sample(),
            NodeKind::Kick(k) => k.next_sample(),
            NodeKind::Hat(h) => h.next_sample(),
            NodeKind::Snare(s) => s.next_sample(),
            NodeKind::Clap(c) => c.next_sample(),
            NodeKind::Noise(noise) => noise.next_sample(),
            NodeKind::Lfo(lfo) => lfo.next_sample(),
            NodeKind::Capture(c) => c.process(inputs[0]),
            NodeKind::Env(env) => env.next_sample(),
            NodeKind::Biquad(m) => m.process(inputs[0]),
            NodeKind::Ladder(m) => m.process(inputs[0]),
            NodeKind::Svf(m) => m.process(inputs[0]),
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
            NodeKind::ParamEq(eq) => eq.process(inputs[0]),
            NodeKind::Clip(c) => c.process(inputs[0]),
            NodeKind::Transient(t) => t.process(inputs[0]),
            NodeKind::Delay(d) => d.process(inputs[0]),
            NodeKind::Reverb(r) => r.process(inputs[0]),

            NodeKind::Output => inputs[0],
        }
    }

    /// Take this sample's modulation, from the ports `mask` says are wired.
    /// Called before [`process`](Self::process), and only on a node that has
    /// something wired into a port.
    #[inline]
    pub fn modulate(&mut self, mods: &[f32; port::COUNT], mask: u8) {
        let cut = (mask & port::bit(port::CUTOFF) != 0).then(|| mods[port::CUTOFF as usize]);
        match self {
            NodeKind::Osc(osc) => osc.modulate(mods, mask),
            NodeKind::Ladder(m) => m.cut_mod = cut,
            NodeKind::Svf(m) => {
                m.cut_mod = cut;
                m.morph_mod = mods[port::MORPH as usize];
            }
            _ => {}
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
            // Each holds state per channel, or follows both channels together:
            // the dual-mono fallback would run both through one state.
            NodeKind::ParamEq(eq) => eq.process_stereo(l, r),
            NodeKind::Clip(c) => c.process_stereo(l, r),
            NodeKind::Transient(t) => t.process_stereo(l, r),
            NodeKind::Bitcrusher(bc) => bc.process_stereo(l, r),
            NodeKind::Saturator(sat) => (sat.process(l), sat.process(r)),
            NodeKind::Gain(g) => (l * *g, r * *g),
            NodeKind::Phaser(p) => p.process_stereo(l, r),
            // Filters and the chorus each hold a delay line or state registers.
            // The dual-mono fallback below clocks them twice per stereo frame
            // and runs both channels through one copy of that state.
            NodeKind::Biquad(m) => m.process_stereo(l, r),
            NodeKind::Ladder(m) => m.process_stereo(l, r),
            NodeKind::Svf(m) => m.process_stereo(l, r),
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
            (NodeKind::Clip(c), "drive") => {
                c.set_drive(value);
                true
            }
            (NodeKind::Gain(g), "gain") => {
                *g = value;
                true
            }
            (NodeKind::Vowel(v), "position") => {
                v.set_position(value);
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
            (NodeKind::Svf(m), "cutoff") => {
                m.base_cutoff = value;
                m.filter.set_cutoff(value);
                m.filter_r.set_cutoff(value);
                true
            }
            // Not the bandpass, which has no mode to turn: it passes the knob
            // on to the next `svf` of the chain.
            (NodeKind::Svf(m), "morph") if !m.bandpass => {
                m.morph = crate::math::clamp(value, 0.0, 1.0);
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
            (NodeKind::Svf(m), "cutoff") => {
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
            NodeKind::Svf(m) => m.set_bpm(bpm),
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
            NodeKind::Svf(m) => m.gate_on(),
            _ => {}
        }
    }

    pub fn gate_off(&mut self) {
        match self {
            NodeKind::Env(env) => env.gate_off(),
            NodeKind::Ladder(m) => m.gate_off(),
            NodeKind::Biquad(m) => m.gate_off(),
            NodeKind::Svf(m) => m.gate_off(),
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
            NodeKind::Svf(m) => m.velocity = vel,
            _ => {}
        }
    }

    pub fn is_idle(&self) -> bool {
        match self {
            NodeKind::Env(env) => env.is_idle(),
            NodeKind::Kick(k) => k.is_idle(),
            NodeKind::Hat(h) => h.is_idle(),
            NodeKind::Snare(s) => s.is_idle(),
            NodeKind::Clap(c) => c.is_idle(),
            _ => false,
        }
    }

    /// Become the drum `spec` describes for hit number `hit` of its
    /// instrument, in place: what a node that sways ([`NodeSpec::sways`]) is
    /// on that hit. Anything else is left as it is.
    pub fn renew_hit(&mut self, spec: &NodeSpec, hit: u32) {
        match self {
            NodeKind::Kick(k) => *k = KickState::new_hit(spec, hit),
            NodeKind::Hat(h) => *h = HatState::new_hit(spec, hit),
            NodeKind::Snare(s) => *s = SnareState::new_hit(spec, hit),
            NodeKind::Clap(c) => *c = ClapState::new_hit(spec, hit),
            _ => {}
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
            (NodeKind::Delay(d), NodeSpec::Delay { feedback, wow, flutter, .. }) => {
                d.reset();
                d.set_feedback(*feedback);
                d.set_wow(*wow);
                d.set_flutter(*flutter);
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
            // A kick is made again on every hit (`renew`); reset only stops it.
            NodeKind::Kick(k) => k.done = true,
            NodeKind::Hat(h) => h.done = true,
            NodeKind::Snare(s) => s.done = true,
            NodeKind::Clap(c) => c.done = true,
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
            NodeKind::Svf(m) => m.reset(),
            NodeKind::Mix | NodeKind::Gain(_) | NodeKind::Vca | NodeKind::Output => {}
            NodeKind::Saturator(_) => {}
            NodeKind::Chorus(c) => c.reset(),
            NodeKind::Bitcrusher(bc) => bc.reset(),
            NodeKind::Compressor(comp) => comp.reset(),
            NodeKind::Limiter(lim) => lim.reset(),
            NodeKind::TiltEq(eq) => eq.reset(),
            NodeKind::ThreeBandEq(eq) => eq.reset(),
            NodeKind::ParamEq(eq) => eq.reset(),
            NodeKind::Clip(c) => c.reset(),
            NodeKind::Transient(t) => t.reset(),
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
        renews_like_new(NodeSpec::Delay { sync_div: 0.25, feedback: 0.6, wow: 0.0, flutter: 0.0 });
        renews_like_new(NodeSpec::Delay { sync_div: 0.25, feedback: 0.6, wow: 0.7, flutter: 0.5 });
        renews_like_new(NodeSpec::Capture { samples: 4_000, start_samples: 0, speed: 0.5, reverse: true, mix: 1.0 });
        renews_like_new(NodeSpec::Chorus { mix: 0.5 });
        renews_like_new(NodeSpec::Saturator { drive: 2.0 });
        renews_like_new(NodeSpec::ParamEq { band: Band::Bell, freq: 300.0, gain_db: -6.0, q: 2.0 });
        renews_like_new(NodeSpec::Clip { drive: 4.0, ceiling: 0.7 });
        renews_like_new(NodeSpec::Transient { attack_db: 6.0, sustain_db: -6.0 });
        renews_like_new(NodeSpec::Kick {
            note: 55.0,
            start: 900.0,
            knee: 165.0,
            snap_ms: 4.0,
            chirp: 3000.0,
            chirp_ms: 1.0,
            body_ms: 70.0,
            attack_ms: 1.0,
            hold_ms: 80.0,
            sag_db: -2.0,
            release_ms: 160.0,
            phase: 0.25,
            click: 0.02,
            tone: 3000.0,
            drive: 0.3,
            vary: 0.0,
            vary_pitch: 0.0,
        });
        renews_like_new(NodeSpec::Hat {
            decay_ms: 80.0,
            band: 8000.0,
            floor: 6000.0,
            noise: 0.3,
            pitch: 205.0,
            gain: 1.0,
            vary: 0.0,
        });
        renews_like_new(NodeSpec::Snare {
            note: 185.0,
            snap_ms: 8.0,
            body_ms: 120.0,
            wires_ms: 180.0,
            band: 4500.0,
            balance: 0.5,
            drive: 0.3,
            vary: 0.0,
        });
        renews_like_new(NodeSpec::Clap { bursts: 4.0, spacing_ms: 11.0, tail_ms: 220.0, band: 1200.0, vary: 0.0 });
    }

    /// With `mid` at the start, a sweep plays sample for sample what it did
    /// before there was a second stage, rising or falling: the old sweep,
    /// written out here as it was.
    #[test]
    fn one_stage_plays_the_old_sweep_bit_for_bit() {
        for (start, end) in [(50.0, 300.0), (300.0, 50.0)] {
            let spec = NodeSpec::PitchOsc {
                waveform: Waveform::Sine,
                start_freq: start,
                end_freq: end,
                decay: 0.999,
                mid_freq: start,
                fast: 0.99,
            };
            let mut node = spec.instantiate();
            let mut osc = Oscillator::new(Waveform::Sine, SAMPLE_RATE);
            osc.set_frequency(start);
            osc.set_drift_enabled(false);
            let mut freq = start;
            for i in 0..20_000 {
                let old = osc.next_sample();
                freq = end + (freq - end) * crate::per_sample(0.999);
                osc.set_frequency(freq);
                let new = node.process(&[0.0; MAX_NODE_INPUTS], 1);
                assert_eq!(new.to_bits(), old.to_bits(), "{start} to {end} differs at sample {i}");
            }
        }
    }

    #[test]
    fn a_node_is_small_enough_to_keep_per_voice() {
        assert!(core::mem::size_of::<NodeKind>() <= 512, "NodeKind is {} bytes", core::mem::size_of::<NodeKind>());
    }
}

#[cfg(test)]
mod kick_tests {
    use super::*;

    /// The highest a kick can be asked to start, chirp and all, plays under
    /// half the rate at whatever rate the engine is built for (the CI renders
    /// at 22050 and 32000 Hz too).
    #[test]
    fn the_kick_never_starts_over_half_the_rate() {
        let spec = NodeSpec::Kick {
            note: 200.0,
            start: 5000.0,
            knee: 2000.0,
            snap_ms: 4.0,
            chirp: 12000.0,
            chirp_ms: 1.0,
            body_ms: 70.0,
            attack_ms: 0.0,
            hold_ms: 80.0,
            sag_db: 0.0,
            release_ms: 160.0,
            phase: 0.0,
            click: 0.0,
            tone: 3000.0,
            drive: 0.0,
            vary: 0.0,
            vary_pitch: 0.0,
        };
        let NodeKind::Kick(k) = spec.instantiate() else { panic!("a kick") };
        let top = k.note + k.body_part + k.snap_part + k.chirp_part;
        assert!(top <= 0.45 * SAMPLE_RATE + 0.5, "starts at {top} Hz at {SAMPLE_RATE} Hz");
        assert!(k.snap_part >= 0.0 && k.chirp_part >= 0.0 && k.body_part > 0.0);
    }

    /// The nodes of one voice of the first instrument, after a second of it.
    fn played(body: &str) -> (crate::graph::GraphTemplate, [Option<NodeKind>; crate::graph::MAX_GRAPH_NODES]) {
        let src = alloc::format!(
            "tempo 120\ninstrument i {{\n{body}\n}}\npattern p {{ C3 }}\ntrack t {{ play p using i out > master }}\n"
        );
        let ast = crate::dsl::parse(&src).unwrap_or_else(|e| panic!("{e:?}"));
        let song = crate::dsl::compiler::compile(&ast).unwrap_or_else(|e| panic!("{e:?}"));
        let template = song.instruments[0].as_graph().expect("an instrument graph").clone();
        let mut nodes = template.create_nodes();
        let mut bufs = alloc::boxed::Box::new([[0.0f32; crate::BLOCK_SIZE]; crate::graph::MAX_GRAPH_NODES]);
        for i in 0..SAMPLE_RATE as usize {
            template.run_frame(&mut bufs, i % crate::BLOCK_SIZE, &mut nodes);
        }
        (template, nodes)
    }

    /// Whether any node has left the path it took before there were ports.
    fn modulated(nodes: &[Option<NodeKind>]) -> bool {
        nodes.iter().flatten().any(|n| match n {
            NodeKind::Osc(o) => o.mods.on,
            NodeKind::Ladder(m) => m.cut_mod.is_some(),
            NodeKind::Svf(m) => m.cut_mod.is_some() || m.morph_mod != 0.0,
            _ => false,
        })
    }

    /// A graph with no port in it never reaches the modulate step: every
    /// mask is zero, and after a second no oscillator has switched to its
    /// modulated path and no filter has been handed a cutoff to add. This
    /// pins the switch, not the samples: that the path behind it is the one
    /// main plays is checked by comparing renders, not here.
    #[test]
    fn a_graph_without_ports_never_reaches_the_modulate_step() {
        const VOICE: &str = "  osc saw(55) as a\n  osc pulse(55, pitch=7) as b\n  lfo(3, 0.5) as wob\n  a > mix\n  b > mix\n  wob > mix\n  mix > svf(800, 0.5) as f\n  f > ladder(2000, 0.3) as two\n  two > adsr(0.001, 0.1, 1.0, 0.1) as amp\n  amp > out";
        let (template, nodes) = played(VOICE);
        assert!(template.mod_masks.iter().all(|&m| m == 0));
        assert!(!modulated(&nodes));
        // The same graph with one port wired does, so the check can fail.
        let (template, nodes) = played(&alloc::format!("{VOICE}\n  lfo(2, 0.1) > a.pitch"));
        assert!(template.mod_masks.iter().any(|&m| m != 0));
        assert!(modulated(&nodes));
    }
}
