//! What `tatum events` prints: one file's events and their spread, or two
//! files side by side with what differs most, as text or as JSON.
//!
//! A figure outside its estimator's domain is not printed as a number: it
//! is a bound ("< -40 dB") or "not measurable here", with the reason, and
//! `--against` gives no advice from it.

use std::fmt::Write as _;

use super::inside::{Fig, Shape};
use super::sustained::Sustained;
use super::{spread, Event, Report, Spread};
use crate::reference::note_name;

/// One thing measured on every event.
pub struct Measure {
    pub key: &'static str,
    pub label: &'static str,
    pub unit: &'static str,
    pub get: fn(&Event) -> Fig,
    /// The smallest difference between two files worth a line: in the
    /// measure's own unit, or as a ratio when `ratio`.
    pub least: f32,
    pub ratio: bool,
    /// What one event's reading can be off by inside the measure's domain:
    /// with fewer than `FEW` events on a side there is no spread to hold a
    /// difference against, and it has to be larger than this.
    pub error: f32,
}

/// Under this many events on either side, differences are held against
/// each measure's own error, not against A's spread.
pub const FEW: usize = 4;

#[allow(clippy::too_many_arguments)]
const fn m(
    key: &'static str,
    label: &'static str,
    unit: &'static str,
    get: fn(&Event) -> Fig,
    least: f32,
    ratio: bool,
    error: f32,
) -> Measure {
    Measure { key, label, unit, get, least, ratio, error }
}

fn length(e: &Event) -> Fig {
    let ms = (e.end_s - e.start_s) * 1000.0;
    if e.inside.lost {
        Fig::No("the partial moves faster than its band follows it")
    } else if e.inside.overlap.is_some_and(|o| o.starts_with("the next event")) {
        Fig::AtLeast(ms)
    } else if let Some(why) = e.inside.overlap.filter(|o| o.starts_with("it starts while") || o.starts_with("its level rises")) {
        Fig::No(why)
    } else {
        Fig::Value(ms)
    }
}

pub const MEASURES: &[Measure] = &[
    m("over_floor_db", "over its floor", "dB", |e| Fig::Value(e.over_floor_db), 4.0, false, 6.0),
    m("length_ms", "length", "ms", length, 1.3, true, 1.5),
    m("attack_ms", "attack 10-90%", "ms", |e| e.inside.attack_ms, 1.5, true, 1.6),
    m("burst_db", "burst over the resonance", "dB", |e| e.inside.burst_db, 4.0, false, 8.0),
    m("t20_ms", "fall to -20 dB", "ms", |e| e.inside.t20_ms, 1.3, true, 1.3),
    m("t40_ms", "fall to -40 dB", "ms", |e| e.inside.t40_ms, 1.3, true, 1.3),
    m("decay_db_ms", "decay", "dB/ms", |e| e.inside.decay_db_ms, 1.3, true, 1.3),
    m("decay_r2", "decay straightness", "r2", |e| e.inside.decay_r2, 0.1, false, 0.2),
    m("start_hz", "start pitch", "Hz", |e| e.inside.start, 1.12, true, 1.12),
    m("end_hz", "end pitch", "Hz", |e| e.inside.end, 1.12, true, 1.12),
    m("bend_st", "bend", "st", |e| e.inside.bend, 1.0, false, 2.0),
    m("abs_bend_st", "bend, either way", "st", |e| abs(e.inside.bend), 1.0, false, 2.0),
    m("bandwidth_hz", "-3 dB width", "Hz", |e| e.inside.bandwidth_hz, 1.4, true, 1.5),
    m("q", "Q (f / width)", "", |e| e.inside.q, 1.4, true, 1.5),
    m("ring_q", "Q (from the decay)", "", |e| e.inside.ring_q, 1.4, true, 1.4),
    m("tonal_db", "partial over the rest", "dB", |e| e.inside.tonal_db, 3.0, false, 6.0),
    m("h2_db", "2nd harmonic", "dB", |e| e.inside.h2_db, 5.0, false, 6.0),
    m("h3_db", "3rd harmonic", "dB", |e| e.inside.h3_db, 5.0, false, 6.0),
    m("over_bed_db", "over the bed in its band", "dB", |e| Fig::Value(e.inside.over_bed_db), 4.0, false, 4.0),
    m("in_mix_db", "against the whole mix", "dB", |e| e.inside.in_mix_db, 3.0, false, 4.0),
    m("pan_db", "left over right", "dB", |e| Fig::Value(e.inside.pan_db), 2.0, false, 2.0),
    m("off_centre_db", "off centre, either side", "dB", |e| Fig::Value(e.inside.pan_db.abs()), 2.0, false, 2.0),
    m("side_mid_db", "side over mid in its band", "dB", |e| e.inside.side_mid_db, 4.0, false, 6.0),
    m("corr", "L/R correlation in its band", "", |e| e.inside.corr, 0.15, false, 0.3),
    m("tail_db", "tail beyond its decay", "dB", |e| e.inside.tail_db, 4.0, false, 6.0),
    m("centroid_hz", "centroid at the peak", "Hz", |e| e.inside.centroid_hz, 1.25, true, 1.25),
    m("centroid_move_st", "centroid by -12 dB", "st", |e| e.inside.centroid_move_st, 2.0, false, 3.0),
];

fn abs(f: Fig) -> Fig {
    match f {
        Fig::Value(v) => Fig::Value(v.abs()),
        Fig::AtMost(v) | Fig::AtLeast(v) => Fig::AtLeast(v.abs()),
        no => no,
    }
}

/// A measure over a file's events: their spread when most were read, a
/// bound when most were under what can be told, or why it was not read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Summary {
    Spread(Spread),
    AtMost(f32, usize),
    AtLeast(f32, usize),
    No(&'static str),
}

impl Summary {
    pub fn spread(&self) -> Option<Spread> {
        match self {
            Summary::Spread(s) => Some(*s),
            _ => None,
        }
    }

    pub fn median(&self) -> Option<f32> {
        self.spread().map(|s| s.median)
    }
}

/// The events a measure is taken over.
fn over(r: &Report) -> Vec<&Event> {
    r.events.iter().collect()
}

pub fn population(r: &Report, measure: &Measure) -> Summary {
    summarize(over(r).into_iter().map(measure.get))
}

pub fn summarize(figs: impl Iterator<Item = Fig>) -> Summary {
    let (mut values, mut most, mut least, mut why) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for f in figs {
        match f {
            Fig::Value(v) if v.is_finite() => values.push(v),
            Fig::AtMost(v) if v.is_finite() => most.push(v),
            Fig::AtLeast(v) if v.is_finite() => least.push(v),
            Fig::No(w) => why.push(w),
            _ => {}
        }
    }
    let bounds = most.len() + least.len();
    if values.is_empty() && bounds == 0 {
        // The reason given most often.
        why.sort_unstable();
        let mut best = ("no events", 0usize);
        let mut i = 0;
        while i < why.len() {
            let j = (i..why.len()).find(|&j| why[j] != why[i]).unwrap_or(why.len());
            if j - i > best.1 {
                best = (why[i], j - i);
            }
            i = j;
        }
        return Summary::No(best.0);
    }
    if 2 * bounds > values.len() + bounds {
        let mid = |v: &[f32]| spread(v.iter().copied()).map_or(0.0, |s| s.median);
        return if most.len() >= least.len() {
            Summary::AtMost(mid(&most), most.len())
        } else {
            Summary::AtLeast(mid(&least), least.len())
        };
    }
    spread(values.into_iter()).map_or(Summary::No("no events"), Summary::Spread)
}

/// A number with as many decimals as its size leaves room for.
fn n(v: f32) -> String {
    let a = v.abs();
    if a >= 100.0 {
        format!("{v:.0}")
    } else if a >= 10.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.2}")
    }
}

fn with_unit(v: f32, unit: &str) -> String {
    if unit.is_empty() || unit == "r2" {
        n(v)
    } else {
        format!("{} {unit}", n(v))
    }
}

/// One event's figure in the per-event table.
fn fig_cell(f: Fig) -> String {
    match f {
        Fig::Value(v) => n(v),
        Fig::AtMost(v) => format!("<{}", n(v)),
        Fig::AtLeast(v) => format!(">{}", n(v)),
        Fig::No(_) => "-".into(),
    }
}

fn head(r: &Report, name: &str) -> String {
    format!(
        "{name}: {:.1} s, {} event{} in {:.0}-{:.0} Hz ({}), {} a second",
        r.seconds,
        r.events.len(),
        if r.events.len() == 1 { "" } else { "s" },
        r.band.0,
        r.band.1,
        if r.band_given { "the band given" } else { "where the onsets are" },
        n(r.events.len() as f32 / r.seconds.max(1e-3))
    )
}

fn shapes(r: &Report) -> String {
    let all = [Shape::Flat, Shape::Straight, Shape::FastThenSlow, Shape::SlowThenFast, Shape::Arch, Shape::Dip];
    let read: Vec<&Event> = r.events.iter().filter(|e| e.inside.bend.value().is_some()).collect();
    if read.is_empty() {
        return "not read: no event's pitch is measurable".into();
    }
    let total = read.len();
    let mut counts: Vec<(Shape, usize)> =
        all.iter().map(|&s| (s, read.iter().filter(|e| e.inside.shape == s).count())).filter(|c| c.1 > 0).collect();
    counts.sort_by_key(|c| std::cmp::Reverse(c.1));
    let falling = read.iter().filter(|e| e.inside.bend_st < -1.0).count();
    let rising = read.iter().filter(|e| e.inside.bend_st > 1.0).count();
    let words: Vec<String> = counts.iter().map(|(s, c)| format!("{} {}%", s.word(), c * 100 / total)).collect();
    format!(
        "{}; {}% fall and {}% rise by more than a semitone{}",
        words.join(", "),
        falling * 100 / total,
        rising * 100 / total,
        if total < r.events.len() { format!(" (of the {total} whose pitch is read)") } else { String::new() }
    )
}

fn wobbles(r: &Report) -> Option<(f32, f32, usize)> {
    let w: Vec<(f32, f32)> = r.events.iter().filter_map(|e| e.inside.wobble).collect();
    let rate = spread(w.iter().map(|w| w.0))?;
    let depth = spread(w.iter().map(|w| w.1))?;
    Some((rate.median, depth.median, w.len()))
}

/// The repeats as a delay: not when they are as loud as the event (a
/// pattern) or cannot be told from the notes' spacing.
fn delay_trusted(r: &Report) -> Option<&super::Delay> {
    r.delay.as_ref().filter(|d| d.ambiguous.is_none() && d.first_db <= -1.5)
}

fn delay_words(r: &Report) -> String {
    match &r.delay {
        None => format!("none found (looked for up to {:.1} s after each event)", super::inside::LONGEST_DELAY_S),
        Some(d) => {
            let mut s = format!("every {:.1} ms; the first {:.1} dB down", d.interval_ms, -d.first_db);
            if let Some(p) = d.per_repeat_db {
                let _ = write!(s, ", then {:.1} dB lost per repeat", -p);
            }
            let _ = write!(s, " (after {} of {} events)", d.confirmed, d.of);
            if let Some(why) = d.ambiguous {
                let _ = write!(s, ": ambiguous with the note grid ({why})");
            } else if d.first_db > -1.5 {
                s.push_str(": as loud as the event, so a pattern rather than an echo");
            }
            s
        }
    }
}

fn summary_cells(s: &Summary, unit: &str) -> (String, String, String) {
    match s {
        Summary::Spread(s) => (with_unit(s.median, unit), format!("{}..{}", n(s.low), n(s.high)), s.n.to_string()),
        Summary::AtMost(v, k) => (format!("< {}", with_unit(*v, unit)), "under its floor".into(), k.to_string()),
        Summary::AtLeast(v, k) => (format!("> {}", with_unit(*v, unit)), "past what is read".into(), k.to_string()),
        Summary::No(_) => (String::new(), String::new(), String::new()),
    }
}

/// What `tatum events <wav>` prints.
pub fn describe(r: &Report, name: &str, first: usize) -> String {
    let mut o = String::new();
    let _ = writeln!(o, "{}", head(r, name));
    if let Some(note) = r.note {
        let _ = writeln!(o, "  note: {note}");
    }
    if !r.events.is_empty() {
        let _ = writeln!(o, "  {:<26} {:>12}   {:<20} {:>4}", "", "median", "25%..75%", "n");
        for me in MEASURES {
            let s = population(r, me);
            if let Summary::No(why) = s {
                let _ = writeln!(o, "  {:<26} not measurable here: {why}", me.label);
                continue;
            }
            let (mid, range, count) = summary_cells(&s, me.unit);
            let _ = writeln!(o, "  {:<26} {:>12}   {:<20} {:>4}", me.label, mid, range, count);
        }
        let _ = writeln!(o, "  {:<26} {}", "pitch shape", shapes(r));
        if let Some((rate, depth, count)) = wobbles(r) {
            let _ = writeln!(
                o,
                "  {:<26} {:.1} Hz, {:.0} cents peak to peak (on {} of {} events)",
                "vibrato",
                rate,
                depth,
                count,
                r.events.len()
            );
        }
        let _ = writeln!(o, "  {:<26} {}", "repeats", delay_words(r));
        let alone = r.events.iter().filter(|e| e.inside.overlap.is_none()).count();
        if alone < r.events.len() {
            let _ = writeln!(
                o,
                "  {:<26} {} of {} events overlap another: purity, harmonics, centroid, tail and width are not read on those",
                "overlap",
                r.events.len() - alone,
                r.events.len()
            );
        }
        let _ = writeln!(o, "first {} events", first.min(r.events.len()));
        let _ = writeln!(
            o,
            "  {:>3} {:>8} {:>7} {:>7} {:>7} {:>7} {:>6} {:>7} {:>6} {:>6} {:>6} {:>6} {:>6} {:>6}",
            "#",
            "at s",
            "Hz",
            "bend st",
            "len ms",
            "att ms",
            "burst",
            "dB/ms",
            "Q",
            "tonal",
            "bed",
            "mix",
            "L-R",
            "tail"
        );
        for (i, e) in r.events.iter().take(first).enumerate() {
            let x = &e.inside;
            let _ = writeln!(
                o,
                "  {:>3} {:>8.3} {:>7} {:>7} {:>7} {:>7} {:>6} {:>7} {:>6} {:>6} {:>6.1} {:>6} {:>+6.1} {:>6}",
                i + 1,
                e.start_s,
                fig_cell(x.start.map(|v| v.round())),
                fig_cell(x.bend),
                fig_cell(length(e).map(|v| v.round())),
                fig_cell(x.attack_ms),
                fig_cell(x.burst_db),
                fig_cell(x.decay_db_ms),
                fig_cell(x.q.map(|v| v.round())),
                fig_cell(x.tonal_db),
                x.over_bed_db,
                fig_cell(x.in_mix_db),
                x.pan_db,
                fig_cell(x.tail_db)
            );
        }
    }
    if let Some(s) = &r.sustained {
        o.push_str(&describe_sustained(s));
    } else if r.events.is_empty() {
        let _ = writeln!(o, "  no events, and nothing that holds: silence, or under half a second of sound");
    }
    let _ = writeln!(o, "clean");
    for line in r.clean.lines() {
        let _ = writeln!(o, "  {line}");
    }
    o
}

fn hz_label(hz: f32) -> String {
    if hz >= 1000.0 {
        format!("{}k", n(hz / 1000.0).trim_end_matches('0').trim_end_matches('.'))
    } else {
        format!("{hz:.0}")
    }
}

fn describe_sustained(s: &Sustained) -> String {
    let mut o = String::new();
    let _ = writeln!(o, "sustained (partials read on the steadiest stretch, {:.1}-{:.1} s)", s.from_s, s.to_s);
    for chunk in s.thirds.chunks(10) {
        let _ = write!(o, "  third octaves ");
        for (hz, _) in chunk {
            let _ = write!(o, "{:>7}", hz_label(*hz));
        }
        let _ = write!(o, "\n  (dB of whole) ");
        for (_, v) in chunk {
            let _ = write!(o, "{v:>7.1}");
        }
        let _ = writeln!(o);
    }
    let _ = writeln!(
        o,
        "  partials over the rest     {:.1} dB; between them, {:.0} dB under the strongest (per bin)",
        s.hnr_db, s.noise_db
    );
    let _ = writeln!(
        o,
        "  note                       {}",
        s.f0.map_or("none: the partials are not multiples of one note".into(), |f| format!(
            "{} Hz ({})",
            n(f),
            note_name(f)
        ))
    );
    let _ = writeln!(o, "  {:>9} {:>7}   {:<22} pitch moves", "Hz", "dB", "level moves");
    for p in &s.partials {
        let rate = |r: Option<f32>| r.map_or(String::new(), |r| format!(" at {} Hz", n(r)));
        let level = format!("{} dB{}", n(p.am_db), rate(p.am_hz));
        let _ = writeln!(o, "  {:>9.1} {:>7.1}   {level:<22} {:.0} cents{}", p.hz, p.db, p.fm_cents, rate(p.fm_hz));
    }
    let beats: Vec<String> = s.beats.iter().map(|b| n(*b)).collect();
    let _ = writeln!(
        o,
        "  beats                      {}",
        if beats.is_empty() { "none".into() } else { format!("{} Hz", beats.join(", ")) }
    );
    let _ = writeln!(
        o,
        "  motion                     flux {:.3}; centroid {:.0} Hz, wandering {} st",
        s.flux,
        s.centroid_hz,
        n(s.centroid_move_st)
    );
    let _ = write!(o, "  width         ");
    for (hz, _, _) in &s.width {
        let _ = write!(o, "{:>7}", hz_label(*hz));
    }
    let _ = write!(o, "\n  (side-mid dB) ");
    for (_, v, _) in &s.width {
        let _ = write!(o, "{v:>7.1}");
    }
    let _ = write!(o, "\n  (correlation) ");
    for (_, _, c) in &s.width {
        let _ = write!(o, "{c:>7.2}");
    }
    let _ = writeln!(o);
    o
}

/// One row of the side-by-side table.
pub struct Row {
    pub key: String,
    pub label: String,
    pub unit: &'static str,
    pub a: Summary,
    pub b: Summary,
    /// B less A; for a ratio measure, B over A. Only between two spreads.
    pub difference: Option<f32>,
    pub ratio: bool,
    /// Off by enough to matter: outside A's own spread and by more than the
    /// measure's least, or, with few events, by more than its own error.
    pub flagged: bool,
    /// How many times the gate it is: what the advice is sorted by.
    pub weight: f32,
}

#[allow(clippy::too_many_arguments)]
fn row(
    key: &str,
    label: &str,
    unit: &'static str,
    a: Summary,
    b: Summary,
    least: f32,
    ratio: bool,
    error: f32,
) -> Row {
    let (av, bv) = (a.spread(), b.spread());
    let (difference, weight, flagged) = match (av, bv) {
        (Some(x), Some(y)) => {
            let few = x.n < FEW || y.n < FEW;
            // With few events there is no spread: only what is past the
            // estimator's own error counts.
            let outside = few || y.median < x.low || y.median > x.high;
            let gate = if few { error.max(least) } else { least };
            if ratio && x.median > 0.0 && y.median > 0.0 {
                let r = y.median / x.median;
                let w = r.ln().abs() / gate.ln();
                (Some(r), w, outside && w >= 1.0)
            } else {
                let d = y.median - x.median;
                let w = d.abs() / gate;
                (Some(d), w, outside && w >= 1.0)
            }
        }
        _ => (None, 0.0, false),
    };
    // Over silence both stand as far as the file's bit depth goes: 60 dB
    // against 40 says nothing about either sound.
    let alone = matches!(key, "over_bed_db" | "over_floor_db")
        && av.zip(bv).is_some_and(|(a, b)| a.median.min(b.median) > 35.0);
    let flagged = flagged && !alone;
    // With nothing on one side there is no ratio: the difference is shown.
    let ratio = ratio && av.zip(bv).is_some_and(|(a, b)| a.median > 0.0 && b.median > 0.0);
    Row { key: key.into(), label: label.into(), unit, a, b, difference, ratio, flagged, weight }
}

fn one(v: Option<f32>, why: &'static str) -> Summary {
    match v.filter(|v| v.is_finite()) {
        Some(v) => Summary::Spread(Spread { median: v, low: v, high: v, n: FEW }),
        None => Summary::No(why),
    }
}

/// Every measure of both files. A is the one B is held against.
pub fn rows(a: &Report, b: &Report) -> Vec<Row> {
    let mut out = Vec::new();
    if !a.events.is_empty() || !b.events.is_empty() {
        let per = |r: &Report| one(Some(r.events.len() as f32 / r.seconds.max(1e-3)), "no events");
        out.push(row("per_second", "events a second", "", per(a), per(b), 1.3, true, 1.3));
        for me in MEASURES {
            out.push(row(me.key, me.label, me.unit, population(a, me), population(b, me), me.least, me.ratio, me.error));
        }
        let (da, db) = (delay_trusted(a), delay_trusted(b));
        let why = "no repeats, or repeats that are a pattern or cannot be told from the notes";
        out.push(row(
            "repeat_ms",
            "repeats every",
            "ms",
            one(da.map(|d| d.interval_ms), why),
            one(db.map(|d| d.interval_ms), why),
            1.03,
            true,
            1.03,
        ));
        out.push(row(
            "repeat_first_db",
            "first repeat",
            "dB",
            one(da.map(|d| d.first_db), why),
            one(db.map(|d| d.first_db), why),
            3.0,
            false,
            3.0,
        ));
        out.push(row(
            "repeat_loss_db",
            "each repeat after",
            "dB",
            one(da.and_then(|d| d.per_repeat_db), why),
            one(db.and_then(|d| d.per_repeat_db), why),
            1.5,
            false,
            1.5,
        ));
    }
    if let (Some(x), Some(y)) = (&a.sustained, &b.sustained) {
        let mut add = |key: &str, label: &str, unit: &'static str, p: f32, q: f32, least: f32, ratio: bool| {
            out.push(row(key, label, unit, one(Some(p), "-"), one(Some(q), "-"), least, ratio, least));
        };
        add("hnr_db", "partials over the rest", "dB", x.hnr_db, y.hnr_db, 3.0, false);
        add("noise_db", "between the partials", "dB", x.noise_db, y.noise_db, 6.0, false);
        add("partials", "partials found", "", x.partials.len() as f32, y.partials.len() as f32, 3.0, false);
        let mid = |s: &Sustained, f: fn(&super::sustained::Partial) -> f32| {
            spread(s.partials.iter().map(f)).map_or(0.0, |s| s.median)
        };
        add("am_db", "partials' level swing", "dB", mid(x, |p| p.am_db), mid(y, |p| p.am_db), 2.0, false);
        add("fm_cents", "partials' pitch swing", "cents", mid(x, |p| p.fm_cents), mid(y, |p| p.fm_cents), 8.0, false);
        add("flux", "spectral flux", "", x.flux, y.flux, 1.5, true);
        add("centroid_hz", "centroid", "Hz", x.centroid_hz, y.centroid_hz, 1.25, true);
        add("centroid_move_st", "centroid wanders", "st", x.centroid_move_st, y.centroid_move_st, 1.0, false);
        for (p, q) in x.width.iter().zip(&y.width) {
            if p.1 > -60.0 || q.1 > -60.0 {
                add(
                    &format!("side_{:.0}", p.0),
                    &format!("side-mid at {} Hz", hz_label(p.0)),
                    "dB",
                    p.1,
                    q.1,
                    4.0,
                    false,
                );
            }
        }
        for (p, q) in x.thirds.iter().zip(&y.thirds) {
            if p.1 > -45.0 || q.1 > -45.0 {
                add(
                    &format!("third_{:.0}", p.0),
                    &format!("1/3 octave {} Hz", hz_label(p.0)),
                    "dB",
                    p.1,
                    q.1,
                    4.0,
                    false,
                );
            }
        }
    }
    out
}

fn name_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn summary_cell(s: &Summary, unit: &str) -> String {
    match s {
        Summary::Spread(s) => with_unit(s.median, unit),
        Summary::AtMost(v, _) => format!("< {}", with_unit(*v, unit)),
        Summary::AtLeast(v, _) => format!("> {}", with_unit(*v, unit)),
        Summary::No(_) => "n/m".into(),
    }
}

/// What `tatum events a.wav --against b.wav` prints. A is the reference:
/// its own spread is what a difference is held against, and the advice says
/// what to change in B.
pub fn against(a: &Report, b: &Report, name_a: &str, name_b: &str) -> String {
    let mut o = String::new();
    let _ = writeln!(o, "A  {}", head(a, name_a));
    let _ = writeln!(o, "B  {}", head(b, name_b));
    for (which, r) in [("A", a), ("B", b)] {
        if let Some(note) = r.note {
            let _ = writeln!(o, "   {which}: {note}");
        }
    }
    let rows = rows(a, b);
    let _ = writeln!(o, "{:<28} {:>12} {:>12} {:>12}", "", "A", "B", "B against A");
    let mut unread: Vec<String> = Vec::new();
    for r in &rows {
        let d = match r.difference {
            None => "-".to_string(),
            Some(d) if r.ratio => format!("x{}", n(d)),
            Some(d) => format!("{}{}", if d >= 0.0 { "+" } else { "" }, n(d)),
        };
        let _ = writeln!(
            o,
            "{:<28} {:>12} {:>12} {:>12}{}",
            r.label,
            summary_cell(&r.a, r.unit),
            summary_cell(&r.b, r.unit),
            d,
            if r.flagged { "  *" } else { "" }
        );
        for (which, s) in [("A", &r.a), ("B", &r.b)] {
            if let Summary::No(why) = s {
                if !r.key.starts_with("repeat_") {
                    unread.push(format!("{} ({which}): {why}", r.label));
                }
            }
        }
    }
    let few = a.events.len() < FEW || b.events.len() < FEW;
    let _ = writeln!(
        o,
        "  * {}; n/m: not measurable here; < and >: bounds, never advised on",
        if few {
            "fewer than 4 events on a side: off by more than the measure's own error on one event"
        } else {
            "outside A's own middle half, and by enough to matter"
        }
    );
    if !unread.is_empty() {
        let _ = writeln!(o, "not measurable here:");
        for u in unread {
            let _ = writeln!(o, "  {u}");
        }
    }
    if !a.events.is_empty() && !b.events.is_empty() {
        let _ = writeln!(o, "pitch shape   A  {}", shapes(a));
        let _ = writeln!(o, "              B  {}", shapes(b));
    }
    let lines = advice(a, b, &rows, name_of(name_b));
    let _ = writeln!(o);
    if lines.is_empty() {
        let _ = writeln!(o, "nothing differs by enough to matter");
    } else {
        let _ = writeln!(o, "what differs most, B against A:");
        for l in lines {
            let _ = writeln!(o, "  - {l}");
        }
    }
    for (which, r) in [("A", a), ("B", b)] {
        for f in r.clean.findings() {
            let _ = writeln!(o, "{which} is not clean: {f}");
        }
    }
    o
}

/// How an event's brightness goes as it falls, in words.
fn moves(v: f32) -> &'static str {
    if v < -1.0 {
        "darken"
    } else if v > 1.0 {
        "brighten"
    } else {
        "keep their brightness"
    }
}

/// What to change in B, from how A's (`x`) and B's (`y`) centroids move as
/// their events fall: by what each one does, not only by which is lower.
fn centroid_move_advice(x: f32, y: f32) -> &'static str {
    match (moves(x), moves(y)) {
        ("darken", "darken") if y < x => "both darken, B more: less filter envelope on B, or a slower one",
        ("darken", "darken") => "both darken, A more: more filter envelope on B, falling faster",
        ("darken", _) => "A darkens and B does not: a filter envelope on B, its cutoff falling with the level",
        (_, "darken") => "B darkens and A does not: less filter envelope on B, or none",
        ("brighten", "brighten") if y > x => {
            "both brighten, B more: B's low end dies faster than A's (less body, or a shorter low partial)"
        }
        ("brighten", "brighten") => "both brighten, A more: A's low end dies faster than its top",
        ("brighten", _) => "A brightens as it falls (its low end dies first) and B does not",
        (_, _) => "B brightens as it falls (its low end dies first) and A does not",
    }
}

/// The flagged rows as sentences, the furthest off first.
pub fn advice(a: &Report, b: &Report, rows: &[Row], _name_b: &str) -> Vec<String> {
    let mut flagged: Vec<&Row> = rows.iter().filter(|r| r.flagged).collect();
    flagged.sort_by(|x, y| y.weight.total_cmp(&x.weight));
    let has = |key: &str| flagged.iter().any(|r| r.key == key);
    let mut out = Vec::new();
    // The width, octave by octave, is one thing to say.
    let sides: Vec<&&Row> = flagged.iter().filter(|r| r.key.starts_with("side_")).collect();
    let mut said_width = false;
    for r in &flagged {
        let (Some(x), Some(y)) = (r.a.median(), r.b.median()) else { continue };
        if r.key.starts_with("side_") && sides.len() >= 3 {
            if !said_width {
                said_width = true;
                let mid = |f: fn(&Row) -> Option<f32>| {
                    spread(sides.iter().filter_map(|r| f(r))).map_or(0.0, |s| s.median)
                };
                let (wa, wb) = (mid(|r| r.a.median()), mid(|r| r.b.median()));
                out.push(format!(
                    "width: over {} octaves B's side sits {} dB against its mid, A's {} dB: {}",
                    sides.len(),
                    n(wb),
                    n(wa),
                    if wb < wa {
                        if wb < -60.0 {
                            "B is mono: a stereo reverb or chorus on it, voices panned apart"
                        } else {
                            "B is narrower: more stereo reverb or chorus, voices panned apart"
                        }
                    } else {
                        "B is wider: less stereo effect"
                    }
                ));
            }
            continue;
        }
        let more = y > x;
        let line = match r.key.as_str() {
            "over_bed_db" | "in_mix_db" if r.key == "in_mix_db" || !has("in_mix_db") => {
                let place = |v: f32| {
                    if v >= 0.0 {
                        format!("{} dB over", n(v))
                    } else {
                        format!("{} dB under", n(-v))
                    }
                };
                let what = if r.key == "in_mix_db" { "the mix around them" } else { "the bed in their own band" };
                format!(
                    "level: B's events sit {} {what}; A's sit {}: {}",
                    place(y),
                    place(x),
                    if more {
                        format!("take the part down about {:.0} dB, or put more under it", y - x)
                    } else {
                        format!("bring the part up about {:.0} dB", x - y)
                    }
                )
            }
            "attack_ms" => format!(
                "attack {} ms against {} ms: {}",
                n(y),
                n(x),
                if more {
                    "a shorter `attack`, or a harder exciter"
                } else {
                    "a longer `attack` on the envelope, or a low-pass on what strikes the resonator"
                }
            ),
            "burst_db" => format!(
                "the first 5 ms carry {} dB above the resonance, beyond what its own onset puts there, against {} dB: {}",
                n(y),
                n(x),
                if more {
                    "there is a click or noise at the start that A does not have; soften the exciter (low-pass it, less noise in it, a few ms of attack)"
                } else {
                    "A starts with more above its resonance; a brighter or noisier exciter"
                }
            ),
            "decay_db_ms" | "t20_ms" if r.key == "decay_db_ms" || !has("decay_db_ms") => {
                let unit = if r.key == "t20_ms" { "ms to -20 dB" } else { "dB/ms" };
                let longer = if r.key == "t20_ms" { more } else { !more };
                format!(
                    "decay {} {unit} against {}: B rings {}: {}",
                    n(y),
                    n(x),
                    if longer { "longer" } else { "shorter" },
                    if longer { "less `resonance`, or a shorter decay" } else { "more `resonance`, or a longer decay" }
                )
            }
            "ring_q" | "q" if r.key == "ring_q" || !has("ring_q") => format!(
                "resonance Q {:.0} against {:.0}{}: {}",
                y,
                x,
                if r.key == "ring_q" { " (from the decay)" } else { " (from the partial's width)" },
                if more { "lower the `resonance`" } else { "raise the `resonance`" }
            ),
            "tonal_db" => format!(
                "the partial stands {} dB over the rest of the event (bed and its own skirt taken off) against {} dB: {}",
                n(y),
                n(x),
                if more {
                    "B is purer than A: A's events carry more beside their partial (noise, other partials)"
                } else {
                    "B carries more beside its partial: less noise or drive in the exciter, fewer harmonics in the source"
                }
            ),
            "h2_db" | "h3_db" => format!(
                "{} harmonic at {} dB against {} dB: {}",
                if r.key == "h2_db" { "2nd" } else { "3rd" },
                n(y),
                n(x),
                if more {
                    "fewer harmonics in the source (a sine or triangle for a pulse or saw, less drive, a lower cutoff)"
                } else {
                    "A has more harmonics: a richer wave or some drive"
                }
            ),
            "off_centre_db" => format!(
                "B's events sit {} dB off centre, A's {} dB: {}",
                n(y),
                n(x),
                if more { "narrower panning" } else { "pan the events apart, or a ping-pong delay on them" }
            ),
            "side_mid_db" => format!(
                "side over mid in the events' band {} dB against {} dB: {}",
                n(y),
                n(x),
                if more {
                    "B is wider: less stereo spread on the events (chorus, a stereo delay, voices panned apart)"
                } else {
                    "B is narrower: more stereo spread on the events (chorus, a stereo delay, voices panned apart)"
                }
            ),
            "tail_db" => {
                let share = |v: f32| 100.0 * 10f32.powf(v / 10.0);
                if more {
                    format!(
                        "after its decay B keeps {:.1}% of an event's energy beyond what the decay itself leaves ({} dB), A {:.1}% ({} dB): less reverb or delay send, a shorter one, or no slow second stage",
                        share(y), n(y), share(x), n(x)
                    )
                } else {
                    format!(
                        "beyond its decay B keeps {:.1}% of an event's energy ({} dB), A {:.1}% ({} dB): more of the part in the reverb or delay",
                        share(y), n(y), share(x), n(x)
                    )
                }
            }
            "centroid_hz" => format!(
                "brightness: the events' centroid is {:.0} Hz against {:.0} Hz: {}",
                y,
                x,
                if more {
                    "B is brighter: a lower cutoff after the source, or less in the exciter"
                } else {
                    "B is darker: open the cutoff"
                }
            ),
            "centroid_move_st" => format!(
                "the centroid moves {} st over an event against {} st: B's events {} as they fall, A's {}: {}",
                n(y),
                n(x),
                moves(y),
                moves(x),
                centroid_move_advice(x, y)
            ),
            "abs_bend_st" => format!("bend {} st against {} st, either way", n(y), n(x)),
            "start_hz" => format!("start pitch {:.0} Hz against {:.0} Hz", y, x),
            "length_ms" => format!("length {:.0} ms against {:.0} ms", y, x),
            "per_second" => format!("{} events a second against {}", n(y), n(x)),
            "repeat_ms" => format!("repeats every {:.1} ms against {:.1} ms: the delay's time", y, x),
            "repeat_first_db" => {
                format!("the first repeat is {} dB down against {} dB: the delay's send", n(-y), n(-x))
            }
            "repeat_loss_db" => format!("each repeat loses {} dB against {} dB: the delay's feedback", n(-y), n(-x)),
            "hnr_db" => format!(
                "partials stand {} dB over what is between them against {} dB: {}",
                n(y),
                n(x),
                if more {
                    "A has more noise or more motion between its partials"
                } else {
                    "B is noisier between its partials"
                }
            ),
            "am_db" => format!(
                "the partials' level swings {} dB against {} dB: {}",
                n(y),
                n(x),
                if more {
                    "less tremolo, beating or filter motion"
                } else {
                    "A's partials move more: detune for beats, or an LFO on level or cutoff"
                }
            ),
            "fm_cents" => format!(
                "the partials' pitch swings {:.0} cents against {:.0}: {}",
                y,
                x,
                if more { "less vibrato or detune" } else { "more vibrato or detune" }
            ),
            "flux" => format!(
                "the spectrum changes {} a frame against {}: {}",
                n(y),
                n(x),
                if more { "B moves more than A" } else { "B is more static than A" }
            ),
            k if k.starts_with("third_") => format!(
                "{}: {} dB of the whole against {} dB: {} it about {:.0} dB",
                r.label,
                n(y),
                n(x),
                if more { "cut" } else { "lift" },
                (y - x).abs()
            ),
            k if k.starts_with("side_") => format!(
                "{}: {} dB against {} dB: {}",
                r.label,
                n(y),
                n(x),
                if more { "B is wider there: less stereo effect in that octave" } else { "B is narrower there: more stereo effect in that octave" }
            ),
            _ => continue,
        };
        out.push(line);
        if out.len() >= 12 {
            break;
        }
    }
    let enough = a.events.len() >= FEW && b.events.len() >= FEW;
    match (delay_trusted(a), delay_trusted(b)) {
        (Some(d), None) if enough && b.delay.is_none() => out.push(format!(
            "A repeats its events every {:.1} ms ({} dB down); B has no repeats: a delay at that time",
            d.interval_ms,
            n(-d.first_db)
        )),
        (None, Some(d)) if enough && a.delay.is_none() => {
            out.push(format!("B repeats its events every {:.1} ms; A has no repeats found", d.interval_ms))
        }
        _ => {}
    }
    out
}

// ── JSON ──

fn js(v: f32) -> String {
    if v.is_finite() {
        let s = format!("{v:.4}");
        let s = s.trim_end_matches('0').trim_end_matches('.');
        if s == "-0" {
            "0".into()
        } else {
            s.into()
        }
    } else {
        "null".into()
    }
}

fn jo(v: Option<f32>) -> String {
    v.map_or("null".into(), js)
}

fn jstr(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            c if (c as u32) < 0x20 => {
                let _ = write!(o, "\\u{:04x}", c as u32);
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// A summary as JSON: the spread, or `median: null` with the bound or the reason.
fn json_summary(s: &Summary, unit: &str) -> String {
    match s {
        Summary::Spread(s) => format!(
            "{{\"unit\":{},\"median\":{},\"p25\":{},\"p75\":{},\"n\":{}}}",
            jstr(unit),
            js(s.median),
            js(s.low),
            js(s.high),
            s.n
        ),
        Summary::AtMost(v, k) => {
            format!("{{\"unit\":{},\"median\":null,\"at_most\":{},\"n\":{k}}}", jstr(unit), js(*v))
        }
        Summary::AtLeast(v, k) => {
            format!("{{\"unit\":{},\"median\":null,\"at_least\":{},\"n\":{k}}}", jstr(unit), js(*v))
        }
        Summary::No(why) => format!("{{\"unit\":{},\"median\":null,\"why\":{},\"n\":0}}", jstr(unit), jstr(why)),
    }
}

fn json_report(r: &Report, name: &str) -> String {
    let mut o = String::from("{");
    let _ = write!(
        o,
        "\"file\":{},\"seconds\":{},\"band_hz\":[{},{}],\"band_given\":{},\"events_per_second\":{},\"note\":{}",
        jstr(name),
        js(r.seconds),
        js(r.band.0),
        js(r.band.1),
        r.band_given,
        js(r.events.len() as f32 / r.seconds.max(1e-3)),
        r.note.map_or("null".into(), jstr)
    );
    o.push_str(",\"measures\":{");
    for (i, me) in MEASURES.iter().enumerate() {
        let _ = write!(
            o,
            "{}{}:{}",
            if i == 0 { "" } else { "," },
            jstr(me.key),
            json_summary(&population(r, me), me.unit)
        );
    }
    o.push('}');
    match wobbles(r) {
        Some((rate, depth, count)) => {
            let _ = write!(o, ",\"vibrato\":{{\"hz\":{},\"cents\":{},\"events\":{count}}}", js(rate), js(depth));
        }
        None => o.push_str(",\"vibrato\":null"),
    }
    match &r.delay {
        Some(d) => {
            let _ = write!(
                o,
                ",\"repeats\":{{\"interval_ms\":{},\"first_db\":{},\"per_repeat_db\":{},\"confirmed\":{},\"of\":{},\"pattern\":{},\"ambiguous\":{}}}",
                js(d.interval_ms),
                js(d.first_db),
                jo(d.per_repeat_db),
                d.confirmed,
                d.of,
                d.first_db > -1.5,
                d.ambiguous.map_or("null".into(), jstr)
            );
        }
        None => o.push_str(",\"repeats\":null"),
    }
    o.push_str(",\"events\":[");
    for (i, e) in r.events.iter().enumerate() {
        let _ = write!(
            o,
            "{}{{\"start_s\":{},\"end_s\":{},\"shape\":{},\"tail_clear\":{},\"overlap\":{}",
            if i == 0 { "" } else { "," },
            js(e.start_s),
            js(e.end_s),
            jstr(e.inside.shape.word()),
            e.inside.tail_clear,
            e.inside.overlap.map_or("null".into(), jstr)
        );
        let mut limits = Vec::new();
        for me in MEASURES {
            let f = (me.get)(e);
            let _ = write!(o, ",{}:{}", jstr(me.key), jo(f.value()));
            match f {
                Fig::Value(_) => {}
                Fig::AtMost(v) => limits.push(format!("{}:{{\"at_most\":{}}}", jstr(me.key), js(v))),
                Fig::AtLeast(v) => limits.push(format!("{}:{{\"at_least\":{}}}", jstr(me.key), js(v))),
                Fig::No(why) => limits.push(format!("{}:{{\"why\":{}}}", jstr(me.key), jstr(why))),
            }
        }
        let _ = write!(o, ",\"limits\":{{{}}}", limits.join(","));
        match e.inside.wobble {
            Some((rate, depth)) => {
                let _ = write!(o, ",\"vibrato_hz\":{},\"vibrato_cents\":{}", js(rate), js(depth));
            }
            None => o.push_str(",\"vibrato_hz\":null,\"vibrato_cents\":null"),
        }
        o.push('}');
    }
    o.push(']');
    match &r.sustained {
        None => o.push_str(",\"sustained\":null"),
        Some(s) => {
            let _ = write!(
                o,
                ",\"sustained\":{{\"from_s\":{},\"to_s\":{},\"hnr_db\":{},\"noise_db\":{},\"f0_hz\":{},\"flux\":{},\"centroid_hz\":{},\"centroid_move_st\":{}",
                js(s.from_s),
                js(s.to_s),
                js(s.hnr_db),
                js(s.noise_db),
                jo(s.f0),
                js(s.flux),
                js(s.centroid_hz),
                js(s.centroid_move_st)
            );
            let list = |v: Vec<String>| format!("[{}]", v.join(","));
            let _ = write!(
                o,
                ",\"third_octaves\":{}",
                list(s.thirds.iter().map(|t| format!("{{\"hz\":{},\"db\":{}}}", js(t.0), js(t.1))).collect())
            );
            let _ = write!(
                o,
                ",\"partials\":{}",
                list(
                    s.partials
                        .iter()
                        .map(|p| format!(
                            "{{\"hz\":{},\"db\":{},\"am_db\":{},\"am_hz\":{},\"fm_cents\":{},\"fm_hz\":{}}}",
                            js(p.hz),
                            js(p.db),
                            js(p.am_db),
                            jo(p.am_hz),
                            js(p.fm_cents),
                            jo(p.fm_hz)
                        ))
                        .collect()
                )
            );
            let _ = write!(o, ",\"beats_hz\":{}", list(s.beats.iter().map(|b| js(*b)).collect()));
            let _ = write!(
                o,
                ",\"width\":{}}}",
                list(
                    s.width
                        .iter()
                        .map(|w| format!(
                            "{{\"hz\":{},\"side_mid_db\":{},\"correlation\":{}}}",
                            js(w.0),
                            js(w.1),
                            js(w.2)
                        ))
                        .collect()
                )
            );
        }
    }
    let c = &r.clean;
    let _ = write!(
        o,
        ",\"clean\":{{\"dc_db\":{},\"aliasing_looked_for\":{},\"aliasing_unread\":{}",
        js(c.dc_db),
        c.alias_unread.is_none(),
        c.alias_unread.map_or("null".into(), jstr)
    );
    match &c.alias {
        Some(a) => {
            let _ = write!(
                o,
                ",\"aliasing\":{{\"db\":{},\"f0_hz\":{},\"at_s\":{},\"harmonic\":{},\"lands_hz\":{},\"harmonic_db\":{},\"lines\":{}}}",
                js(a.db),
                js(a.f0),
                js(a.at_s),
                a.worst.0,
                js(a.worst.1),
                js(a.worst.2),
                a.lines
            );
        }
        None => o.push_str(",\"aliasing\":null"),
    }
    match &c.alias_floor {
        Some((examined, floor)) => {
            let _ = write!(o, ",\"aliasing_examined\":{{\"folds\":{examined},\"floor_db\":{}}}", js(*floor));
        }
        None => o.push_str(",\"aliasing_examined\":null"),
    }
    let steps: Vec<String> =
        c.steps.iter().map(|s| format!("{{\"at_s\":{},\"db\":{}}}", js(s.at_s), js(s.db))).collect();
    let _ = write!(o, ",\"steps\":[{}]", steps.join(","));
    match &c.zipper {
        Some(z) => {
            let _ = write!(o, ",\"zipper\":{{\"period_samples\":{},\"db\":{}}}", z.period, js(z.db));
        }
        None => o.push_str(",\"zipper\":null"),
    }
    match &c.buzz {
        Some(b) => {
            let _ = write!(o, ",\"quiet\":{{\"hz\":{},\"db\":{},\"at_s\":{}}}", js(b.hz), js(b.db), js(b.at_s));
        }
        None => o.push_str(",\"quiet\":null"),
    }
    let found: Vec<String> = c.findings().iter().map(|f| jstr(f)).collect();
    let _ = write!(o, ",\"findings\":[{}]}}", found.join(","));
    o.push('}');
    o
}

/// One file as JSON.
pub fn json(r: &Report, name: &str) -> String {
    let mut o = json_report(r, name);
    o.push('\n');
    o
}

/// Two files as JSON: each as `json` gives it, the rows of the table, and the advice.
pub fn json_against(a: &Report, b: &Report, name_a: &str, name_b: &str) -> String {
    let rows = rows(a, b);
    let mut o = format!("{{\"a\":{},\"b\":{},\"rows\":[", json_report(a, name_a), json_report(b, name_b));
    let bound = |s: &Summary| match s {
        Summary::Spread(_) => "null".to_string(),
        Summary::AtMost(v, _) => format!("{{\"at_most\":{}}}", js(*v)),
        Summary::AtLeast(v, _) => format!("{{\"at_least\":{}}}", js(*v)),
        Summary::No(why) => format!("{{\"why\":{}}}", jstr(why)),
    };
    for (i, r) in rows.iter().enumerate() {
        let _ = write!(
            o,
            "{}{{\"key\":{},\"label\":{},\"unit\":{},\"a\":{},\"b\":{},\"a_limit\":{},\"b_limit\":{},\"{}\":{},\"flagged\":{}}}",
            if i == 0 { "" } else { "," },
            jstr(&r.key),
            jstr(&r.label),
            jstr(r.unit),
            jo(r.a.median()),
            jo(r.b.median()),
            bound(&r.a),
            bound(&r.b),
            if r.ratio { "b_over_a" } else { "b_less_a" },
            jo(r.difference),
            r.flagged
        );
    }
    let lines: Vec<String> = advice(a, b, &rows, name_of(name_b)).iter().map(|l| jstr(l)).collect();
    let _ = writeln!(o, "],\"advice\":[{}]}}", lines.join(","));
    o
}
