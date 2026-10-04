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
    "usage: tatum tui-shot <song.synth | set dir> [--step N] [--at <seconds>] [--size 160x48] [--font <file.ttf>] [--frames N] [--fps 10] [--see-through] [--glass] [--select <track>] [--mute <track>] [--solo <track>] [--pick N] [--help] [--log] [--look N] [--mark <track>] [--knobs N] [-o shot.png | -o dir]";
const CELL_W: usize = 12;
const CELL_H: usize = 24;

pub fn cmd(args: &[String]) -> Result<(), String> {
    let (mut path, mut at, mut size, mut out) = (None, 20.0f32, (160u16, 48u16), "tui-shot.png".to_string());
    let mut step: usize = 1;
    let mut see_through = false;
    let mut glass = false;
    let mut select: Option<String> = None;
    let mut help = false;
    let mut log = false;
    let mut look: Option<usize> = None;
    let mut marks: Vec<String> = Vec::new();
    let mut knob_row: Option<usize> = None;
    let mut mute: Vec<String> = Vec::new();
    let mut solo: Option<String> = None;
    let mut pick: Option<usize> = None;
    let mut font: Option<String> = None;
    let mut frames: usize = 0;
    let mut fps: f32 = 10.0;
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
            "--glass" => glass = true,
            "--help" => help = true,
            "--log" => log = true,
            "--mute" => {
                mute.push(value(i)?);
                i += 1;
            }
            "--knobs" => {
                knob_row = Some(value(i)?.parse().map_err(|_| "--knobs needs a row number")?);
                i += 1;
            }
            "--mark" => {
                marks.push(value(i)?);
                i += 1;
            }
            "--look" => {
                look = Some(value(i)?.parse().map_err(|_| "--look needs a step number")?);
                i += 1;
            }
            "--pick" => {
                pick = Some(value(i)?.parse().map_err(|_| "--pick needs a step number")?);
                i += 1;
            }
            "--solo" => {
                solo = Some(value(i)?);
                i += 1;
            }
            "--select" => {
                select = Some(value(i)?);
                i += 1;
            }
            "--font" => {
                font = Some(value(i)?);
                i += 1;
            }
            "--frames" => {
                frames = value(i)?.parse().map_err(|_| "--frames needs a number")?;
                i += 1;
            }
            "--fps" => {
                fps = value(i)?.parse().map_err(|_| "--fps needs a number")?;
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
            bar: 1,
            bars: steps[current].bars as usize,
            cues: steps[current].cues.clone(),
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

    // Mutes as the keys make them, through the same planner.
    let mut plans = Vec::new();
    for m in &mute {
        plans.extend(planner.toggle_mute(m, player.generation()));
    }
    if let Some(s) = &solo {
        plans.extend(planner.toggle_solo(s, player.generation()));
    }
    for p in plans {
        player.apply(p);
    }
    screen.soloed = planner.soloed().to_vec();
    let font = font.map(|f| Font::load(Path::new(&f))).transpose()?;
    let column = (Screen::column_period().as_secs_f32() * SAMPLE_RATE) as usize;
    let total = (at * SAMPLE_RATE) as usize;
    let (mut done, mut since) = (0usize, 0usize);
    let (mut l, mut r) = ([0.0f32; BLOCK_SIZE], [0.0f32; BLOCK_SIZE]);
    // Play on to `until` samples, feeding the screen as the live session does.
    let mut run = |until: usize, screen: &mut Screen, player: &mut LivePlayer| {
        while done < until {
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
            // The step on its own from its first bar, as `set play` counts it.
            if let Some(set) = screen.set.as_mut() {
                set.bar = e.current_bar() + 1;
            }
        }
    };
    run(total, &mut screen, &mut player);
    screen.glass = glass;
    for m in &marks {
        screen.mark(m);
    }
    if let Some(n) = look {
        screen.look_at(n);
    }
    screen.draw_offline(select.as_deref(), help);
    // The knob list with cc 74 turned most of the way, as trying it would.
    if let Some(n) = knob_row {
        screen.open_knobs(n, None);
        if let Some(target) = screen.knob_target() {
            let turn = planner.try_knob(&target.dotted(), 96, player.generation());
            screen.knob_turned(74, turn.readings.first().cloned());
        }
    }
    if log {
        screen.open_log();
    }
    if let Some(n) = pick {
        screen.open_picker(n);
    }
    if frames == 0 {
        paint(&screen.snapshot(size.0, size.1), see_through, glass, font.as_ref()).save(Path::new(&out))?;
        eprintln!("wrote {} ({}x{} cells, {:.1} s in)", out, size.0, size.1, at);
        return Ok(());
    }
    // A run of frames from `at` on, `fps` a second, into the directory `-o`
    // names: frame-0000.png, frame-0001.png... for a GIF or a video.
    std::fs::create_dir_all(&out).map_err(|e| format!("cannot create {}: {e}", out))?;
    let step = (SAMPLE_RATE / fps.max(0.1)) as usize;
    for n in 0..frames {
        if n > 0 {
            run(total + n * step, &mut screen, &mut player);
            screen.draw_offline(select.as_deref(), help);
        }
        let file = Path::new(&out).join(format!("frame-{:04}.png", n));
        paint(&screen.snapshot(size.0, size.1), see_through, glass, font.as_ref()).save(&file)?;
    }
    eprintln!(
        "wrote {} frames into {} ({}x{} cells, {:.1} s from {:.1} s in)",
        frames,
        out,
        size.0,
        size.1,
        frames as f32 / fps,
        at
    );
    Ok(())
}

/// A TrueType font for the text, in place of the pictures' 5x7 one: sized so
/// its advance fills a cell, with its bold face beside it when the file's
/// name says Regular and a Bold one sits next to it.
pub struct Font {
    regular: fontdue::Font,
    bold: Option<fontdue::Font>,
    px: f32,
    ascent: f32,
}

impl Font {
    fn load(path: &Path) -> Result<Font, String> {
        let read = |p: &Path| -> Result<fontdue::Font, String> {
            let bytes = std::fs::read(p).map_err(|e| format!("cannot read {}: {e}", p.display()))?;
            fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default())
                .map_err(|e| format!("{}: {e}", p.display()))
        };
        let regular = read(path)?;
        let name = path.to_string_lossy();
        let bold =
            name.contains("Regular").then(|| name.replace("Regular", "Bold")).and_then(|b| read(Path::new(&b)).ok());
        // A monospace face: one advance for every glyph. Size it to the cell.
        let advance = regular.metrics('M', 100.0).advance_width / 100.0;
        let px = (CELL_W as f32 / advance).min(CELL_H as f32 * 0.85);
        let lines = regular.horizontal_line_metrics(px).ok_or("the font has no horizontal metrics")?;
        let ascent = lines.ascent + ((CELL_H as f32 - (lines.ascent - lines.descent)) / 2.0).max(0.0);
        Ok(Font { regular, bold, px, ascent })
    }

    /// One character in the cell at (px, py), its coverage laid over what
    /// is there in `fg`.
    fn draw(&self, c: &mut Canvas, px: usize, py: usize, ch: char, fg: [u8; 3], bold: bool) {
        let face = if bold { self.bold.as_ref().unwrap_or(&self.regular) } else { &self.regular };
        // A character the face lacks: its nearest look-alike the face has,
        // else the pictures' own font, the way a terminal falls back.
        let ch = match ch {
            k if face.lookup_glyph_index(k) != 0 => k,
            '⟲' | '⟳' if ['↺', '↻'].iter().any(|&a| face.lookup_glyph_index(a) != 0) => {
                ['↺', '↻'].into_iter().find(|&a| face.lookup_glyph_index(a) != 0).unwrap_or(ch)
            }
            k => {
                c.text(px + 1, py + (CELL_H - GLYPH_HEIGHT * 2) / 2, &k.to_string(), fg, 2);
                return;
            }
        };
        let (m, bitmap) = face.rasterize(ch, self.px);
        let baseline = py as f32 + self.ascent;
        let top = baseline - m.height as f32 - m.ymin as f32;
        for gy in 0..m.height {
            for gx in 0..m.width {
                let a = bitmap[gy * m.width + gx] as f32 / 255.0;
                if a > 0.0 {
                    let (x, y) = (px as i32 + m.xmin + gx as i32, top as i32 + gy as i32);
                    if x >= 0 && y >= 0 {
                        c.blend(x as usize, y as usize, fg, a);
                    }
                }
            }
        }
    }
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
/// With `glass`, cell backgrounds are laid over the wallpaper at 0.88, the
/// way Ghostty's `background-opacity-cells` draws them.
fn paint(buf: &Buffer, see_through: bool, glass: bool, font: Option<&Font>) -> Canvas {
    let area = buf.area;
    let mut c = Canvas::new(area.width as usize * CELL_W, area.height as usize * CELL_H);
    for y in 0..area.height {
        for x in 0..area.width {
            let cell = &buf[(x, y)];
            let (px, py) = (x as usize * CELL_W, y as usize * CELL_H);
            let fg = rgb(cell.fg, [220, 210, 235]);
            if see_through && (cell.bg == Color::Reset || glass) {
                // A dusk-blue wallpaper, darker towards the bottom, with the
                // cell's own background over it when the cells are glass.
                let over = match cell.bg {
                    Color::Rgb(r, g, b) if glass => Some([r, g, b]),
                    _ => None,
                };
                for yy in 0..CELL_H {
                    let t = (py + yy) as f32 / (area.height as usize * CELL_H) as f32;
                    let wall = [40.0 + 30.0 * (1.0 - t), 70.0 + 50.0 * (1.0 - t), 110.0 + 60.0 * (1.0 - t)];
                    let shade = match over {
                        Some(o) => [0, 1, 2].map(|k| (o[k] as f32 * 0.88 + wall[k] * 0.12) as u8),
                        None => wall.map(|v| v as u8),
                    };
                    c.fill(px, py + yy, CELL_W, 1, shade);
                }
            } else {
                c.fill(px, py, CELL_W, CELL_H, rgb(cell.bg, [8, 7, 14]));
            }
            let (cx, cy) = (px + CELL_W / 2, py + CELL_H / 2);
            // With a font, everything but the blocks is its text.
            if let Some(font) = font {
                let sym = cell.symbol();
                if !matches!(sym, " " | "" | "▀" | "▄" | "▔" | "▮") {
                    let bold = cell.modifier.contains(ratatui::style::Modifier::BOLD);
                    for ch in sym.chars().take(1) {
                        font.draw(&mut c, px, py, ch, fg, bold);
                    }
                    continue;
                }
            }
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
