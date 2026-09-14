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

/// Full description of one named parameter.
#[derive(Debug, Clone, Copy)]
pub struct ParamSpec {
    pub name: &'static str,
    pub id: ParamId,
    pub range: Range,
    pub default: f32,
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
pub const LFO_SYNC: &[&str] = &["free", "quarter", "eighth", "sixteenth", "dotted_eighth", "triplet_eighth"];
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

const ENV_TIME_DOC: &str = "Exponential 1ms..2s";

macro_rules! spec {
    ($name:expr, $id:expr, $range:expr, $default:expr, $doc:expr) => {
        ParamSpec { name: $name, id: $id, range: $range, default: $default, doc: $doc }
    };
}

// ── Bass ──

pub const BASS_PARAMS: &[ParamSpec] = &[
    spec!("cutoff", ParamId::Bass(BassParam::Cutoff), Range::Unit, 0.43, "Ladder filter cutoff, exponential 20Hz..20kHz (0.5 ≈ 630Hz)"),
    spec!("cutoff_env", ParamId::Bass(BassParam::CutoffEnv), Range::Unit, 0.88, "Filter envelope depth, exponential 20Hz..8kHz sweep"),
    spec!("resonance", ParamId::Bass(BassParam::Resonance), Range::Unit, 0.3, "Ladder resonance, self-oscillates near 1.0"),
    spec!("glide", ParamId::Bass(BassParam::Glide), Range::Unit, 0.06, "Portamento rate between notes (0 = instant)"),
    spec!("attack", ParamId::Bass(BassParam::Attack), Range::Unit, 0.0, ENV_TIME_DOC),
    spec!("decay", ParamId::Bass(BassParam::Decay), Range::Unit, 0.5, ENV_TIME_DOC),
    spec!("sustain", ParamId::Bass(BassParam::Sustain), Range::Unit, 0.7, "Amp envelope sustain level"),
    spec!("release", ParamId::Bass(BassParam::Release), Range::Unit, 0.3, ENV_TIME_DOC),
    spec!("lfo_rate", ParamId::Bass(BassParam::LfoRate), Range::Unit, 0.0, "LFO rate 0.1..20Hz when lfo_sync is free"),
    spec!("lfo_depth", ParamId::Bass(BassParam::LfoDepth), Range::Unit, 0.0, "LFO depth; 0 disables the LFO"),
    spec!("lfo_waveform", ParamId::Bass(BassParam::LfoWaveform), Range::Choice(LFO_WAVEFORMS), 0.0, "LFO shape"),
    spec!("lfo_target", ParamId::Bass(BassParam::LfoTarget), Range::Choice(FILTER_LFO_TARGETS), 0.0, "What the LFO modulates"),
    spec!("lfo_sync", ParamId::Bass(BassParam::LfoSync), Range::Choice(LFO_SYNC), 0.0, "Free Hz or tempo-synced subdivision"),
    spec!("osc2_pitch", ParamId::Bass(BassParam::Osc2Pitch), Range::Unit, 0.5, "Osc 2 offset in semitones: 0.0 = -24, 0.5 = 0, 1.0 = +24 (steps of 1/48)"),
    spec!("osc3_pitch", ParamId::Bass(BassParam::Osc3Pitch), Range::Unit, 0.5, "Osc 3 offset in semitones: 0.0 = -24, 0.5 = 0, 1.0 = +24 (steps of 1/48)"),
    spec!("osc1_wave", ParamId::Bass(BassParam::Osc1Wave), Range::Choice(OSC_WAVES), 0.0, "Osc 1 waveform"),
    spec!("osc2_wave", ParamId::Bass(BassParam::Osc2Wave), Range::Choice(OSC_WAVES), 0.0, "Osc 2 waveform"),
    spec!("osc3_wave", ParamId::Bass(BassParam::Osc3Wave), Range::Choice(OSC_WAVES), 0.0, "Osc 3 waveform"),
    spec!("keytrack", ParamId::Bass(BassParam::Keytrack), Range::Unit, 0.0, "Filter cutoff follows note pitch"),
    spec!("vibrato_rate", ParamId::Bass(BassParam::VibratoRate), Range::Unit, 0.0, "Vibrato rate 0.5..10Hz"),
    spec!("vibrato_depth", ParamId::Bass(BassParam::VibratoDepth), Range::Unit, 0.0, "Vibrato depth, up to half a semitone"),
    spec!("vel_env", ParamId::Bass(BassParam::VelEnv), Range::Unit, 0.0, "Velocity scales filter envelope depth"),
];

// ── FM ──

pub const FM_PARAMS: &[ParamSpec] = &[
    spec!("algorithm", ParamId::Fm(FmParam::Algorithm), Range::Choice(FM_ALGORITHMS), 0.0, "Operator routing"),
    spec!("mod_index", ParamId::Fm(FmParam::ModIndex), Range::Unit, 0.5, "Modulation index, exponential 0.1..4.0"),
    spec!("feedback", ParamId::Fm(FmParam::Feedback), Range::Unit, 0.0, "Global operator feedback (scaled to 0..0.7)"),
    spec!("waveform", ParamId::Fm(FmParam::Waveform), Range::Choice(FM_WAVEFORMS), 0.0, "Operator waveform"),
    spec!("chorus_mix", ParamId::Fm(FmParam::ChorusMix), Range::Unit, 0.0, "Dedicated chorus wet mix"),
    spec!("attack", ParamId::Fm(FmParam::Attack), Range::Unit, 0.0, "Carrier (op0) attack, exponential 1ms..2s"),
    spec!("decay", ParamId::Fm(FmParam::Decay), Range::Unit, 0.5, "Carrier (op0) decay, exponential 1ms..2s"),
    spec!("sustain", ParamId::Fm(FmParam::Sustain), Range::Unit, 0.7, "Carrier (op0) sustain level"),
    spec!("release", ParamId::Fm(FmParam::Release), Range::Unit, 0.3, "Carrier (op0) release, exponential 1ms..2s"),
    spec!("lfo_rate", ParamId::Fm(FmParam::LfoRate), Range::Unit, 0.0, "LFO rate 0.1..20Hz when lfo_sync is free"),
    spec!("lfo_depth", ParamId::Fm(FmParam::LfoDepth), Range::Unit, 0.0, "LFO depth; 0 disables the LFO"),
    spec!("lfo_waveform", ParamId::Fm(FmParam::LfoWaveform), Range::Choice(LFO_WAVEFORMS), 0.0, "LFO shape"),
    spec!("lfo_target", ParamId::Fm(FmParam::LfoTarget), Range::Choice(FM_LFO_TARGETS), 0.0, "What the LFO modulates"),
    spec!("lfo_sync", ParamId::Fm(FmParam::LfoSync), Range::Choice(LFO_SYNC), 0.0, "Free Hz or tempo-synced subdivision"),
    spec!("vibrato_rate", ParamId::Fm(FmParam::VibratoRate), Range::Unit, 0.0, "Vibrato rate 0.5..10Hz"),
    spec!("vibrato_depth", ParamId::Fm(FmParam::VibratoDepth), Range::Unit, 0.0, "Vibrato depth, up to half a semitone"),
    spec!("op0_attack", ParamId::Fm(FmParam::Op0Attack), Range::Unit, 0.0, ENV_TIME_DOC),
    spec!("op0_decay", ParamId::Fm(FmParam::Op0Decay), Range::Unit, 0.5, ENV_TIME_DOC),
    spec!("op0_sustain", ParamId::Fm(FmParam::Op0Sustain), Range::Unit, 0.7, "Operator sustain level"),
    spec!("op0_release", ParamId::Fm(FmParam::Op0Release), Range::Unit, 0.3, ENV_TIME_DOC),
    spec!("op1_attack", ParamId::Fm(FmParam::Op1Attack), Range::Unit, 0.0, ENV_TIME_DOC),
    spec!("op1_decay", ParamId::Fm(FmParam::Op1Decay), Range::Unit, 0.5, ENV_TIME_DOC),
    spec!("op1_sustain", ParamId::Fm(FmParam::Op1Sustain), Range::Unit, 0.7, "Operator sustain level"),
    spec!("op1_release", ParamId::Fm(FmParam::Op1Release), Range::Unit, 0.3, ENV_TIME_DOC),
    spec!("op2_attack", ParamId::Fm(FmParam::Op2Attack), Range::Unit, 0.0, ENV_TIME_DOC),
    spec!("op2_decay", ParamId::Fm(FmParam::Op2Decay), Range::Unit, 0.5, ENV_TIME_DOC),
    spec!("op2_sustain", ParamId::Fm(FmParam::Op2Sustain), Range::Unit, 0.7, "Operator sustain level"),
    spec!("op2_release", ParamId::Fm(FmParam::Op2Release), Range::Unit, 0.3, ENV_TIME_DOC),
    spec!("op3_attack", ParamId::Fm(FmParam::Op3Attack), Range::Unit, 0.0, ENV_TIME_DOC),
    spec!("op3_decay", ParamId::Fm(FmParam::Op3Decay), Range::Unit, 0.5, ENV_TIME_DOC),
    spec!("op3_sustain", ParamId::Fm(FmParam::Op3Sustain), Range::Unit, 0.7, "Operator sustain level"),
    spec!("op3_release", ParamId::Fm(FmParam::Op3Release), Range::Unit, 0.3, ENV_TIME_DOC),
    spec!("op0_feedback", ParamId::Fm(FmParam::Op0Feedback), Range::Unit, 0.0, "Self-modulation of operator 0"),
    spec!("op1_feedback", ParamId::Fm(FmParam::Op1Feedback), Range::Unit, 0.0, "Self-modulation of operator 1"),
    spec!("op2_feedback", ParamId::Fm(FmParam::Op2Feedback), Range::Unit, 0.0, "Self-modulation of operator 2"),
    spec!("op3_feedback", ParamId::Fm(FmParam::Op3Feedback), Range::Unit, 0.0, "Self-modulation of operator 3"),
    spec!("op0_ratio", ParamId::Fm(FmParam::Op0Ratio), Range::Unit, 0.032, "Frequency ratio, linear 0.5..16.0 (ratio = 0.5 + v*15.5)"),
    spec!("op1_ratio", ParamId::Fm(FmParam::Op1Ratio), Range::Unit, 0.032, "Frequency ratio, linear 0.5..16.0 (ratio = 0.5 + v*15.5)"),
    spec!("op2_ratio", ParamId::Fm(FmParam::Op2Ratio), Range::Unit, 0.032, "Frequency ratio, linear 0.5..16.0 (ratio = 0.5 + v*15.5)"),
    spec!("op3_ratio", ParamId::Fm(FmParam::Op3Ratio), Range::Unit, 0.032, "Frequency ratio, linear 0.5..16.0 (ratio = 0.5 + v*15.5)"),
];

// ── Keys ──

pub const KEYS_PARAMS: &[ParamSpec] = &[
    spec!("cutoff", ParamId::Keys(KeysParam::Cutoff), Range::Unit, 0.59, "Biquad lowpass cutoff, exponential 200Hz..20kHz"),
    spec!("resonance", ParamId::Keys(KeysParam::Resonance), Range::Unit, 0.2, "Filter resonance"),
    spec!("detune", ParamId::Keys(KeysParam::Detune), Range::Unit, 0.25, "Osc detune, up to 2%"),
    spec!("chorus_mix", ParamId::Keys(KeysParam::ChorusMix), Range::Unit, 0.0, "Chorus wet mix"),
    spec!("level", ParamId::Keys(KeysParam::Level), Range::Gain { max: 2.0 }, 0.5, "Module output gain"),
    spec!("voice_mode", ParamId::Keys(KeysParam::VoiceMode), Range::Choice(VOICE_MODES), 0.0, "Volca Keys style voice allocation"),
    spec!("attack", ParamId::Keys(KeysParam::Attack), Range::Unit, 0.0, ENV_TIME_DOC),
    spec!("decay", ParamId::Keys(KeysParam::Decay), Range::Unit, 0.5, ENV_TIME_DOC),
    spec!("sustain", ParamId::Keys(KeysParam::Sustain), Range::Unit, 0.7, "Amp envelope sustain level"),
    spec!("release", ParamId::Keys(KeysParam::Release), Range::Unit, 0.3, ENV_TIME_DOC),
    spec!("lfo_rate", ParamId::Keys(KeysParam::LfoRate), Range::Unit, 0.0, "LFO rate 0.1..20Hz when lfo_sync is free"),
    spec!("lfo_depth", ParamId::Keys(KeysParam::LfoDepth), Range::Unit, 0.0, "LFO depth; 0 disables the LFO"),
    spec!("lfo_waveform", ParamId::Keys(KeysParam::LfoWaveform), Range::Choice(LFO_WAVEFORMS), 0.0, "LFO shape"),
    spec!("lfo_target", ParamId::Keys(KeysParam::LfoTarget), Range::Choice(FILTER_LFO_TARGETS), 0.0, "What the LFO modulates"),
    spec!("lfo_sync", ParamId::Keys(KeysParam::LfoSync), Range::Choice(LFO_SYNC), 0.0, "Free Hz or tempo-synced subdivision"),
    spec!("vibrato_rate", ParamId::Keys(KeysParam::VibratoRate), Range::Unit, 0.0, "Vibrato rate 0.5..10Hz"),
    spec!("vibrato_depth", ParamId::Keys(KeysParam::VibratoDepth), Range::Unit, 0.0, "Vibrato depth, up to half a semitone"),
];

// ── Beats ──

pub const BEATS_PARAMS: &[ParamSpec] = &[
    spec!("level", ParamId::Beats(BeatsParam::Level), Range::Gain { max: 2.0 }, 1.0, "Module output gain"),
    spec!("kick_level", ParamId::Beats(BeatsParam::KickLevel), Range::Unit, 1.0, "Kick level"),
    spec!("kick_decay", ParamId::Beats(BeatsParam::KickDecay), Range::Unit, 0.5, "Kick amp decay, tight..long"),
    spec!("kick_pitch", ParamId::Beats(BeatsParam::KickPitch), Range::Unit, 0.33, "Kick pitch multiplier 0.5..2.0"),
    spec!("kick_click", ParamId::Beats(BeatsParam::KickClick), Range::Unit, 0.3, "Noise transient at kick onset"),
    spec!("kick_drive", ParamId::Beats(BeatsParam::KickDrive), Range::Unit, 0.0, "Kick saturation, warm..heavy"),
    spec!("kick_pan", ParamId::Beats(BeatsParam::KickPan), Range::Bipolar, 0.0, "Kick stereo position"),
    spec!("snare_level", ParamId::Beats(BeatsParam::SnareLevel), Range::Gain { max: 2.0 }, 1.0, "Snare level"),
    spec!("snare_decay", ParamId::Beats(BeatsParam::SnareDecay), Range::Unit, 0.5, "Snare decay, tight..long"),
    spec!("snare_pitch", ParamId::Beats(BeatsParam::SnarePitch), Range::Unit, 0.33, "Snare pitch multiplier 0.5..2.0"),
    spec!("snare_drive", ParamId::Beats(BeatsParam::SnareDrive), Range::Unit, 0.0, "Snare saturation, clean..gritty"),
    spec!("snare_snap", ParamId::Beats(BeatsParam::SnareSnap), Range::Unit, 0.4, "Snare noise crack intensity"),
    spec!("snare_pan", ParamId::Beats(BeatsParam::SnarePan), Range::Bipolar, -0.1, "Snare stereo position"),
    spec!("hihat_level", ParamId::Beats(BeatsParam::HihatLevel), Range::Unit, 1.0, "Hi-hat level"),
    spec!("hihat_decay", ParamId::Beats(BeatsParam::HihatDecay), Range::Unit, 0.5, "Closed hat decay, tight..ringy"),
    spec!("hihat_pitch", ParamId::Beats(BeatsParam::HihatPitch), Range::Unit, 0.33, "Hat pitch multiplier 0.5..2.0"),
    spec!("hihat_pan", ParamId::Beats(BeatsParam::HihatPan), Range::Bipolar, 0.3, "Hat stereo position"),
    spec!("clap_level", ParamId::Beats(BeatsParam::ClapLevel), Range::Unit, 1.0, "Clap level"),
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
        if best.map_or(true, |(bd, _)| d < bd) {
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

fn levenshtein(a: &str, b: &str) -> usize {
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

// ── Reference generation (shared by the CLI and the MCP server) ──

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out
}

/// Markdown reference for the given module kinds.
pub fn markdown(kinds: &[ModuleKind]) -> String {
    let mut out = String::new();
    out.push_str("# Module parameters\n\n");
    out.push_str("Values are floats in the listed range. `choice` params take an option name\n");
    out.push_str("(`waveform half_sine`); a numeric value encodes as index / (options - 1).\n");
    out.push_str("FM modules also accept `op<N>_envelope <attack> <decay> <sustain> <release>` as a\n");
    out.push_str("shorthand for the four per-operator envelope params.\n");
    out.push_str("Generated by `synth params`; do not edit by hand.\n\n");
    for kind in kinds {
        out.push_str(&alloc::format!("## {}\n\n", kind.as_str()));
        out.push_str("| name | range | default | description |\n");
        out.push_str("|------|-------|---------|-------------|\n");
        for s in specs(*kind) {
            let (range, default) = match s.range {
                Range::Choice(names) => (
                    alloc::format!("choice: {}", names.join(", ")),
                    alloc::format!("`{}`", s.choice_name(s.default).unwrap_or("")),
                ),
                _ => (s.range.describe(), alloc::format!("{}", s.default)),
            };
            out.push_str(&alloc::format!("| `{}` | {} | {} | {} |\n", s.name, range, default, s.doc));
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
            out.push_str(&alloc::format!(
                "    {{\"name\": \"{}\", \"type\": \"{}\", \"min\": {}, \"max\": {}{}, \"default\": {}, \"doc\": \"{}\"}}{}\n",
                s.name, ty, s.range.min(), s.range.max(), extra, s.default, json_escape(s.doc),
                if i + 1 < list.len() { "," } else { "" }
            ));
        }
        out.push_str(&alloc::format!("  ]{}\n", if ki + 1 < kinds.len() { "," } else { "" }));
    }
    out.push_str("}\n");
    out
}
