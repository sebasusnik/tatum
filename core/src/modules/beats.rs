use crate::math;
use crate::rng::Rng;
use crate::primitives::filter::{BiquadFilter, FilterType};
use crate::{Module, SAMPLE_RATE, BLOCK_SIZE};

// ─── Kick Drum ───
// Sine with pitch envelope + click transient + tanh saturation
struct Kick {
    phase: f32,
    freq: f32,
    start_freq: f32,
    end_freq: f32,
    pitch_decay: f32,
    amp: f32,
    amp_decay: f32,
    level: f32,
    pitch: f32,
    // Click transient
    click_amp: f32,
    click_level: f32,
    click_decay: f32,
    click_filter: BiquadFilter,
    /// The click used to be white noise with only a highpass on it, so it ran
    /// at full level to 20 kHz and read as a tick rather than a beater. This
    /// keeps it in the band where a beater actually lives.
    click_tone: BiquadFilter,
    rng: Rng,
    drive: f32,
    active: bool,
}

impl Kick {
    fn new() -> Self {
        let mut click_filter = BiquadFilter::new(SAMPLE_RATE);
        click_filter.set_params(FilterType::HighPass, 1400.0, 0.3);
        let mut click_tone = BiquadFilter::new(SAMPLE_RATE);
        click_tone.set_params(FilterType::LowPass, 5500.0, 0.4);
        Self {
            phase: 0.0,
            freq: 50.0,
            start_freq: 280.0,
            end_freq: 50.0,
            pitch_decay: 0.994,
            amp: 0.0,
            amp_decay: 0.9992,
            level: 1.0,
            pitch: 1.0,
            click_amp: 0.0,
            click_level: 0.7,
            click_decay: 0.955,
            click_filter,
            click_tone,
            rng: Rng::new(88888),
            drive: 1.8,
            active: false,
        }
    }

    fn trigger(&mut self, velocity: f32) {
        self.phase = 0.0;
        self.freq = self.start_freq * self.pitch;
        self.amp = velocity;
        self.click_amp = velocity * self.click_level;
        self.active = true;
    }

    fn next_sample(&mut self) -> f32 {
        if !self.active {
            return 0.0;
        }

        // Click transient: highpass-filtered noise burst
        let click = if self.click_amp > 0.001 {
            let noise = self.rng.next_bipolar() * self.click_amp;
            let filtered_click = self.click_tone.process(self.click_filter.process(noise));
            self.click_amp *= self.click_decay; // ~0.5 ms at 44.1 kHz
            filtered_click
        } else {
            self.click_amp = 0.0;
            0.0
        };

        let body = math::sin(self.phase * math::TWO_PI) * self.amp;
        let out = math::tanh((body + click) * self.drive);

        self.phase += self.freq / SAMPLE_RATE;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }

        // Pitch envelope: sweep down
        let end = self.end_freq * self.pitch;
        self.freq = end + (self.freq - end) * self.pitch_decay;

        // Amplitude envelope
        self.amp *= self.amp_decay;
        if self.amp < 0.001 && self.click_amp < 0.001 {
            self.active = false;
        }

        out * 1.0 * self.level
    }
}

// ─── Snare Drum ───
// Jungle/DnB snare: dual sine oscillators + noise + crack snap + wire sizzle tail
// snap controls noise/crack intensity, drive controls tanh saturation
struct Snare {
    // Dual tone oscillators
    body_phase_1: f32,
    body_phase_2: f32,
    body_freq_1: f32,
    body_freq_2: f32,
    body_target_freq_1: f32,
    body_target_freq_2: f32,
    body_amp: f32,
    // Noise (snappy component)
    noise_amp: f32,
    noise_hp: BiquadFilter,   // HP ~3kHz
    noise_bp: BiquadFilter,   // BP ~5.5kHz for presence
    // Transient click
    transient_amp: f32,
    transient_filter: BiquadFilter, // BP ~4kHz wide
    // Crack: sharp bandpassed noise snap (~3.5kHz, ~20ms)
    crack_amp: f32,
    crack_filter: BiquadFilter,
    // Tail: wire sizzle (~7kHz, ~100ms, stereo)
    tail_amp: f32,
    tail_decay: f32,
    tail_bp: BiquadFilter,
    // shared
    rng: Rng,
    active: bool,
    decay_rate: f32,
    level: f32,
    pitch: f32,
    drive: f32,   // tanh saturation drive (default 1.8)
    snap: f32,    // noise/crack intensity (default 1.0)
}

impl Snare {
    fn new() -> Self {
        let mut noise_hp = BiquadFilter::new(SAMPLE_RATE);
        noise_hp.set_params(FilterType::HighPass, 3000.0, 0.15);
        let mut noise_bp = BiquadFilter::new(SAMPLE_RATE);
        noise_bp.set_params(FilterType::BandPass, 5500.0, 0.3);
        let mut transient_filter = BiquadFilter::new(SAMPLE_RATE);
        transient_filter.set_params(FilterType::BandPass, 4000.0, 0.2);
        let mut crack_filter = BiquadFilter::new(SAMPLE_RATE);
        crack_filter.set_params(FilterType::BandPass, 3500.0, 0.5);
        let mut tail_bp = BiquadFilter::new(SAMPLE_RATE);
        tail_bp.set_params(FilterType::BandPass, 7000.0, 0.4);
        Self {
            body_phase_1: 0.0,
            body_phase_2: 0.0,
            body_freq_1: 175.0,
            body_freq_2: 225.0,
            body_target_freq_1: 155.0,
            body_target_freq_2: 200.0,
            body_amp: 0.0,
            noise_amp: 0.0,
            noise_hp,
            noise_bp,
            transient_amp: 0.0,
            transient_filter,
            crack_amp: 0.0,
            crack_filter,
            tail_amp: 0.0,
            tail_decay: 0.9985,
            tail_bp,
            rng: Rng::new(12345),
            active: false,
            decay_rate: 0.9985,
            level: 1.0,
            pitch: 1.0,
            drive: 1.8,
            snap: 1.0,
        }
    }

    fn trigger(&mut self, velocity: f32) {
        // Micro-humanization: ±4% amplitude jitter
        let jitter = 1.0 + (self.rng.next_bipolar() * 0.04);

        // Ghost note fix: scale amplitude down for quiet hits
        let is_ghost = velocity < 0.4;
        let ghost_scale = if is_ghost { 0.30 } else { 1.0 };
        let vel = velocity * jitter * ghost_scale;

        self.body_phase_1 = 0.0;
        self.body_phase_2 = 0.0;
        self.body_freq_1 = 175.0 * self.pitch;
        self.body_freq_2 = 225.0 * self.pitch;
        self.body_target_freq_1 = 155.0 * self.pitch;
        self.body_target_freq_2 = 200.0 * self.pitch;
        self.body_amp = vel * 1.2;
        self.noise_amp = vel * self.snap * 1.5;
        self.transient_amp = vel * 0.6 * self.snap;
        self.crack_amp = vel * 1.4 * self.snap;
        self.tail_amp = vel * 0.5;

        self.noise_hp.set_params(FilterType::HighPass, 3000.0 * self.pitch, 0.15);
        self.noise_bp.set_params(FilterType::BandPass, 5500.0 * self.pitch, 0.3);
        self.transient_filter.set_params(FilterType::BandPass, 4000.0 * self.pitch, 0.2);
        self.crack_filter.set_params(FilterType::BandPass, 3500.0 * self.pitch, 0.5);
        self.tail_bp.set_params(FilterType::BandPass, 7000.0 * self.pitch, 0.4);
        self.active = true;
    }

    /// Returns (left, right) with slight stereo noise decorrelation.
    fn next_sample_stereo(&mut self) -> (f32, f32) {
        if !self.active {
            return (0.0, 0.0);
        }

        let noise = self.rng.next_bipolar();

        // Transient: broadband noise burst, ~1ms
        let transient = self.transient_filter.process(noise) * self.transient_amp;
        self.transient_amp *= 0.85;

        // Dual tone oscillators (175Hz + 225Hz)
        let tone1 = math::sin(self.body_phase_1 * math::TWO_PI);
        let tone2 = math::sin(self.body_phase_2 * math::TWO_PI);
        let tone = (tone1 + tone2) * 0.5 * self.body_amp;

        self.body_phase_1 += self.body_freq_1 / SAMPLE_RATE;
        self.body_phase_2 += self.body_freq_2 / SAMPLE_RATE;
        if self.body_phase_1 >= 1.0 { self.body_phase_1 -= 1.0; }
        if self.body_phase_2 >= 1.0 { self.body_phase_2 -= 1.0; }

        // Subtle pitch sweep (~20-30Hz drop over ~30ms)
        self.body_freq_1 += (self.body_target_freq_1 - self.body_freq_1) * 0.006;
        self.body_freq_2 += (self.body_target_freq_2 - self.body_freq_2) * 0.006;
        self.body_amp *= 0.9975; // ~40ms decay

        // Noise: HP + BP filtered
        let noise_hp = self.noise_hp.process(noise);
        let noise_bp = self.noise_bp.process(noise);
        let noise_out = (noise_hp * 0.6 + noise_bp * 0.4) * self.noise_amp;
        self.noise_amp *= 0.9988; // ~100ms decay

        // Crack: sharp bandpassed snap (~20ms)
        let crack = self.crack_filter.process(noise) * self.crack_amp;
        self.crack_amp *= 0.9925;

        // Tail: wire sizzle (~100ms, stereo spread)
        let tail = self.tail_bp.process(noise) * self.tail_amp;
        self.tail_amp *= self.tail_decay;

        if self.body_amp < 0.001 && self.noise_amp < 0.001 && self.tail_amp < 0.001 {
            self.active = false;
        }

        // Mix: tone + transient + crack in center, noise + tail in stereo
        let center = tone + transient + crack * 1.6;
        let stereo_jitter = self.rng.next_bipolar() * 0.2; // ±20% stereo width
        let tail_pan = self.rng.next_bipolar() * 0.2;
        let left  = center + noise_out * (1.0 - stereo_jitter) * 0.7
                   + tail * (1.0 - tail_pan) * 0.5;
        let right = center + noise_out * (1.0 + stereo_jitter) * 0.7
                   + tail * (1.0 + tail_pan) * 0.5;

        // Saturation
        let sat_l = math::tanh(left * self.drive);
        let sat_r = math::tanh(right * self.drive);

        (sat_l * self.level, sat_r * self.level)
    }

    /// Mono convenience — sums L+R and halves.
    fn next_sample(&mut self) -> f32 {
        let (l, r) = self.next_sample_stereo();
        (l + r) * 0.5
    }
}

// ─── Hi-Hat ───
// Multiple square waves at inharmonic frequencies + highpass + short envelope
struct HiHat {
    phases: [f32; 6],
    freqs: [f32; 6],
    base_freqs: [f32; 6],
    amp: f32,
    decay_rate: f32,
    filter: BiquadFilter,
    bp_filter: BiquadFilter,
    /// Six square oscillators through a 7.5 kHz highpass leaves only their
    /// top harmonics, which ran to Nyquist: 60% of the hat's energy sat above
    /// 10 kHz with its peak at 15 kHz. This puts a ceiling on it.
    tone: BiquadFilter,
    active: bool,
    level: f32,
    pitch: f32,
    rng: Rng,
}

impl HiHat {
    fn new() -> Self {
        let mut filter = BiquadFilter::new(SAMPLE_RATE);
        filter.set_params(FilterType::HighPass, 7500.0, 0.15);
        let mut bp_filter = BiquadFilter::new(SAMPLE_RATE);
        bp_filter.set_params(FilterType::BandPass, 9000.0, 0.25);
        let mut tone = BiquadFilter::new(SAMPLE_RATE);
        tone.set_params(FilterType::LowPass, 10000.0, 0.02);
        let base_freqs = [280.0, 350.0, 420.0, 495.0, 618.0, 725.0];
        Self {
            phases: [0.0; 6],
            freqs: base_freqs,
            base_freqs,
            amp: 0.0,
            decay_rate: 0.9975,
            filter,
            bp_filter,
            tone,
            active: false,
            level: 1.0,
            pitch: 1.0,
            rng: Rng::new(77777),
        }
    }

    fn trigger(&mut self, velocity: f32, open: bool) {
        self.phases = [0.0; 6];
        self.amp = velocity * 0.85;
        self.decay_rate = if open { 0.9997 } else { 0.9975 };
        // Randomize frequencies ±3% for metallic variation per hit
        for i in 0..6 {
            self.freqs[i] = self.base_freqs[i] * (0.97 + self.rng.next_f32() * 0.06);
        }
        self.filter.set_params(FilterType::HighPass, 7500.0 * self.pitch, 0.15);
        self.bp_filter.set_params(FilterType::BandPass, 9000.0 * self.pitch, 0.25);
        self.tone.set_params(FilterType::LowPass, 10000.0 * self.pitch, 0.02);
        self.active = true;
    }

    fn next_sample(&mut self) -> f32 {
        if !self.active {
            return 0.0;
        }

        let mut sum = 0.0;
        for i in 0..6 {
            let sq = if self.phases[i] < 0.5 { 1.0 } else { -1.0 };
            sum += sq;
            self.phases[i] += (self.freqs[i] * self.pitch) / SAMPLE_RATE;
            if self.phases[i] >= 1.0 {
                self.phases[i] -= 1.0;
            }
        }
        sum /= 6.0;

        let input = sum * self.amp;
        let hp = self.filter.process(input);
        let bp = self.bp_filter.process(input);
        let filtered = self.tone.process(hp * 0.7 + bp * 0.3);

        self.amp *= self.decay_rate;
        if self.amp < 0.001 {
            self.active = false;
        }

        filtered * 2.0 * self.level
    }
}

// ─── Clap ───
// Filtered noise with multi-trigger envelope
struct Clap {
    rng: Rng,
    amp: f32,
    filter: BiquadFilter,
    tail_hp: BiquadFilter,
    active: bool,
    stage: u8,
    stage_counter: u32,
    decay_rate: f32,
    level: f32,
}

impl Clap {
    fn new() -> Self {
        let mut filter = BiquadFilter::new(SAMPLE_RATE);
        filter.set_params(FilterType::BandPass, 2200.0, 0.35);
        let mut tail_hp = BiquadFilter::new(SAMPLE_RATE);
        tail_hp.set_params(FilterType::HighPass, 1500.0, 0.1);
        Self {
            rng: Rng::new(54321),
            amp: 0.0,
            filter,
            tail_hp,
            active: false,
            stage: 0,
            stage_counter: 0,
            decay_rate: 0.99955,
            level: 1.0,
        }
    }

    fn trigger(&mut self, velocity: f32) {
        self.amp = velocity;
        self.active = true;
        self.stage = 0;
        self.stage_counter = 0;
    }

    fn next_sample(&mut self) -> f32 {
        if !self.active {
            return 0.0;
        }

        let noise = self.rng.next_bipolar();

        // Multi-trigger: 4 quick bursts then decay (909 style)
        let env = match self.stage {
            0 | 1 | 2 | 3 => {
                self.stage_counter += 1;
                if self.stage_counter > 150 {
                    self.stage += 1;
                    self.stage_counter = 0;
                }
                if self.stage_counter < 60 {
                    self.amp
                } else {
                    0.0
                }
            }
            _ => {
                self.amp *= self.decay_rate;
                if self.amp < 0.001 {
                    self.active = false;
                }
                self.amp
            }
        };

        let bp = self.filter.process(noise * env);
        // Brighter tail via HP filter
        let hp = self.tail_hp.process(bp);
        let out = if self.stage > 3 { bp * 0.6 + hp * 0.4 } else { bp };
        out * 1.3 * self.level
    }
}

// ─── Tom ───
// Sine oscillator with pitch envelope (like kick but higher, less sweep, shorter)
struct Tom {
    phase: f32,
    freq: f32,
    start_freq: f32,
    end_freq: f32,
    pitch_decay: f32,
    amp: f32,
    amp_decay: f32,
    level: f32,
    active: bool,
    click_amp: f32,
    click_decay: f32,
    rng: Rng,
}

impl Tom {
    fn new() -> Self {
        Self {
            phase: 0.0,
            freq: 165.0,
            start_freq: 300.0,
            end_freq: 165.0,
            pitch_decay: 0.996,
            amp: 0.0,
            amp_decay: 0.9990,
            level: 1.0,
            active: false,
            click_amp: 0.0,
            click_decay: 0.90,
            rng: Rng::new(66666),
        }
    }

    fn trigger(&mut self, velocity: f32, note: u8) {
        self.phase = 0.0;
        // 3 toms with different pitch ranges based on MIDI note
        match note {
            43 => { self.start_freq = 220.0; self.end_freq = 110.0; } // low tom
            45 => { self.start_freq = 300.0; self.end_freq = 165.0; } // mid tom
            47 => { self.start_freq = 400.0; self.end_freq = 220.0; } // high tom
            _  => { self.start_freq = 300.0; self.end_freq = 165.0; } // default mid
        }
        self.freq = self.start_freq;
        self.amp = velocity;
        self.click_amp = velocity * 0.3;
        self.active = true;
    }

    fn next_sample(&mut self) -> f32 {
        if !self.active {
            return 0.0;
        }

        // Click transient: short noise burst
        let click = self.rng.next_bipolar() * self.click_amp;
        self.click_amp *= self.click_decay;

        let sine = math::sin(self.phase * math::TWO_PI) * self.amp;
        let out = math::tanh((sine + click) * 1.8); // 909 punch

        self.phase += self.freq / SAMPLE_RATE;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }

        // Pitch envelope: sweep down
        self.freq = self.end_freq + (self.freq - self.end_freq) * self.pitch_decay;

        // Amplitude envelope
        self.amp *= self.amp_decay;
        if self.amp < 0.001 && self.click_amp < 0.001 {
            self.active = false;
        }

        out * 0.8 * self.level
    }
}

// ─── Crash ───
// Metallic oscillator bank + noise, 909-style digital cymbal
struct Crash {
    rng: Rng,
    amp: f32,
    phases: [f32; 4],
    freqs: [f32; 4],
    hp_filter: BiquadFilter,
    bp_filter: BiquadFilter,
    lp_filter: BiquadFilter,
    decay_rate: f32,
    level: f32,
    active: bool,
}

impl Crash {
    fn new() -> Self {
        let mut hp_filter = BiquadFilter::new(SAMPLE_RATE);
        hp_filter.set_params(FilterType::HighPass, 3000.0, 0.15);
        let mut bp_filter = BiquadFilter::new(SAMPLE_RATE);
        bp_filter.set_params(FilterType::BandPass, 6500.0, 0.2);
        let mut lp_filter = BiquadFilter::new(SAMPLE_RATE);
        lp_filter.set_params(FilterType::LowPass, 12000.0, 0.1);
        Self {
            rng: Rng::new(99999),
            amp: 0.0,
            phases: [0.0; 4],
            freqs: [587.0, 845.0, 1273.0, 1704.0],
            hp_filter,
            bp_filter,
            lp_filter,
            decay_rate: 0.99996,
            level: 1.0,
            active: false,
        }
    }

    fn trigger(&mut self, velocity: f32) {
        self.amp = velocity;
        self.phases = [0.0; 4];
        self.active = true;
    }

    fn next_sample(&mut self) -> f32 {
        if !self.active {
            return 0.0;
        }

        // Metallic oscillator bank (4 square waves at inharmonic frequencies)
        let mut metal = 0.0;
        for i in 0..4 {
            metal += if self.phases[i] < 0.5 { 1.0 } else { -1.0 };
            self.phases[i] += self.freqs[i] / SAMPLE_RATE;
            if self.phases[i] >= 1.0 { self.phases[i] -= 1.0; }
        }
        metal /= 4.0;

        let noise = self.rng.next_bipolar();
        let mixed = (noise * 0.5 + metal * 0.5) * self.amp;
        let hp = self.hp_filter.process(mixed);
        let bp = self.bp_filter.process(mixed);
        let raw = hp * 0.6 + bp * 0.4;
        let out = self.lp_filter.process(raw);

        self.amp *= self.decay_rate;
        if self.amp < 0.001 {
            self.active = false;
        }

        out * 1.8 * self.level
    }
}

// ─── Pan Helper ───

/// Equal-power panning: pan in -1.0 (left) .. 1.0 (right)
fn pan_equal_power(signal: f32, pan: f32) -> (f32, f32) {
    let r = (pan + 1.0) * 0.5; // 0.0..1.0
    (signal * math::sqrt(1.0 - r), signal * math::sqrt(r))
}

// ─── Beats Module ───
// MIDI mapping: kick=36, snare=38, clap=39, hihat=42, tom=43/45/47, open_hihat=46, crash=49

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeatsParam {
    Level,
    KickDecay,
    SnareDecay,
    KickPan,
    SnarePan,
    HihatPan,
    ClapPan,
    // Section 10: new params
    KickClick,
    KickLevel,
    SnareLevel,
    HihatLevel,
    ClapLevel,
    KickPitch,
    SnarePitch,
    HihatPitch,
    StutterRate,
    StutterDrum,
    TomPan,
    CrashPan,
    SnareDrive,
    SnareSnap,
    KickDrive,
    HihatDecay,
}

pub struct BeatsModule {
    pub harmony: Option<crate::harmony::HarmonyContext>, // always None
    pub kick_env: [f32; BLOCK_SIZE],
    kick: Kick,
    snare: Snare,
    hihat: HiHat,
    clap: Clap,
    tom: Tom,
    crash: Crash,
    level: f32,
    kick_pan: f32,
    snare_pan: f32,
    hihat_pan: f32,
    clap_pan: f32,
    tom_pan: f32,
    crash_pan: f32,
    // Stutter/retrigger (pub for roll access from song_engine)
    pub stutter_drum: Option<u8>,
    pub stutter_interval: u32,
    pub stutter_counter: u32,
    pub stutter_velocity: f32,
    pub stutter_remaining: u32,
    bpm: f32,
}

impl BeatsModule {
    pub fn set_param(&mut self, param: BeatsParam, value: f32) {
        match param {
            BeatsParam::Level => self.level = value,
            BeatsParam::KickDecay => self.kick.amp_decay = 0.999 + value * 0.0009,
            BeatsParam::SnareDecay => self.snare.decay_rate = 0.9985 + value * 0.0013,
            BeatsParam::KickPan => self.kick_pan = math::clamp(value, -1.0, 1.0),
            BeatsParam::SnarePan => self.snare_pan = math::clamp(value, -1.0, 1.0),
            BeatsParam::HihatPan => self.hihat_pan = math::clamp(value, -1.0, 1.0),
            BeatsParam::ClapPan => self.clap_pan = math::clamp(value, -1.0, 1.0),
            BeatsParam::KickClick => self.kick.click_level = math::clamp(value, 0.0, 1.0),
            BeatsParam::KickLevel => self.kick.level = math::clamp(value, 0.0, 1.0),
            BeatsParam::SnareLevel => self.snare.level = value.max(0.0),
            BeatsParam::HihatLevel => self.hihat.level = math::clamp(value, 0.0, 1.0),
            BeatsParam::ClapLevel => self.clap.level = math::clamp(value, 0.0, 1.0),
            BeatsParam::KickPitch => self.kick.pitch = crate::params::DRUM_PITCH.to_real(value),
            BeatsParam::SnarePitch => self.snare.pitch = crate::params::DRUM_PITCH.to_real(value),
            BeatsParam::HihatPitch => self.hihat.pitch = crate::params::DRUM_PITCH.to_real(value),
            BeatsParam::StutterRate => self.set_stutter_rate(value),
            BeatsParam::StutterDrum => {
                // 0.0 = kick(36), 0.25 = snare(38), 0.5 = hihat(42), 0.75 = clap(39), 1.0 = tom(45)
                let note = if value < 0.15 { 36 }
                    else if value < 0.35 { 38 }
                    else if value < 0.55 { 42 }
                    else if value < 0.85 { 39 }
                    else { 45 };
                self.stutter_drum = Some(note);
            }
            BeatsParam::TomPan => self.tom_pan = math::clamp(value, -1.0, 1.0),
            BeatsParam::CrashPan => self.crash_pan = math::clamp(value, -1.0, 1.0),
            // snare_drive: 0.0 = clean (0.8), 1.0 = heavy grit (4.0)
            BeatsParam::SnareDrive => self.snare.drive = 0.8 + value * 3.2,
            // snare_snap: noise/crack intensity (0.2 minimum ensures some bite)
            BeatsParam::SnareSnap => self.snare.snap = 0.2 + value * 2.0,
            // kick_drive: 0.0 = warm (0.8), 1.0 = heavy (3.0)
            BeatsParam::KickDrive => self.kick.drive = 0.8 + value * 2.2,
            // hihat_decay: 0.0 = very tight, 1.0 = ringy
            BeatsParam::HihatDecay => self.hihat.decay_rate = 0.995 + value * 0.004,
        }
    }

    pub fn new() -> Self {
        Self {
            harmony: None,
            kick_env: [0.0; BLOCK_SIZE],
            kick: Kick::new(),
            snare: Snare::new(),
            hihat: HiHat::new(),
            clap: Clap::new(),
            tom: Tom::new(),
            crash: Crash::new(),
            level: 1.0,
            kick_pan: 0.0,
            snare_pan: -0.1,
            hihat_pan: 0.3,
            clap_pan: 0.15,
            tom_pan: 0.0,
            crash_pan: -0.2,
            stutter_drum: None,
            stutter_interval: 0,
            stutter_counter: 0,
            stutter_velocity: 0.0,
            stutter_remaining: 0,
            bpm: 120.0,
        }
    }

    pub fn set_bpm(&mut self, bpm: f32) {
        self.bpm = bpm;
        // Recalculate stutter interval if active
        if self.stutter_remaining > 0 && self.stutter_interval > 0 {
            // Keep current stutter going, new interval will apply on next set_stutter
        }
    }

    /// Set stutter: rate maps to subdivisions
    /// 0 = off, 1 = 8th notes, 2 = 16th notes, 3 = 32nd notes
    pub fn set_stutter(&mut self, note: u8, rate: u8, count: u32) {
        if rate == 0 {
            self.stutter_remaining = 0;
            self.stutter_drum = None;
            return;
        }
        self.stutter_drum = Some(note);
        self.stutter_velocity = 0.8;
        self.stutter_remaining = count;
        // Calculate interval in samples from BPM
        let beat_samples = SAMPLE_RATE * 60.0 / self.bpm;
        self.stutter_interval = match rate {
            1 => (beat_samples / 2.0) as u32,  // 8th notes
            2 => (beat_samples / 4.0) as u32,  // 16th notes
            3 => (beat_samples / 8.0) as u32,  // 32nd notes
            _ => (beat_samples / 4.0) as u32,
        };
        self.stutter_counter = 0;
    }

    /// Set stutter rate from normalized 0.0-1.0 value (for param locks)
    fn set_stutter_rate(&mut self, value: f32) {
        if value < 0.1 {
            self.stutter_remaining = 0;
            return;
        }
        let rate = if value < 0.4 { 1 } else if value < 0.7 { 2 } else { 3 };
        let beat_samples = SAMPLE_RATE * 60.0 / self.bpm;
        self.stutter_interval = match rate {
            1 => (beat_samples / 2.0) as u32,
            2 => (beat_samples / 4.0) as u32,
            _ => (beat_samples / 8.0) as u32,
        };
        self.stutter_remaining = 4; // default 4 retriggers
        self.stutter_counter = 0;
        self.stutter_velocity = 0.8;
    }

    /// Trigger a drum by MIDI note (used internally by stutter)
    fn trigger_drum(&mut self, note: u8, velocity: f32) {
        match note {
            36 => self.kick.trigger(velocity),
            38 => self.snare.trigger(velocity),
            39 => self.clap.trigger(velocity),
            42 => self.hihat.trigger(velocity, false),
            43 | 45 | 47 => self.tom.trigger(velocity, note),
            46 => self.hihat.trigger(velocity, true),
            49 => self.crash.trigger(velocity),
            _ => {}
        }
    }

    /// Process stutter retrigger (called per sample)
    fn process_stutter(&mut self) {
        if self.stutter_remaining == 0 || self.stutter_interval == 0 {
            return;
        }
        self.stutter_counter += 1;
        if self.stutter_counter >= self.stutter_interval {
            self.stutter_counter = 0;
            self.stutter_remaining -= 1;
            if let Some(note) = self.stutter_drum {
                self.trigger_drum(note, self.stutter_velocity);
            }
            // Slight velocity decay per retrigger
            self.stutter_velocity *= 0.9;
        }
    }

    /// Process a block with per-drum stereo panning (equal-power).
    pub fn process_block_stereo(&mut self, output_l: &mut [f32], output_r: &mut [f32]) {
        for i in 0..output_l.len() {
            self.process_stutter();

            let k = self.kick.next_sample();
            self.kick_env[i] = math::abs(k);
            let (snare_l, snare_r) = self.snare.next_sample_stereo();
            let h = self.hihat.next_sample();
            let c = self.clap.next_sample();
            let t = self.tom.next_sample();
            let cr = self.crash.next_sample();

            let (kl, kr) = pan_equal_power(k, self.kick_pan);
            // Snare: apply pan offset to already-stereo signal
            let pan_r = (self.snare_pan + 1.0) * 0.5;
            let sl = snare_l * math::sqrt(1.0 - pan_r);
            let sr = snare_r * math::sqrt(pan_r);
            let (hl, hr) = pan_equal_power(h, self.hihat_pan);
            let (cl, cright) = pan_equal_power(c, self.clap_pan);
            let (tl, tr) = pan_equal_power(t, self.tom_pan);
            let (crl, crr) = pan_equal_power(cr, self.crash_pan);

            output_l[i] = (kl + sl + hl + cl + tl + crl) * self.level;
            output_r[i] = (kr + sr + hr + cright + tr + crr) * self.level;
        }
    }
}

impl Module for BeatsModule {
    fn process_block(&mut self, output: &mut [f32]) {
        for sample in output.iter_mut() {
            self.process_stutter();

            let k = self.kick.next_sample();
            let s = self.snare.next_sample();
            let h = self.hihat.next_sample();
            let c = self.clap.next_sample();
            let t = self.tom.next_sample();
            let cr = self.crash.next_sample();
            *sample = (k + s + h + c + t + cr) * self.level;
        }
    }

    fn note_on(&mut self, note: u8, velocity: f32) {
        self.trigger_drum(note, velocity);
    }

    fn note_off(&mut self, _note: u8) {
        // Percussion doesn't respond to note off
    }

    fn reset(&mut self) {
        self.kick.active = false;
        self.snare.active = false;
        self.hihat.active = false;
        self.clap.active = false;
        self.tom.active = false;
        self.crash.active = false;
        self.stutter_remaining = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BLOCK_SIZE;
    use alloc::vec;
    use alloc::vec::Vec;

    fn render_beats(module: &mut BeatsModule, note: u8, vel: f32, samples: usize) -> Vec<f32> {
        let mut out = vec![0.0f32; samples];
        module.note_on(note, vel);
        let mut pos = 0;
        while pos < samples {
            let bl = BLOCK_SIZE.min(samples - pos);
            module.process_block(&mut out[pos..pos + bl]);
            pos += bl;
        }
        out
    }

    #[test]
    fn test_kick_click_transient() {
        let mut beats = BeatsModule::new();
        beats.set_param(BeatsParam::KickClick, 1.0); // max click
        let samples = render_beats(&mut beats, 36, 1.0, 4410); // 100ms

        // First ~2ms (88 samples) should have higher energy than without click
        let early_rms: f32 = (samples[..88].iter().map(|x| x * x).sum::<f32>() / 88.0).sqrt();
        // The click should add noticeable energy in the attack
        assert!(early_rms > 0.1, "Kick click should add energy in first 2ms, got RMS={}", early_rms);
    }

    #[test]
    fn test_kick_no_click() {
        let mut beats = BeatsModule::new();
        beats.set_param(BeatsParam::KickClick, 0.0); // no click
        let samples = render_beats(&mut beats, 36, 1.0, 4410);

        // Kick should still produce sound
        let peak = samples.iter().fold(0.0f32, |a, &b| a.max(math::abs(b)));
        assert!(peak > 0.3, "Kick should produce output even without click");
    }

    #[test]
    fn test_per_drum_level() {
        let mut beats = BeatsModule::new();
        let full = render_beats(&mut beats, 36, 1.0, 4410);
        let full_peak = full.iter().fold(0.0f32, |a, &b| a.max(math::abs(b)));

        let mut beats = BeatsModule::new();
        beats.set_param(BeatsParam::KickLevel, 0.5);
        let half = render_beats(&mut beats, 36, 1.0, 4410);
        let half_peak = half.iter().fold(0.0f32, |a, &b| a.max(math::abs(b)));

        // Half level should be significantly quieter
        assert!(half_peak < full_peak * 0.8, "Half level should be quieter: full={}, half={}", full_peak, half_peak);
    }

    #[test]
    fn test_tom_synthesis() {
        let mut beats = BeatsModule::new();
        // Low tom
        let low = render_beats(&mut beats, 43, 0.9, 4410);
        let low_peak = low.iter().fold(0.0f32, |a, &b| a.max(math::abs(b)));

        let mut beats = BeatsModule::new();
        // High tom
        let high = render_beats(&mut beats, 47, 0.9, 4410);
        let high_peak = high.iter().fold(0.0f32, |a, &b| a.max(math::abs(b)));

        assert!(low_peak > 0.2, "Low tom should produce output, got peak={}", low_peak);
        assert!(high_peak > 0.2, "High tom should produce output, got peak={}", high_peak);
    }

    #[test]
    fn test_crash_synthesis() {
        let mut beats = BeatsModule::new();
        let samples = render_beats(&mut beats, 49, 0.9, 44100); // 1 second

        let peak = samples.iter().fold(0.0f32, |a, &b| a.max(math::abs(b)));
        assert!(peak > 0.05, "Crash should produce output, got peak={}", peak);

        // Crash should still have energy near end (~1s) due to long decay
        let late_rms: f32 = (samples[40000..44100].iter().map(|x| x * x).sum::<f32>() / 4100.0).sqrt();
        assert!(late_rms > 0.001, "Crash should have long tail, late RMS={}", late_rms);
    }

    #[test]
    fn test_hihat_variation() {
        // Trigger hihat twice and verify frequencies differ
        let mut hihat = HiHat::new();
        hihat.trigger(0.8, false);
        let freqs1 = hihat.freqs;

        hihat.trigger(0.8, false);
        let freqs2 = hihat.freqs;

        let mut any_different = false;
        for i in 0..6 {
            if (freqs1[i] - freqs2[i]).abs() > 0.01 {
                any_different = true;
                break;
            }
        }
        assert!(any_different, "Hi-hat frequencies should vary between triggers");
    }

    #[test]
    fn test_stutter_retrigger() {
        let mut beats = BeatsModule::new();
        beats.set_bpm(120.0);
        // Trigger kick then set up stutter
        beats.note_on(36, 1.0);
        beats.set_stutter(36, 3, 4); // 32nd notes, 4 retriggers

        // Process enough samples for stutter to complete
        let samples = 44100; // 1 second
        let mut out = vec![0.0f32; samples];
        let mut pos = 0;
        while pos < samples {
            let bl = BLOCK_SIZE.min(samples - pos);
            beats.process_block(&mut out[pos..pos + bl]);
            pos += bl;
        }

        // Should have multiple amplitude peaks from retriggers
        let peak = out.iter().fold(0.0f32, |a, &b| a.max(math::abs(b)));
        assert!(peak > 0.3, "Stutter should produce audible output");
    }

    #[test]
    fn test_pitch_scaling() {
        let mut beats = BeatsModule::new();
        beats.set_param(BeatsParam::KickPitch, 1.0); // 2× pitch
        let high = render_beats(&mut beats, 36, 1.0, 4410);

        let mut beats = BeatsModule::new();
        beats.set_param(BeatsParam::KickPitch, 0.0); // 0.5× pitch
        let low = render_beats(&mut beats, 36, 1.0, 4410);

        // Both should produce output
        let high_peak = high.iter().fold(0.0f32, |a, &b| a.max(math::abs(b)));
        let low_peak = low.iter().fold(0.0f32, |a, &b| a.max(math::abs(b)));
        assert!(high_peak > 0.2, "High pitch kick should produce output");
        assert!(low_peak > 0.2, "Low pitch kick should produce output");
    }
}
