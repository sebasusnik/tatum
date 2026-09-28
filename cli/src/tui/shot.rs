//! `tatum tui-shot`: a picture of the live screen, without a terminal or an
//! audio device. The song is rendered offline up to a moment and the screen
//! is fed on the audio's own clock, so what the picture shows is what the
//! screen would show at that second of a live session.
//!
//! It exists to look at the screen from somewhere a terminal cannot be seen,
//! the way `tatum debug`'s sheet exists to look at a song.

use std::path::Path;
use std::sync::Arc;

use ratatui::buffer::Buffer;
use ratatui::style::Color;
use tatum_core::live::{LivePlanner, LivePlayer};
use tatum_core::{BLOCK_SIZE, SAMPLE_RATE};
use tatum_debug::picture::{Canvas, GLYPH_HEIGHT};

use super::{Screen, SongInfo, Telemetry, Tone};
use crate::include::Source;

const USAGE: &str =
    "usage: tatum tui-shot <song.synth | set dir> [--step N] [--at <seconds>] [--size 160x48] [--see-through] [--select <track>] [--help] [-o shot.png]";
const CELL_W: usize = 12;
const CELL_H: usize = 24;

pub fn cmd(args: &[String]) -> Result<(), String> {
    let (mut path, mut at, mut size, mut out) = (None, 20.0f32, (160u16, 48u16), "tui-shot.png".to_string());
    let mut step: usize = 1;
    let mut see_through = false;
    let mut select: Option<String> = None;
    let mut help = false;
    let mut i = 0;
    while i < args.len() {
        let value = |i: usize| args.get(i + 1).cloned().ok_or(USAGE);
        match args[i].as_str() {
            "--at" => {
                at = value(i)?.parse().map_err(|_| "--at needs seconds")?;
                i += 1;
            }
            "--size" => {
                let v = value(i)?;
                let (w, h) = v.split_once('x').ok_or("--size is WIDTHxHEIGHT in cells")?;
                size = (w.parse().map_err(|_| USAGE)?, h.parse().map_err(|_| USAGE)?);
                i += 1;
            }
            // Paint the terminal's own background as a wallpaper, to show
            // what a transparent window lets through.
            "--see-through" => see_through = true,
            "--help" => help = true,
            "--select" => {
                select = Some(value(i)?);
                i += 1;
            }
            "--step" => {
                step = value(i)?.parse().map_err(|_| "--step needs a number")?;
                i += 1;
            }
            "-o" => {
                out = value(i)?;
                i += 1;
            }
            other if other.starts_with('-') => return Err(format!("unknown flag '{}'\n{}", other, USAGE)),
            other => path = Some(other.to_string()),
        }
        i += 1;
    }
    let mut path = path.ok_or(USAGE)?;
    // A set directory: picture its step as `set play` shows it.
    let mut set = None;
    if Path::new(&path).is_dir() {
        let steps = crate::set::load(Path::new(&path), crate::set::DEFAULT_BARS)?;
        let current = step.clamp(1, steps.len()) - 1;
        path = steps[current].path.to_string_lossy().into_owned();
        set = Some(super::SetView {
            steps: steps.iter().map(|s| s.name()).collect(),
            current,
            next: (current + 1 < steps.len()).then_some((current + 1, 3)),
            phrase: 8,
        });
    }
    let source = Source::load(Path::new(&path))?;
    let mut planner = LivePlanner::new();
    let mut player = LivePlayer::new();
    let plan = planner.plan(&source.text, player.generation()).map_err(|e| source.error_lines(&e).join("\n"))?;
    player.apply(plan);
    player.start();

    let telemetry = Arc::new(Telemetry::new());
    let mut screen = Screen::headless(Arc::clone(&telemetry), "offline".into(), vec![]);
    let title = Path::new(&path).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    screen.set_song(SongInfo::from_source(&title, &source.text).ok_or("the song does not compile")?);
    screen.set = set;
    screen.say(format!("watching {}", path), Tone::Info);
    screen.say("saved: parameter change, applied instantly", Tone::Good);

    let column = (Screen::column_period().as_secs_f32() * SAMPLE_RATE) as usize;
    let total = (at * SAMPLE_RATE) as usize;
    let (mut done, mut since) = (0usize, 0usize);
    let (mut l, mut r) = ([0.0f32; BLOCK_SIZE], [0.0f32; BLOCK_SIZE]);
    while done < total {
        if let Some(e) = player.engine_mut() {
            e.set_band_metering(true);
        }
        player.process(&mut l, &mut r);
        telemetry.push(&l, &r);
        if let Some(e) = player.engine_mut() {
            telemetry.meter(e);
        }
        done += BLOCK_SIZE;
        since += BLOCK_SIZE;
        if since >= column {
            since -= column;
            if let Some(e) = player.engine() {
                screen.position(e.current_bar(), e.global_step(), e.tempo());
            }
            screen.column();
        }
    }
    if let Some(e) = player.engine() {
        screen.position(e.current_bar(), e.global_step(), e.tempo());
    }
    screen.draw_offline(select.as_deref(), help);
    paint(&screen.snapshot(size.0, size.1), see_through).save(Path::new(&out))?;
    eprintln!("wrote {} ({}x{} cells, {:.1} s in)", out, size.0, size.1, at);
    Ok(())
}

fn rgb(c: Color, fallback: [u8; 3]) -> [u8; 3] {
    match c {
        Color::Rgb(r, g, b) => [r, g, b],
        Color::Black => [0, 0, 0],
        _ => fallback,
    }
}

/// Each cell as a CELL_W x CELL_H block: the half blocks as two colours, the
/// few shapes the screen uses drawn as shapes, text in the pictures' font.
fn paint(buf: &Buffer, see_through: bool) -> Canvas {
    let area = buf.area;
    let mut c = Canvas::new(area.width as usize * CELL_W, area.height as usize * CELL_H);
    for y in 0..area.height {
        for x in 0..area.width {
            let cell = &buf[(x, y)];
            let (px, py) = (x as usize * CELL_W, y as usize * CELL_H);
            let fg = rgb(cell.fg, [220, 210, 235]);
            if see_through && cell.bg == Color::Reset {
                // A dusk-blue wallpaper, darker towards the bottom.
                for yy in 0..CELL_H {
                    let t = (py + yy) as f32 / (area.height as usize * CELL_H) as f32;
                    let shade = [
                        (40.0 + 30.0 * (1.0 - t)) as u8,
                        (70.0 + 50.0 * (1.0 - t)) as u8,
                        (110.0 + 60.0 * (1.0 - t)) as u8,
                    ];
                    c.fill(px, py + yy, CELL_W, 1, shade);
                }
            } else {
                c.fill(px, py, CELL_W, CELL_H, rgb(cell.bg, [8, 7, 14]));
            }
            let (cx, cy) = (px + CELL_W / 2, py + CELL_H / 2);
            match cell.symbol() {
                " " | "" => {}
                "▀" => c.fill(px, py, CELL_W, CELL_H / 2, fg),
                "▄" => c.fill(px, py + CELL_H / 2, CELL_W, CELL_H / 2, fg),
                "▔" => c.fill(px, py, CELL_W, 2, fg),
                "▮" => c.fill(px + 3, py + 5, CELL_W - 6, CELL_H - 10, fg),
                "●" => c.fill(cx - 4, cy - 4, 8, 8, fg),
                "○" => {
                    c.fill(cx - 4, cy - 4, 8, 2, fg);
                    c.fill(cx - 4, cy + 2, 8, 2, fg);
                    c.fill(cx - 4, cy - 4, 2, 8, fg);
                    c.fill(cx + 2, cy - 4, 2, 8, fg);
                }
                "▼" => {
                    for k in 0..6 {
                        c.fill(cx - 5 + k, py + 6 + k * 2, 10 - k * 2, 2, fg);
                    }
                }
                "◆" => {
                    for k in 0..5 {
                        c.fill(cx - k, cy - 5 + k, k * 2 + 1, 1, fg);
                        c.fill(cx - k, cy + 5 - k, k * 2 + 1, 1, fg);
                    }
                }
                "▸" => c.fill(cx - 3, cy - 3, 6, 6, fg),
                "─" | "━" => c.fill(px, cy, CELL_W, 1, fg),
                "·" => c.fill(cx - 1, cy - 1, 2, 2, fg),
                s => {
                    c.text(px + 1, py + (CELL_H - GLYPH_HEIGHT * 2) / 2, s, fg, 2);
                }
            }
        }
    }
    c
}
