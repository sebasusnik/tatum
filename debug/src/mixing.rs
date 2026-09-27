//! What the report says about the mix rather than about noise: which parts
//! are in each other's way, where, and when.
//!
//! `render` already warns when two tracks share a band, but it averages the
//! whole song: a bass and a sub that fight through every drop and never meet
//! in the intro read the same as two that brush past each other once. Here
//! the question is asked per moment -- one spectrogram column at a time --
//! and the answer is kept per section, which is where a mix gets fixed.

use crate::spectrogram::{cell_db, hz_row, Spectrogram, ROWS};


/// The ranges a mix is talked about in, low to high.
pub const BANDS: [(f32, f32, &str); 8] = [
    (20.0, 60.0, "20-60 Hz"),
    (60.0, 150.0, "60-150 Hz"),
    (150.0, 400.0, "150-400 Hz"),
    (400.0, 1000.0, "400 Hz-1 kHz"),
    (1000.0, 2500.0, "1-2.5 kHz"),
    (2500.0, 6000.0, "2.5-6 kHz"),
    (6000.0, 12000.0, "6-12 kHz"),
    (12000.0, 20000.0, "12-20 kHz"),
];

/// Below this a part is not there as far as the mix is concerned.
const PRESENT_DB: f32 = -60.0;
/// Two parts this close in one band cannot be told apart by level: neither
/// sits behind the other.
const CLOSE_DB: f32 = 6.0;
/// And both have to be near the loudest thing in that band at that moment; a
/// pair fighting 20 dB under the bass is not what anyone hears.
const LEADING_DB: f32 = 10.0;
/// A band this far under a part's own loudest band at that moment is not
/// where the part is: it is the edge of the part, or the analysis window's
/// own spill from the band next door.
const OWN_DB: f32 = 30.0;
/// A clash is reported for a section where it holds this share of the time.
const MOST_OF_THE_TIME: f32 = 0.3;

/// Per column, each band's level in dB: the power average of its rows.
pub fn band_levels(spec: &Spectrogram) -> Vec<[f32; 8]> {
    let rows: Vec<(usize, usize)> = BANDS.iter()
        .map(|&(lo, hi, _)| (hz_row(lo).max(0.0) as usize, (hz_row(hi) as usize).min(ROWS)))
        .collect();
    (0..spec.columns()).map(|c| {
        let mut out = [0.0f32; 8];
        for (b, &(r0, r1)) in rows.iter().enumerate() {
            let p: f32 = (r0..r1).map(|r| 10f32.powf(cell_db(spec.cell(c, r)) / 10.0)).sum::<f32>()
                / (r1 - r0).max(1) as f32;
            out[b] = 10.0 * p.max(1e-20).log10();
        }
        out
    }).collect()
}

/// Two parts level with each other and on top of one band, for a good part
/// of one or more sections.
pub struct Clash {
    pub a: usize,
    pub b: usize,
    pub band: usize,
    /// (section index, share of its time).
    pub sections: Vec<(usize, f32)>,
}

/// `levels[part][column]`, `sections` as column ranges.
pub fn clashes(levels: &[Vec<[f32; 8]>], sections: &[(usize, usize)]) -> Vec<Clash> {
    let n = levels.len();
    let mut out: Vec<Clash> = Vec::new();
    for band in 0..BANDS.len() {
        for a in 0..n {
            for b in a + 1..n {
                let mut found = Vec::new();
                for (si, &(c0, c1)) in sections.iter().enumerate() {
                    if c1 <= c0 { continue }
                    let together = (c0..c1).filter(|&c| {
                        (levels[a][c][band] - levels[b][c][band]).abs() <= CLOSE_DB
                            && on_top(levels, a, c, band) && on_top(levels, b, c, band)
                    }).count();
                    let share = together as f32 / (c1 - c0) as f32;
                    if share >= MOST_OF_THE_TIME { found.push((si, share)) }
                }
                if !found.is_empty() {
                    out.push(Clash { a, b, band, sections: found });
                }
            }
        }
    }
    // Worst first: the one that holds for the most time over the song.
    out.sort_by(|x, y| weight(y).total_cmp(&weight(x)));
    out
}

fn weight(c: &Clash) -> f32 { c.sections.iter().map(|(_, s)| s).sum() }

/// Whether a part counts as on top of a band at one column: there, near the
/// loudest thing in the band, and not just its own edge. The same test the
/// clashes use.
fn on_top(levels: &[Vec<[f32; 8]>], part: usize, c: usize, band: usize) -> bool {
    let l = levels[part][c][band];
    let own = levels[part][c].iter().copied().fold(f32::MIN, f32::max);
    let top = levels.iter().map(|p| p[c][band]).fold(f32::MIN, f32::max);
    l >= PRESENT_DB && l >= own - OWN_DB && l > top - LEADING_DB
}

/// For each column and band, how many parts are on top of it. One is a clear
/// mix; three or more in one range at once is mud. Smoothed over a few
/// columns, the most any of them saw, so a moment shows as a block rather
/// than as a speckle.
pub fn crowding(levels: &[Vec<[f32; 8]>], columns: usize) -> Vec<[u8; 8]> {
    let raw: Vec<[u8; 8]> = (0..columns).map(|c| {
        let mut out = [0u8; 8];
        for (band, n) in out.iter_mut().enumerate() {
            *n = (0..levels.len()).filter(|&p| on_top(levels, p, c, band)).count().min(255) as u8;
        }
        out
    }).collect();
    const SPREAD: usize = 2;
    (0..columns).map(|c| {
        let mut out = [0u8; 8];
        for (band, n) in out.iter_mut().enumerate() {
            *n = raw[c.saturating_sub(SPREAD)..(c + SPREAD + 1).min(columns)].iter()
                .map(|r| r[band]).max().unwrap_or(0);
        }
        out
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: f32, gain: f32) -> Spectrogram {
        let mut s = Spectrogram::new(512);
        for i in 0..44100 {
            s.push(gain * (2.0 * std::f32::consts::PI * hz * i as f32 / 44100.0).sin());
        }
        s.finish(80);
        s
    }

    #[test]
    fn two_parts_level_in_one_band_clash_and_one_far_under_does_not() {
        let (bass, sub, quiet, high) = (tone(90.0, 0.5), tone(110.0, 0.4), tone(100.0, 0.01), tone(3000.0, 0.5));
        let levels: Vec<_> = [&bass, &sub, &quiet, &high].iter().map(|s| band_levels(s)).collect();
        let found = clashes(&levels, &[(10, 70)]);
        assert_eq!(found.len(), 1, "{:?}", found.iter().map(|c| (c.a, c.b, c.band)).collect::<Vec<_>>());
        assert_eq!((found[0].a, found[0].b, found[0].band), (0, 1, 1));
        let crowd = crowding(&levels, 80);
        assert_eq!(crowd[40][1], 2, "bass and sub on top of 60-150 Hz");
        assert_eq!(crowd[40][5], 1, "the high tone alone at 2.5-6 kHz");
    }
}
