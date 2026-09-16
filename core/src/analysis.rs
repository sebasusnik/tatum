//! Mix analysis shared by the engine's per-track meters and the render report.
//!
//! One implementation, one calibration test: the band split used to live only in
//! the MCP server, where a one-pole rolloff leaked midrange into the "high"
//! bucket and made a dark mix read as bright. Two tuning passes went the wrong
//! way before the test tones in `band_tests` caught it.

use crate::math;

/// Corner frequencies between the five bands.
///
/// The split at 60 Hz exists because a listener made it before the report
/// could: a mix can be too heavy on speakers and thin on headphones at the same
/// time, which is 60-250 Hz too loud and under 60 Hz too quiet. One "low"
/// bucket cannot say that.
pub const BAND_EDGES_HZ: [f32; 4] = [60.0, 250.0, 2000.0, 5000.0];

/// sub (<60 Hz, felt more than heard, and absent on most speakers), low
/// (60-250 Hz, the weight and also the mud), mid (250 Hz-2 kHz), harsh
/// (2-5 kHz, where the ear is most sensitive) and air (>5 kHz).
pub const BAND_NAMES: [&str; 5] = ["sub", "low", "mid", "harsh", "air"];

const POLES: usize = 3;

/// Cascading N one-poles drags the overall -3 dB point down; scaling each
/// section's corner up by 1/sqrt(2^(1/N) - 1) puts the stated edge back where
/// the name says it is.
const CASCADE_FIX: f32 = 1.9615;

/// Splits a mono signal into four bands and accumulates energy in each.
#[derive(Clone)]
pub struct BandMeter {
    coeffs: [f32; 4],
    states: [[f32; POLES]; 4],
    energy: [f64; 5],
}

impl BandMeter {
    pub fn new(sample_rate: f32) -> Self {
        let coeff = |hz: f32| math::exp(-2.0 * math::PI * hz * CASCADE_FIX / sample_rate);
        Self {
            coeffs: [coeff(BAND_EDGES_HZ[0]), coeff(BAND_EDGES_HZ[1]), coeff(BAND_EDGES_HZ[2]), coeff(BAND_EDGES_HZ[3])],
            states: [[0.0; POLES]; 4],
            energy: [0.0; 5],
        }
    }

    /// Feed one mono sample.
    pub fn push(&mut self, x: f32) {
        let mut below = [0.0f32; 4];
        for (edge, b) in below.iter_mut().enumerate() {
            let c = self.coeffs[edge];
            let mut v = x;
            for s in self.states[edge].iter_mut() {
                *s = v * (1.0 - c) + *s * c;
                v = *s;
            }
            *b = v;
        }
        let bands = [
            below[0],
            below[1] - below[0],
            below[2] - below[1],
            below[3] - below[2],
            x - below[3],
        ];
        for (acc, b) in self.energy.iter_mut().zip(bands) {
            *acc += (b * b) as f64;
        }
    }

    /// Feed one stereo sample as its mono sum.
    pub fn push_stereo(&mut self, l: f32, r: f32) {
        self.push((l + r) * 0.5);
    }

    pub fn energy(&self) -> [f64; 5] {
        self.energy
    }

    /// Share of total energy per band, in percent, rounded to one decimal.
    /// All zeros when nothing has been fed in.
    pub fn percentages(&self) -> [f32; 5] {
        let total: f64 = self.energy.iter().sum();
        if total <= 0.0 {
            return [0.0; 5];
        }
        let mut out = [0.0f32; 5];
        for (o, e) in out.iter_mut().zip(self.energy) {
            *o = math::floor((e / total * 1000.0) as f32 + 0.5) / 10.0;
        }
        out
    }

    /// The band holding the most energy, or `None` if there is no signal.
    pub fn dominant(&self) -> Option<usize> {
        let total: f64 = self.energy.iter().sum();
        if total <= 0.0 {
            return None;
        }
        let mut best = 0;
        for i in 1..5 {
            if self.energy[i] > self.energy[best] {
                best = i;
            }
        }
        Some(best)
    }

    pub fn reset(&mut self) {
        self.states = [[0.0; POLES]; 4];
        self.energy = [0.0; 5];
    }
}

/// Peak and RMS of a stereo buffer.
pub fn peak_rms(l: &[f32], r: &[f32]) -> (f32, f32) {
    let n = l.len().max(1);
    let mut sum = 0.0f64;
    let mut peak = 0.0f32;
    for (a, b) in l.iter().zip(r) {
        sum += (a * a + b * b) as f64;
        peak = peak.max(math::abs(*a)).max(math::abs(*b));
    }
    (peak, math::sqrt((sum / (2 * n) as f64) as f32))
}

/// Share of a stereo signal's energy that is not shared between the channels,
/// as a fraction. 0 is mono, 0.5 is a hard-panned single source, 1 is fully
/// out of phase. Instruments that sit in the same place cannot be told apart,
/// and nothing in this report looked at it until twenty-one of the twenty-five
/// songs in the corpus measured under 2%.
pub fn stereo_width(l: &[f32], r: &[f32]) -> f32 {
    let (mut mid, mut side) = (0.0f64, 0.0f64);
    for (a, b) in l.iter().zip(r) {
        let m = (a + b) as f64;
        let s = (a - b) as f64;
        mid += m * m;
        side += s * s;
    }
    let total = mid + side;
    if total <= 0.0 { 0.0 } else { (side / total) as f32 }
}

/// Stereo width of everything above `hz`. The plain figure is dominated by the
/// low end, which in most genres is mono on purpose -- a techno mix with a
/// hard-panned pad still reads 1% because the kick and the sub carry the energy.
/// Measured above 250 Hz it answers the question anyone actually has: can I
/// tell the instruments apart.
pub fn stereo_width_above(l: &[f32], r: &[f32], hz: f32, sample_rate: f32) -> f32 {
    const POLES: usize = 3;
    let c = math::exp(-2.0 * math::PI * hz * CASCADE_FIX / sample_rate);
    let (mut sl, mut sr) = ([0.0f32; POLES], [0.0f32; POLES]);
    let (mut mid, mut side) = (0.0f64, 0.0f64);
    for (a, b) in l.iter().zip(r) {
        let mut low_l = *a;
        for s in sl.iter_mut() { *s = low_l * (1.0 - c) + *s * c; low_l = *s; }
        let mut low_r = *b;
        for s in sr.iter_mut() { *s = low_r * (1.0 - c) + *s * c; low_r = *s; }
        let (hl, hr) = (a - low_l, b - low_r);
        let m = (hl + hr) as f64;
        let d = (hl - hr) as f64;
        mid += m * m;
        side += d * d;
    }
    let total = mid + side;
    if total <= 0.0 { 0.0 } else { (side / total) as f32 }
}

/// Crest factor: peak over RMS. Around 4-6 for a punchy mix; a limiter that is
/// working hard pulls it towards 2-3 and takes the transients with it.
pub fn crest(peak: f32, rms: f32) -> f32 {
    if rms <= 1e-9 { 0.0 } else { peak / rms }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    fn tone(hz: f32, secs: f32, sample_rate: f32) -> BandMeter {
        let mut m = BandMeter::new(sample_rate);
        for i in 0..(sample_rate * secs) as usize {
            m.push(math::sin(i as f32 / sample_rate * hz * math::TWO_PI) * 0.5);
        }
        m
    }

    /// A pure tone must land in the band its frequency belongs to. Without the
    /// cascade correction a 1 kHz tone leaked most of its energy into "air",
    /// and the report told me to brighten an already harsh mix.
    #[test]
    fn each_band_catches_its_own_tone() {
        const SR: f32 = 44100.0;
        for (hz, idx) in [(35.0, 0usize), (120.0, 1), (1000.0, 2), (3000.0, 3), (9000.0, 4)] {
            let m = tone(hz, 1.0, SR);
            let pct = m.percentages();
            assert_eq!(
                m.dominant(), Some(idx),
                "a {} Hz tone should read mostly '{}', got {:?}", hz, BAND_NAMES[idx], pct
            );
            // The bands overlap (three poles is still gentle, and 2-5 kHz is
            // barely more than an octave wide), so the bar is that the right
            // band dominates by a clear margin, not that it takes everything.
            assert!(pct[idx] > 50.0, "{} Hz: only {}% in '{}'", hz, pct[idx], BAND_NAMES[idx]);
        }
    }

    #[test]
    fn silence_reports_nothing_rather_than_guessing() {
        let m = BandMeter::new(44100.0);
        assert_eq!(m.percentages(), [0.0; 5]);
        assert_eq!(m.dominant(), None);
    }

    #[test]
    fn width_is_zero_for_mono_and_a_half_for_one_hard_panned_source() {
        let x: Vec<f32> = (0..1000).map(|i| (i as f32 * 0.01).sin()).collect();
        let zero = vec![0.0f32; 1000];
        assert!(stereo_width(&x, &x) < 1e-6, "identical channels are mono");
        assert!((stereo_width(&x, &zero) - 0.5).abs() < 1e-5, "one channel only is half");
        let inv: Vec<f32> = x.iter().map(|v| -v).collect();
        assert!(stereo_width(&x, &inv) > 0.999, "out of phase is all side");
    }

    #[test]
    fn crest_of_a_sine_is_the_square_root_of_two() {
        const SR: f32 = 44100.0;
        let sine: Vec<f32> = (0..4410)
            .map(|i| math::sin(i as f32 / SR * 100.0 * math::TWO_PI))
            .collect();
        let (p, r) = peak_rms(&sine, &sine);
        assert!((crest(p, r) - core::f32::consts::SQRT_2).abs() < 0.02, "got {}", crest(p, r));
    }
}
