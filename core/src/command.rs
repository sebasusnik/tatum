//! CommandBus — universal command protocol for the synth engine.
//!
//! Every host (WASM, VST, native, test harness) pushes Commands into the bus.
//! The engine drains the bus at the start of each process() call.
//! Commands are plain data — serializable, scriptable, replayable.

use crate::engine::Engine;

// ── Command enum ────────────────────────────────────────

/// Every action the engine can perform, as a flat data variant.
/// Adding a new feature = adding a variant here + handling it in Engine::execute().
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    // ── Transport ──
    Start,
    Stop,
    Reset,
    SetBpm(f32),

    // ── Notes (module: 0=bass, 1=keys, 2=fm, 3=beats) ──
    NoteOn { module: u8, note: u8, velocity: f32 },
    NoteOff { module: u8, note: u8 },
    AllNotesOff,

    // ── Universal params (0-255 flat space) ──
    SetParam { id: u8, value: f32 },

    // ── Sequencer: melodic tracks (track: 0=bass, 1=keys, 2=fm) ──
    SetStep { track: u8, idx: u8, note: u8, velocity: f32, gate: f32 },
    ClearStep { track: u8, idx: u8 },
    SetStepSlide { track: u8, idx: u8, slide: bool },
    SetStepLock { track: u8, idx: u8, param_id: u8, value: f32 },
    ClearStepLocks { track: u8, idx: u8 },
    SetStepLockSlide { track: u8, idx: u8, lock_slide: bool },
    SetStepProbability { track: u8, idx: u8, prob: f32 },
    SetStepActive { track: u8, idx: u8, active: bool },

    // ── Sequencer: drum lanes ──
    SetDrumHit { lane: u8, step: u8, on: bool, velocity: f32 },
    ClearDrumLane(u8),
    SetDrumLaneNote { lane: u8, note: u8 },

    // ── Sequencer: global ──
    SetNumSteps(u8),
    SetSwing(f32),
    SetHumanize(f32),
    SetTrackEnabled { track: u8, enabled: bool },

    // ── Harmony ──
    SetHarmony { root: u8, scale: u8, degree: u8 },

    // ── Effects ──
    SetDelayTime(f32),
    SetDelayFeedback(f32),
    SetDelayFilter(f32),
    SetDelayMix(f32),
    SetDelaySync(u8),       // 0=free, 1=quarter, 2=eighth, 3=dotted8, 4=triplet8, 5=sixteenth
    SetTiltEq(f32),
    SetReverbSize(f32),
    SetReverbDamping(f32),
    SetReverbMix(f32),
    SetEqLow(f32),
    SetEqMid(f32),
    SetEqHigh(f32),
    SetCompThreshold(f32),
    SetCompRatio(f32),
    SetCompAttack(f32),
    SetCompRelease(f32),
    SetCompMakeup(f32),
    SetSidechain(f32),
    SetPitchBend(f32),

    // ── Module mute/solo ──
    SetMute { module: u8, muted: bool },
    SetSolo { module: u8, solo: bool },

    // ── Limiter ──
    SetLimiterThreshold(f32),
    SetLimiterRelease(f32),
    SetLimiterMakeup(f32),

    // ── Module level & pan ──
    SetModuleLevel { module: u8, level: f32 },
    SetModulePan { module: u8, pan: f32 },

    // ── Master ──
    SetMasterLevel(f32),

    // ── Pitch bend range ──
    SetBendRange(f32),

    // ── Reverb pre-delay ──
    SetReverbPreDelay(f32),

    // ── Delay LFO ──
    SetDelayLfoRate(f32),
    SetDelayLfoDepth(f32),

    // ── Per-module sends ──
    SetModuleDelaySend { module: u8, amount: f32 },
    SetModuleReverbSend { module: u8, amount: f32 },

    // ── Track routing (new) ──
    SetTrackInstrument { track: u8, kind: u8 },
    SetTrackArpEnabled { track: u8, enabled: bool },
    SetTrackArpRate { track: u8, rate: f32 },
    SetTrackArpGate { track: u8, gate: f32 },
    SetTrackArpPattern { track: u8, pattern: f32 },
    SetTrackArpOctaves { track: u8, octaves: f32 },
    SetTrackParam { track: u8, param: u8, value: f32 },

    // ── Per-track insert FX ──
    SetTrackInsertFx { track: u8, slot: u8, fx_type: u8 },
    SetTrackInsertParam { track: u8, slot: u8, param_idx: u8, value: f32 },
    SetTrackInsertEnabled { track: u8, slot: u8, enabled: bool },
    ClearTrackInsert { track: u8, slot: u8 },
    MoveTrackInsert { track: u8, from_slot: u8, to_slot: u8 },
    SetTrackInsertInstance { track: u8, slot: u8, instance: u8 },

    // ── Master insert FX ──
    SetMasterInsertFx { slot: u8, fx_type: u8 },
    SetMasterInsertParam { slot: u8, param_idx: u8, value: f32 },
    SetMasterInsertEnabled { slot: u8, enabled: bool },
    ClearMasterInsert { slot: u8 },
}

// ── Command tag constants (for binary serialization) ────

/// Tag bytes for binary encoding. Each command gets a unique u8 tag.
/// The tag is the first byte; payload follows in fixed layout.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tag {
    Start = 0,
    Stop = 1,
    Reset = 2,
    SetBpm = 3,
    NoteOn = 4,
    NoteOff = 5,
    AllNotesOff = 6,
    SetParam = 7,
    SetStep = 8,
    ClearStep = 9,
    SetStepSlide = 10,
    SetStepLock = 11,
    ClearStepLocks = 12,
    SetStepLockSlide = 13,
    SetStepProbability = 14,
    SetStepActive = 15,
    SetDrumHit = 16,
    ClearDrumLane = 17,
    SetDrumLaneNote = 18,
    SetNumSteps = 19,
    SetSwing = 20,
    SetHumanize = 21,
    SetTrackEnabled = 22,
    SetHarmony = 23,
    SetDelayTime = 24,
    SetDelayFeedback = 25,
    SetDelayFilter = 26,
    SetReverbSize = 27,
    SetReverbDamping = 28,
    SetReverbMix = 29,
    SetEqLow = 30,
    SetEqMid = 31,
    SetEqHigh = 32,
    SetCompThreshold = 33,
    SetCompRatio = 34,
    SetCompAttack = 35,
    SetCompRelease = 36,
    SetCompMakeup = 37,
    SetSidechain = 38,
    SetPitchBend = 39,
    SetMute = 40,
    SetSolo = 41,
    SetDelayMix = 42,
    SetDelaySync = 43,
    SetTiltEq = 44,
    SetLimiterThreshold = 45,
    SetLimiterRelease = 46,
    SetLimiterMakeup = 47,
    SetModuleLevel = 48,
    SetModulePan = 49,
    SetMasterLevel = 50,
    SetBendRange = 51,
    SetReverbPreDelay = 52,
    SetDelayLfoRate = 53,
    SetDelayLfoDepth = 54,
    SetModuleDelaySend = 55,
    SetModuleReverbSend = 56,
    // Track routing (new)
    SetTrackInstrument = 57,
    SetTrackArpEnabled = 58,
    SetTrackArpRate = 59,
    SetTrackArpGate = 60,
    SetTrackArpPattern = 61,
    SetTrackArpOctaves = 62,
    SetTrackParam = 63,
    // Per-track insert FX
    SetTrackInsertFx = 64,
    SetTrackInsertParam = 65,
    SetTrackInsertEnabled = 66,
    ClearTrackInsert = 67,
    MoveTrackInsert = 68,
    SetTrackInsertInstance = 69,
    // Master insert FX
    SetMasterInsertFx = 70,
    SetMasterInsertParam = 71,
    SetMasterInsertEnabled = 72,
    ClearMasterInsert = 73,
}

impl Tag {
    pub fn from_u8(v: u8) -> Option<Tag> {
        if v <= 73 { Some(unsafe { core::mem::transmute(v) }) } else { None }
    }
}

// ── Flat encoding: (tag, a, b, c, d, e) as f32s ────────
//
// This format is optimized for the WASM bridge where we pass
// a tag + up to 5 f32 args. Simple, no allocation, no parsing.

impl Command {
    /// Decode from a flat (tag, a, b, c, d, e) tuple.
    /// Used by the WASM bridge: JS sends 6 numbers, Rust decodes to a Command.
    pub fn from_flat(tag: u8, a: f32, b: f32, c: f32, d: f32, e: f32) -> Option<Command> {
        let tag = Tag::from_u8(tag)?;
        Some(match tag {
            Tag::Start => Command::Start,
            Tag::Stop => Command::Stop,
            Tag::Reset => Command::Reset,
            Tag::SetBpm => Command::SetBpm(a),

            Tag::NoteOn => Command::NoteOn { module: a as u8, note: b as u8, velocity: c },
            Tag::NoteOff => Command::NoteOff { module: a as u8, note: b as u8 },
            Tag::AllNotesOff => Command::AllNotesOff,

            Tag::SetParam => Command::SetParam { id: a as u8, value: b },

            Tag::SetStep => Command::SetStep { track: a as u8, idx: b as u8, note: c as u8, velocity: d, gate: e },
            Tag::ClearStep => Command::ClearStep { track: a as u8, idx: b as u8 },
            Tag::SetStepSlide => Command::SetStepSlide { track: a as u8, idx: b as u8, slide: c > 0.5 },
            Tag::SetStepLock => Command::SetStepLock { track: a as u8, idx: b as u8, param_id: c as u8, value: d },
            Tag::ClearStepLocks => Command::ClearStepLocks { track: a as u8, idx: b as u8 },
            Tag::SetStepLockSlide => Command::SetStepLockSlide { track: a as u8, idx: b as u8, lock_slide: c > 0.5 },
            Tag::SetStepProbability => Command::SetStepProbability { track: a as u8, idx: b as u8, prob: c },
            Tag::SetStepActive => Command::SetStepActive { track: a as u8, idx: b as u8, active: c > 0.5 },

            Tag::SetDrumHit => Command::SetDrumHit { lane: a as u8, step: b as u8, on: c > 0.5, velocity: d },
            Tag::ClearDrumLane => Command::ClearDrumLane(a as u8),
            Tag::SetDrumLaneNote => Command::SetDrumLaneNote { lane: a as u8, note: b as u8 },

            Tag::SetNumSteps => Command::SetNumSteps(a as u8),
            Tag::SetSwing => Command::SetSwing(a),
            Tag::SetHumanize => Command::SetHumanize(a),
            Tag::SetTrackEnabled => Command::SetTrackEnabled { track: a as u8, enabled: b > 0.5 },

            Tag::SetHarmony => Command::SetHarmony { root: a as u8, scale: b as u8, degree: c as u8 },

            Tag::SetDelayTime => Command::SetDelayTime(a),
            Tag::SetDelayFeedback => Command::SetDelayFeedback(a),
            Tag::SetDelayFilter => Command::SetDelayFilter(a),
            Tag::SetDelayMix => Command::SetDelayMix(a),
            Tag::SetDelaySync => Command::SetDelaySync(a as u8),
            Tag::SetTiltEq => Command::SetTiltEq(a),
            Tag::SetReverbSize => Command::SetReverbSize(a),
            Tag::SetReverbDamping => Command::SetReverbDamping(a),
            Tag::SetReverbMix => Command::SetReverbMix(a),
            Tag::SetEqLow => Command::SetEqLow(a),
            Tag::SetEqMid => Command::SetEqMid(a),
            Tag::SetEqHigh => Command::SetEqHigh(a),
            Tag::SetCompThreshold => Command::SetCompThreshold(a),
            Tag::SetCompRatio => Command::SetCompRatio(a),
            Tag::SetCompAttack => Command::SetCompAttack(a),
            Tag::SetCompRelease => Command::SetCompRelease(a),
            Tag::SetCompMakeup => Command::SetCompMakeup(a),
            Tag::SetSidechain => Command::SetSidechain(a),
            Tag::SetPitchBend => Command::SetPitchBend(a),

            Tag::SetMute => Command::SetMute { module: a as u8, muted: b > 0.5 },
            Tag::SetSolo => Command::SetSolo { module: a as u8, solo: b > 0.5 },
            Tag::SetLimiterThreshold => Command::SetLimiterThreshold(a),
            Tag::SetLimiterRelease => Command::SetLimiterRelease(a),
            Tag::SetLimiterMakeup => Command::SetLimiterMakeup(a),
            Tag::SetModuleLevel => Command::SetModuleLevel { module: a as u8, level: b },
            Tag::SetModulePan => Command::SetModulePan { module: a as u8, pan: b },
            Tag::SetMasterLevel => Command::SetMasterLevel(a),
            Tag::SetBendRange => Command::SetBendRange(a),
            Tag::SetReverbPreDelay => Command::SetReverbPreDelay(a),
            Tag::SetDelayLfoRate => Command::SetDelayLfoRate(a),
            Tag::SetDelayLfoDepth => Command::SetDelayLfoDepth(a),
            Tag::SetModuleDelaySend => Command::SetModuleDelaySend { module: a as u8, amount: b },
            Tag::SetModuleReverbSend => Command::SetModuleReverbSend { module: a as u8, amount: b },

            Tag::SetTrackInstrument => Command::SetTrackInstrument { track: a as u8, kind: b as u8 },
            Tag::SetTrackArpEnabled => Command::SetTrackArpEnabled { track: a as u8, enabled: b > 0.5 },
            Tag::SetTrackArpRate => Command::SetTrackArpRate { track: a as u8, rate: b },
            Tag::SetTrackArpGate => Command::SetTrackArpGate { track: a as u8, gate: b },
            Tag::SetTrackArpPattern => Command::SetTrackArpPattern { track: a as u8, pattern: b },
            Tag::SetTrackArpOctaves => Command::SetTrackArpOctaves { track: a as u8, octaves: b },
            Tag::SetTrackParam => Command::SetTrackParam { track: a as u8, param: b as u8, value: c },

            Tag::SetTrackInsertFx => Command::SetTrackInsertFx { track: a as u8, slot: b as u8, fx_type: c as u8 },
            Tag::SetTrackInsertParam => Command::SetTrackInsertParam { track: a as u8, slot: b as u8, param_idx: c as u8, value: d },
            Tag::SetTrackInsertEnabled => Command::SetTrackInsertEnabled { track: a as u8, slot: b as u8, enabled: c > 0.5 },
            Tag::ClearTrackInsert => Command::ClearTrackInsert { track: a as u8, slot: b as u8 },
            Tag::MoveTrackInsert => Command::MoveTrackInsert { track: a as u8, from_slot: b as u8, to_slot: c as u8 },
            Tag::SetTrackInsertInstance => Command::SetTrackInsertInstance { track: a as u8, slot: b as u8, instance: c as u8 },

            Tag::SetMasterInsertFx => Command::SetMasterInsertFx { slot: a as u8, fx_type: b as u8 },
            Tag::SetMasterInsertParam => Command::SetMasterInsertParam { slot: a as u8, param_idx: b as u8, value: c },
            Tag::SetMasterInsertEnabled => Command::SetMasterInsertEnabled { slot: a as u8, enabled: b > 0.5 },
            Tag::ClearMasterInsert => Command::ClearMasterInsert { slot: a as u8 },
        })
    }

    /// Encode to a flat (tag, a, b, c, d, e) tuple for the WASM bridge.
    pub fn to_flat(&self) -> (u8, f32, f32, f32, f32, f32) {
        match *self {
            Command::Start => (Tag::Start as u8, 0.0, 0.0, 0.0, 0.0, 0.0),
            Command::Stop => (Tag::Stop as u8, 0.0, 0.0, 0.0, 0.0, 0.0),
            Command::Reset => (Tag::Reset as u8, 0.0, 0.0, 0.0, 0.0, 0.0),
            Command::SetBpm(v) => (Tag::SetBpm as u8, v, 0.0, 0.0, 0.0, 0.0),

            Command::NoteOn { module, note, velocity } => (Tag::NoteOn as u8, module as f32, note as f32, velocity, 0.0, 0.0),
            Command::NoteOff { module, note } => (Tag::NoteOff as u8, module as f32, note as f32, 0.0, 0.0, 0.0),
            Command::AllNotesOff => (Tag::AllNotesOff as u8, 0.0, 0.0, 0.0, 0.0, 0.0),

            Command::SetParam { id, value } => (Tag::SetParam as u8, id as f32, value, 0.0, 0.0, 0.0),

            Command::SetStep { track, idx, note, velocity, gate } => (Tag::SetStep as u8, track as f32, idx as f32, note as f32, velocity, gate),
            Command::ClearStep { track, idx } => (Tag::ClearStep as u8, track as f32, idx as f32, 0.0, 0.0, 0.0),
            Command::SetStepSlide { track, idx, slide } => (Tag::SetStepSlide as u8, track as f32, idx as f32, if slide { 1.0 } else { 0.0 }, 0.0, 0.0),
            Command::SetStepLock { track, idx, param_id, value } => (Tag::SetStepLock as u8, track as f32, idx as f32, param_id as f32, value, 0.0),
            Command::ClearStepLocks { track, idx } => (Tag::ClearStepLocks as u8, track as f32, idx as f32, 0.0, 0.0, 0.0),
            Command::SetStepLockSlide { track, idx, lock_slide } => (Tag::SetStepLockSlide as u8, track as f32, idx as f32, if lock_slide { 1.0 } else { 0.0 }, 0.0, 0.0),
            Command::SetStepProbability { track, idx, prob } => (Tag::SetStepProbability as u8, track as f32, idx as f32, prob, 0.0, 0.0),
            Command::SetStepActive { track, idx, active } => (Tag::SetStepActive as u8, track as f32, idx as f32, if active { 1.0 } else { 0.0 }, 0.0, 0.0),

            Command::SetDrumHit { lane, step, on, velocity } => (Tag::SetDrumHit as u8, lane as f32, step as f32, if on { 1.0 } else { 0.0 }, velocity, 0.0),
            Command::ClearDrumLane(lane) => (Tag::ClearDrumLane as u8, lane as f32, 0.0, 0.0, 0.0, 0.0),
            Command::SetDrumLaneNote { lane, note } => (Tag::SetDrumLaneNote as u8, lane as f32, note as f32, 0.0, 0.0, 0.0),

            Command::SetNumSteps(n) => (Tag::SetNumSteps as u8, n as f32, 0.0, 0.0, 0.0, 0.0),
            Command::SetSwing(v) => (Tag::SetSwing as u8, v, 0.0, 0.0, 0.0, 0.0),
            Command::SetHumanize(v) => (Tag::SetHumanize as u8, v, 0.0, 0.0, 0.0, 0.0),
            Command::SetTrackEnabled { track, enabled } => (Tag::SetTrackEnabled as u8, track as f32, if enabled { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0),

            Command::SetHarmony { root, scale, degree } => (Tag::SetHarmony as u8, root as f32, scale as f32, degree as f32, 0.0, 0.0),

            Command::SetDelayTime(v) => (Tag::SetDelayTime as u8, v, 0.0, 0.0, 0.0, 0.0),
            Command::SetDelayFeedback(v) => (Tag::SetDelayFeedback as u8, v, 0.0, 0.0, 0.0, 0.0),
            Command::SetDelayFilter(v) => (Tag::SetDelayFilter as u8, v, 0.0, 0.0, 0.0, 0.0),
            Command::SetDelayMix(v) => (Tag::SetDelayMix as u8, v, 0.0, 0.0, 0.0, 0.0),
            Command::SetDelaySync(v) => (Tag::SetDelaySync as u8, v as f32, 0.0, 0.0, 0.0, 0.0),
            Command::SetTiltEq(v) => (Tag::SetTiltEq as u8, v, 0.0, 0.0, 0.0, 0.0),
            Command::SetReverbSize(v) => (Tag::SetReverbSize as u8, v, 0.0, 0.0, 0.0, 0.0),
            Command::SetReverbDamping(v) => (Tag::SetReverbDamping as u8, v, 0.0, 0.0, 0.0, 0.0),
            Command::SetReverbMix(v) => (Tag::SetReverbMix as u8, v, 0.0, 0.0, 0.0, 0.0),
            Command::SetEqLow(v) => (Tag::SetEqLow as u8, v, 0.0, 0.0, 0.0, 0.0),
            Command::SetEqMid(v) => (Tag::SetEqMid as u8, v, 0.0, 0.0, 0.0, 0.0),
            Command::SetEqHigh(v) => (Tag::SetEqHigh as u8, v, 0.0, 0.0, 0.0, 0.0),
            Command::SetCompThreshold(v) => (Tag::SetCompThreshold as u8, v, 0.0, 0.0, 0.0, 0.0),
            Command::SetCompRatio(v) => (Tag::SetCompRatio as u8, v, 0.0, 0.0, 0.0, 0.0),
            Command::SetCompAttack(v) => (Tag::SetCompAttack as u8, v, 0.0, 0.0, 0.0, 0.0),
            Command::SetCompRelease(v) => (Tag::SetCompRelease as u8, v, 0.0, 0.0, 0.0, 0.0),
            Command::SetCompMakeup(v) => (Tag::SetCompMakeup as u8, v, 0.0, 0.0, 0.0, 0.0),
            Command::SetSidechain(v) => (Tag::SetSidechain as u8, v, 0.0, 0.0, 0.0, 0.0),
            Command::SetPitchBend(v) => (Tag::SetPitchBend as u8, v, 0.0, 0.0, 0.0, 0.0),

            Command::SetMute { module, muted } => (Tag::SetMute as u8, module as f32, if muted { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0),
            Command::SetSolo { module, solo } => (Tag::SetSolo as u8, module as f32, if solo { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0),
            Command::SetLimiterThreshold(v) => (Tag::SetLimiterThreshold as u8, v, 0.0, 0.0, 0.0, 0.0),
            Command::SetLimiterRelease(v) => (Tag::SetLimiterRelease as u8, v, 0.0, 0.0, 0.0, 0.0),
            Command::SetLimiterMakeup(v) => (Tag::SetLimiterMakeup as u8, v, 0.0, 0.0, 0.0, 0.0),
            Command::SetModuleLevel { module, level } => (Tag::SetModuleLevel as u8, module as f32, level, 0.0, 0.0, 0.0),
            Command::SetModulePan { module, pan } => (Tag::SetModulePan as u8, module as f32, pan, 0.0, 0.0, 0.0),
            Command::SetMasterLevel(v) => (Tag::SetMasterLevel as u8, v, 0.0, 0.0, 0.0, 0.0),
            Command::SetBendRange(v) => (Tag::SetBendRange as u8, v, 0.0, 0.0, 0.0, 0.0),
            Command::SetReverbPreDelay(v) => (Tag::SetReverbPreDelay as u8, v, 0.0, 0.0, 0.0, 0.0),
            Command::SetDelayLfoRate(v) => (Tag::SetDelayLfoRate as u8, v, 0.0, 0.0, 0.0, 0.0),
            Command::SetDelayLfoDepth(v) => (Tag::SetDelayLfoDepth as u8, v, 0.0, 0.0, 0.0, 0.0),
            Command::SetModuleDelaySend { module, amount } => (Tag::SetModuleDelaySend as u8, module as f32, amount, 0.0, 0.0, 0.0),
            Command::SetModuleReverbSend { module, amount } => (Tag::SetModuleReverbSend as u8, module as f32, amount, 0.0, 0.0, 0.0),

            Command::SetTrackInstrument { track, kind } => (Tag::SetTrackInstrument as u8, track as f32, kind as f32, 0.0, 0.0, 0.0),
            Command::SetTrackArpEnabled { track, enabled } => (Tag::SetTrackArpEnabled as u8, track as f32, if enabled { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0),
            Command::SetTrackArpRate { track, rate } => (Tag::SetTrackArpRate as u8, track as f32, rate, 0.0, 0.0, 0.0),
            Command::SetTrackArpGate { track, gate } => (Tag::SetTrackArpGate as u8, track as f32, gate, 0.0, 0.0, 0.0),
            Command::SetTrackArpPattern { track, pattern } => (Tag::SetTrackArpPattern as u8, track as f32, pattern, 0.0, 0.0, 0.0),
            Command::SetTrackArpOctaves { track, octaves } => (Tag::SetTrackArpOctaves as u8, track as f32, octaves, 0.0, 0.0, 0.0),
            Command::SetTrackParam { track, param, value } => (Tag::SetTrackParam as u8, track as f32, param as f32, value, 0.0, 0.0),

            Command::SetTrackInsertFx { track, slot, fx_type } => (Tag::SetTrackInsertFx as u8, track as f32, slot as f32, fx_type as f32, 0.0, 0.0),
            Command::SetTrackInsertParam { track, slot, param_idx, value } => (Tag::SetTrackInsertParam as u8, track as f32, slot as f32, param_idx as f32, value, 0.0),
            Command::SetTrackInsertEnabled { track, slot, enabled } => (Tag::SetTrackInsertEnabled as u8, track as f32, slot as f32, if enabled { 1.0 } else { 0.0 }, 0.0, 0.0),
            Command::ClearTrackInsert { track, slot } => (Tag::ClearTrackInsert as u8, track as f32, slot as f32, 0.0, 0.0, 0.0),
            Command::MoveTrackInsert { track, from_slot, to_slot } => (Tag::MoveTrackInsert as u8, track as f32, from_slot as f32, to_slot as f32, 0.0, 0.0),
            Command::SetTrackInsertInstance { track, slot, instance } => (Tag::SetTrackInsertInstance as u8, track as f32, slot as f32, instance as f32, 0.0, 0.0),

            Command::SetMasterInsertFx { slot, fx_type } => (Tag::SetMasterInsertFx as u8, slot as f32, fx_type as f32, 0.0, 0.0, 0.0),
            Command::SetMasterInsertParam { slot, param_idx, value } => (Tag::SetMasterInsertParam as u8, slot as f32, param_idx as f32, value, 0.0, 0.0),
            Command::SetMasterInsertEnabled { slot, enabled } => (Tag::SetMasterInsertEnabled as u8, slot as f32, if enabled { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0),
            Command::ClearMasterInsert { slot } => (Tag::ClearMasterInsert as u8, slot as f32, 0.0, 0.0, 0.0, 0.0),
        }
    }
}

// ── CommandBus ──────────────────────────────────────────

const BUS_SIZE: usize = 2048;

/// Lock-free single-producer single-consumer ring buffer for Commands.
/// The host pushes commands; the engine drains them at the start of process().
pub struct CommandBus {
    ring: [Option<Command>; BUS_SIZE],
    write: usize,
    read: usize,
}

impl CommandBus {
    pub fn new() -> Self {
        Self {
            ring: [None; BUS_SIZE],
            write: 0,
            read: 0,
        }
    }

    /// Push a command into the bus. Silently drops if full.
    pub fn push(&mut self, cmd: Command) {
        let next = (self.write + 1) % BUS_SIZE;
        if next == self.read {
            return; // buffer full, drop command
        }
        self.ring[self.write] = Some(cmd);
        self.write = next;
    }

    /// Drain all pending commands, executing each on the engine.
    pub fn drain(&mut self, engine: &mut Engine) {
        while self.read != self.write {
            if let Some(cmd) = self.ring[self.read].take() {
                engine.execute(cmd);
            }
            self.read = (self.read + 1) % BUS_SIZE;
        }
    }

    /// Number of pending commands.
    pub fn len(&self) -> usize {
        if self.write >= self.read {
            self.write - self.read
        } else {
            BUS_SIZE - self.read + self.write
        }
    }

    /// Whether the bus is empty.
    pub fn is_empty(&self) -> bool {
        self.write == self.read
    }
}
