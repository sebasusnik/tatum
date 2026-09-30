//! The colours of `tatum debug`'s pictures -- black through purple and red
//! to pale yellow, the same function -- so the live screen and the
//! spectrograms read the same.

use ratatui::style::Color;

/// `t` in 0..1: the ramp `tatum debug` paints its spectrograms with.
pub fn inferno(t: f32) -> Color {
    let [r, g, b] = tatum_debug::picture::heat(t);
    Color::Rgb(r, g, b)
}

/// The terminal's own background, so a transparent window stays
/// transparent behind the screen.
pub const BG: Color = Color::Reset;
pub const PANEL: Color = Color::Rgb(18, 15, 30);
pub const DIM: Color = Color::Rgb(150, 138, 178);
pub const TEXT: Color = Color::Rgb(220, 210, 235);
pub const HOT: Color = Color::Rgb(249, 140, 10);
pub const GOLD: Color = Color::Rgb(249, 201, 50);
/// The keycap a step's key is drawn on.
pub const KEYCAP: Color = Color::Rgb(62, 56, 88);
pub const ERR: Color = Color::Rgb(255, 80, 80);
pub const OK: Color = Color::Rgb(120, 220, 160);

/// Hue in degrees, saturation and value in 0..1.
pub fn hsv(h: f32, s: f32, v: f32) -> Color {
    let c = v * s;
    let hp = (h.rem_euclid(360.0)) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let byte = |u: f32| ((u + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    Color::Rgb(byte(r), byte(g), byte(b))
}
