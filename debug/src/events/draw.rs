//! The events of one file, or of two on the same scales, on one sheet: each
//! event up close twice -- with a short window, where an attack and an echo
//! show, and with a long one, where a partial and what is beside it show --
//! and under them the events' level and pitch laid over each other.

use super::{Event, Report, SR};
use crate::fft::fft;
use crate::picture::{heat, text_width, Canvas, Rgb, DIM_TEXT, TEXT};

const LEFT: usize = 64;
const PANEL_W: usize = 220;
const PANEL_H: usize = 150;
const GAP: usize = 12;
const LABEL_H: usize = 14;
const AXIS_H: usize = 16;
const CURVES_H: usize = 230;
const A_COLOUR: Rgb = [250, 190, 70];
const B_COLOUR: Rgb = [90, 200, 250];
const GUIDE: Rgb = [255, 255, 255];
/// The colour scale runs this many dB down from each file's own event peak.
const RANGE_DB: f32 = 60.0;
/// The short and the long window, samples, for a band from `lo` Hz. The
/// short one holds three cycles of the lowest note, or it would show each
/// cycle of a drone as a stripe; the long one is sixteen times that.
fn windows(lo: f32) -> (usize, usize) {
    let fast = ((3.0 * SR / lo) as usize).next_power_of_two().clamp(256, 2048);
    (fast, fast * 16)
}

/// The events shown: of the half that stands clearest of its bed, `count`
/// spread evenly over the file.
fn pick(r: &Report, count: usize) -> Vec<(usize, &Event)> {
    let mut by_level: Vec<(usize, &Event)> = r.events.iter().enumerate().collect();
    by_level.sort_by(|a, b| b.1.inside.over_bed_db.total_cmp(&a.1.inside.over_bed_db));
    by_level.truncate((r.events.len().div_ceil(2)).max(count.min(r.events.len())));
    by_level.sort_by_key(|e| e.0);
    if by_level.len() <= count {
        return by_level;
    }
    (0..count).map(|k| by_level[k * (by_level.len() - 1) / (count - 1).max(1)]).collect()
}

struct Scale {
    /// Milliseconds a panel spans, and how many of them come before the start.
    span_ms: f32,
    lead_ms: f32,
    lo: f32,
    hi: f32,
}

impl Scale {
    fn y(&self, hz: f32) -> f32 {
        (1.0 - (hz / self.lo).ln() / (self.hi / self.lo).ln()) * PANEL_H as f32
    }

    fn hz(&self, y: f32) -> f32 {
        self.lo * (self.hi / self.lo).powf(1.0 - y / PANEL_H as f32)
    }
}

fn nice(v: f32) -> f32 {
    let mag = 10f32.powf(v.log10().floor());
    let f = v / mag;
    mag * if f < 1.5 {
        1.0
    } else if f < 3.5 {
        2.0
    } else if f < 7.5 {
        5.0
    } else {
        10.0
    }
}

/// One panel's cells, power per (column, row): `window` samples of `mono`
/// per column, the event's start `lead_ms` in.
fn cells(mono: &[f32], e: &Event, scale: &Scale, window: usize) -> Vec<f32> {
    let size = (window * 4).next_power_of_two().clamp(2048, 32768).max(window);
    let hann: Vec<f64> =
        (0..window).map(|i| 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / window as f64).cos()).collect();
    let norm = (window as f64 / 4.0).powi(2);
    let bin_hz = SR / size as f32;
    let mut out = vec![0.0f32; PANEL_W * PANEL_H];
    let (mut re, mut im) = (vec![0.0f64; size], vec![0.0f64; size]);
    for x in 0..PANEL_W {
        let ms = e.start_s * 1000.0 - scale.lead_ms + (x as f32 + 0.5) * scale.span_ms / PANEL_W as f32;
        let centre = (ms * 0.001 * SR) as isize;
        re.fill(0.0);
        im.fill(0.0);
        for (i, w) in hann.iter().enumerate() {
            let at = centre + i as isize - (window / 2) as isize;
            if at >= 0 && (at as usize) < mono.len() {
                re[i] = mono[at as usize] as f64 * w;
            }
        }
        fft(&mut re, &mut im);
        for y in 0..PANEL_H {
            let (top, bottom) = (scale.hz(y as f32), scale.hz(y as f32 + 1.0));
            let a = ((bottom / bin_hz).round() as usize).clamp(1, size / 2 - 1);
            let b = ((top / bin_hz).round() as usize).clamp(a, size / 2 - 1);
            let p = (a..=b).map(|k| re[k] * re[k] + im[k] * im[k]).fold(0.0f64, f64::max);
            out[x * PANEL_H + y] = (p / norm) as f32;
        }
    }
    out
}

fn hz_label(hz: f32) -> String {
    if hz >= 1000.0 {
        format!("{}k", format!("{:.1}", hz / 1000.0).trim_end_matches('0').trim_end_matches('.'))
    } else {
        format!("{hz:.0}")
    }
}

/// A row of panels for one file at one window; returns the reference level
/// (0 dB on the colour scale), in dB against full scale.
#[allow(clippy::too_many_arguments)]
fn panel_row(
    c: &mut Canvas,
    y0: usize,
    mono: &[f32],
    shown: &[(usize, &Event)],
    scale: &Scale,
    window: usize,
    label: &str,
    colour: Rgb,
) -> f32 {
    let all: Vec<Vec<f32>> = shown.iter().map(|(_, e)| cells(mono, e, scale, window)).collect();
    let mut peaks: Vec<f32> = all.iter().map(|p| p.iter().fold(0.0f32, |m, v| m.max(*v))).collect();
    peaks.sort_by(f32::total_cmp);
    let reference = peaks.get(peaks.len() / 2).copied().unwrap_or(1.0).max(1e-12);
    c.text(4, y0 + LABEL_H + 2, label, colour, 1);
    let py = y0 + LABEL_H;
    // The frequency axis, once at the left.
    for decade in 1..=4 {
        for lead in [1.0, 2.0, 5.0] {
            let hz = lead * 10f32.powi(decade);
            if hz < scale.lo || hz > scale.hi {
                continue;
            }
            let y = py + scale.y(hz).clamp(0.0, PANEL_H as f32 - 1.0) as usize;
            let text = hz_label(hz);
            c.text(LEFT - 4 - text_width(&text, 1), y.saturating_sub(3), &text, DIM_TEXT, 1);
            for k in 0..shown.len() {
                c.hline(LEFT + k * (PANEL_W + GAP), y, PANEL_W, GUIDE, 0.12);
            }
        }
    }
    c.text(4, py + PANEL_H - 10, "hz, log", DIM_TEXT, 1);
    for (k, ((index, e), p)) in shown.iter().zip(&all).enumerate() {
        let x0 = LEFT + k * (PANEL_W + GAP);
        c.text(
            x0,
            y0 + 2,
            &format!(
                "#{} {:.2}s {:.0}hz {:+.1}db",
                index + 1,
                e.start_s,
                e.inside.start_hz,
                e.inside.over_bed_db.min(99.0)
            ),
            DIM_TEXT,
            1,
        );
        for x in 0..PANEL_W {
            for y in 0..PANEL_H {
                let d = 10.0 * (p[x * PANEL_H + y] / reference).max(1e-12).log10();
                c.set(x0 + x, py + y, heat(0.92 + d / (RANGE_DB / 0.92)));
            }
        }
        // The start, and time under the panel.
        let start_x = x0 + (scale.lead_ms / scale.span_ms * PANEL_W as f32) as usize;
        c.vline(start_x, py, PANEL_H, GUIDE, 0.25);
        let step = nice(scale.span_ms / 4.0);
        let mut t = 0.0;
        while t < scale.span_ms - scale.lead_ms {
            let x = start_x + (t / scale.span_ms * PANEL_W as f32) as usize;
            c.vline(x, py + PANEL_H, 3, GUIDE, 0.6);
            let text = if t == 0.0 { "0".to_string() } else { format!("{t:.0}") };
            if x + text_width(&text, 1) < x0 + PANEL_W {
                c.text(x + 2, py + PANEL_H + 4, &text, DIM_TEXT, 1);
            }
            t += step;
        }
        if k == 0 {
            c.text(x0 + PANEL_W - text_width("ms", 1), py + PANEL_H + 4, "ms", DIM_TEXT, 1);
        }
    }
    10.0 * reference.log10()
}

/// The middle event's curve at each millisecond: the median over the events
/// that have a reading there, while a third of them do.
fn median_curve(
    events: &[Event],
    read: &dyn Fn(&Event, f32) -> Option<f32>,
    from_ms: f32,
    to_ms: f32,
) -> Vec<(f32, f32)> {
    let mut out = Vec::new();
    let step = ((to_ms - from_ms) / 400.0).max(0.25);
    let mut ms = from_ms;
    while ms <= to_ms {
        let mut v: Vec<f32> = events.iter().filter_map(|e| read(e, ms)).collect();
        if v.len() * 3 >= events.len().max(1) && !v.is_empty() {
            v.sort_by(f32::total_cmp);
            out.push((ms, v[v.len() / 2]));
        }
        ms += step;
    }
    out
}

fn level_at(e: &Event, ms: f32) -> Option<f32> {
    // The curve starts 10 ms before the event, a reading a millisecond.
    let i = ms + 10.0;
    (i >= 0.0).then(|| e.inside.level.get(i as usize).copied()).flatten()
}

fn pitch_at(e: &Event, ms: f32) -> Option<f32> {
    let p = &e.inside.pitch;
    let i = p.iter().position(|q| q.0 >= ms)?;
    if i == 0 {
        return (p[0].0 - ms < 2.0).then(|| 12.0 * (p[0].1 / e.inside.start_hz).log2());
    }
    let u = (ms - p[i - 1].0) / (p[i].0 - p[i - 1].0).max(1e-6);
    Some(12.0 * ((p[i - 1].1 * (1.0 - u) + p[i].1 * u) / e.inside.start_hz).log2())
}

#[allow(clippy::too_many_arguments)]
fn plot(
    c: &mut Canvas,
    x0: usize,
    y0: usize,
    w: usize,
    h: usize,
    title: &str,
    x_range: (f32, f32),
    y_range: (f32, f32),
    y_unit: &str,
    curves: &[(Vec<(f32, f32)>, Rgb)],
) {
    c.text(x0, y0, title, TEXT, 1);
    let (px, py, pw, ph) = (x0 + 40, y0 + 14, w - 48, h - 34);
    c.fill(px, py, pw, ph, [4, 4, 8]);
    let to_x = |v: f32| px as f32 + (v - x_range.0) / (x_range.1 - x_range.0) * (pw as f32 - 1.0);
    let to_y = |v: f32| py as f32 + (y_range.1 - v) / (y_range.1 - y_range.0) * (ph as f32 - 1.0);
    let y_step = nice((y_range.1 - y_range.0) / 5.0);
    let mut v = (y_range.0 / y_step).ceil() * y_step;
    while v <= y_range.1 + 1e-3 {
        let y = to_y(v) as usize;
        c.hline(px, y, pw, GUIDE, if v.abs() < 1e-6 { 0.3 } else { 0.1 });
        let text = if y_step < 1.0 { format!("{v:.1}") } else { format!("{v:.0}") };
        c.text(px - 4 - text_width(&text, 1), y.saturating_sub(3), &text, DIM_TEXT, 1);
        v += y_step;
    }
    c.text(x0, py + ph - 8, y_unit, DIM_TEXT, 1);
    let x_step = nice((x_range.1 - x_range.0) / 6.0);
    let mut v = (x_range.0 / x_step).ceil() * x_step;
    while v <= x_range.1 + 1e-3 {
        let x = to_x(v) as usize;
        c.vline(x, py, ph, GUIDE, if v.abs() < 1e-6 { 0.3 } else { 0.1 });
        let text = format!("{v:.0}");
        c.text(x.saturating_sub(text_width(&text, 1) / 2), py + ph + 4, &text, DIM_TEXT, 1);
        v += x_step;
    }
    c.text(px + pw - text_width("ms from the start", 1), py + ph + 14, "ms from the start", DIM_TEXT, 1);
    for (points, colour) in curves {
        for pair in points.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let (xa, xb) = (to_x(a.0), to_x(b.0));
            let steps = ((xb - xa).abs().max((to_y(b.1) - to_y(a.1)).abs()) as usize).max(1);
            for k in 0..=steps {
                let u = k as f32 / steps as f32;
                let (x, y) = (xa + (xb - xa) * u, to_y(a.1 + (b.1 - a.1) * u));
                if y >= py as f32 && y < (py + ph) as f32 && x >= px as f32 && x < (px + pw) as f32 {
                    c.set(x as usize, y as usize, *colour);
                    c.set(x as usize, y as usize + 1, *colour);
                }
            }
        }
    }
}

/// The sheet. `b` is the second file, when there is one; `count` events of each.
pub fn sheet(a: &Report, name_a: &str, b: Option<(&Report, &str)>, count: usize) -> Canvas {
    let count = count.clamp(1, 12);
    let files: Vec<(&Report, &str, Rgb, &str)> = match b {
        Some((rb, nb)) => vec![(a, name_a, A_COLOUR, "A"), (rb, nb, B_COLOUR, "B")],
        None => vec![(a, name_a, A_COLOUR, "A")],
    };
    let typical = |r: &Report| {
        let mut v: Vec<f32> = r.events.iter().map(|e| (e.end_s - e.start_s) * 1000.0).collect();
        v.sort_by(f32::total_cmp);
        v.get(v.len() / 2).copied().unwrap_or(100.0)
    };
    let longest = files.iter().map(|f| typical(f.0)).fold(0.0f32, f32::max);
    let wanted = 2.5 * longest + 40.0;
    let span_ms = [300.0f32, 500.0, 1000.0, 2000.0, 4000.0].into_iter().find(|s| *s >= wanted).unwrap_or(8000.0);
    let scale = Scale {
        span_ms,
        lead_ms: span_ms / 8.0,
        lo: files.iter().map(|f| f.0.band.0).fold(f32::MAX, f32::min).max(20.0) / 1.3,
        hi: (files.iter().map(|f| f.0.band.1).fold(0.0f32, f32::max) * 1.6).min(SR * 0.49),
    };
    let row_h = LABEL_H + PANEL_H + AXIS_H + 6;
    let width = LEFT + count * (PANEL_W + GAP) + 8;
    let title_h = 22 + 12 * files.len();
    let height = title_h + 2 * files.len() * row_h + 2 * 16 + CURVES_H + 40;
    let mut c = Canvas::new(width, height);
    c.text(8, 6, "events, each from its start", TEXT, 2);
    for (i, (r, name, colour, tag)) in files.iter().enumerate() {
        c.text(
            8,
            26 + 12 * i,
            &format!("{tag}: {name}   {} events in {:.0}-{:.0} hz", r.events.len(), r.band.0, r.band.1),
            *colour,
            1,
        );
    }
    let monos: Vec<Vec<f32>> = files.iter().map(|f| f.0.mono()).collect();
    let shown: Vec<Vec<(usize, &Event)>> = files.iter().map(|f| pick(f.0, count)).collect();
    let mut y = title_h;
    let mut notes = Vec::new();
    let (fast, fine) = windows(scale.lo * 1.3);
    for (window, what) in [(fast, "short window"), (fine, "long window")] {
        c.text(
            8,
            y + 2,
            &format!("{what}: {:.1} ms, {:.0} hz wide", window as f32 * 1000.0 / SR, 1.44 * SR / window as f32),
            TEXT,
            1,
        );
        y += 16;
        for (i, (_, _, colour, tag)) in files.iter().enumerate() {
            let zero = panel_row(&mut c, y, &monos[i], &shown[i], &scale, window, tag, *colour);
            notes.push(format!("{} {tag} {zero:.0}", what.split(' ').next().unwrap_or(what)));
            y += row_h;
        }
    }
    // The events of each file over each other.
    let half = (width - 16) / 2;
    let x_range = (-scale.lead_ms.min(10.0), scale.span_ms - scale.lead_ms);
    let level: Vec<(Vec<(f32, f32)>, Rgb)> =
        files.iter().map(|f| (median_curve(&f.0.events, &level_at, x_range.0, x_range.1), f.2)).collect();
    let pitch: Vec<(Vec<(f32, f32)>, Rgb)> =
        files.iter().map(|f| (median_curve(&f.0.events, &pitch_at, 0.0, x_range.1), f.2)).collect();
    let reach = pitch.iter().flat_map(|p| p.0.iter().map(|q| q.1.abs())).fold(1.0f32, f32::max);
    let reach = nice(reach * 1.2).max(2.0);
    plot(
        &mut c,
        8,
        y + 4,
        half,
        CURVES_H,
        "level of the partial, median over the events",
        x_range,
        (-50.0, 3.0),
        "db",
        &level,
    );
    plot(
        &mut c,
        8 + half,
        y + 4,
        half,
        CURVES_H,
        "pitch against each event's own start, median over the events",
        x_range,
        (-reach, reach),
        "st",
        &pitch,
    );
    y += CURVES_H + 8;
    c.text(
        8,
        y,
        &format!(
            "each panel: {:.0} ms ({:.2} ms a pixel), the line is the event's start; {:.0}-{:.0} hz, log",
            scale.span_ms,
            scale.span_ms / PANEL_W as f32,
            scale.lo,
            scale.hi
        ),
        DIM_TEXT,
        1,
    );
    c.text(
        8,
        y + 12,
        &format!(
            "colour: brightest = the file's own middle event peak, black {RANGE_DB:.0} db under it; that peak in db full scale: {}",
            notes.join(", ")
        ),
        DIM_TEXT,
        1,
    );
    c
}
