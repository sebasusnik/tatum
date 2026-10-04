//! A canvas of RGB pixels, a colour scale for loudness, a small bitmap font
//! for labels, and PNG out. Enough to draw a spectrogram with its names on it
//! and nothing else.

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

pub type Rgb = [u8; 3];

pub const BACKGROUND: Rgb = [12, 12, 16];
pub const TEXT: Rgb = [220, 220, 225];
pub const DIM_TEXT: Rgb = [130, 130, 140];

pub struct Canvas {
    pub width: usize,
    pub height: usize,
    pixels: Vec<u8>,
}

impl Canvas {
    pub fn new(width: usize, height: usize) -> Canvas {
        let mut c = Canvas { width, height, pixels: vec![0; width * height * 3] };
        c.fill(0, 0, width, height, BACKGROUND);
        c
    }

    pub fn set(&mut self, x: usize, y: usize, c: Rgb) {
        if x < self.width && y < self.height {
            let i = (y * self.width + x) * 3;
            self.pixels[i..i + 3].copy_from_slice(&c);
        }
    }

    /// Mix `c` over what is there, `alpha` of the way. For guide lines that
    /// should be seen without hiding the spectrum under them.
    pub fn blend(&mut self, x: usize, y: usize, c: Rgb, alpha: f32) {
        if x < self.width && y < self.height {
            let i = (y * self.width + x) * 3;
            for (px, &target) in self.pixels[i..i + 3].iter_mut().zip(&c) {
                let old = *px as f32;
                *px = (old + (target as f32 - old) * alpha) as u8;
            }
        }
    }

    pub fn fill(&mut self, x: usize, y: usize, w: usize, h: usize, c: Rgb) {
        for yy in y..(y + h).min(self.height) {
            for xx in x..(x + w).min(self.width) {
                self.set(xx, yy, c);
            }
        }
    }

    pub fn vline(&mut self, x: usize, y: usize, h: usize, c: Rgb, alpha: f32) {
        for yy in y..(y + h).min(self.height) {
            self.blend(x, yy, c, alpha);
        }
    }

    pub fn hline(&mut self, x: usize, y: usize, w: usize, c: Rgb, alpha: f32) {
        for xx in x..(x + w).min(self.width) {
            self.blend(xx, y, c, alpha);
        }
    }

    /// Write `text` with its top-left corner at (x, y), each font pixel drawn
    /// as a `scale` x `scale` square. Returns the x just past the last glyph.
    pub fn text(&mut self, x: usize, y: usize, text: &str, c: Rgb, scale: usize) -> usize {
        let mut cx = x;
        for ch in text.chars() {
            let rows = glyph(ch);
            for (ry, row) in rows.iter().enumerate() {
                for (rx, b) in row.bytes().enumerate() {
                    if b == b'#' {
                        self.fill(cx + rx * scale, y + ry * scale, scale, scale, c);
                    }
                }
            }
            cx += GLYPH_ADVANCE * scale;
        }
        cx
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let file = File::create(path).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        let mut enc = png::Encoder::new(BufWriter::new(file), self.width as u32, self.height as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().map_err(|e| format!("{}: {e}", path.display()))?;
        w.write_image_data(&self.pixels).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// Width of a glyph plus the gap after it, in font pixels.
pub const GLYPH_ADVANCE: usize = 6;
pub const GLYPH_HEIGHT: usize = 7;

pub fn text_width(text: &str, scale: usize) -> usize {
    text.chars().count() * GLYPH_ADVANCE * scale
}

/// Loudness to colour: black for silence through purple and red to a pale
/// yellow at full scale. `t` is 0..1. A scale that only gets brighter reads
/// right on any screen and in greyscale, and quiet noise shows as a dim purple
/// haze instead of vanishing into the black.
pub fn heat(t: f32) -> Rgb {
    const STOPS: [(f32, Rgb); 6] = [
        (0.00, [0, 0, 4]),
        (0.25, [40, 12, 90]),
        (0.50, [140, 30, 110]),
        (0.70, [225, 70, 70]),
        (0.87, [250, 160, 50]),
        (1.00, [252, 250, 190]),
    ];
    let t = t.clamp(0.0, 1.0);
    for w in STOPS.windows(2) {
        let ((a, ca), (b, cb)) = (w[0], w[1]);
        if t <= b {
            let f = (t - a) / (b - a);
            return [
                (ca[0] as f32 + (cb[0] as f32 - ca[0] as f32) * f) as u8,
                (ca[1] as f32 + (cb[1] as f32 - ca[1] as f32) * f) as u8,
                (ca[2] as f32 + (cb[2] as f32 - ca[2] as f32) * f) as u8,
            ];
        }
    }
    STOPS[STOPS.len() - 1].1
}

/// 5x7 glyphs for what labels need: lower-case letters, digits, punctuation
/// and the arrows the live screen's keys are named with. Upper case is drawn
/// as lower case and an accented letter as its plain one; anything else as a
/// blank.
fn glyph(c: char) -> [&'static str; 7] {
    let c = match c.to_lowercase().next().unwrap_or(c) {
        'á' | 'à' | 'ä' | 'â' => 'a',
        'é' | 'è' | 'ë' | 'ê' => 'e',
        'í' | 'ì' | 'ï' | 'î' => 'i',
        'ó' | 'ò' | 'ö' | 'ô' => 'o',
        'ú' | 'ù' | 'ü' | 'û' => 'u',
        'ñ' => 'n',
        other => other,
    };
    match c {
        'a' => [".....", ".....", ".###.", "....#", ".####", "#...#", ".####"],
        'b' => ["#....", "#....", "####.", "#...#", "#...#", "#...#", "####."],
        'c' => [".....", ".....", ".####", "#....", "#....", "#....", ".####"],
        'd' => ["....#", "....#", ".####", "#...#", "#...#", "#...#", ".####"],
        'e' => [".....", ".....", ".###.", "#...#", "#####", "#....", ".###."],
        'f' => ["..##.", ".#...", "####.", ".#...", ".#...", ".#...", ".#..."],
        'g' => [".....", ".####", "#...#", "#...#", ".####", "....#", ".###."],
        'h' => ["#....", "#....", "####.", "#...#", "#...#", "#...#", "#...#"],
        'i' => ["..#..", ".....", ".##..", "..#..", "..#..", "..#..", ".###."],
        'j' => ["...#.", ".....", "..##.", "...#.", "...#.", "#..#.", ".##.."],
        'k' => ["#....", "#....", "#..#.", "#.#..", "##...", "#.#..", "#..#."],
        'l' => [".##..", "..#..", "..#..", "..#..", "..#..", "..#..", ".###."],
        'm' => [".....", ".....", "##.#.", "#.#.#", "#.#.#", "#.#.#", "#.#.#"],
        'n' => [".....", ".....", "####.", "#...#", "#...#", "#...#", "#...#"],
        'o' => [".....", ".....", ".###.", "#...#", "#...#", "#...#", ".###."],
        'p' => [".....", "####.", "#...#", "#...#", "####.", "#....", "#...."],
        'q' => [".....", ".####", "#...#", "#...#", ".####", "....#", "....#"],
        'r' => [".....", ".....", "#.##.", "##..#", "#....", "#....", "#...."],
        's' => [".....", ".....", ".####", "#....", ".###.", "....#", "####."],
        't' => [".#...", ".#...", "####.", ".#...", ".#...", ".#..#", "..##."],
        'u' => [".....", ".....", "#...#", "#...#", "#...#", "#..##", ".##.#"],
        'v' => [".....", ".....", "#...#", "#...#", "#...#", ".#.#.", "..#.."],
        'w' => [".....", ".....", "#...#", "#.#.#", "#.#.#", "#.#.#", ".#.#."],
        'x' => [".....", ".....", "#...#", ".#.#.", "..#..", ".#.#.", "#...#"],
        'y' => [".....", "#...#", "#...#", "#...#", ".####", "....#", ".###."],
        'z' => [".....", ".....", "#####", "...#.", "..#..", ".#...", "#####"],
        '0' => [".###.", "#...#", "#..##", "#.#.#", "##..#", "#...#", ".###."],
        '1' => ["..#..", ".##..", "..#..", "..#..", "..#..", "..#..", ".###."],
        '2' => [".###.", "#...#", "....#", "...#.", "..#..", ".#...", "#####"],
        '3' => ["####.", "....#", "....#", ".###.", "....#", "....#", "####."],
        '4' => ["...#.", "..##.", ".#.#.", "#..#.", "#####", "...#.", "...#."],
        '5' => ["#####", "#....", "####.", "....#", "....#", "#...#", ".###."],
        '6' => ["..##.", ".#...", "#....", "####.", "#...#", "#...#", ".###."],
        '7' => ["#####", "....#", "...#.", "..#..", ".#...", ".#...", ".#..."],
        '8' => [".###.", "#...#", "#...#", ".###.", "#...#", "#...#", ".###."],
        '9' => [".###.", "#...#", "#...#", ".####", "....#", "...#.", ".##.."],
        '.' => [".....", ".....", ".....", ".....", ".....", ".##..", ".##.."],
        ',' => [".....", ".....", ".....", ".....", ".##..", "..#..", ".#..."],
        ':' => [".....", ".##..", ".##..", ".....", ".##..", ".##..", "....."],
        '-' => [".....", ".....", ".....", "#####", ".....", ".....", "....."],
        '+' => [".....", "..#..", "..#..", "#####", "..#..", "..#..", "....."],
        '_' => [".....", ".....", ".....", ".....", ".....", ".....", "#####"],
        '/' => [".....", "....#", "...#.", "..#..", ".#...", "#....", "....."],
        '(' => ["...#.", "..#..", ".#...", ".#...", ".#...", "..#..", "...#."],
        ')' => [".#...", "..#..", "...#.", "...#.", "...#.", "..#..", ".#..."],
        '%' => ["##...", "##..#", "...#.", "..#..", ".#...", "#..##", "...##"],
        '|' => ["..#..", "..#..", "..#..", "..#..", "..#..", "..#..", "..#.."],
        '?' => [".###.", "#...#", "....#", "...#.", "..#..", ".....", "..#.."],
        '!' => ["..#..", "..#..", "..#..", "..#..", "..#..", ".....", "..#.."],
        '\'' => ["..#..", "..#..", ".#...", ".....", ".....", ".....", "....."],
        '`' => [".#...", "..#..", ".....", ".....", ".....", ".....", "....."],
        '"' => [".#.#.", ".#.#.", ".....", ".....", ".....", ".....", "....."],
        '[' => [".###.", ".#...", ".#...", ".#...", ".#...", ".#...", ".###."],
        ']' => [".###.", "...#.", "...#.", "...#.", "...#.", "...#.", ".###."],
        '<' => ["...#.", "..#..", ".#...", "#....", ".#...", "..#..", "...#."],
        '>' => [".#...", "..#..", "...#.", "....#", "...#.", "..#..", ".#..."],
        '=' => [".....", ".....", "#####", ".....", "#####", ".....", "....."],
        '*' => [".....", "#.#.#", ".###.", "#####", ".###.", "#.#.#", "....."],
        '#' => [".#.#.", ".#.#.", "#####", ".#.#.", "#####", ".#.#.", ".#.#."],
        '~' => [".....", ".....", ".#...", "#.#.#", "...#.", ".....", "....."],
        '&' => [".##..", "#..#.", ".##..", ".#...", "#.#.#", "#..#.", ".##.#"],
        '←' => [".....", "..#..", ".#...", "#####", ".#...", "..#..", "....."],
        '→' => [".....", "..#..", "...#.", "#####", "...#.", "..#..", "....."],
        '↑' => ["..#..", ".###.", "#.#.#", "..#..", "..#..", "..#..", "..#.."],
        '↓' => ["..#..", "..#..", "..#..", "..#..", "#.#.#", ".###.", "..#.."],
        _ => [".....", ".....", ".....", ".....", ".....", ".....", "....."],
    }
}
