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
pub const ERR: Color = Color::Rgb(255, 80, 80);
pub const OK: Color = Color::Rgb(120, 220, 160);
