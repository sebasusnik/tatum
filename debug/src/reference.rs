//! `tatum analyze` and `tatum compare`: a reference track measured, and the
//! song measured the same way next to it.
//!
//! It exists because a song named as a reference reached the model writing
//! the song as a word: "make the kick sound like that record" gave a kick
//! that sounded like nothing in particular, because the model cannot hear
//! the record or its own render. Measured, the record is numbers it can
//! close in on -- where the kick's sweep lands, how long it rings, how much
//! of the mix sits at 250 Hz -- and the comparison says which way each one
//! is off and what in a `.synth` file moves it.
//!
//! Everything is measured at the engine's rate, so a 48 kHz file is
//! converted first, and everything that compares two pieces of audio is
//! relative to their own level: the engine brings a song to -18 LUFS and a
//! record is mastered far louder, so absolute levels would only ever say
//! that.

use std::fmt::Write as _;

use tatum_core::analysis;
use tatum_core::dsl::compiler::CompiledSong;
use tatum_core::song_engine::SongEngine;
use tatum_core::output::Loudness;
use tatum_core::{BLOCK_SIZE, SAMPLE_RATE};

use crate::fft::fft;
use crate::wav::Audio;

const SR: f64 = SAMPLE_RATE as f64;

/// Onset frames: 5.8 ms at 44.1 kHz.
const HOP: usize = 256;

/// Centres of the octave bands the spectrum is reported in.
pub const BAND_HZ: [f32; 10] = [31.5, 63.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];

const MASKED: &str =
    "something as loud as the kick fills its band between hits (a held bass?): the tail and the decay \
     are partly that. --from/--to on a stretch where the kick plays alone reads it cleanly";

const NOTE_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

/// What a piece of audio measures.
#[derive(Debug, Clone)]
pub struct Profile {
    pub seconds: f32,
    /// Integrated loudness, BS.1770.
    pub lufs: Option<f32>,
    /// Sample peak, dBFS.
    pub peak_db: f32,
    /// Stereo width above 250 Hz (see `analysis::stereo_width_above`).
    pub width: f32,
    /// Stereo width under 150 Hz: on a club system it should be next to 0.
    pub width_low: f32,
    pub tempo: Option<f32>,
    /// Lowest and highest tempo, when it moves by more than 1.5 BPM.
    pub tempo_span: Option<(f32, f32)>,
    pub kick: Option<Kick>,
    /// Energy per octave band, dB against the sum of all of them.
    pub bands: [f32; 10],
    /// Share of the tonal energy under 250 Hz per pitch class, C first.
    pub low_notes: [f32; 12],
    pub key: Option<Key>,
}

/// The kick, from the median of every hit found.
#[derive(Debug, Clone)]
pub struct Kick {
    pub hits: usize,
    /// The hits the figures below come from: the ones with the least else
    /// in the kick's band.
    pub measured: usize,
    /// Frequency of the first cycle after the hit.
    pub start_hz: f32,
    /// Where the sweep settles: the body's note.
    pub tail_hz: f32,
    /// From the hit until the pitch is within 10% of the tail.
    pub sweep_ms: f32,
    /// From the peak until the body is 12 and 24 dB down. `None`: it was
    /// still above that when the next hit came.
    pub decay12_ms: Option<f32>,
    pub decay24_ms: Option<f32>,
    /// The first 10 ms above 2 kHz against the first 100 ms under 400 Hz.
    pub click_db: f32,
    /// How loud the band under 400 Hz is late in the beat, when the kick has
    /// died, against the kick's peak, on the hits measured. Above about
    /// -10 dB something else -- a held bass -- fills the band even there,
    /// and the tail and the decay are partly that something.
    pub under_db: f32,
}

impl Kick {
    pub fn masked(&self) -> bool {
        self.under_db > -10.0
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Key {
    /// Pitch class of the tonic, C = 0.
    pub tonic: usize,
    pub minor: bool,
    /// Correlation with the key's profile: under 0.5 the music hardly has one.
    pub strength: f32,
}

impl Key {
    pub fn name(&self) -> String {
        format!("{} {}", NOTE_NAMES[self.tonic], if self.minor { "minor" } else { "major" })
    }

    /// The key with its relative, which has the same notes: what the notes
    /// alone cannot tell apart, and techno often sits between.
    pub fn with_relative(&self) -> String {
        let (tonic, minor) = if self.minor { ((self.tonic + 3) % 12, false) } else { ((self.tonic + 9) % 12, true) };
        format!("{} / {}", self.name(), Key { tonic, minor, strength: self.strength }.name())
    }
}

/// `Hz` as a note name with its octave: 49 Hz is G1.
pub fn note_name(hz: f32) -> String {
    let midi = 69.0 + 12.0 * (hz / 440.0).log2();
    let n = midi.round() as i32;
    let cents = ((midi - n as f32) * 100.0).round() as i32;
    let name = format!("{}{}", NOTE_NAMES[n.rem_euclid(12) as usize], n.div_euclid(12) - 1);
    if cents.abs() >= 15 {
        format!("{name} {cents:+}c")
    } else {
        name
    }
}

/// The lowest and highest sample rates a reference may have. Under 8 kHz
/// nothing a kick or a mix is judged by survives; over 384 kHz no converter
/// records. A header outside them is broken or made up, and converting from
/// it would ask for more memory than there is.
pub const MIN_RATE: u32 = 8_000;
pub const MAX_RATE: u32 = 384_000;

/// The longest stretch measured at once: a long extended mix. A whole DJ set
/// would take minutes and gigabytes; `--from`/`--to` pick the part to read.
pub const MAX_SECONDS: f32 = 15.0 * 60.0;

/// Measure audio at any rate. `from`/`to` trim it, in seconds. The trim is
/// made at the file's own rate, before converting, so only the stretch asked
/// for is ever converted.
pub fn analyze(audio: &Audio, from: Option<f32>, to: Option<f32>) -> Result<Profile, String> {
    let (left, right) = stretch(audio, from, to, MAX_SECONDS)?;
    if left.len() < SAMPLE_RATE as usize * 2 {
        return Err("less than two seconds of audio; give it at least a few bars".into());
    }
    Ok(measure(&left, &right))
}

/// The stretch `from`..`to` of `audio`, checked, no longer than `longest`
/// seconds, and at the engine's rate.
fn stretch(audio: &Audio, from: Option<f32>, to: Option<f32>, longest: f32) -> Result<(Vec<f32>, Vec<f32>), String> {
    let rate = audio.rate;
    if !(MIN_RATE..=MAX_RATE).contains(&rate) {
        return Err(format!("a sample rate of {rate} Hz; a reference has to be between {MIN_RATE} and {MAX_RATE} Hz"));
    }
    let len = audio.left.len().min(audio.right.len());
    let at = |s: f32| ((s.max(0.0) as f64 * rate as f64) as usize).min(len);
    let a = from.map_or(0, at);
    let b = to.map_or(len, at);
    if b <= a {
        return Err("--from is past --to, or past the end of the file".into());
    }
    let seconds = (b - a) as f64 / rate as f64;
    if seconds > longest as f64 {
        return Err(format!(
            "{} of audio; at most {} is measured at once. Pick a stretch with --from/--to (from/to), \
             e.g. the intro and a drop",
            clock(seconds),
            clock(longest as f64)
        ));
    }
    Ok((to_engine_rate(&audio.left[a..b], rate), to_engine_rate(&audio.right[a..b], rate)))
}

/// `m:ss`.
fn clock(seconds: f64) -> String {
    let s = seconds.round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Measure audio already at the engine's rate.
pub fn measure(l: &[f32], r: &[f32]) -> Profile {
    let mono: Vec<f32> = l.iter().zip(r).map(|(a, b)| 0.5 * (a + b)).collect();

    let mut meter = Loudness::new();
    let mut peak = 0.0f32;
    for (a, b) in l.iter().zip(r) {
        meter.push(*a, *b);
        peak = peak.max(a.abs()).max(b.abs());
    }

    let low_l = Biquad::lowpass(150.0).run_twice(l);
    let low_r = Biquad::lowpass(150.0).run_twice(r);

    let odf = Onsets::new(&mono);
    let (tempo, tempo_span) = odf.tempo();
    let kick = kick(&mono, &odf, tempo);
    let (low_notes, key) = notes(&mono);

    Profile {
        seconds: (l.len() as f64 / SR) as f32,
        lufs: meter.lufs(),
        peak_db: db(peak),
        width: analysis::stereo_width_above(l, r, 250.0, SAMPLE_RATE),
        width_low: analysis::stereo_width(&low_l, &low_r),
        tempo,
        tempo_span,
        kick,
        bands: octave_bands(&mono),
        low_notes,
        key,
    }
}

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-9).log10()
}

/// A song rendered as `tatum render` renders it, at the gain the engine
/// gives the whole song, so a solo is measured at its level in the mix.
/// `bars` is a first and last bar counted from 1, cut where the bars really
/// fall: each scene at its own tempo, as `tatum debug` counts them. A range
/// that runs past the end stops at the end; one that starts past it is an
/// error.
pub fn render(song: CompiledSong, gain: f32, bars: Option<(u32, u32)>) -> Result<(Vec<f32>, Vec<f32>), String> {
    let mut engine = SongEngine::from_compiled(song);
    engine.set_output_gain(gain);
    let clock = crate::Clock::new(&engine);
    let total = clock.bars();
    if total == 0 {
        return Err("nothing to measure: the arrangement is empty".into());
    }
    let (first, last) = match bars {
        Some((a, _)) if a > total => return Err(format!("bars {a}: the song has {total} bars")),
        Some((a, b)) => (a.max(1), b.clamp(a.max(1), total)),
        None => (1, total),
    };
    let start = clock.bar_starts[first as usize - 1];
    let end = clock.bar_starts[last as usize];
    let (mut l, mut r) = (Vec::with_capacity(end - start), Vec::with_capacity(end - start));
    let (mut bl, mut br) = ([0.0f32; BLOCK_SIZE], [0.0f32; BLOCK_SIZE]);
    engine.start();
    let mut pos = 0usize;
    while pos < end && engine.running() {
        let chunk = (end - pos).min(BLOCK_SIZE);
        engine.process_block_stereo(&mut bl[..chunk], &mut br[..chunk]);
        let from = start.saturating_sub(pos).min(chunk);
        l.extend_from_slice(&bl[from..chunk]);
        r.extend_from_slice(&br[from..chunk]);
        pos += chunk;
    }
    Ok((l, r))
}

// ── Resampling ──

/// The audio at 44.1 kHz. Windowed sinc, 64 taps, from a table of 2048
/// phases: a position is rounded to 1/2048 of a sample, which is far below
/// anything measured here. Below the input's rate the sinc's cutoff moves to
/// the output's Nyquist, so nothing folds back.
fn to_engine_rate(x: &[f32], from: u32) -> Vec<f32> {
    const HALF: usize = 32;
    const PHASES: usize = 2048;
    let to = SAMPLE_RATE as u32;
    if from == to || x.is_empty() {
        return x.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let cutoff = (1.0 / ratio).min(1.0) * 0.97;
    let mut table = vec![0.0f32; (PHASES + 1) * 2 * HALF];
    for p in 0..=PHASES {
        let frac = p as f64 / PHASES as f64;
        let row = &mut table[p * 2 * HALF..(p + 1) * 2 * HALF];
        let mut sum = 0.0;
        for (k, w) in row.iter_mut().enumerate() {
            // Tap k reads input sample floor(t) - (HALF - 1) + k.
            let d = frac + (HALF as f64 - 1.0) - k as f64;
            let u = d / HALF as f64;
            let window = if u.abs() >= 1.0 {
                0.0
            } else {
                0.42 + 0.5 * (std::f64::consts::PI * u).cos() + 0.08 * (2.0 * std::f64::consts::PI * u).cos()
            };
            let arg = std::f64::consts::PI * d * cutoff;
            let sinc = if arg.abs() < 1e-9 { 1.0 } else { arg.sin() / arg };
            let v = sinc * window;
            sum += v;
            *w = v as f32;
        }
        // Each row sums to one, so a constant comes out as itself.
        for w in row.iter_mut() {
            *w /= sum as f32;
        }
    }
    let n = (x.len() as f64 / ratio) as usize;
    let mut out = Vec::with_capacity(n);
    for j in 0..n {
        let t = j as f64 * ratio;
        let c = t.floor() as isize;
        let p = ((t - c as f64) * PHASES as f64).round() as usize;
        let row = &table[p * 2 * HALF..(p + 1) * 2 * HALF];
        let mut acc = 0.0f32;
        for (k, w) in row.iter().enumerate() {
            let i = c - (HALF as isize - 1) + k as isize;
            if i >= 0 && (i as usize) < x.len() {
                acc += w * x[i as usize];
            }
        }
        out.push(acc);
    }
    out
}

// ── Filters ──

/// RBJ biquad, in f64: the analysis runs over minutes of audio and a sub
/// filter at 44.1 kHz has its poles very near the unit circle.
#[derive(Clone, Copy)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
}

impl Biquad {
    fn lowpass(hz: f64) -> Self {
        let w = 2.0 * std::f64::consts::PI * hz / SR;
        let alpha = w.sin() / (2.0 * std::f64::consts::FRAC_1_SQRT_2);
        let c = w.cos();
        Self::norm([(1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0], [1.0 + alpha, -2.0 * c, 1.0 - alpha])
    }

    fn highpass(hz: f64) -> Self {
        let w = 2.0 * std::f64::consts::PI * hz / SR;
        let alpha = w.sin() / (2.0 * std::f64::consts::FRAC_1_SQRT_2);
        let c = w.cos();
        Self::norm([(1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0], [1.0 + alpha, -2.0 * c, 1.0 - alpha])
    }

    fn norm(b: [f64; 3], a: [f64; 3]) -> Self {
        Biquad { b: [b[0] / a[0], b[1] / a[0], b[2] / a[0]], a: [a[1] / a[0], a[2] / a[0]] }
    }

    fn run(&self, x: &[f32]) -> Vec<f32> {
        let (mut z1, mut z2) = (0.0f64, 0.0f64);
        x.iter()
            .map(|&v| {
                let v = v as f64;
                let y = self.b[0] * v + z1;
                z1 = self.b[1] * v - self.a[0] * y + z2;
                z2 = self.b[2] * v - self.a[1] * y;
                y as f32
            })
            .collect()
    }

    /// Two in series: 24 dB an octave.
    fn run_twice(&self, x: &[f32]) -> Vec<f32> {
        self.run(&self.run(x))
    }
}

// ── Onsets and tempo ──

/// How much new energy arrives in each hop, in three bands, and the low band
/// alone, which is where the kick is.
struct Onsets {
    /// All three bands, each normalised to its own mean.
    all: Vec<f32>,
    low: Vec<f32>,
    /// Energy under 150 Hz per hop.
    low_energy: Vec<f32>,
}

impl Onsets {
    fn new(mono: &[f32]) -> Self {
        let low = Biquad::lowpass(150.0).run_twice(mono);
        let mid = Biquad::lowpass(5000.0).run(&Biquad::highpass(150.0).run(mono));
        let high = Biquad::highpass(5000.0).run(mono);
        let energy = |x: &[f32]| -> Vec<f32> {
            x.chunks(HOP).map(|c| c.iter().map(|v| v * v).sum::<f32>() / c.len() as f32).collect()
        };
        let flux = |e: &[f32]| -> Vec<f32> {
            let mean = e.iter().sum::<f32>() / e.len().max(1) as f32;
            if mean <= 0.0 {
                return vec![0.0; e.len()];
            }
            // Log-compressed, so a quiet hat counts next to a loud kick.
            let c: Vec<f32> = e.iter().map(|v| (1.0 + 100.0 * v / mean).ln()).collect();
            let d: Vec<f32> = (0..c.len()).map(|i| if i == 0 { 0.0 } else { (c[i] - c[i - 1]).max(0.0) }).collect();
            let m = d.iter().sum::<f32>() / d.len().max(1) as f32;
            if m > 0.0 {
                d.iter().map(|v| v / m).collect()
            } else {
                d
            }
        };
        // The low band over four hops, 23 ms: one hop is a third of a cycle
        // of a 50 Hz tail, and its energy would rise and fall with the wave.
        let low_energy = {
            let e = energy(&low);
            (0..e.len())
                .map(|i| e[i.saturating_sub(3)..=i].iter().sum::<f32>() / (i.min(3) + 1) as f32)
                .collect::<Vec<_>>()
        };
        let (fl, fm, fh) = (flux(&low_energy), flux(&energy(&mid)), flux(&energy(&high)));
        let all = (0..fl.len()).map(|i| fl[i] + 0.5 * fm[i] + 0.5 * fh[i]).collect();
        Onsets { all, low: fl, low_energy }
    }

    /// The tempo, and its lowest and highest when it moves: measured over
    /// 16 s windows every 8 s, so a set that climbs from 137 to 150 reads as
    /// that and not as a tempo in between that it never plays at.
    fn tempo(&self) -> (Option<f32>, Option<(f32, f32)>) {
        let window = (16.0 * SR / HOP as f64) as usize;
        let windows: Vec<(f32, f32)> = if self.all.len() < window * 3 / 2 {
            Self::tempo_of(&self.all).into_iter().collect()
        } else {
            (0..=self.all.len() - window)
                .step_by(window / 2)
                .filter_map(|a| Self::tempo_of(&self.all[a..a + window]))
                .collect()
        };
        // A breakdown with no beat in it still yields a tempo, a wrong one
        // and a weak one: windows far less periodic than the rest are left out.
        let mut strength: Vec<f32> = windows.iter().map(|w| w.1).collect();
        let typical = median(&mut strength);
        let mut found: Vec<f32> = windows.iter().filter(|w| w.1 >= 0.5 * typical).map(|w| w.0).collect();
        if found.is_empty() {
            return (None, None);
        }
        let mid = median(&mut found);
        // A stretch felt in threes (91 under 137) or in halves is the same
        // tempo heard another way, not a tempo change: it stays out of the span.
        let related = |t: f32| {
            [0.5f32, 2.0 / 3.0, 0.75, 0.8, 1.25, 4.0 / 3.0, 1.5, 2.0].iter().any(|r| (t / (mid * r) - 1.0).abs() < 0.02)
        };
        found.retain(|&t| !related(t));
        let (lo, hi) = (found[found.len() / 10], found[(found.len() * 9 / 10).min(found.len() - 1)]);
        (Some(mid), (hi - lo > 1.5).then_some((lo, hi)))
    }

    /// Tempo between 60 and 200 BPM, from the autocorrelation of the onsets.
    /// A lag is scored with its double and its quadruple, so a beat whose
    /// bars line up beats the half beat whose do not, and then refined on the
    /// longest multiple that fits, where a hop of error is a quarter or an
    /// eighth of what it would be on the beat.
    fn tempo_of(onsets: &[f32]) -> Option<(f32, f32)> {
        let hop_s = HOP as f64 / SR;
        let n = onsets.len();
        if (n as f64) * hop_s < 4.0 {
            return None;
        }
        let mean = onsets.iter().map(|&v| v as f64).sum::<f64>() / n as f64;
        let x: Vec<f64> = onsets.iter().map(|&v| v as f64 - mean).collect();
        let lag_of = |bpm: f64| 60.0 / (bpm * hop_s);
        let max_lag = ((lag_of(60.0) * 8.0) as usize + 4).min(n / 2);
        let ac: Vec<f64> = (0..=max_lag)
            .map(|lag| {
                let m = n - lag;
                x[..m].iter().zip(&x[lag..]).map(|(a, b)| a * b).sum::<f64>() / m as f64
            })
            .collect();
        if ac[0] <= 0.0 {
            return None;
        }
        let at = |lag: usize| ac.get(lag).copied().unwrap_or(0.0);
        let (lo, hi) = (lag_of(200.0).floor() as usize, (lag_of(60.0).ceil() as usize).min(max_lag));
        let mut best = (0usize, f64::MIN);
        for lag in lo..=hi {
            let bpm = 60.0 / (lag as f64 * hop_s);
            // A preference for the range dance music lives in, so 130 does
            // not come out as 65 when a bass line repeats every two beats.
            let prior = (-0.5 * ((bpm / 125.0).log2() / 0.6).powi(2)).exp();
            let score = (at(lag) + 0.5 * at(2 * lag) + 0.25 * at(4 * lag)) * prior;
            if score > best.1 {
                best = (lag, score);
            }
        }
        if best.1 <= 0.0 || at(best.0) < 0.05 * ac[0] {
            return None;
        }
        let mut lag = best.0 as f64;
        for m in [8usize, 4, 2] {
            let centre = best.0 * m;
            if centre + m + 1 >= ac.len() {
                continue;
            }
            let (a, b) = (centre - m, centre + m);
            let peak = (a..=b).max_by(|&i, &j| ac[i].total_cmp(&ac[j])).unwrap_or(centre);
            if peak == 0 || peak + 1 >= ac.len() {
                continue;
            }
            let (y0, y1, y2) = (ac[peak - 1], ac[peak], ac[peak + 1]);
            let den = y0 - 2.0 * y1 + y2;
            let shift = if den.abs() > 1e-12 { 0.5 * (y0 - y2) / den } else { 0.0 };
            lag = (peak as f64 + shift.clamp(-0.5, 0.5)) / m as f64;
            break;
        }
        Some(((60.0 / (lag * hop_s)) as f32, (at(best.0) / ac[0]) as f32))
    }

    /// Hops where a kick lands: peaks of the low band's onset at least most
    /// of a beat apart, and loud: within 9 dB of the loudest hit, so a bass
    /// note that starts on its own is not taken for one.
    fn kicks(&self, tempo: Option<f32>) -> Vec<usize> {
        let hop_s = HOP as f32 / SAMPLE_RATE;
        let spacing = tempo.map_or(0.2, |t| 0.45 * 60.0 / t);
        let gap = ((spacing / hop_s) as usize).max(1);
        let mut sorted = self.low.clone();
        sorted.sort_by(f32::total_cmp);
        // A kick is one or two hops in eighty, so the threshold is taken
        // from the top half percent, not from where most of the hops are.
        let top = sorted.get(sorted.len() * 995 / 1000).copied().unwrap_or(0.0);
        if top <= 0.0 {
            return Vec::new();
        }
        let mut picked: Vec<usize> = Vec::new();
        for i in 0..self.low.len() {
            let v = self.low[i];
            if v < 0.25 * top {
                continue;
            }
            let (a, b) = (i.saturating_sub(gap / 2), (i + gap / 2 + 1).min(self.low.len()));
            if self.low[a..b].iter().any(|&w| w > v) {
                continue;
            }
            if picked.last().is_some_and(|&p| i - p < gap) {
                continue;
            }
            picked.push(i);
        }
        // Level of each hit: the most low-band energy in the 60 ms after it.
        let span = (0.06 / hop_s) as usize + 1;
        let level =
            |i: usize| self.low_energy[i..(i + span).min(self.low_energy.len())].iter().copied().fold(0.0f32, f32::max);
        let loudest = picked.iter().map(|&i| level(i)).fold(0.0f32, f32::max);
        picked.retain(|&i| level(i) >= loudest * 0.126);
        picked
    }
}

// ── The kick ──

fn kick(mono: &[f32], odf: &Onsets, tempo: Option<f32>) -> Option<Kick> {
    let hops = odf.kicks(tempo);
    if hops.len() < 2 {
        return None;
    }
    let body = Biquad::lowpass(400.0).run_twice(mono);
    let top = Biquad::highpass(2000.0).run_twice(mono);
    let samples = |ms: f64| (ms * SR / 1000.0) as usize;

    // Where each hit starts: the body's peak in the 40 ms from just before
    // the onset hop, then back from it to where the last 3 ms were all
    // under a tenth of that. Walking forward from the hop instead began some
    // hits mid-wave, a few samples later or earlier each time, and their
    // average came out a smear.
    let mut starts = Vec::with_capacity(hops.len());
    let quiet = samples(3.0);
    for &h in &hops {
        let a = (h * HOP).saturating_sub(2 * HOP);
        let b = (a + samples(40.0)).min(body.len());
        if b <= a {
            continue;
        }
        let (pk, pv) = body[a..b].iter().enumerate().map(|(i, v)| (a + i, v.abs())).fold((a, 0.0f32), |m, x| {
            if x.1 > m.1 {
                x
            } else {
                m
            }
        });
        if pv <= 0.0 {
            continue;
        }
        let floor = pk.saturating_sub(samples(30.0));
        let mut on = pk;
        while on > floor && body[on.saturating_sub(quiet)..on].iter().any(|v| v.abs() >= 0.1 * pv) {
            on -= 1;
        }
        starts.push(on);
    }
    if starts.len() < 2 {
        return None;
    }

    // Every hit is the same sound, and nothing else in the mix lands on it
    // the same way each time, so the hits are averaged: the bass, the pads
    // and the reverb under the kick's tail cancel out and the kick is left.
    // Measured one hit at a time, a sub bass under a 37 Hz kick read as a
    // 69 Hz tail. The average is taken twice, the second time with each hit
    // slid to where it best matches the first average, because a hit found
    // a millisecond late is a third of a cycle out at 280 Hz.
    let mut gaps: Vec<f32> = starts.windows(2).map(|w| (w[1] - w[0]) as f32).collect();
    let gap = median(&mut gaps) as usize;
    let len = gap.min(samples(600.0)).max(samples(60.0));

    // A bass that starts with every kick is locked to it, and averaging
    // keeps it: a sub on C under a 37 Hz kick read as 41 Hz. So the hits are
    // ranked by what is left in the band late in each beat, when the kick
    // has died and only the rest of the mix is there, and only the cleanest
    // are measured -- the intro and outro a track made for DJs nearly always
    // has, where the kick plays alone.
    let residual = |on: usize| -> Option<f32> {
        let peak = body.get(on..on + samples(40.0))?.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let late = body.get(on + gap * 6 / 10..on + gap * 95 / 100)?;
        let rms = (late.iter().map(|v| v * v).sum::<f32>() / late.len().max(1) as f32).sqrt();
        (peak > 0.0).then(|| db(rms * std::f32::consts::SQRT_2) - db(peak))
    };
    let mut ranked: Vec<(usize, f32)> = starts.iter().filter_map(|&on| Some((on, residual(on)?))).collect();
    ranked.sort_by(|a, b| a.1.total_cmp(&b.1));
    let cleanest = ranked.first().map_or(0.0, |r| r.1);
    let keep = ranked.iter().filter(|r| r.1 <= cleanest + 6.0).count().max(8.min(ranked.len()));
    let under_db = {
        let mut v: Vec<f32> = ranked[..keep].iter().map(|r| r.1).collect();
        if v.is_empty() {
            -99.0
        } else {
            median(&mut v)
        }
    };
    let total_hits = starts.len();
    let starts: Vec<usize> = if keep >= 2 { ranked[..keep].iter().map(|r| r.0).collect() } else { starts };
    let average = |starts: &[usize]| {
        let mut sum = vec![0.0f64; len];
        let mut n = 0;
        for &on in starts {
            if on + len <= body.len() {
                for (s, v) in sum.iter_mut().zip(&body[on..on + len]) {
                    *s += *v as f64;
                }
                n += 1;
            }
        }
        (n > 0).then(|| sum.iter().map(|v| (v / n as f64) as f32).collect::<Vec<f32>>())
    };
    let first = average(&starts)?;
    let (reach, span) = (samples(2.0), samples(30.0).min(len));
    let aligned: Vec<usize> = starts
        .iter()
        .map(|&on| {
            let score = |at: usize| -> f32 {
                body.get(at..at + span).map_or(f32::MIN, |x| x.iter().zip(&first).map(|(a, b)| a * b).sum())
            };
            (on.saturating_sub(reach)..=on + reach).max_by(|&a, &b| score(a).total_cmp(&score(b))).unwrap_or(on)
        })
        .collect();
    let x = average(&aligned)?;

    let ms = |n: f32| n * 1000.0 / SAMPLE_RATE;
    // Envelope: the peak within 7.5 ms either side, every sample. Anything
    // shorter than half a cycle of the tail reads the zero crossings of a
    // 40 Hz sine as the kick dying.
    let half = samples(7.5);
    let env: Vec<f32> = (0..x.len())
        .map(|i| x[i.saturating_sub(half)..(i + half).min(x.len())].iter().fold(0.0f32, |m, v| m.max(v.abs())))
        .collect();
    let (tp, pv) = env.iter().copied().enumerate().fold((0, 0.0f32), |m, e| if e.1 > m.1 { e } else { m });
    if pv <= 0.0 {
        return None;
    }
    let fall = |ratio: f32| env[tp..].iter().position(|&v| v < pv * ratio).map(|i| ms(i as f32));
    let onset = x.iter().position(|v| v.abs() >= 0.1 * pv).unwrap_or(0);

    // Pitch from the zero crossings, both ways, so a half cycle is a
    // reading: while the body is within 18 dB of its peak, which is where
    // the kick still owns the band.
    let mut crossings = Vec::new();
    for i in onset + 1..x.len() {
        if env[i] < pv * 0.126 {
            break;
        }
        if (x[i - 1] < 0.0) != (x[i] < 0.0) {
            crossings.push((i - 1) as f32 + x[i - 1] / (x[i - 1] - x[i]));
        }
    }
    if crossings.len() < 4 {
        return None;
    }
    let cycles: Vec<(f32, f32)> = crossings
        .windows(2)
        .map(|w| (ms(0.5 * (w[0] + w[1]) - onset as f32), SAMPLE_RATE / (2.0 * (w[1] - w[0]))))
        .collect();
    let start_hz = cycles.iter().take_while(|c| c.0 <= 10.0).map(|c| c.1).fold(cycles[0].1, f32::max);
    // Where the sweep is going. A pitch envelope is an exponential towards
    // its end note -- `pitch_osc` and the `beats` kick are exactly that --
    // so three readings 12 ms apart give the note it is heading for, even
    // when the body has died before it got there: a slow sweep read off its
    // last cycles came out two semitones sharp. Only readings within 12 dB
    // of the peak are used, where the kick still owns its band; a sub bass
    // under it pulls anything later towards its own note, even averaged,
    // because a bass that starts with every kick is locked to it too.
    let level_at = |t_ms: f32| env.get(onset + samples(t_ms.max(0.0) as f64)).copied().unwrap_or(0.0);
    let limit = cycles.iter().rev().find(|c| level_at(c.0) >= pv * 0.251).map_or(cycles[cycles.len() - 1].0, |c| c.0);
    let f_at = |t: f32| -> Option<f32> {
        let k = cycles.windows(2).position(|w| w[0].0 <= t && t <= w[1].0)?;
        let (a, b) = (cycles[k], cycles[k + 1]);
        Some(a.1 + (b.1 - a.1) * (t - a.0) / (b.0 - a.0).max(1e-6))
    };
    let mut ends = Vec::new();
    let step = 12.0;
    let mut t = cycles[0].0;
    while t + 2.0 * step <= limit {
        if let (Some(f1), Some(f2), Some(f3)) = (f_at(t), f_at(t + step), f_at(t + 2.0 * step)) {
            let den = f1 + f3 - 2.0 * f2;
            if f1 > f2 && f2 > f3 && den > 0.002 * f2 {
                let end = (f1 * f3 - f2 * f2) / den;
                if end > 0.4 * f3 && end <= f3 {
                    ends.push(end);
                }
            }
        }
        t += 1.0;
    }
    // A sweep that has already landed gives no triple that still falls: then
    // the tail is what it reads once landed, in the same 12 dB.
    let tail_hz = if ends.len() >= 5 {
        median(&mut ends)
    } else {
        let mut v: Vec<f32> = cycles.iter().filter(|c| c.0 <= limit).map(|c| c.1).collect();
        let n = v.len();
        let mut late = v.split_off(n / 2);
        if late.is_empty() {
            cycles[cycles.len() - 1].1
        } else {
            median(&mut late)
        }
    };
    let sweep_ms = cycles.iter().find(|c| c.1 <= tail_hz * 1.1).map_or(cycles[cycles.len() - 1].0, |c| c.0);

    // The click is not averaged: hats land on some hits and not others, and
    // the median of the hits leaves them out.
    let mut click: Vec<f32> = aligned
        .iter()
        .filter_map(|&on| {
            let e_top: f32 = top.get(on..on + samples(10.0))?.iter().map(|v| v * v).sum();
            let e_body: f32 = body.get(on..on + samples(100.0))?.iter().map(|v| v * v).sum();
            (e_body > 0.0).then(|| 10.0 * (e_top.max(1e-12) / e_body).log10())
        })
        .collect();
    Some(Kick {
        hits: total_hits,
        measured: aligned.len(),
        under_db,
        start_hz,
        tail_hz,
        sweep_ms: sweep_ms.max(0.0),
        decay12_ms: fall(0.251),
        decay24_ms: fall(0.063),
        click_db: if click.is_empty() { -60.0 } else { median(&mut click) },
    })
}

fn median(v: &mut [f32]) -> f32 {
    v.sort_by(f32::total_cmp);
    let n = v.len();
    if n == 0 {
        0.0
    } else if n % 2 == 1 {
        v[n / 2]
    } else {
        0.5 * (v[n / 2 - 1] + v[n / 2])
    }
}

// ── Spectrum ──

fn hann(n: usize) -> Vec<f64> {
    (0..n).map(|i| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / n as f64).cos()).collect()
}

/// Power spectra of Hann frames, `size` long every `size / 4`, passed one
/// by one to `each`.
fn frames(mono: &[f32], size: usize, mut each: impl FnMut(&[f64])) {
    let w = hann(size);
    let mut re = vec![0.0f64; size];
    let mut im = vec![0.0f64; size];
    let mut power = vec![0.0f64; size / 2];
    let mut start = 0;
    while start + size <= mono.len() {
        for i in 0..size {
            re[i] = mono[start + i] as f64 * w[i];
            im[i] = 0.0;
        }
        fft(&mut re, &mut im);
        for k in 0..size / 2 {
            power[k] = re[k] * re[k] + im[k] * im[k];
        }
        each(&power);
        start += size / 4;
    }
}

/// Long-term energy per octave band, dB against all of them together.
fn octave_bands(mono: &[f32]) -> [f32; 10] {
    const SIZE: usize = 8192;
    let mut sum = [0.0f64; 10];
    let bin_hz = SR / SIZE as f64;
    frames(mono, SIZE, |p| {
        for (k, v) in p.iter().enumerate().skip(1) {
            let f = k as f64 * bin_hz;
            if let Some(b) = BAND_HZ.iter().position(|&c| f < c as f64 * std::f64::consts::SQRT_2) {
                if f >= BAND_HZ[b] as f64 / std::f64::consts::SQRT_2 {
                    sum[b] += v;
                }
            }
        }
    });
    let total: f64 = sum.iter().sum();
    let mut out = [-99.0f32; 10];
    if total > 0.0 {
        for (o, s) in out.iter_mut().zip(sum) {
            *o = (10.0 * (s.max(1e-30) / total).log10()).max(-99.0) as f32;
        }
    }
    out
}

/// Pitch classes from the spectral peaks of long frames: under 250 Hz for
/// the bass notes, 100 Hz to 2 kHz for the key. A peak more than a third of
/// a semitone from any note is a drum or noise and is left out.
fn notes(mono: &[f32]) -> ([f32; 12], Option<Key>) {
    const SIZE: usize = 16384;
    let bin_hz = SR / SIZE as f64;
    let (mut low, mut all) = ([0.0f64; 12], [0.0f64; 12]);
    frames(mono, SIZE, |p| {
        let lo = (40.0 / bin_hz) as usize;
        let hi = ((2000.0 / bin_hz) as usize).min(p.len() - 2);
        let loudest = p[lo..=hi].iter().copied().fold(0.0, f64::max);
        if loudest <= 0.0 {
            return;
        }
        for k in lo.max(1)..=hi {
            if p[k] < loudest * 0.0025 || p[k] < p[k - 1] || p[k] < p[k + 1] {
                continue;
            }
            // Parabolic interpolation on the log magnitude.
            let (a, b, c) = (p[k - 1].max(1e-30).ln(), p[k].ln(), p[k + 1].max(1e-30).ln());
            let den = a - 2.0 * b + c;
            let shift = if den.abs() > 1e-12 { (0.5 * (a - c) / den).clamp(-0.5, 0.5) } else { 0.0 };
            let f = (k as f64 + shift) * bin_hz;
            let midi = 69.0 + 12.0 * (f / 440.0).log2();
            if (midi - midi.round()).abs() > 0.33 {
                continue;
            }
            let pc = (midi.round() as i64).rem_euclid(12) as usize;
            let mag = p[k].sqrt();
            // The key leaves out everything under 100 Hz: a kick's tail is
            // the loudest steady pitch in most dance music, and a kick tuned
            // to D# read a song in F minor as D# major.
            if f >= 100.0 {
                all[pc] += mag;
            }
            if f < 250.0 {
                low[pc] += mag;
            }
        }
    });
    let share = |v: [f64; 12]| {
        let t: f64 = v.iter().sum();
        let mut out = [0.0f32; 12];
        if t > 0.0 {
            for (o, x) in out.iter_mut().zip(v) {
                *o = (x / t) as f32;
            }
        }
        out
    };
    (share(low), key(&all))
}

/// Krumhansl-Kessler key profiles.
const MAJOR: [f64; 12] = [6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88];
const MINOR: [f64; 12] = [6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17];

fn key(chroma: &[f64; 12]) -> Option<Key> {
    if chroma.iter().sum::<f64>() <= 0.0 {
        return None;
    }
    let corr = |profile: &[f64; 12], tonic: usize| {
        let x: Vec<f64> = (0..12).map(|i| chroma[(i + tonic) % 12]).collect();
        let (mx, my) = (x.iter().sum::<f64>() / 12.0, profile.iter().sum::<f64>() / 12.0);
        let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
        for i in 0..12 {
            let (dx, dy) = (x[i] - mx, profile[i] - my);
            sxy += dx * dy;
            sxx += dx * dx;
            syy += dy * dy;
        }
        if sxx > 0.0 && syy > 0.0 {
            sxy / (sxx * syy).sqrt()
        } else {
            0.0
        }
    };
    let mut best = Key { tonic: 0, minor: false, strength: f32::MIN };
    for tonic in 0..12 {
        for (minor, profile) in [(false, &MAJOR), (true, &MINOR)] {
            let c = corr(profile, tonic) as f32;
            if c > best.strength {
                best = Key { tonic, minor, strength: c };
            }
        }
    }
    Some(best)
}

// ── Reports ──

fn top_notes(share: &[f32; 12]) -> String {
    let mut idx: Vec<usize> = (0..12).collect();
    idx.sort_by(|&a, &b| share[b].total_cmp(&share[a]));
    let words: Vec<String> = idx
        .iter()
        .take(3)
        .filter(|&&i| share[i] >= 0.05)
        .map(|&i| format!("{} {:.0}%", NOTE_NAMES[i], share[i] * 100.0))
        .collect();
    if words.is_empty() {
        "-".into()
    } else {
        words.join(", ")
    }
}

fn band_label(hz: f32) -> String {
    if hz >= 1000.0 {
        format!("{}k", hz / 1000.0)
    } else {
        format!("{hz}")
    }
}

fn tempo_words(p: &Profile) -> String {
    match (p.tempo, p.tempo_span) {
        (None, _) => "not found".into(),
        (Some(t), None) => format!("{t:.1} BPM"),
        (Some(t), Some((lo, hi))) => format!("{t:.1} BPM ({lo:.0}..{hi:.0})"),
    }
}

fn opt_ms(v: Option<f32>) -> String {
    v.map_or("past the next hit".into(), |v| format!("{v:.0} ms"))
}

/// What `tatum analyze` prints.
pub fn describe(p: &Profile, name: &str) -> String {
    let mut o = String::new();
    let _ = writeln!(o, "{name}: {:.1} s", p.seconds);
    let _ = writeln!(o, "  tempo        {}", tempo_words(p));
    let _ = writeln!(
        o,
        "  key          {}",
        p.key.map_or("-".into(), |k| format!(
            "{}{}",
            k.with_relative(),
            if k.strength < 0.5 { ", weakly: little in it is tonal" } else { "" }
        ))
    );
    let _ = writeln!(o, "  low notes    {}   (under 250 Hz, the kick's tail included)", top_notes(&p.low_notes));
    let _ = writeln!(
        o,
        "  loudness     {}, peak {:.1} dBFS, peak to loudness {}",
        p.lufs.map_or("-".into(), |l| format!("{l:.1} LUFS")),
        p.peak_db,
        p.lufs.map_or("-".into(), |l| format!("{:.1} dB", p.peak_db - l)),
    );
    let _ =
        writeln!(o, "  width        {:.0}% above 250 Hz, {:.1}% under 150 Hz", p.width * 100.0, p.width_low * 100.0);
    match &p.kick {
        Some(k) => {
            let _ =
                writeln!(o, "  kick         {} hits, measured on the {} with the least under them", k.hits, k.measured);
            let _ = writeln!(o, "    hit        {:.0} Hz (the first half cycle)", k.start_hz);
            let _ = writeln!(o, "    tail       {:.1} Hz ({})", k.tail_hz, note_name(k.tail_hz));
            let _ = writeln!(o, "    sweep      {:.0} ms to within 10% of the tail", k.sweep_ms);
            let _ =
                writeln!(o, "    decay      -12 dB at {}, -24 dB at {}", opt_ms(k.decay12_ms), opt_ms(k.decay24_ms));
            let _ = writeln!(o, "    click      {:.1} dB (first 10 ms over 2 kHz against the body)", k.click_db);
            let _ = writeln!(o, "    under it   {:.1} dB (the band under 400 Hz late in the beat)", k.under_db);
            if k.masked() {
                let _ = writeln!(o, "    {MASKED}");
            }
        }
        None => {
            let _ = writeln!(o, "  kick         not found");
        }
    }
    let _ = write!(o, "  spectrum   ");
    for hz in BAND_HZ {
        let _ = write!(o, "{:>6}", band_label(hz));
    }
    let _ = write!(o, "\n  (dB)       ");
    for v in p.bands {
        let _ = write!(o, "{v:>6.1}");
    }
    let _ = writeln!(o);
    o
}

/// What `tatum compare` prints: the two side by side, then what differs by
/// enough to hear, each with what in a `.synth` file moves it.
pub fn compare(reference: &Profile, yours: &Profile, ref_name: &str, your_name: &str) -> String {
    let (r, y) = (reference, yours);
    let mut o = String::new();
    let mut todo: Vec<String> = Vec::new();
    let w = ref_name.len().clamp(9, 28);
    let _ = writeln!(o, "{:<16} {:<w$}  {}", "", trim(ref_name, w), your_name);
    let row = |o: &mut String, label: &str, a: String, b: String| {
        let _ = writeln!(o, "{label:<16} {a:<w$}  {b}");
    };

    row(&mut o, "tempo", tempo_words(r), tempo_words(y));
    if let (Some(a), Some(b)) = (r.tempo, y.tempo) {
        if (a - b).abs() > 0.5 {
            todo.push(format!("tempo: the reference is at {a:.1} BPM, yours at {b:.1}: `tempo {:.0}`", a.round()));
        }
    }

    let key = |k: Option<Key>| k.map_or("-".into(), |k| k.name());
    row(&mut o, "key", key(r.key), key(y.key));
    row(&mut o, "low notes", top_notes(&r.low_notes), top_notes(&y.low_notes));

    let lufs = |l: Option<f32>| l.map_or("-".into(), |l| format!("{l:.1} LUFS"));
    row(&mut o, "loudness", lufs(r.lufs), lufs(y.lufs));
    let plr = |p: &Profile| p.lufs.map(|l| p.peak_db - l);
    let fmt_plr = |v: Option<f32>| v.map_or("-".into(), |v| format!("{v:.1} dB"));
    row(&mut o, "peak to loudness", fmt_plr(plr(r)), fmt_plr(plr(y)));
    if let (Some(a), Some(b)) = (plr(r), plr(y)) {
        if b - a > 3.0 {
            todo.push(format!(
                "density: the reference's peaks sit {a:.1} dB over its loudness and yours {b:.1}. The engine matches loudness, \
                 not density: a denser mix comes from saturating or clipping the kick and the drums and compressing \
                 the drum bus, not from the master's level"
            ));
        }
    }
    row(&mut o, "width >250 Hz", format!("{:.0}%", r.width * 100.0), format!("{:.0}%", y.width * 100.0));
    row(&mut o, "width <150 Hz", format!("{:.1}%", r.width_low * 100.0), format!("{:.1}%", y.width_low * 100.0));
    if y.width_low > 0.05 && y.width_low > r.width_low * 2.0 {
        todo.push(format!(
            "the low end is {:.0}% wide against {:.0}%: keep everything under 150 Hz in the middle (`pan 0`, no chorus or stereo delay on the bass and kick)",
            y.width_low * 100.0,
            r.width_low * 100.0
        ));
    }
    if (r.width - y.width).abs() > 0.1 {
        todo.push(format!(
            "width above 250 Hz: {:.0}% against {:.0}%: {}",
            y.width * 100.0,
            r.width * 100.0,
            if y.width < r.width {
                "pan, `chorus_mix` and the delay and reverb sends spread it"
            } else {
                "less pan and chorus"
            }
        ));
    }

    match (&r.kick, &y.kick) {
        (Some(a), Some(b)) => {
            let _ = writeln!(o, "kick");
            row(
                &mut o,
                "  hits",
                format!("{} ({} measured)", a.hits, a.measured),
                format!("{} ({} measured)", b.hits, b.measured),
            );
            row(&mut o, "  hit", format!("{:.0} Hz", a.start_hz), format!("{:.0} Hz", b.start_hz));
            row(
                &mut o,
                "  tail",
                format!("{:.1} Hz {}", a.tail_hz, note_name(a.tail_hz)),
                format!("{:.1} Hz {}", b.tail_hz, note_name(b.tail_hz)),
            );
            row(&mut o, "  sweep", format!("{:.0} ms", a.sweep_ms), format!("{:.0} ms", b.sweep_ms));
            row(&mut o, "  decay -12 dB", opt_ms(a.decay12_ms), opt_ms(b.decay12_ms));
            row(&mut o, "  decay -24 dB", opt_ms(a.decay24_ms), opt_ms(b.decay24_ms));
            row(&mut o, "  click", format!("{:.1} dB", a.click_db), format!("{:.1} dB", b.click_db));
            row(&mut o, "  under it", format!("{:.1} dB", a.under_db), format!("{:.1} dB", b.under_db));
            if a.masked() || b.masked() {
                let which = if a.masked() { "the reference" } else { "yours" };
                todo.push(format!("kick: in {which}, {MASKED}; its tail, sweep and decay are not compared"));
            }
            kick_advice(a, b, &mut todo);
        }
        (None, _) => {
            let _ = writeln!(o, "kick             not found in the reference: give it a stretch where the kick plays");
        }
        (_, None) => {
            let _ = writeln!(o, "kick             not found in yours");
        }
    }

    let _ = write!(o, "spectrum (dB)  ");
    for hz in BAND_HZ {
        let _ = write!(o, "{:>6}", band_label(hz));
    }
    let _ = writeln!(o);
    for (label, p) in [("  reference", r), ("  yours", y)] {
        let _ = write!(o, "{label:<15}");
        for v in p.bands {
            let _ = write!(o, "{v:>6.1}");
        }
        let _ = writeln!(o);
    }
    let _ = write!(o, "{:<15}", "  difference");
    let mut off = Vec::new();
    for (i, hz) in BAND_HZ.iter().enumerate() {
        let d = y.bands[i] - r.bands[i];
        let _ = write!(o, "{d:>+6.1}");
        if d.abs() >= 3.0 && r.bands[i] > -45.0 {
            off.push((*hz, d));
        }
    }
    let _ = writeln!(o);
    // The four furthest off, furthest first: past that it is a list.
    off.sort_by(|a, b| b.1.abs().total_cmp(&a.1.abs()));
    off.truncate(4);
    for (hz, d) in off {
        let what = match hz as u32 {
            0..=40 => "the sub: the kick's tail and the bass's lowest octave",
            41..=90 => "the weight: kick body and bass fundamental",
            91..=180 => "the bass's upper body and the kick's punch",
            181..=360 => "the mud band: pads, chords and the bass's harmonics pile up here",
            361..=1400 => "the middle: chords, stabs, the body of the leads",
            1401..=5000 => "presence: where the ear is most sensitive, clicks and snares",
            _ => "air: hats, cymbals, the top of the reverb",
        };
        todo.push(format!(
            "{:+.1} dB at {}, {what}: {}; `tatum debug` says which tracks sit there",
            d,
            if hz >= 1000.0 { format!("{} kHz", hz / 1000.0) } else { format!("{hz} Hz") },
            if d > 0.0 { "take it down there" } else { "bring it up there" }
        ));
    }

    if todo.is_empty() {
        let _ = writeln!(o, "\nnothing differs by enough to hear in these measures");
    } else {
        let _ = writeln!(o, "\nwhat differs:");
        for t in todo {
            let _ = writeln!(o, "  - {t}");
        }
    }
    o
}

fn trim(s: &str, w: usize) -> String {
    if s.chars().count() <= w {
        s.to_string()
    } else {
        let tail: String = s.chars().rev().take(w - 1).collect::<Vec<_>>().into_iter().rev().collect();
        format!("…{tail}")
    }
}

/// What to change on the kick, in the words of a `pitch_osc` + `perc` kick
/// and of the `beats` kick.
fn kick_advice(r: &Kick, y: &Kick, todo: &mut Vec<String>) {
    if r.masked() || y.masked() {
        click_advice(r, y, todo);
        return;
    }
    let st = 12.0 * (y.tail_hz / r.tail_hz).log2();
    if st.abs() >= 0.75 {
        todo.push(format!(
            "kick tail: yours settles {:.1} semitones {} ({:.1} Hz against {:.1} Hz, {}): `pitch_osc`'s end frequency to {:.0}, or `kick_pitch` on `beats`",
            st.abs(),
            if st > 0.0 { "higher" } else { "lower" },
            y.tail_hz,
            r.tail_hz,
            note_name(r.tail_hz),
            r.tail_hz
        ));
    }
    if (y.start_hz / r.start_hz - 1.0).abs() > 0.2 {
        todo.push(format!(
            "kick hit: yours starts at {:.0} Hz against {:.0} Hz: `pitch_osc`'s start frequency",
            y.start_hz, r.start_hz
        ));
    }
    // The two below are given as a factor on what the file already says,
    // not as a value: an exponential's time scales with its constant, and
    // that holds whatever else is in the voice. A sweep's time scales with
    // 1 / (1 - decay), so the distance from 1 is what gets multiplied.
    let ratio = y.sweep_ms / r.sweep_ms.max(0.1);
    if !(0.7..=1.4).contains(&ratio) && r.start_hz > r.tail_hz * 1.2 {
        todo.push(format!(
            "kick sweep: yours takes {:.0} ms to land against {:.0} ms: a {} drop -- multiply how far `pitch_osc`'s decay is from 1 by {:.2} (0.995 becomes {:.4})",
            y.sweep_ms,
            r.sweep_ms,
            if ratio > 1.0 { "faster" } else { "slower" },
            ratio,
            1.0 - 0.005 * ratio
        ));
    }
    if let (Some(a), Some(b)) = (r.decay12_ms, y.decay12_ms) {
        let ratio = a / b.max(1.0);
        if !(0.75..=1.33).contains(&ratio) {
            todo.push(format!(
                "kick length: yours is 12 dB down at {b:.0} ms against {a:.0} ms: {} -- multiply the body's `perc` decay by {ratio:.2}; on `beats`, {} `kick_decay`",
                if ratio < 1.0 { "shorter" } else { "longer" },
                if ratio < 1.0 { "less" } else { "more" }
            ));
        }
    }
    click_advice(r, y, todo);
}

fn click_advice(r: &Kick, y: &Kick, todo: &mut Vec<String>) {
    let d = y.click_db - r.click_db;
    if d.abs() >= 4.0 {
        todo.push(format!(
            "kick click: {:+.1} dB against the reference: {} the noise layer of the hit, or `kick_click` on `beats`",
            d,
            if d > 0.0 { "less of" } else { "more of" }
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A kick like the engine's `pitch_osc` + `perc`, four to the bar.
    fn kicks(bpm: f32, seconds: f32, start: f32, end: f32, decay: f32, tau: f32) -> Vec<f32> {
        let n = (seconds * SAMPLE_RATE) as usize;
        let beat = (60.0 / bpm * SAMPLE_RATE) as usize;
        let mut out = vec![0.0f32; n];
        let mut at = 0;
        while at < n {
            let (mut f, mut phase) = (start, 0.0f32);
            for i in 0..beat.min(n - at) {
                let env = (-(i as f32) / (tau * SAMPLE_RATE)).exp();
                out[at + i] += 0.8 * env * (2.0 * std::f32::consts::PI * phase).sin();
                phase = (phase + f / SAMPLE_RATE).fract();
                f = end + (f - end) * decay;
            }
            at += beat;
        }
        out
    }

    #[test]
    fn a_kick_is_measured_where_it_was_made() {
        let x = kicks(128.0, 16.0, 200.0, 49.0, 0.9993, 0.12);
        let p = measure(&x, &x);
        let t = p.tempo.expect("tempo");
        assert!((t - 128.0).abs() < 0.3, "tempo {t}");
        let k = p.kick.expect("kick");
        assert!(k.hits >= 30, "hits {}", k.hits);
        assert!((12.0 * (k.tail_hz / 49.0).log2()).abs() < 0.5, "tail {}", k.tail_hz);
        assert!(k.start_hz > 120.0, "start {}", k.start_hz);
        // -12 dB at 1.386 time constants.
        let d = k.decay12_ms.expect("decay");
        assert!((d - 166.0).abs() < 25.0, "decay {d}");
        assert!(p.width_low < 0.001);
    }

    #[test]
    fn a_shorter_kick_is_called_shorter_and_given_a_number() {
        let a = measure(&kicks(126.0, 12.0, 220.0, 52.0, 0.9993, 0.08), &kicks(126.0, 12.0, 220.0, 52.0, 0.9993, 0.08));
        let b = measure(&kicks(126.0, 12.0, 220.0, 62.0, 0.9993, 0.2), &kicks(126.0, 12.0, 220.0, 62.0, 0.9993, 0.2));
        let report = compare(&a, &b, "ref.wav", "mine.synth");
        assert!(report.contains("kick tail: yours settles"), "{report}");
        assert!(report.contains("kick length"), "{report}");
        assert!(report.contains("shorter"), "{report}");
    }

    #[test]
    fn the_tempo_survives_another_sample_rate() {
        let x = kicks(124.0, 12.0, 180.0, 55.0, 0.9993, 0.1);
        // Rendered at 44.1 kHz, then taken to 48 kHz and back the way a file would be.
        let up: Vec<f32> = {
            let ratio = 44100.0 / 48000.0;
            (0..(x.len() as f64 / ratio) as usize)
                .map(|j| {
                    let t = j as f64 * ratio;
                    let i = t as usize;
                    let f = (t - i as f64) as f32;
                    x[i] * (1.0 - f) + x.get(i + 1).copied().unwrap_or(0.0) * f
                })
                .collect()
        };
        let audio = Audio { left: up.clone(), right: up, rate: 48000 };
        let p = analyze(&audio, None, None).unwrap();
        assert!((p.tempo.unwrap() - 124.0).abs() < 0.3, "{:?}", p.tempo);
        assert!((12.0 * (p.kick.unwrap().tail_hz / 55.0).log2()).abs() < 0.5);
    }

    #[test]
    fn a_rate_no_converter_records_is_refused() {
        for rate in [1, 4000, 1_000_000] {
            let audio = Audio { left: vec![0.0; 64], right: vec![0.0; 64], rate };
            let e = analyze(&audio, None, None).err().unwrap();
            assert!(e.contains("sample rate"), "{rate}: {e}");
        }
    }

    #[test]
    fn a_stretch_is_cut_before_it_is_converted() {
        // Ten seconds at 48 kHz, silent but for a click at 5 s.
        let mut x = vec![0.0f32; 480_000];
        x[240_000] = 1.0;
        let audio = Audio { left: x.clone(), right: x, rate: 48000 };
        // Longer than allowed whole, so only the stretch can be converted.
        let e = stretch(&audio, None, None, 4.0).err().unwrap();
        assert!(e.contains("--from"), "{e}");
        let (l, r) = stretch(&audio, Some(4.0), Some(6.0), 4.0).unwrap();
        assert_eq!(l.len(), r.len());
        assert!((l.len() as i64 - 88_200).abs() <= 1, "{}", l.len());
        // The click is a second into the stretch.
        let at = (0..l.len()).max_by(|&a, &b| l[a].abs().total_cmp(&l[b].abs())).unwrap();
        assert!((at as i64 - 44_100).abs() <= 2, "{at}");
    }

    #[test]
    fn a_tone_lands_in_its_band_and_its_note() {
        let n = (4.0 * SAMPLE_RATE) as usize;
        let x: Vec<f32> =
            (0..n).map(|i| 0.5 * (2.0 * std::f32::consts::PI * 110.0 * i as f32 / SAMPLE_RATE).sin()).collect();
        let bands = octave_bands(&x);
        let loudest = (0..10).max_by(|&a, &b| bands[a].total_cmp(&bands[b])).unwrap();
        assert_eq!(BAND_HZ[loudest], 125.0);
        let (low, _) = notes(&x);
        assert!(low[9] > 0.9, "A: {:?}", low);
        assert_eq!(note_name(49.0), "G1");
        assert_eq!(note_name(55.0), "A1");
    }

    #[test]
    fn resampling_keeps_a_tone_where_it_was() {
        let n = 48000;
        let x: Vec<f32> = (0..n).map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 48000.0).sin()).collect();
        let y = to_engine_rate(&x, 48000);
        assert!((y.len() as i64 - 44100).abs() <= 1);
        // Upward zero crossings in the middle second: a thousand of them.
        let crossings = y[1000..43100].windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
        assert!((crossings as i64 - 955).abs() <= 1, "{crossings}");
        let peak = y[1000..43100].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!((peak - 1.0).abs() < 0.01, "{peak}");
    }
}
