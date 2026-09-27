//! Parameter registry — the single source of truth for every named module parameter.
//!
//! Everything that talks about a parameter by name (DSL compiler, live param
//! changes, automation, CLI docs, UI knobs) goes through this table. Adding a
//! parameter means adding one `ParamSpec` here; unknown names are rejected
//! everywhere instead of being silently ignored.
//!
//! Values are still normalized floats on the wire (the ranges the modules'
//! `set_param` expect). `Range::Choice` parameters additionally accept a
//! symbolic name in the DSL (`waveform half_sine`), which maps to
//! `index / (count - 1)` — the encoding every module already uses.

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::modules::bass::BassParam;
use crate::modules::fm::FmParam;
use crate::modules::keys::KeysParam;
use crate::modules::beats::BeatsParam;

/// Which built-in module a parameter belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleKind {
    Bass,
    Fm,
    Keys,
    Beats,
}

impl ModuleKind {
    // should_implement_trait: FromStr would have to return Result, which means
    // inventing an error none of the eight call sites want; renaming is a
    // breaking change to a public method for no gain.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "bass" => Some(Self::Bass),
            "fm" => Some(Self::Fm),
            "keys" => Some(Self::Keys),
            "beats" => Some(Self::Beats),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bass => "bass",
            Self::Fm => "fm",
            Self::Keys => "keys",
            Self::Beats => "beats",
        }
    }

    pub const ALL: [ModuleKind; 4] = [Self::Bass, Self::Fm, Self::Keys, Self::Beats];
}

/// Typed parameter id, wrapping each module's own enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamId {
    Bass(BassParam),
    Fm(FmParam),
    Keys(KeysParam),
    Beats(BeatsParam),
}

/// Accepted value range for a parameter, as the module's `set_param` interprets it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Range {
    /// Normalized 0.0 ..= 1.0.
    Unit,
    /// Bipolar -1.0 ..= 1.0 (pans).
    Bipolar,
    /// Linear gain 0.0 ..= max.
    Gain { max: f32 },
    /// Discrete choice. Index `i` encodes as `i / (len - 1)`.
    Choice(&'static [&'static str]),
}

impl Range {
    pub fn min(&self) -> f32 {
        match self {
            Range::Bipolar => -1.0,
            _ => 0.0,
        }
    }

    pub fn max(&self) -> f32 {
        match self {
            Range::Gain { max } => *max,
            _ => 1.0,
        }
    }

    pub fn contains(&self, v: f32) -> bool {
        v >= self.min() && v <= self.max()
    }

    /// Human-readable description of the range, for docs and error messages.
    pub fn describe(&self) -> String {
        match self {
            Range::Unit => String::from("0.0..1.0"),
            Range::Bipolar => String::from("-1.0..1.0"),
            Range::Gain { max } => alloc::format!("0.0..{:.1}", max),
            Range::Choice(names) => names.join(" | "),
        }
    }
}

/// The real-world quantity a parameter's knob maps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Hz,
    /// Milliseconds.
    Ms,
    Semitones,
    /// A plain multiplier (2.0 = double the note's frequency).
    Ratio,
}

impl Unit {
    pub fn suffix(self) -> &'static str {
        match self {
            Unit::Hz => "hz",
            Unit::Ms => "ms",
            Unit::Semitones => "st",
            Unit::Ratio => "x",
        }
    }

    /// A written suffix, and the factor that brings it to this unit's canonical
    /// form (`khz` is 1000 Hz, `s` is 1000 ms).
    pub fn from_suffix(s: &str) -> Option<(Unit, f32)> {
        match s {
            "hz" => Some((Unit::Hz, 1.0)),
            "khz" => Some((Unit::Hz, 1000.0)),
            "ms" => Some((Unit::Ms, 1.0)),
            "s" | "sec" => Some((Unit::Ms, 1000.0)),
            "st" => Some((Unit::Semitones, 1.0)),
            "x" => Some((Unit::Ratio, 1.0)),
            _ => None,
        }
    }
}

/// How a normalized 0..1 knob maps to a real quantity.
///
/// The module's `set_param` uses the same constant, so there is one definition
/// of each mapping rather than one in the module and one in the docs. Thirty-five
/// registry defaults were fiction for exactly that reason.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Curve {
    /// No natural unit: the number is the value (resonance, depths, mixes).
    None,
    /// `real = scale * base^knob`
    Exp { scale: f32, base: f32, unit: Unit },
    /// `real = min + knob * (max - min)`
    Lin { min: f32, max: f32, unit: Unit },
}

impl Curve {
    pub const fn exp(scale: f32, base: f32, unit: Unit) -> Self {
        Curve::Exp { scale, base, unit }
    }

    pub const fn lin(min: f32, max: f32, unit: Unit) -> Self {
        Curve::Lin { min, max, unit }
    }

    pub fn unit(&self) -> Option<Unit> {
        match self {
            Curve::None => None,
            Curve::Exp { unit, .. } | Curve::Lin { unit, .. } => Some(*unit),
        }
    }

    /// Knob position to real quantity. This is what the modules call.
    pub fn to_real(&self, knob: f32) -> f32 {
        match self {
            Curve::None => knob,
            Curve::Exp { scale, base, .. } => scale * crate::math::pow(*base, knob),
            Curve::Lin { min, max, .. } => min + knob * (max - min),
        }
    }

    /// Real quantity back to knob position, for `cutoff 800hz` in a song.
    pub fn to_knob(&self, real: f32) -> f32 {
        match self {
            Curve::None => real,
            Curve::Exp { scale, base, .. } => {
                if real <= 0.0 || *scale <= 0.0 || *base <= 1.0 {
                    0.0
                } else {
                    crate::math::ln(real / scale) / crate::math::ln(*base)
                }
            }
            Curve::Lin { min, max, .. } => {
                if (max - min).abs() < 1e-12 { 0.0 } else { (real - min) / (max - min) }
            }
        }
    }

    /// The span this curve covers, for docs and error messages.
    pub fn describe(&self) -> String {
        match self {
            Curve::None => String::new(),
            Curve::Exp { unit, .. } | Curve::Lin { unit, .. } => {
                let (lo, hi) = (self.to_real(0.0), self.to_real(1.0));
                alloc::format!("{}..{}", write_amount(lo, *unit), write_amount(hi, *unit))
            }
        }
    }
}

/// A quantity as someone would write it: `20hz`, `20khz`, `2s`, `-24st`.
/// Rounded to three significant figures, because the engine's `pow` is accurate
/// to 0.07% and a range that reads `19987.07hz` is noise, not precision.
pub fn write_amount(v: f32, unit: Unit) -> String {
    let (v, suffix) = match unit {
        Unit::Hz if crate::math::abs(v) >= 1000.0 => (v / 1000.0, "khz"),
        Unit::Ms if crate::math::abs(v) >= 1000.0 => (v / 1000.0, "s"),
        _ => (v, unit.suffix()),
    };
    let magnitude = crate::math::abs(v);
    let text = if magnitude >= 100.0 {
        alloc::format!("{}", crate::math::floor(v + 0.5) as i32)
    } else if magnitude >= 10.0 {
        round_to(v, 10.0)
    } else {
        round_to(v, 100.0)
    };
    alloc::format!("{}{}", text, suffix)
}

/// Round to a fixed number of decimals and drop trailing zeros.
fn round_to(v: f32, scale: f32) -> String {
    let rounded = crate::math::floor(v * scale + if v < 0.0 { -0.5 } else { 0.5 }) / scale;
    let whole = crate::math::floor(rounded + if rounded < 0.0 { -0.5 } else { 0.5 });
    if crate::math::abs(rounded - whole) < 1e-4 {
        alloc::format!("{}", whole as i32)
    } else if scale >= 100.0 {
        alloc::format!("{:.2}", rounded)
    } else {
        alloc::format!("{:.1}", rounded)
    }
}

/// Full description of one named parameter.
#[derive(Debug, Clone, Copy)]
pub struct ParamSpec {
    pub name: &'static str,
    pub id: ParamId,
    pub range: Range,
    pub default: f32,
    /// The real quantity the knob maps to, shared with the module that reads it.
    pub curve: Curve,
    /// What the normalized value maps to inside the module (units, curve).
    pub doc: &'static str,
}

impl ParamSpec {
    /// Resolve a symbolic value (`half_sine`, `unison`) for `Choice` params.
    pub fn value_from_name(&self, name: &str) -> Option<f32> {
        match self.range {
            Range::Choice(names) => {
                let idx = names.iter().position(|n| *n == name)?;
                Some(choice_value(idx, names.len()))
            }
            _ => None,
        }
    }

    /// Name of the choice a normalized value decodes to, if this is a `Choice` param.
    pub fn choice_name(&self, value: f32) -> Option<&'static str> {
        match self.range {
            Range::Choice(names) => {
                let idx = (value * (names.len() - 1) as f32) as usize;
                names.get(idx.min(names.len() - 1)).copied()
            }
            _ => None,
        }
    }

    /// Resolve a number written with a unit (`800hz`, `20ms`, `6db`, `80%`)
    /// into the normalized value the module wants. The error says what this
    /// parameter does accept, because a wrong unit is the kind of mistake that
    /// otherwise lands inside the valid range and goes unnoticed.
    pub fn value_from_quantity(&self, value: f32, suffix: &str) -> Result<f32, String> {
        match suffix {
            // A percentage is a percentage of unity, which reads the same on a
            // 0..1 knob and on a gain: `kick_level 100%` is 1.0 either way.
            "%" => match self.range {
                Range::Unit | Range::Bipolar | Range::Gain { .. } => Ok(value / 100.0),
                Range::Choice(_) => Err(self.unit_help(suffix)),
            },
            "db" => match self.range {
                Range::Gain { .. } => Ok(crate::math::pow(10.0, value / 20.0)),
                _ => Err(self.unit_help(suffix)),
            },
            _ => {
                let Some((unit, factor)) = Unit::from_suffix(suffix) else {
                    return Err(self.unit_help(suffix));
                };
                if self.curve.unit() != Some(unit) {
                    return Err(self.unit_help(suffix));
                }
                let real = value * factor;
                let mut knob = self.curve.to_knob(real);
                // The curve goes through the engine's approximate `log`, so
                // the top of a range written exactly (`attack 2s`) can land a
                // hair past the end of the knob. That is the end of the knob.
                if let Range::Unit = self.range {
                    if (-1e-3..0.0).contains(&knob) { knob = 0.0 }
                    if (1.0..1.0 + 1e-3).contains(&knob) { knob = 1.0 }
                }
                if !self.range.contains(knob) {
                    return Err(alloc::format!(
                        "'{}' = {}{} is outside {}",
                        self.name, value, suffix, self.curve.describe()
                    ));
                }
                Ok(knob)
            }
        }
    }

    /// A knob position written in this parameter's unit, with as few decimals
    /// as read back to the same knob, or `None` when it has no unit. What
    /// `tatum fmt --units` writes, so a song rewritten by it sounds the same.
    pub fn write_in_units(&self, knob: f32) -> Option<String> {
        let unit = self.curve.unit()?;
        let suffix = unit.suffix();
        let back = |real: f32| self.value_from_quantity(real, suffix).ok();
        // The quantity the knob maps to does not read back as the same knob:
        // the curves go through the engine's own `exp` and `log`, which do
        // not quite invert each other, and the round trip moved a knob by up
        // to 1e-4 -- enough to be heard drifting over a whole song. So the
        // quantity is searched for, by what it reads back as.
        let mut lo = self.curve.to_real(knob - 0.01);
        let mut hi = self.curve.to_real(knob + 0.01);
        let rising = back(hi).zip(back(lo)).is_none_or(|(h, l)| h >= l);
        if !rising { core::mem::swap(&mut lo, &mut hi); }
        for _ in 0..64 {
            let mid = (lo + hi) * 0.5;
            let below = match back(mid) {
                Some(k) => k < knob,
                None => mid < self.curve.to_real(knob),
            };
            if below == rising { lo = mid } else { hi = mid }
        }
        let real = (lo + hi) * 0.5;
        let mut best = (f32::MAX, String::new());
        for decimals in 0..=6 {
            let mut text = alloc::format!("{:.*}", decimals, real);
            if text.contains('.') {
                text = String::from(text.trim_end_matches('0').trim_end_matches('.'));
            }
            let miss = text.parse::<f32>().ok().and_then(back)
                .map_or(f32::MAX, |k| crate::math::abs(k - knob));
            if miss < best.0 { best = (miss, text); }
            if best.0 <= 1e-6 { break }
        }
        Some(alloc::format!("{}{suffix}", best.1))
    }

    fn unit_help(&self, suffix: &str) -> String {
        let mut accepted = Vec::new();
        if let Some(unit) = self.curve.unit() {
            accepted.push(alloc::format!("{} ({})", unit.suffix(), self.curve.describe()));
        }
        match self.range {
            Range::Unit | Range::Bipolar => accepted.push(String::from("a percentage like 80%")),
            Range::Gain { .. } => {
                accepted.push(String::from("decibels like 6db"));
                accepted.push(String::from("a percentage of unity like 80%"));
            }
            Range::Choice(_) => {}
        }
        accepted.push(alloc::format!("a plain number ({})", self.range.describe()));
        alloc::format!("'{}' has no unit '{}'. It takes {}", self.name, suffix, accepted.join(", or "))
    }

    /// Check a numeric value against the range. Returns a message on failure.
    pub fn validate(&self, value: f32) -> Result<(), String> {
        if self.range.contains(value) {
            Ok(())
        } else {
            Err(alloc::format!(
                "'{}' = {} is out of range ({})",
                self.name, value, self.range.describe()
            ))
        }
    }
}

/// Encode a choice index as the normalized float the modules expect.
pub fn choice_value(idx: usize, count: usize) -> f32 {
    if count <= 1 { 0.0 } else { idx as f32 / (count - 1) as f32 }
}

// ── Shared choice tables ──

pub const LFO_WAVEFORMS: &[&str] = &["sine", "triangle", "saw", "square", "sample_hold"];
pub const LFO_SYNC: &[&str] = &["free", "quarter", "eighth", "sixteenth", "dotted_eighth", "triplet_eighth", "bar", "bars_2", "bars_4", "bars_8", "bars_12", "bars_16"];
pub const OSC_WAVES: &[&str] = &["saw", "square"];
pub const FM_ALGORITHMS: &[&str] = &[
    "serial3_plus_carrier",   // 0: Op3 -> Op2 -> Op1 -> Out (+ Op4 -> Out)
    "dual_pairs",             // 1: (Op2 -> Op1) -> Out, (Op4 -> Op3) -> Out
    "two_op",                 // 2: Op2 -> Op1 -> Out
    "additive",               // 3: all carriers
    "serial4",                // 4: Op4 -> Op3 -> Op2 -> Op1 -> Out
    "dual_mod",               // 5: (Op3 + Op4) -> Op2 -> Op1 -> Out
    "triple_mod",             // 6: (Op2 + Op3 + Op4) -> Op1 -> Out
    "pair_plus_two",          // 7: (Op2 -> Op1) -> Out + Op3 -> Out + Op4 -> Out
];
pub const FM_WAVEFORMS: &[&str] = &["sine", "half_sine", "abs_sine", "quarter_sine"];
pub const FM_LFO_TARGETS: &[&str] = &["pitch", "amplitude", "mod_index"];
pub const FILTER_LFO_TARGETS: &[&str] = &["cutoff", "pitch", "amplitude"];
pub const VOICE_MODES: &[&str] = &["poly", "unison", "octave", "fifth", "ringmod"];
pub const STUTTER_DRUMS: &[&str] = &["kick", "snare", "hihat", "clap", "tom"];

const ENV_TIME_DOC: &str = "Knob 0..1, not seconds. Maps exponentially to 1ms..2s: 0.25 ≈ 6.7ms, 0.5 ≈ 45ms, 0.75 ≈ 300ms, 1.0 = 2s";

macro_rules! spec {
    ($name:expr, $id:expr, $range:expr, $default:expr, $doc:expr) => {
        ParamSpec { name: $name, id: $id, range: $range, default: $default, curve: Curve::None, doc: $doc }
    };
    ($name:expr, $id:expr, $range:expr, $default:expr, $curve:expr, $doc:expr) => {
        ParamSpec { name: $name, id: $id, range: $range, default: $default, curve: $curve, doc: $doc }
    };
}

// ── Curves: one definition each, referenced by both the registry and the module ──

/// Envelope stage time, 1 ms to 2 s. In milliseconds, so the modules divide by
/// 1000 for the seconds their envelopes take.
pub const ENV_TIME: Curve = Curve::exp(1.0, 2000.0, Unit::Ms);
/// Ladder filter cutoff on `bass`.
pub const BASS_CUTOFF: Curve = Curve::exp(20.0, 1000.0, Unit::Hz);
/// Filter envelope depth on `bass`, added to the cutoff in Hz.
pub const BASS_CUTOFF_ENV: Curve = Curve::exp(20.0, 400.0, Unit::Hz);
/// Biquad cutoff on `keys`.
pub const KEYS_CUTOFF: Curve = Curve::exp(200.0, 100.0, Unit::Hz);
/// Free-running LFO rate.
pub const LFO_RATE: Curve = Curve::lin(0.1, 20.0, Unit::Hz);
/// Vibrato rate.
pub const VIBRATO_RATE: Curve = Curve::lin(0.5, 10.0, Unit::Hz);
/// FM operator frequency ratio.
pub const OP_RATIO: Curve = Curve::lin(0.5, 16.0, Unit::Ratio);
/// Oscillator 2 and 3 offset on `bass`.
pub const OSC_PITCH: Curve = Curve::lin(-24.0, 24.0, Unit::Semitones);
/// Drum pitch multiplier.
pub const DRUM_PITCH: Curve = Curve::lin(0.5, 2.0, Unit::Ratio);

// ── Bass ──

pub const BASS_PARAMS: &[ParamSpec] = &[
    spec!("cutoff", ParamId::Bass(BassParam::Cutoff), Range::Unit, 0.433677, BASS_CUTOFF, "Ladder filter cutoff, exponential 20Hz..20kHz (0.25 ≈ 110Hz, 0.5 ≈ 630Hz, 0.75 ≈ 3.5kHz)"),
    spec!("cutoff_env", ParamId::Bass(BassParam::CutoffEnv), Range::Unit, 0.884311, BASS_CUTOFF_ENV, "Filter envelope depth, exponential 20Hz..8kHz sweep"),
    spec!("resonance", ParamId::Bass(BassParam::Resonance), Range::Unit, 0.3, "Ladder resonance, self-oscillates near 1.0"),
    spec!("glide", ParamId::Bass(BassParam::Glide), Range::Unit, 0.058, "Portamento rate between notes (0 = instant)"),
    spec!("attack", ParamId::Bass(BassParam::Attack), Range::Unit, 0.211743, ENV_TIME, ENV_TIME_DOC),
    spec!("decay", ParamId::Bass(BassParam::Decay), Range::Unit, 0.697064, ENV_TIME, ENV_TIME_DOC),
    spec!("sustain", ParamId::Bass(BassParam::Sustain), Range::Unit, 0.8, "Amp envelope sustain level"),
    spec!("release", ParamId::Bass(BassParam::Release), Range::Unit, 0.659216, ENV_TIME, ENV_TIME_DOC),
    spec!("lfo_rate", ParamId::Bass(BassParam::LfoRate), Range::Unit, 0.0, LFO_RATE, "LFO rate 0.1..20Hz when lfo_sync is free"),
    spec!("lfo_depth", ParamId::Bass(BassParam::LfoDepth), Range::Unit, 0.0, "LFO depth; 0 disables the LFO. On cutoff it is relative: 1.0 sweeps ±4 octaves, so 0.1 is a gentle wobble"),
    spec!("lfo_waveform", ParamId::Bass(BassParam::LfoWaveform), Range::Choice(LFO_WAVEFORMS), 0.0, "LFO shape"),
    spec!("lfo_target", ParamId::Bass(BassParam::LfoTarget), Range::Choice(FILTER_LFO_TARGETS), 0.0, "What the LFO modulates"),
    spec!("lfo_sync", ParamId::Bass(BassParam::LfoSync), Range::Choice(LFO_SYNC), 0.0, "Free Hz, a tempo-synced subdivision, or a slow cycle over 1..16 bars"),
    spec!("osc2_pitch", ParamId::Bass(BassParam::Osc2Pitch), Range::Unit, 0.5, OSC_PITCH, "Osc 2 offset in semitones: 0.0 = -24, 0.5 = 0, 1.0 = +24 (steps of 1/48)"),
    spec!("osc3_pitch", ParamId::Bass(BassParam::Osc3Pitch), Range::Unit, 0.5, OSC_PITCH, "Osc 3 offset in semitones: 0.0 = -24, 0.5 = 0, 1.0 = +24 (steps of 1/48)"),
    spec!("osc1_wave", ParamId::Bass(BassParam::Osc1Wave), Range::Choice(OSC_WAVES), 0.0, "Osc 1 waveform"),
    spec!("osc2_wave", ParamId::Bass(BassParam::Osc2Wave), Range::Choice(OSC_WAVES), 0.0, "Osc 2 waveform"),
    spec!("osc3_wave", ParamId::Bass(BassParam::Osc3Wave), Range::Choice(OSC_WAVES), 0.0, "Osc 3 waveform"),
    spec!("keytrack", ParamId::Bass(BassParam::Keytrack), Range::Unit, 0.0, "Filter cutoff follows note pitch"),
    spec!("vibrato_rate", ParamId::Bass(BassParam::VibratoRate), Range::Unit, 0.0, VIBRATO_RATE, "Vibrato rate 0.5..10Hz"),
    spec!("vibrato_depth", ParamId::Bass(BassParam::VibratoDepth), Range::Unit, 0.0, "Vibrato depth, up to half a semitone"),
    spec!("vel_env", ParamId::Bass(BassParam::VelEnv), Range::Unit, 0.0, "Velocity scales filter envelope depth"),
];

// ── FM ──

pub const FM_PARAMS: &[ParamSpec] = &[
    spec!("level", ParamId::Fm(FmParam::Level), Range::Gain { max: 4.0 }, 1.0, "Module output gain. FM sums quieter than the other modules: try 2.0 to sit near keys at the same track level"),
    spec!("algorithm", ParamId::Fm(FmParam::Algorithm), Range::Choice(FM_ALGORITHMS), 0.0, "Operator routing"),
    spec!("mod_index", ParamId::Fm(FmParam::ModIndex), Range::Unit, 0.624196, "Modulation index, exponential 0.1..4.0 (0.5 ≈ 0.63, 0.75 ≈ 1.6)"),
    spec!("feedback", ParamId::Fm(FmParam::Feedback), Range::Unit, 0.0, "Global operator feedback (scaled to 0..0.7)"),
    spec!("waveform", ParamId::Fm(FmParam::Waveform), Range::Choice(FM_WAVEFORMS), 0.0, "Operator waveform"),
    spec!("chorus_mix", ParamId::Fm(FmParam::ChorusMix), Range::Unit, 0.0, "Dedicated chorus wet mix"),
    spec!("attack", ParamId::Fm(FmParam::Attack), Range::Unit, 0.302936, ENV_TIME, "Carrier (op0) attack, exponential 1ms..2s"),
    spec!("decay", ParamId::Fm(FmParam::Decay), Range::Unit, 0.605872, ENV_TIME, "Carrier (op0) decay, exponential 1ms..2s"),
    spec!("sustain", ParamId::Fm(FmParam::Sustain), Range::Unit, 0.7, "Carrier (op0) sustain level"),
    spec!("release", ParamId::Fm(FmParam::Release), Range::Unit, 0.750409, ENV_TIME, "Carrier (op0) release, exponential 1ms..2s"),
    spec!("lfo_rate", ParamId::Fm(FmParam::LfoRate), Range::Unit, 0.0, LFO_RATE, "LFO rate 0.1..20Hz when lfo_sync is free"),
    spec!("lfo_depth", ParamId::Fm(FmParam::LfoDepth), Range::Unit, 0.0, "LFO depth; 0 disables the LFO"),
    spec!("lfo_waveform", ParamId::Fm(FmParam::LfoWaveform), Range::Choice(LFO_WAVEFORMS), 0.0, "LFO shape"),
    spec!("lfo_target", ParamId::Fm(FmParam::LfoTarget), Range::Choice(FM_LFO_TARGETS), 0.0, "What the LFO modulates"),
    spec!("lfo_sync", ParamId::Fm(FmParam::LfoSync), Range::Choice(LFO_SYNC), 0.0, "Free Hz or tempo-synced subdivision"),
    spec!("vibrato_rate", ParamId::Fm(FmParam::VibratoRate), Range::Unit, 0.0, VIBRATO_RATE, "Vibrato rate 0.5..10Hz"),
    spec!("vibrato_depth", ParamId::Fm(FmParam::VibratoDepth), Range::Unit, 0.0, "Vibrato depth, up to half a semitone"),
    spec!("op0_attack", ParamId::Fm(FmParam::Op0Attack), Range::Unit, 0.302936, ENV_TIME, ENV_TIME_DOC),
    spec!("op0_decay", ParamId::Fm(FmParam::Op0Decay), Range::Unit, 0.605872, ENV_TIME, ENV_TIME_DOC),
    spec!("op0_sustain", ParamId::Fm(FmParam::Op0Sustain), Range::Unit, 0.7, "Operator sustain level"),
    spec!("op0_release", ParamId::Fm(FmParam::Op0Release), Range::Unit, 0.750409, ENV_TIME, ENV_TIME_DOC),
    spec!("op1_attack", ParamId::Fm(FmParam::Op1Attack), Range::Unit, 0.302936, ENV_TIME, ENV_TIME_DOC),
    spec!("op1_decay", ParamId::Fm(FmParam::Op1Decay), Range::Unit, 0.605872, ENV_TIME, ENV_TIME_DOC),
    spec!("op1_sustain", ParamId::Fm(FmParam::Op1Sustain), Range::Unit, 0.7, "Operator sustain level"),
    spec!("op1_release", ParamId::Fm(FmParam::Op1Release), Range::Unit, 0.750409, ENV_TIME, ENV_TIME_DOC),
    spec!("op2_attack", ParamId::Fm(FmParam::Op2Attack), Range::Unit, 0.302936, ENV_TIME, ENV_TIME_DOC),
    spec!("op2_decay", ParamId::Fm(FmParam::Op2Decay), Range::Unit, 0.605872, ENV_TIME, ENV_TIME_DOC),
    spec!("op2_sustain", ParamId::Fm(FmParam::Op2Sustain), Range::Unit, 0.7, "Operator sustain level"),
    spec!("op2_release", ParamId::Fm(FmParam::Op2Release), Range::Unit, 0.750409, ENV_TIME, ENV_TIME_DOC),
    spec!("op3_attack", ParamId::Fm(FmParam::Op3Attack), Range::Unit, 0.302936, ENV_TIME, ENV_TIME_DOC),
    spec!("op3_decay", ParamId::Fm(FmParam::Op3Decay), Range::Unit, 0.605872, ENV_TIME, ENV_TIME_DOC),
    spec!("op3_sustain", ParamId::Fm(FmParam::Op3Sustain), Range::Unit, 0.7, "Operator sustain level"),
    spec!("op3_release", ParamId::Fm(FmParam::Op3Release), Range::Unit, 0.750409, ENV_TIME, ENV_TIME_DOC),
    spec!("op0_feedback", ParamId::Fm(FmParam::Op0Feedback), Range::Unit, 0.0, "Self-modulation of operator 0"),
    spec!("op1_feedback", ParamId::Fm(FmParam::Op1Feedback), Range::Unit, 0.0, "Self-modulation of operator 1"),
    spec!("op2_feedback", ParamId::Fm(FmParam::Op2Feedback), Range::Unit, 0.0, "Self-modulation of operator 2"),
    spec!("op3_feedback", ParamId::Fm(FmParam::Op3Feedback), Range::Unit, 0.0, "Self-modulation of operator 3"),
    spec!("op0_ratio", ParamId::Fm(FmParam::Op0Ratio), Range::Unit, 0.032258, OP_RATIO, "Frequency ratio, linear 0.5..16.0 (ratio = 0.5 + v*15.5: 0.032 = 1.0, 0.097 = 2.0, 0.161 = 3.0)"),
    spec!("op1_ratio", ParamId::Fm(FmParam::Op1Ratio), Range::Unit, 0.032258, OP_RATIO, "Frequency ratio, linear 0.5..16.0 (ratio = 0.5 + v*15.5: 0.032 = 1.0, 0.097 = 2.0, 0.161 = 3.0)"),
    spec!("op2_ratio", ParamId::Fm(FmParam::Op2Ratio), Range::Unit, 0.032258, OP_RATIO, "Frequency ratio, linear 0.5..16.0 (ratio = 0.5 + v*15.5: 0.032 = 1.0, 0.097 = 2.0, 0.161 = 3.0)"),
    spec!("op3_ratio", ParamId::Fm(FmParam::Op3Ratio), Range::Unit, 0.032258, OP_RATIO, "Frequency ratio, linear 0.5..16.0 (ratio = 0.5 + v*15.5: 0.032 = 1.0, 0.097 = 2.0, 0.161 = 3.0)"),
];

// ── Keys ──

pub const KEYS_PARAMS: &[ParamSpec] = &[
    spec!("cutoff", ParamId::Keys(KeysParam::Cutoff), Range::Unit, 0.588152, KEYS_CUTOFF, "Biquad lowpass cutoff, exponential 200Hz..20kHz (0.5 ≈ 2kHz)"),
    spec!("resonance", ParamId::Keys(KeysParam::Resonance), Range::Unit, 0.2, "Filter resonance"),
    spec!("detune", ParamId::Keys(KeysParam::Detune), Range::Unit, 0.15, "Osc detune, up to 2%"),
    spec!("chorus_mix", ParamId::Keys(KeysParam::ChorusMix), Range::Unit, 0.3, "Chorus wet mix"),
    spec!("level", ParamId::Keys(KeysParam::Level), Range::Gain { max: 2.0 }, 0.5, "Module output gain"),
    spec!("voice_mode", ParamId::Keys(KeysParam::VoiceMode), Range::Choice(VOICE_MODES), 0.0, "Volca Keys style voice allocation"),
    spec!("attack", ParamId::Keys(KeysParam::Attack), Range::Unit, 0.750409, ENV_TIME, ENV_TIME_DOC),
    spec!("decay", ParamId::Keys(KeysParam::Decay), Range::Unit, 0.817615, ENV_TIME, ENV_TIME_DOC),
    spec!("sustain", ParamId::Keys(KeysParam::Sustain), Range::Unit, 0.7, "Amp envelope sustain level"),
    spec!("release", ParamId::Keys(KeysParam::Release), Range::Unit, 0.908807, ENV_TIME, ENV_TIME_DOC),
    spec!("lfo_rate", ParamId::Keys(KeysParam::LfoRate), Range::Unit, 0.0, LFO_RATE, "LFO rate 0.1..20Hz when lfo_sync is free"),
    spec!("lfo_depth", ParamId::Keys(KeysParam::LfoDepth), Range::Unit, 0.0, "LFO depth; 0 disables the LFO. On cutoff it is relative: 1.0 sweeps ±4 octaves, so 0.1 is a gentle wobble"),
    spec!("lfo_waveform", ParamId::Keys(KeysParam::LfoWaveform), Range::Choice(LFO_WAVEFORMS), 0.0, "LFO shape"),
    spec!("lfo_target", ParamId::Keys(KeysParam::LfoTarget), Range::Choice(FILTER_LFO_TARGETS), 0.0, "What the LFO modulates"),
    spec!("lfo_sync", ParamId::Keys(KeysParam::LfoSync), Range::Choice(LFO_SYNC), 0.0, "Free Hz or tempo-synced subdivision"),
    spec!("vibrato_rate", ParamId::Keys(KeysParam::VibratoRate), Range::Unit, 0.0, VIBRATO_RATE, "Vibrato rate 0.5..10Hz"),
    spec!("vibrato_depth", ParamId::Keys(KeysParam::VibratoDepth), Range::Unit, 0.0, "Vibrato depth, up to half a semitone"),
];

// ── Beats ──

pub const BEATS_PARAMS: &[ParamSpec] = &[
    spec!("level", ParamId::Beats(BeatsParam::Level), Range::Gain { max: 2.0 }, 1.0, "Module output gain"),
    spec!("kick_level", ParamId::Beats(BeatsParam::KickLevel), Range::Gain { max: 2.0 }, 1.0, "Kick level. Applied after the voice's own saturator, so above 1.0 it pushes a signal already bounded at 1.0 past full scale: it is a boost, not a fader"),
    spec!("kick_decay", ParamId::Beats(BeatsParam::KickDecay), Range::Unit, 0.222222, "Kick amp decay, tight..long"),
    spec!("kick_pitch", ParamId::Beats(BeatsParam::KickPitch), Range::Unit, 0.333333, DRUM_PITCH, "Kick pitch multiplier 0.5..2.0"),
    spec!("kick_click", ParamId::Beats(BeatsParam::KickClick), Range::Unit, 0.7, "Noise transient at kick onset"),
    spec!("kick_drive", ParamId::Beats(BeatsParam::KickDrive), Range::Unit, 0.454545, "Kick saturation, warm..heavy"),
    spec!("kick_pan", ParamId::Beats(BeatsParam::KickPan), Range::Bipolar, 0.0, "Kick stereo position"),
    spec!("snare_level", ParamId::Beats(BeatsParam::SnareLevel), Range::Gain { max: 2.0 }, 1.0, "Snare level. Applied after the voice's own saturator, so above 1.0 it pushes a signal already bounded at 1.0 past full scale: it is a boost, not a fader"),
    spec!("snare_decay", ParamId::Beats(BeatsParam::SnareDecay), Range::Unit, 0.0, "Snare decay, tight..long"),
    spec!("snare_pitch", ParamId::Beats(BeatsParam::SnarePitch), Range::Unit, 0.333333, DRUM_PITCH, "Snare pitch multiplier 0.5..2.0"),
    spec!("snare_drive", ParamId::Beats(BeatsParam::SnareDrive), Range::Unit, 0.3125, "Snare saturation, clean..gritty"),
    spec!("snare_snap", ParamId::Beats(BeatsParam::SnareSnap), Range::Unit, 0.4, "Snare noise crack intensity"),
    spec!("snare_pan", ParamId::Beats(BeatsParam::SnarePan), Range::Bipolar, -0.1, "Snare stereo position"),
    spec!("hihat_level", ParamId::Beats(BeatsParam::HihatLevel), Range::Gain { max: 2.0 }, 1.0, "Hi-hat level. Applied after the voice's own saturator, so above 1.0 it pushes a signal already bounded at 1.0 past full scale: it is a boost, not a fader"),
    spec!("hihat_decay", ParamId::Beats(BeatsParam::HihatDecay), Range::Unit, 0.625, "Closed hat decay, tight..ringy"),
    spec!("hihat_pitch", ParamId::Beats(BeatsParam::HihatPitch), Range::Unit, 0.333333, DRUM_PITCH, "Hat pitch multiplier 0.5..2.0"),
    spec!("hihat_pan", ParamId::Beats(BeatsParam::HihatPan), Range::Bipolar, 0.3, "Hat stereo position"),
    spec!("clap_level", ParamId::Beats(BeatsParam::ClapLevel), Range::Gain { max: 2.0 }, 1.0, "Clap level. Applied after the voice's own saturator, so above 1.0 it pushes a signal already bounded at 1.0 past full scale: it is a boost, not a fader"),
    spec!("clap_pan", ParamId::Beats(BeatsParam::ClapPan), Range::Bipolar, 0.15, "Clap stereo position"),
    spec!("tom_pan", ParamId::Beats(BeatsParam::TomPan), Range::Bipolar, 0.0, "Tom stereo position"),
    spec!("crash_pan", ParamId::Beats(BeatsParam::CrashPan), Range::Bipolar, -0.2, "Crash stereo position"),
    spec!("stutter_rate", ParamId::Beats(BeatsParam::StutterRate), Range::Unit, 0.0, "Retrigger rate; below 0.1 = off"),
    spec!("stutter_drum", ParamId::Beats(BeatsParam::StutterDrum), Range::Choice(STUTTER_DRUMS), 0.0, "Which drum the stutter retriggers"),
];

// ── Lookup ──

/// All parameters of a module kind.
pub fn specs(kind: ModuleKind) -> &'static [ParamSpec] {
    match kind {
        ModuleKind::Bass => BASS_PARAMS,
        ModuleKind::Fm => FM_PARAMS,
        ModuleKind::Keys => KEYS_PARAMS,
        ModuleKind::Beats => BEATS_PARAMS,
    }
}

/// Find a parameter by name for a module kind.
pub fn lookup(kind: ModuleKind, name: &str) -> Option<&'static ParamSpec> {
    specs(kind).iter().find(|s| s.name == name)
}

/// Closest known parameter name for a typo, if reasonably close.
pub fn suggest(kind: ModuleKind, name: &str) -> Option<&'static str> {
    let mut best: Option<(usize, &'static str)> = None;
    for s in specs(kind) {
        let d = levenshtein(name, s.name);
        if best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, s.name));
        }
    }
    // Accept suggestions within a third of the name length (min 2 edits).
    let limit = (name.len() / 3).max(2);
    best.filter(|(d, _)| *d <= limit).map(|(_, n)| n)
}

/// Names that exist on some other module kind — used to explain "wrong module" mistakes.
pub fn kinds_with_param(name: &str) -> Vec<ModuleKind> {
    ModuleKind::ALL.iter().copied().filter(|k| lookup(*k, name).is_some()).collect()
}

pub(crate) fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = alloc::vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        core::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_per_module() {
        for kind in ModuleKind::ALL {
            let list = specs(kind);
            for (i, a) in list.iter().enumerate() {
                for b in &list[i + 1..] {
                    assert_ne!(a.name, b.name, "duplicate '{}' in {:?}", a.name, kind);
                }
            }
        }
    }

    #[test]
    fn defaults_are_in_range() {
        for kind in ModuleKind::ALL {
            for s in specs(kind) {
                assert!(s.range.contains(s.default), "{:?}.{} default out of range", kind, s.name);
            }
        }
    }

    #[test]
    fn choice_roundtrip() {
        let spec = lookup(ModuleKind::Fm, "algorithm").unwrap();
        for (i, name) in FM_ALGORITHMS.iter().enumerate() {
            let v = spec.value_from_name(name).unwrap();
            assert_eq!((v * 7.0) as usize, i, "algorithm {} must decode to index {}", name, i);
            assert_eq!(spec.choice_name(v), Some(*name));
        }
        let wave = lookup(ModuleKind::Bass, "osc1_wave").unwrap();
        assert_eq!(wave.value_from_name("saw"), Some(0.0));
        assert_eq!(wave.value_from_name("square"), Some(1.0));
        assert_eq!(wave.value_from_name("sine"), None);
    }

    #[test]
    fn suggestions() {
        assert_eq!(suggest(ModuleKind::Bass, "cutof"), Some("cutoff"));
        assert_eq!(suggest(ModuleKind::Bass, "resonanse"), Some("resonance"));
        assert_eq!(suggest(ModuleKind::Fm, "mod_indx"), Some("mod_index"));
        assert_eq!(suggest(ModuleKind::Bass, "kick_level"), None);
        assert_eq!(kinds_with_param("kick_level"), alloc::vec![ModuleKind::Beats]);
    }
}

// ── Track options (not module params, but the same "only valid names" promise) ──

/// One `track { }` option.
#[derive(Debug, Clone, Copy)]
pub struct TrackOption {
    pub name: &'static str,
    pub range: &'static str,
    pub default: &'static str,
    pub doc: &'static str,
}

pub const TRACK_OPTIONS: &[TrackOption] = &[
    TrackOption { name: "play", range: "pattern name", default: "required", doc: "Pattern the track loops" },
    TrackOption { name: "using", range: "module or instrument name", default: "required", doc: "Instrument that plays it" },
    TrackOption { name: "level", range: "0.0..4.0", default: "0.8", doc: "Output level after the instrument. Above 1.0 is boost: 2.0 is +6 dB, 4.0 is +12 dB" },
    TrackOption { name: "pan", range: "-1.0..1.0", default: "0.0", doc: "Stereo position" },
    TrackOption { name: "velocity", range: "0.0..1.0", default: "0.8", doc: "Scales every note's velocity" },
    TrackOption { name: "gate", range: "0.0..1.0", default: "0.85", doc: "Note length as a fraction of the step" },
    TrackOption { name: "delay_send", range: "0.0..1.0", default: "0.0", doc: "Amount into the global delay" },
    TrackOption { name: "reverb_send", range: "0.0..1.0", default: "0.0", doc: "Amount into the global reverb" },
    TrackOption { name: "sidechain", range: "0.0..1.0", default: "global `sidechain`", doc: "How much the kick ducks this track; only acts in scenes that have a beats track" },
    TrackOption { name: "arp", range: "up | down | updown | off, rate=4|8|16|32, gate=0.1..1, octaves=1..4", default: "none", doc: "Arpeggiate the held notes; a scene can add, change or turn it off" },
    TrackOption { name: "out", range: "`> node(...) > ... > master` or `> <bus>`", default: "`> master`", doc: "Insert chain and destination; see effects and nodes" },
];

/// Markdown table of track options.
pub fn track_markdown() -> String {
    let mut out = String::from("## track options\n\n");
    out.push_str("Inside `track name { ... }` at top level or in a scene. A scene track overrides\n");
    out.push_str("the top-level track with the same name; unset options are inherited.\n\n");
    out.push_str("| option | range | default | description |\n|--------|-------|---------|-------------|\n");
    for t in TRACK_OPTIONS {
        out.push_str(&alloc::format!("| `{}` | {} | {} | {} |\n", t.name, t.range, t.default, t.doc));
    }
    out.push('\n');
    out
}

/// JSON list of track options.
pub fn track_json() -> String {
    let mut out = String::from("[\n");
    for (i, t) in TRACK_OPTIONS.iter().enumerate() {
        out.push_str(&alloc::format!(
            "  {{\"name\": \"{}\", \"range\": \"{}\", \"default\": \"{}\", \"doc\": \"{}\"}}{}\n",
            t.name, json_escape(t.range), json_escape(t.default), json_escape(t.doc),
            if i + 1 < TRACK_OPTIONS.len() { "," } else { "" }
        ));
    }
    out.push_str("]\n");
    out
}

// ── Reference generation (shared by the CLI and the MCP server) ──

use crate::dsl::error::json_escape;

/// Markdown reference for the given module kinds.
pub fn markdown(kinds: &[ModuleKind]) -> String {
    let mut out = String::new();
    out.push_str("# Module parameters\n\n");
    out.push_str("Values are floats in the listed range. Parameters with a unit column can be\n");
    out.push_str("written that way instead, which is the readable form: `cutoff 800hz`,\n");
    out.push_str("`attack 20ms`, `release 1.5s`, `osc2_pitch -12st`. Any 0..1 parameter also\n");
    out.push_str("takes a percentage (`resonance 80%`) and any gain takes decibels (`level 6db`).\n");
    out.push_str("`choice` params take an option name\n");
    out.push_str("(`waveform half_sine`); a numeric value encodes as index / (options - 1).\n");
    out.push_str("FM modules also accept `op<N>_envelope <attack> <decay> <sustain> <release>` as a\n");
    out.push_str("shorthand for the four per-operator envelope params.\n");
    out.push_str("Generated by `tatum params`; do not edit by hand.\n\n");
    for kind in kinds {
        out.push_str(&alloc::format!("## {}\n\n", kind.as_str()));
        out.push_str("| name | range | in units | default | description |\n");
        out.push_str("|------|-------|----------|---------|-------------|\n");
        for s in specs(*kind) {
            let (range, default) = match s.range {
                Range::Choice(names) => (
                    alloc::format!("choice: {}", names.join(", ")),
                    alloc::format!("`{}`", s.choice_name(s.default).unwrap_or("")),
                ),
                _ => (s.range.describe(), alloc::format!("{}", s.default)),
            };
            let units = match s.curve.unit() {
                Some(unit) => alloc::format!(
                    "{} (default {})",
                    s.curve.describe(),
                    write_amount(s.curve.to_real(s.default), unit)
                ),
                None => match s.range {
                    Range::Unit | Range::Bipolar => String::from("%"),
                    Range::Gain { .. } => String::from("db"),
                    Range::Choice(_) => String::from("—"),
                },
            };
            out.push_str(&alloc::format!("| `{}` | {} | {} | {} | {} |\n", s.name, range, units, default, s.doc));
        }
        out.push('\n');
    }
    out
}

/// JSON reference for the given module kinds: `{ "bass": [ {name, type, min, max, ...} ] }`.
pub fn json(kinds: &[ModuleKind]) -> String {
    let mut out = String::from("{\n");
    for (ki, kind) in kinds.iter().enumerate() {
        out.push_str(&alloc::format!("  \"{}\": [\n", kind.as_str()));
        let list = specs(*kind);
        for (i, s) in list.iter().enumerate() {
            let (ty, extra) = match s.range {
                Range::Unit => ("unit", String::new()),
                Range::Bipolar => ("bipolar", String::new()),
                Range::Gain { max } => ("gain", alloc::format!(", \"max\": {}", max)),
                Range::Choice(names) => {
                    let items: Vec<String> = names.iter().map(|n| alloc::format!("\"{}\"", n)).collect();
                    ("choice", alloc::format!(", \"choices\": [{}]", items.join(", ")))
                }
            };
            let unit = match s.curve.unit() {
                Some(unit) => alloc::format!(
                    ", \"unit\": \"{}\", \"unit_range\": \"{}\", \"unit_default\": \"{}\"",
                    unit.suffix(), s.curve.describe(),
                    write_amount(s.curve.to_real(s.default), unit)
                ),
                None => String::new(),
            };
            out.push_str(&alloc::format!(
                "    {{\"name\": \"{}\", \"type\": \"{}\", \"min\": {}, \"max\": {}{}, \"default\": {}{}, \"doc\": \"{}\"}}{}\n",
                s.name, ty, s.range.min(), s.range.max(), extra, s.default, unit, json_escape(s.doc),
                if i + 1 < list.len() { "," } else { "" }
            ));
        }
        out.push_str(&alloc::format!("  ]{}\n", if ki + 1 < kinds.len() { "," } else { "" }));
    }
    out.push_str("}\n");
    out
}
