//! `tatum events`: what each event of a sound is like inside, and how it
//! sits in its mix. `hit` reads a kick on its beat grid; this reads events
//! that are on no grid -- bubbles, pings, drops, struck notes -- over
//! whatever bed of pad and reverb runs under them.
//!
//! An event is found where the spectrum in a band jumps over its own running
//! floor, and followed along its strongest partial until that partial is back
//! in the floor. Each one is then demodulated by its own partial -- the audio
//! multiplied by a tone that follows the partial, and low-passed -- which
//! leaves its level and its pitch sample by sample, however far it glides,
//! with only a few semitones either side of it let in. From that come the
//! attack, the decay, the bend, the width of the resonance, where it sits
//! between the speakers and what is left after it; from the spectrogram
//! with the floor taken off, how much of the event is its partial and how
//! much is anything else.
//!
//! `sustained` measures material with no onsets (pads, held notes), `clean`
//! looks for what should not be in a render at all, `report` prints one file
//! or two side by side, and `draw` puts the events of two files on one sheet.

use tatum_core::SAMPLE_RATE;

use crate::fft::fft;
use crate::wav::Audio;

pub mod clean;
pub mod draw;
pub mod inside;
pub mod report;
pub mod sustained;

pub(crate) const SR: f32 = SAMPLE_RATE;

/// The longest stretch read at once: the spectrogram is kept whole.
pub const MAX_SECONDS: f32 = 120.0;

#[derive(Clone, Debug)]
pub struct Options {
    /// The band events are looked for in, Hz. `None`: where the onsets are.
    pub band: Option<(f32, f32)>,
    /// Two onsets closer than this are one event.
    pub min_gap_ms: f32,
    /// How far over the floor of its own bin an event has to stand, dB. The
    /// floor is the bin's 10th percentile, 10 dB under the mean of a hiss,
    /// whose own peaks reach 18 dB over it all the time and 24 almost never.
    pub threshold_db: f32,
    /// Measure the sustained material too, even when events are found.
    pub sustained: bool,
    /// Rows of the per-event table.
    pub first: usize,
}

impl Default for Options {
    fn default() -> Options {
        Options { band: None, min_gap_ms: 30.0, threshold_db: 24.0, sustained: false, first: 12 }
    }
}

/// Power per frame and bin, Hann windowed; 1.0 is a full-scale sine. Frame
/// `t` is centred on sample `t * hop`.
pub struct Stft {
    pub size: usize,
    pub hop: usize,
    pub bins: usize,
    power: Vec<f32>,
}

impl Stft {
    pub fn new(x: &[f32], size: usize, hop: usize) -> Stft {
        Stft::padded(x, size, size, hop)
    }

    /// A window of `window` samples in an FFT of `size`: the bins are
    /// interpolated, the resolution is the window's.
    pub fn padded(x: &[f32], window: usize, size: usize, hop: usize) -> Stft {
        let bins = size / 2 + 1;
        let frames = x.len() / hop + 1;
        let hann: Vec<f64> =
            (0..window).map(|i| 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / window as f64).cos()).collect();
        let norm = (window as f64 / 4.0).powi(2);
        let mut power = Vec::with_capacity(frames * bins);
        let (mut re, mut im) = (vec![0.0f64; size], vec![0.0f64; size]);
        for t in 0..frames {
            let centre = (t * hop) as isize;
            re.fill(0.0);
            im.fill(0.0);
            for (i, w) in hann.iter().enumerate() {
                let at = centre + i as isize - (window / 2) as isize;
                if at >= 0 && (at as usize) < x.len() {
                    re[i] = x[at as usize] as f64 * w;
                }
            }
            fft(&mut re, &mut im);
            power.extend((0..bins).map(|b| ((re[b] * re[b] + im[b] * im[b]) / norm) as f32));
        }
        Stft { size, hop, bins, power }
    }

    pub fn frames(&self) -> usize {
        self.power.len() / self.bins
    }

    pub fn row(&self, t: usize) -> &[f32] {
        &self.power[t * self.bins..(t + 1) * self.bins]
    }

    pub fn at(&self, t: usize, b: usize) -> f32 {
        self.power[t * self.bins + b]
    }

    pub fn bin_hz(&self) -> f32 {
        SR / self.size as f32
    }

    pub fn frame_s(&self) -> f32 {
        self.hop as f32 / SR
    }

    /// The bin a frequency falls in, kept inside the spectrum.
    pub fn bin(&self, hz: f32) -> usize {
        ((hz / self.bin_hz()).round() as usize).clamp(1, self.bins - 1)
    }
}

/// The level each bin sits at when nothing is happening in it: the 10th
/// percentile of its power over three seconds, so a pad's partial is floor
/// and a ping over it is not, and a bin a tom rings in most of the time
/// still has a floor under the tom. Never under -100 dB, so silence divides.
pub struct Floor {
    step: usize,
    bins: usize,
    rows: Vec<f32>,
}

impl Floor {
    pub fn new(s: &Stft) -> Floor {
        let frames = s.frames();
        let half = ((1.5 / s.frame_s()) as usize).max(4);
        let step = ((0.1 / s.frame_s()) as usize).max(1);
        let stride = (2 * half / 96).max(1);
        let mut rows = Vec::with_capacity((frames / step + 1) * s.bins);
        let mut scratch: Vec<f32> = Vec::new();
        for k in 0..frames.div_ceil(step) {
            let t = k * step;
            let (a, b) = (t.saturating_sub(half), (t + half).min(frames));
            for bin in 0..s.bins {
                scratch.clear();
                scratch.extend((a..b).step_by(stride).map(|u| s.at(u, bin)));
                let at = scratch.len() / 10;
                let (_, v, _) = scratch.select_nth_unstable_by(at, f32::total_cmp);
                rows.push(v.max(1e-10));
            }
            // A tenth percentile read from a hundred frames is itself a
            // draw, and in a hiss one bin in a few hundred draws low enough
            // for its next peak to pass for an event. No bin's floor is
            // under the middle one of the nine around it.
            let row = &mut rows[k * s.bins..];
            let raw = row.to_vec();
            let mut near = [0.0f32; 9];
            for (bin, v) in row.iter_mut().enumerate() {
                let (a, b) = (bin.saturating_sub(4), (bin + 5).min(raw.len()));
                near[..b - a].copy_from_slice(&raw[a..b]);
                near[..b - a].sort_by(f32::total_cmp);
                *v = v.max(near[(b - a) / 2]);
            }
        }
        Floor { step, bins: s.bins, rows }
    }

    pub fn at(&self, t: usize, b: usize) -> f32 {
        let k = (t / self.step).min(self.rows.len() / self.bins - 1);
        self.rows[k * self.bins + b]
    }
}

/// Where an event's strongest partial is, frame by frame.
#[derive(Clone, Debug)]
pub struct Ridge {
    /// The frame the ridge starts on.
    pub frame: usize,
    /// Hz, one per frame.
    pub hz: Vec<f32>,
    /// Power, one per frame.
    pub power: Vec<f32>,
}

impl Ridge {
    /// The partial's frequency at sample `n`, held before and after.
    pub fn hz_at(&self, n: f64, hop: usize) -> f32 {
        let f = n / hop as f64 - self.frame as f64;
        if f <= 0.0 {
            return self.hz[0];
        }
        let i = f as usize;
        if i + 1 >= self.hz.len() {
            return self.hz[self.hz.len() - 1];
        }
        let u = (f - i as f64) as f32;
        self.hz[i] * (1.0 - u) + self.hz[i + 1] * u
    }
}

#[derive(Clone, Debug)]
pub struct Event {
    /// Seconds from the start of the stretch.
    pub start_s: f32,
    pub end_s: f32,
    /// Its partial at its loudest, over the floor of its bin, dB.
    pub over_floor_db: f32,
    pub ridge: Ridge,
    pub inside: inside::Inside,
}

/// Echoes found after the events.
#[derive(Clone, Debug)]
pub struct Delay {
    pub interval_ms: f32,
    /// The first repeat against the event, dB.
    pub first_db: f32,
    /// Each later repeat against the one before, dB.
    pub per_repeat_db: Option<f32>,
    /// Events with a repeat found after them, of those looked at.
    pub confirmed: usize,
    pub of: usize,
    /// Why the interval cannot be told from the notes' own spacing, when it cannot.
    pub ambiguous: Option<&'static str>,
}

pub struct Report {
    pub seconds: f32,
    pub band: (f32, f32),
    pub band_given: bool,
    pub events: Vec<Event>,
    pub delay: Option<Delay>,
    pub sustained: Option<sustained::Sustained>,
    pub clean: clean::Clean,
    /// Said before the figures when the file is read another way than usual.
    pub note: Option<&'static str>,
    /// Left and right at the engine's rate: the pictures are drawn from them.
    pub left: Vec<f32>,
    pub right: Vec<f32>,
}

impl Report {
    pub fn mono(&self) -> Vec<f32> {
        self.left.iter().zip(&self.right).map(|(a, b)| 0.5 * (a + b)).collect()
    }
}

/// Measure a WAV at any rate; `from`/`to` trim it, in seconds.
pub fn analyze(audio: &Audio, from: Option<f32>, to: Option<f32>, opts: &Options) -> Result<Report, String> {
    let (l, r) = crate::reference::stretch(audio, from, to, MAX_SECONDS)?;
    if l.len() < (0.2 * SR) as usize {
        return Err("less than a fifth of a second of audio".into());
    }
    if let Some((lo, hi)) = opts.band {
        if !(lo >= 15.0 && hi > lo * 1.05 && hi <= SR / 2.0) {
            return Err(format!("--band takes two frequencies in Hz, low then high, from 15 up to {:.0}", SR / 2.0));
        }
    }
    // A file converted from another rate has been through a filter that
    // takes off what would fold: its aliasing is not measured.
    Ok(measure(l, r, opts, audio.rate == SR as u32))
}

/// Measure audio already at the engine's rate.
pub fn measure(left: Vec<f32>, right: Vec<f32>, opts: &Options, native: bool) -> Report {
    let mono: Vec<f32> = left.iter().zip(&right).map(|(a, b)| 0.5 * (a + b)).collect();
    // Channels that cancel when summed are read one at a time: the sum of
    // an antiphase file is silence, and so would be its report.
    let energy = |x: &[f32]| x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>();
    let (el, er, em) = (energy(&left), energy(&right), energy(&mono));
    let (mono, note, sign) = if em < 0.25 * 0.5 * (el + er) {
        let side: Vec<f32> = left.iter().zip(&right).map(|(a, b)| 0.5 * (a - b)).collect();
        (side, Some("left and right cancel when summed (they are in antiphase): read on their difference"), -1.0)
    } else {
        (mono, None, 1.0)
    };
    let found = find(&mono, opts);
    let band = found.band;
    let ctx = inside::Ctx { left: &left, right: &right, sign, mono: &mono, stft: &found.stft, floor: &found.floor };
    let hop = found.stft.hop;
    // Read every ridge against every other's onset; then again against the
    // onsets that turned out to be events, so a rise that is no event (two
    // stages of one decay beating) does not cut the event it sits in short.
    let read_all = |keep: &[bool]| -> Vec<Event> {
        let onsets: Vec<(usize, f32)> = found
            .ridges
            .iter()
            .zip(keep)
            .filter(|(_, k)| **k)
            .map(|((r, _), _)| (r.frame * hop, r.hz[0]))
            .collect();
        let mut events: Vec<Event> =
            found.ridges.iter().filter_map(|(ridge, over)| inside::read(&ctx, ridge, *over, &onsets)).collect();
        events.sort_by(|a, b| a.start_s.total_cmp(&b.start_s));
        // Two ridges that turn out to start within the gap are one event: the
        // louder is kept.
        let gap = opts.min_gap_ms * 0.001;
        let mut kept: Vec<Event> = Vec::new();
        for e in events {
            match kept.last_mut() {
                Some(last) if e.start_s - last.start_s < gap => {
                    if e.over_floor_db > last.over_floor_db {
                        *last = e;
                    }
                }
                _ => kept.push(e),
            }
        }
        kept
    };
    let first = read_all(&vec![true; found.ridges.len()]);
    let survived: Vec<bool> =
        found.ridges.iter().map(|(r, _)| first.iter().any(|e| e.ridge.frame == r.frame)).collect();
    let kept = if survived.iter().all(|k| *k) { first } else { read_all(&survived) };
    let mut events = kept;
    inside::tails(&mut events);
    let delay = inside::delay(&ctx, &events, band);
    let sustained = (opts.sustained || events.len() < 3).then(|| sustained::measure(&left, &right)).flatten();
    let clean = clean::measure(&ctx, &events, sustained.as_ref(), native);
    Report {
        seconds: mono.len() as f32 / SR,
        band,
        band_given: opts.band.is_some(),
        events,
        delay,
        sustained,
        clean,
        note,
        left,
        right,
    }
}

/// The window an event in a band from `lo` Hz is read with: eight cycles of
/// the lowest note fit, and a ping at 1 kHz is still placed to 3 ms.
fn window_for(lo: f32) -> usize {
    ((8.0 * SR / lo) as usize).next_power_of_two().clamp(1024, 8192)
}

struct Found {
    band: (f32, f32),
    stft: Stft,
    floor: Floor,
    /// Every event's ridge and its level over the floor, dB.
    ridges: Vec<(Ridge, f32)>,
}

fn find(mono: &[f32], opts: &Options) -> Found {
    let top = SR * 0.45;
    let band = opts.band.unwrap_or_else(|| {
        let s = Stft::new(mono, 1024, 128);
        let floor = Floor::new(&s);
        found_band(&s, &floor, opts.threshold_db).unwrap_or((100.0, top.min(8000.0)))
    });
    let band = (band.0.max(15.0), band.1.min(SR / 2.0));
    let size = window_for(band.0);
    let hop = size / 8;
    let s = Stft::new(mono, size, hop);
    let floor = Floor::new(&s);
    let (b0, b1) = (s.bin(band.0), s.bin(band.1));
    let frames = s.frames();
    let lag = ((0.008 / s.frame_s()).round() as usize).max(1);
    // How much each frame gains over the one `lag` before it, counting only
    // what stands the threshold over the floor: a bed that hisses gains
    // nothing, and a ping in one bin is not lost in the flutter of the
    // hundreds beside it.
    // An onset is looked for from 6 dB under the threshold, so one that
    // only just makes it still has a rise to find.
    let ratio = 10f32.powf(opts.threshold_db / 10.0);
    let stand = 0.5 * ratio.sqrt();
    let over = |t: usize, b: usize| (s.at(t, b).sqrt() - stand * floor.at(t, b).sqrt()).max(0.0);
    // Before the file there is nothing: an event in its first frames gains
    // against silence.
    let mut flux = vec![0.0f32; frames];
    for (t, v) in flux.iter_mut().enumerate() {
        *v = (b0..=b1).map(|b| (over(t, b) - if t >= lag { over(t - lag, b) } else { 0.0 }).max(0.0)).sum();
    }
    let gap = ((opts.min_gap_ms * 0.001 / s.frame_s()).ceil() as usize).max(1);
    let peaks: Vec<usize> = (0..frames)
        .filter(|&t| {
            flux[t] > 0.0
                && (t.saturating_sub(gap)..(t + gap + 1).min(frames))
                    .all(|u| flux[u] < flux[t] || (flux[u] == flux[t] && u >= t))
        })
        .collect();
    // An onset has to stand over what the bed's own flutter gains, and be
    // within 26 dB of the strong ones.
    let mut sorted = flux.clone();
    sorted.sort_by(f32::total_cmp);
    let median = sorted[sorted.len() / 2];
    let mut dev: Vec<f32> = flux.iter().map(|v| (v - median).abs()).collect();
    dev.sort_by(f32::total_cmp);
    let mad = dev[dev.len() / 2];
    let mut heights: Vec<f32> = peaks.iter().map(|&t| flux[t]).collect();
    heights.sort_by(f32::total_cmp);
    let strong = heights.get(heights.len() * 9 / 10).copied().unwrap_or(0.0);
    let need = (median + 3.0 * mad).max(0.05 * strong);

    let mut ridges: Vec<(Ridge, f32)> = Vec::new();
    let mut starts: Vec<(usize, usize)> = Vec::new();
    for &t in peaks.iter().filter(|&&t| flux[t] >= need) {
        let was = |u: usize, x: usize| if u >= lag { s.at(u - lag, x) } else { 0.0 };
        let Some(b) = (b0..=b1).max_by(|&x, &y| (s.at(t, x) - was(t, x)).total_cmp(&(s.at(t, y) - was(t, y)))) else {
            continue;
        };
        let ridge = follow(&s, &floor, t, b);
        // The onset's own peak: up to where the ridge has fallen 6 dB from
        // the most it reached, or 300 ms. A flutter of a bed that a ridge
        // follows on into a later, louder sound is not that sound's onset.
        let reach = ((0.3 / s.frame_s()) as usize).max(2);
        let (mut peak_i, mut peak) = (0usize, 0.0f32);
        for (i, &p) in ridge.power.iter().enumerate().take(reach) {
            if p > peak {
                (peak_i, peak) = (i, p);
            } else if p < 0.25 * peak {
                break;
            }
        }
        let peak_bin = s.bin(ridge.hz[peak_i]);
        let before = if t > lag {
            (b.saturating_sub(1)..=(b + 1).min(s.bins - 1)).map(|x| s.at(t - lag - 1, x)).fold(0.0f32, f32::max)
        } else {
            0.0
        };
        if peak < ratio * floor.at(t + peak_i, peak_bin) || peak < 4.0 * before || ridge.hz.len() < 2 {
            continue;
        }
        let db = 10.0 * (peak / floor.at(t + peak_i, peak_bin)).log10();
        starts.push((t, b));
        ridges.push((ridge, db.min(99.0)));
    }
    // A later onset on a ridge's own partial is one of two things. If the
    // partial had fallen 6 dB from its peak by then, it was struck again:
    // the ridge ends there. If it had not, nothing started: a partial with
    // a vibrato, or gliding, keeps arriving in new bins, and each arrival
    // gains like an onset. Those are not events.
    let mut gone = vec![false; ridges.len()];
    for i in 0..ridges.len() {
        if gone[i] {
            continue;
        }
        let first = ridges[i].0.frame;
        for j in i + 1..ridges.len() {
            let (t, b) = starts[j];
            if t >= first + ridges[i].0.hz.len() {
                break;
            }
            if gone[j] || t <= first || s.bin(ridges[i].0.hz[t - first]).abs_diff(b) > 3 {
                continue;
            }
            let at = t - first;
            let so_far = ridges[i].0.power[..=at].iter().fold(0.0f32, |m, p| m.max(*p));
            if ridges[i].0.power[at.saturating_sub(lag + 1)] >= 0.25 * so_far {
                gone[j] = true;
            } else {
                let keep = at.max(2);
                ridges[i].0.hz.truncate(keep);
                ridges[i].0.power.truncate(keep);
                break;
            }
        }
    }
    let mut k = 0;
    ridges.retain(|_| {
        k += 1;
        !gone[k - 1]
    });
    Found { band, stft: s, floor, ridges }
}

/// The band nine tenths of the onsets' energy is in, a sixth of an octave
/// wider each side. `None` when nothing rises.
fn found_band(s: &Stft, floor: &Floor, threshold_db: f32) -> Option<(f32, f32)> {
    let stand = 0.5 * 10f32.powf(threshold_db / 20.0);
    let lag = ((0.008 / s.frame_s()).round() as usize).max(1);
    let lowest = s.bin(40.0);
    let highest = s.bin(SR * 0.45);
    let mut gain = vec![0.0f64; s.bins];
    for t in lag..s.frames() {
        for (b, g) in gain.iter_mut().enumerate().take(highest + 1).skip(lowest) {
            let over = |t: usize| (s.at(t, b).sqrt() - stand * floor.at(t, b).sqrt()).max(0.0);
            *g += (over(t) - over(t - lag)).max(0.0) as f64;
        }
    }
    let total: f64 = gain.iter().sum();
    if total <= 0.0 {
        return None;
    }
    let mut sum = 0.0;
    let (mut lo, mut hi) = (None, None);
    for (b, g) in gain.iter().enumerate() {
        sum += g;
        if lo.is_none() && sum >= 0.05 * total {
            lo = Some(b);
        }
        if hi.is_none() && sum >= 0.95 * total {
            hi = Some(b);
        }
    }
    let widen = 2f32.powf(1.0 / 6.0);
    let (lo, hi) = (lo? as f32 * s.bin_hz() / widen, (hi? as f32 + 1.0) * s.bin_hz() * widen);
    // At least an octave, so a single ping has room to bend.
    let (lo, hi) = if hi < 2.0 * lo { ((lo * hi).sqrt() / 1.42, (lo * hi).sqrt() * 1.42) } else { (lo, hi) };
    Some((lo.max(30.0), hi.min(SR * 0.45)))
}

/// Follow the strongest partial from frame `t`, bin `b`, until it is back in
/// the floor or 40 dB under its own peak.
fn follow(s: &Stft, floor: &Floor, t: usize, b: usize) -> Ridge {
    let longest = (12.0 / s.frame_s()) as usize;
    let mut ridge = Ridge { frame: t, hz: Vec::new(), power: Vec::new() };
    let (mut bin, mut peak, mut missed, mut good) = (b, 0.0f32, 0usize, 0usize);
    for u in t..s.frames().min(t + longest) {
        let (a, z) = (bin.saturating_sub(2).max(1), (bin + 2).min(s.bins - 2));
        bin = (a..=z).max_by(|&x, &y| s.at(u, x).total_cmp(&s.at(u, y))).unwrap_or(bin);
        let p = s.at(u, bin);
        peak = peak.max(p);
        // The top of a Hann peak in log power is a parabola through three bins.
        let (l, c, r) = (s.at(u, bin - 1).max(1e-20).ln(), p.max(1e-20).ln(), s.at(u, bin + 1).max(1e-20).ln());
        let d = if 2.0 * c - l - r > 1e-9 { (0.5 * (l - r) / (l - 2.0 * c + r)).clamp(-0.5, 0.5) } else { 0.0 };
        ridge.hz.push((bin as f32 + d) * s.bin_hz());
        ridge.power.push(p);
        if p < 1.6 * floor.at(u, bin) || p < 1e-4 * peak {
            missed += 1;
            if missed >= 3 {
                break;
            }
        } else {
            missed = 0;
            good = ridge.hz.len();
        }
    }
    ridge.hz.truncate(good.max(1));
    ridge.power.truncate(good.max(1));
    ridge
}

/// The middle, and the quarter either side of it, of what was measured.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spread {
    pub median: f32,
    pub low: f32,
    pub high: f32,
    pub n: usize,
}

pub fn spread(values: impl Iterator<Item = f32>) -> Option<Spread> {
    let mut v: Vec<f32> = values.filter(|v| v.is_finite()).collect();
    if v.is_empty() {
        return None;
    }
    v.sort_by(f32::total_cmp);
    let at = |q: f32| {
        let x = q * (v.len() - 1) as f32;
        let i = x as usize;
        let u = x - i as f32;
        v[i] * (1.0 - u) + v[(i + 1).min(v.len() - 1)] * u
    };
    Some(Spread { median: at(0.5), low: at(0.25), high: at(0.75), n: v.len() })
}

pub(crate) fn db(power: f32) -> f32 {
    10.0 * power.max(1e-20).log10()
}

#[cfg(test)]
mod tests;
