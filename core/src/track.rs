use crate::primitives::arp_processor::ArpProcessor;

pub const MAX_TRACKS: usize = 6;
pub const MAX_INSERT_FX: usize = 4;
pub const MAX_MASTER_FX: usize = 6;

#[derive(Clone, Copy, PartialEq)]
pub enum InstrumentKind {
    Bass,
    Fm,
    Keys,
    Beats,
    None,
}

impl InstrumentKind {
    /// Whether this instrument produces stereo output natively.
    pub fn is_stereo(self) -> bool {
        matches!(self, InstrumentKind::Keys | InstrumentKind::Beats)
    }

    /// Whether this instrument is a drum machine.
    pub fn is_drum(self) -> bool {
        matches!(self, InstrumentKind::Beats)
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum InsertFxType {
    None,
    Filter,       // 1  — BiquadFilter (LP/HP/BP)
    Saturator,    // 2  — Waveshaper
    Chorus,       // 3  — Modulated delay line
    TiltEq,       // 4  — Tilt EQ
    Compressor,   // 5  — Dynamics
    Delay,        // 6  — Delay line
    Reverb,       // 7  — Dattorro plate
    Limiter,      // 8  — Brickwall limiter
    ThreeBandEq,  // 9  — Low/mid/high EQ
    Bitcrusher,   // 10 — Bit depth + rate reduction
    TapeStop,     // 11 — Turntable stop effect
}

impl InsertFxType {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => InsertFxType::Filter,
            2 => InsertFxType::Saturator,
            3 => InsertFxType::Chorus,
            4 => InsertFxType::TiltEq,
            5 => InsertFxType::Compressor,
            6 => InsertFxType::Delay,
            7 => InsertFxType::Reverb,
            8 => InsertFxType::Limiter,
            9 => InsertFxType::ThreeBandEq,
            10 => InsertFxType::Bitcrusher,
            11 => InsertFxType::TapeStop,
            _ => InsertFxType::None,
        }
    }

    pub fn to_u8(self) -> u8 {
        match self {
            InsertFxType::None => 0,
            InsertFxType::Filter => 1,
            InsertFxType::Saturator => 2,
            InsertFxType::Chorus => 3,
            InsertFxType::TiltEq => 4,
            InsertFxType::Compressor => 5,
            InsertFxType::Delay => 6,
            InsertFxType::Reverb => 7,
            InsertFxType::Limiter => 8,
            InsertFxType::ThreeBandEq => 9,
            InsertFxType::Bitcrusher => 10,
            InsertFxType::TapeStop => 11,
        }
    }
}

#[derive(Clone, Copy)]
pub struct InsertFxSlot {
    pub fx_type: InsertFxType,
    pub instance_idx: u8,
    pub enabled: bool,
}

impl InsertFxSlot {
    pub fn empty() -> Self {
        Self {
            fx_type: InsertFxType::None,
            instance_idx: 0,
            enabled: false,
        }
    }

    pub fn new(fx_type: InsertFxType, instance_idx: u8) -> Self {
        Self {
            fx_type,
            instance_idx,
            enabled: true,
        }
    }
}

pub struct TrackSlot {
    pub kind: InstrumentKind,
    pub instance_idx: u8,
    pub level: f32,
    pub pan: f32,
    pub delay_send: f32,
    pub reverb_send: f32,
    pub muted: bool,
    pub solo: bool,
    pub arp: Option<ArpProcessor>,
    pub insert_fx: [InsertFxSlot; MAX_INSERT_FX],
}

impl TrackSlot {
    pub fn new(kind: InstrumentKind, instance_idx: u8) -> Self {
        Self {
            kind,
            instance_idx,
            level: 0.5,
            pan: 0.0,
            delay_send: 0.0,
            reverb_send: 0.0,
            muted: false,
            solo: false,
            arp: None,
            insert_fx: [InsertFxSlot::empty(); MAX_INSERT_FX],
        }
    }

    pub fn with_level(mut self, level: f32) -> Self {
        self.level = level;
        self
    }

    pub fn with_pan(mut self, pan: f32) -> Self {
        self.pan = pan;
        self
    }

    pub fn empty() -> Self {
        Self::new(InstrumentKind::None, 0).with_level(0.0)
    }
}
