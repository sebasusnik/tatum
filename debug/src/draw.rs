//! The pictures: every part stacked on one sheet, and each part on its own.
//!
//! Both share the same time axis -- one pixel column per spectrogram column
//! -- with a line at every bar (or every few, when bars get too narrow to
//! tell apart) and a brighter one where the section changes, so "the hiss in
//! the pad starts at the drop" can be read off the sheet without counting.

use crate::picture::{heat, text_width, Canvas, Rgb, DIM_TEXT, GLYPH_HEIGHT, TEXT};
use crate::spectrogram::{hz_row, Spectrogram, ROWS};

/// Where the bars and sections fall, in spectrogram columns.
pub struct Ruler {
    /// (column, bar number as the song counts it, from 1).
    pub bars: Vec<(usize, u32)>,
    /// (column, section name).
    pub sections: Vec<(usize, String)>,
}

/// What to mark on a part's strip.
#[derive(Default)]
pub struct Marks {
    /// Columns of clicks.
    pub clicks: Vec<usize>,
    /// Column ranges where the part should have been quiet and was not.
    pub floors: Vec<(usize, usize)>,
}

pub struct Strip<'a> {
    pub name: &'a str,
    pub note: String,
    pub spec: &'a Spectrogram,
    pub marks: Marks,
}

const LABEL_W: usize = 170;
const RULER_H: usize = 44;
const STRIP_H: usize = 72;
const GAP: usize = 4;
const CLICK: Rgb = [80, 230, 255];
const FLOOR: Rgb = [255, 230, 60];
const GUIDE: Rgb = [255, 255, 255];

/// Every strip stacked, sharing the ruler at the top. `crowd`, when given, is
/// drawn as one more strip just above the last one (the mix): how many parts
/// sit on top of each other at each moment and frequency.
pub fn sheet(strips: &[Strip], crowd: Option<&[[u8; 8]]>, ruler: &Ruler, columns: usize) -> Canvas {
    let rows = strips.len() + crowd.is_some() as usize;
    let height = RULER_H + rows * (STRIP_H + GAP) + 26;
    let mut c = Canvas::new(LABEL_W + columns + 8, height);
    draw_ruler(&mut c, ruler, columns, height);
    let mut y = RULER_H;
    for (i, s) in strips.iter().enumerate() {
        if i + 1 == strips.len() {
            if let Some(counts) = crowd {
                draw_crowd(&mut c, counts, y, STRIP_H, columns);
                c.text(8, y + 6, "crowded", TEXT, 2);
                c.text(8, y + 6 + GLYPH_HEIGHT * 2 + 6, "parts on top of", DIM_TEXT, 1);
                c.text(8, y + 6 + GLYPH_HEIGHT * 2 + 16, "each other: 2 3 4+", DIM_TEXT, 1);
                let key_x = 8 + crate::picture::text_width("each other: ", 1);
                for (k, n) in [2u8, 3, 4].iter().enumerate() {
                    c.fill(key_x + k * 12, y + 6 + GLYPH_HEIGHT * 2 + 26, 7, 4, crowd_colour(*n));
                }
                y += STRIP_H + GAP;
            }
        }
        draw_strip(&mut c, s, y, STRIP_H, columns);
        c.text(8, y + 6, s.name, TEXT, 2);
        c.text(8, y + 6 + GLYPH_HEIGHT * 2 + 6, &s.note, DIM_TEXT, 1);
        y += STRIP_H + GAP;
    }
    for (x, _) in &ruler.sections {
        c.vline(LABEL_W + x, RULER_H, height - RULER_H - 26, GUIDE, 0.55);
    }
    let foot = height - 18;
    let mut x = c.text(8, foot, "faint lines: 100 hz, 1 khz, 10 khz", DIM_TEXT, 1) + 24;
    c.fill(x, foot, 10, 7, CLICK);
    x = c.text(x + 16, foot, "click", DIM_TEXT, 1) + 24;
    c.fill(x, foot + 2, 10, 3, FLOOR);
    c.text(x + 16, foot, "does not go quiet between notes", DIM_TEXT, 1);
    c
}

/// One part, tall enough to read frequencies off.
pub fn part(strip: &Strip, ruler: &Ruler, columns: usize) -> Canvas {
    let h = ROWS * 2;
    let height = RULER_H + h + 8;
    let mut c = Canvas::new(LABEL_W + columns + 8, height);
    draw_ruler(&mut c, ruler, columns, height);
    draw_strip(&mut c, strip, RULER_H, h, columns);
    c.text(8, RULER_H + 4, strip.name, TEXT, 2);
    c.text(8, RULER_H + 4 + GLYPH_HEIGHT * 2 + 6, &strip.note, DIM_TEXT, 1);
    for (hz, label) in [
        (50.0, "50"),
        (100.0, "100"),
        (200.0, "200"),
        (500.0, "500"),
        (1000.0, "1k"),
        (2000.0, "2k"),
        (5000.0, "5k"),
        (10000.0, "10k"),
    ] {
        let y = RULER_H + h - 1 - (hz_row(hz) / ROWS as f32 * h as f32) as usize;
        c.hline(LABEL_W, y, columns, GUIDE, 0.18);
        let label_x = LABEL_W - 8 - text_width(label, 1);
        c.text(label_x, y - GLYPH_HEIGHT / 2, label, DIM_TEXT, 1);
    }
    for (x, _) in &ruler.sections {
        c.vline(LABEL_W + x, RULER_H, h, GUIDE, 0.55);
    }
    c
}

fn draw_ruler(c: &mut Canvas, ruler: &Ruler, columns: usize, height: usize) {
    c.text(8, 6, "section", DIM_TEXT, 1);
    c.text(8, 24, "bar", DIM_TEXT, 1);
    let mut free_from = 0;
    for (x, name) in &ruler.sections {
        if *x >= free_from {
            free_from = c.text(LABEL_W + x + 3, 4, name, TEXT, 1) - LABEL_W + 6;
        }
    }
    // A line per bar, unless that packs them closer than 10 pixels; then every
    // second, fourth... bar, always on a power of two so they land on phrases.
    let per_bar = columns as f32 / ruler.bars.len().max(1) as f32;
    let mut every = 1;
    while per_bar * (every as f32) < 10.0 {
        every *= 2
    }
    let mut label_from = 0;
    for (x, bar) in &ruler.bars {
        if (bar - 1) % every != 0 {
            continue;
        }
        c.vline(LABEL_W + x, 20, height - 20, GUIDE, 0.12);
        let label = bar.to_string();
        if *x >= label_from {
            label_from = c.text(LABEL_W + x + 2, 24, &label, DIM_TEXT, 1) - LABEL_W + 8;
        }
    }
}

fn draw_strip(c: &mut Canvas, s: &Strip, y0: usize, h: usize, columns: usize) {
    let x0 = LABEL_W;
    for x in 0..columns.min(s.spec.columns()) {
        for y in 0..h {
            // Pixel row y covers these spectrogram rows; the loudest wins, so
            // a thin tone survives being shrunk.
            let from_bottom = h - 1 - y;
            let r0 = from_bottom * ROWS / h;
            let r1 = ((from_bottom + 1) * ROWS / h).max(r0 + 1);
            let v = (r0..r1).map(|r| s.spec.cell(x, r)).max().unwrap_or(0);
            c.set(x0 + x, y0 + y, heat(v as f32 / 255.0));
        }
    }
    if h <= STRIP_H {
        for hz in [100.0, 1000.0, 10000.0] {
            let y = y0 + h - 1 - (hz_row(hz) / ROWS as f32 * h as f32) as usize;
            c.hline(x0, y, columns, GUIDE, 0.10);
        }
    }
    for &(a, b) in &s.marks.floors {
        let w = (b.saturating_sub(a)).max(3);
        c.fill(x0 + a, y0 + h - 3, w, 3, FLOOR);
    }
    for &x in &s.marks.clicks {
        c.fill(x0 + x.saturating_sub(1), y0, 3, 7, CLICK);
    }
}

/// One moment up close: the wave itself over 80 ms, where a click is a visible
/// step, and the spectrum over half a second, where it is a vertical line
/// through every frequency. The moment is in the middle of both.
pub fn zoom(title: &str, wave: &[f32], spec: &Spectrogram, width: usize, spec_columns: usize) -> Canvas {
    const TITLE_H: usize = 34;
    const WAVE_H: usize = 220;
    let spec_h = ROWS * 2;
    let height = TITLE_H + WAVE_H + GAP * 3 + spec_h + 8;
    let mut c = Canvas::new(LABEL_W + width + 8, height);
    c.text(8, 8, title, TEXT, 2);

    // The wave, scaled to fit: what matters is its shape, not its level.
    let y0 = TITLE_H;
    let peak = wave.iter().fold(0.0f32, |a, v| a.max(v.abs())).max(1e-9);
    let mid = y0 + WAVE_H / 2;
    c.hline(LABEL_W, mid, width, GUIDE, 0.15);
    for x in 0..width {
        let a = x * wave.len() / width;
        let b = ((x + 1) * wave.len() / width).max(a + 1).min(wave.len());
        if a >= wave.len() {
            break;
        }
        let (lo, hi) = wave[a..b].iter().fold((f32::MAX, f32::MIN), |(l, h), &v| (l.min(v), h.max(v)));
        let to_y = |v: f32| (mid as f32 - v / peak * (WAVE_H as f32 / 2.0 - 4.0)) as usize;
        let (top, bottom) = (to_y(hi), to_y(lo));
        c.fill(LABEL_W + x, top, 1, bottom.saturating_sub(top) + 1, [120, 220, 200]);
    }
    c.vline(LABEL_W + width / 2, y0, WAVE_H, CLICK, 0.35);
    c.text(8, y0 + 6, "wave", TEXT, 2);
    c.text(8, y0 + 30, "80 ms, scaled", DIM_TEXT, 1);
    c.text(8, y0 + 42, &format!("peak {:.0} db", 20.0 * peak.log10()), DIM_TEXT, 1);

    let sy = y0 + WAVE_H + GAP * 3;
    let strip = Strip { name: "", note: String::new(), spec, marks: Marks::default() };
    draw_strip(&mut c, &strip, sy, spec_h, spec_columns);
    c.vline(LABEL_W + spec_columns / 2, sy, spec_h, CLICK, 0.35);
    c.text(8, sy + 6, "spectrum", TEXT, 2);
    c.text(8, sy + 30, "half a second", DIM_TEXT, 1);
    for (hz, label) in [(50.0, "50"), (200.0, "200"), (1000.0, "1k"), (5000.0, "5k"), (10000.0, "10k")] {
        let y = sy + spec_h - 1 - (hz_row(hz) / ROWS as f32 * spec_h as f32) as usize;
        c.hline(LABEL_W, y, spec_columns, GUIDE, 0.18);
        c.text(LABEL_W - 8 - text_width(label, 1), y - GLYPH_HEIGHT / 2, label, DIM_TEXT, 1);
    }
    c
}

/// One part alone is a clear mix and draws as nothing; two, three, four or
/// more on top of each other go from amber to red.
fn crowd_colour(n: u8) -> Rgb {
    match n {
        0 | 1 => [8, 8, 12],
        2 => [150, 110, 20],
        3 => [235, 120, 30],
        _ => [235, 35, 45],
    }
}

fn draw_crowd(c: &mut Canvas, counts: &[[u8; 8]], y0: usize, h: usize, columns: usize) {
    use crate::mixing::BANDS;
    use crate::spectrogram::row_hz;
    for y in 0..h {
        // The band this pixel row falls in, on the same scale as the strips.
        let hz = row_hz((h - 1 - y) as f32 / h as f32 * ROWS as f32);
        let Some(band) = BANDS.iter().position(|&(lo, hi, _)| hz >= lo && hz < hi) else { continue };
        for (x, n) in counts.iter().take(columns).enumerate() {
            c.set(LABEL_W + x, y0 + y, crowd_colour(n[band]));
        }
    }
    for &(lo, _, _) in BANDS.iter().skip(1) {
        let y = y0 + h - 1 - (hz_row(lo) / ROWS as f32 * h as f32) as usize;
        c.hline(LABEL_W, y, columns, GUIDE, 0.08);
    }
}
