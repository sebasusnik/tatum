//! What `tatum events` prints: one file's events and their spread, or two
//! files side by side with what differs most, as text or as JSON.

use std::fmt::Write as _;

use super::inside::Shape;
use super::sustained::Sustained;
use super::{spread, Event, Report, Spread};
use crate::reference::note_name;

/// One thing measured on every event.
pub struct Measure {
    pub key: &'static str,
    pub label: &'static str,
    pub unit: &'static str,
    pub get: fn(&Event) -> Option<f32>,
    /// The smallest difference between two files worth a line: in the
    /// measure's own unit, or as a ratio when `ratio`.
    pub least: f32,
    pub ratio: bool,
}

const fn m(
    key: &'static str,
    label: &'static str,
    unit: &'static str,
    get: fn(&Event) -> Option<f32>,
    least: f32,
    ratio: bool,
) -> Measure {
    Measure { key, label, unit, get, least, ratio }
}

pub const MEASURES: &[Measure] = &[
    m("over_floor_db", "over its floor", "dB", |e| Some(e.over_floor_db), 4.0, false),
    m("length_ms", "length", "ms", |e| Some((e.end_s - e.start_s) * 1000.0), 1.3, true),
    m("attack_ms", "attack 10-90%", "ms", |e| Some(e.inside.attack_ms), 1.5, true),
    m("burst_db", "burst over the resonance", "dB", |e| e.inside.burst_db, 4.0, false),
    m("t20_ms", "fall to -20 dB", "ms", |e| e.inside.t20_ms, 1.3, true),
    m("t40_ms", "fall to -40 dB", "ms", |e| e.inside.t40_ms, 1.3, true),
    m("decay_db_ms", "decay", "dB/ms", |e| e.inside.decay_db_ms, 1.3, true),
    m("decay_r2", "decay straightness", "r2", |e| e.inside.decay_r2, 0.1, false),
    m("start_hz", "start pitch", "Hz", |e| Some(e.inside.start_hz), 1.12, true),
    m("end_hz", "end pitch", "Hz", |e| Some(e.inside.end_hz), 1.12, true),
    m("bend_st", "bend", "st", |e| Some(e.inside.bend_st), 1.0, false),
    m("abs_bend_st", "bend, either way", "st", |e| Some(e.inside.bend_st.abs()), 1.0, false),
    m("bandwidth_hz", "-3 dB width", "Hz", |e| Some(e.inside.bandwidth_hz), 1.4, true),
    m("q", "Q (f / width)", "", |e| Some(e.inside.q), 1.4, true),
    m("ring_q", "Q (from the decay)", "", |e| e.inside.ring_q, 1.4, true),
    m("tonal_db", "partial over the rest", "dB", |e| Some(e.inside.tonal_db), 3.0, false),
    m("h2_db", "2nd harmonic", "dB", |e| e.inside.h2_db, 5.0, false),
    m("h3_db", "3rd harmonic", "dB", |e| e.inside.h3_db, 5.0, false),
    m("over_bed_db", "over the bed in its band", "dB", |e| Some(e.inside.over_bed_db), 4.0, false),
    m("in_mix_db", "against the whole mix", "dB", |e| Some(e.inside.in_mix_db), 3.0, false),
    m("pan_db", "left over right", "dB", |e| Some(e.inside.pan_db), 2.0, false),
    m("off_centre_db", "off centre, either side", "dB", |e| Some(e.inside.pan_db.abs()), 2.0, false),
    m("corr", "left/right correlation", "", |e| Some(e.inside.corr), 0.15, false),
    m("tail_db", "tail, 50-400 ms after", "dB", |e| e.inside.tail_db, 4.0, false),
    m("centroid_hz", "centroid at the peak", "Hz", |e| Some(e.inside.centroid_hz), 1.25, true),
    m("centroid_move_st", "centroid by -12 dB", "st", |e| e.inside.centroid_move_st, 2.0, false),
];

/// The events a measure is taken over: every one, but for the tail, which
/// is read on the events nothing near their pitch follows when there are
/// three of those.
fn over<'a>(r: &'a Report, key: &str) -> Vec<&'a Event> {
    if key == "tail_db" && tails_clear(r) {
        r.events.iter().filter(|e| e.inside.tail_clear).collect()
    } else {
        r.events.iter().collect()
    }
}

fn tails_clear(r: &Report) -> bool {
    r.events.iter().filter(|e| e.inside.tail_clear && e.inside.tail_db.is_some()).count() >= 3.min(r.events.len())
}

pub fn population(r: &Report, measure: &Measure) -> Option<Spread> {
    spread(over(r, measure.key).into_iter().filter_map(measure.get))
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
    let total = r.events.len().max(1);
    let mut counts: Vec<(Shape, usize)> =
        all.iter().map(|&s| (s, r.events.iter().filter(|e| e.inside.shape == s).count())).filter(|c| c.1 > 0).collect();
    counts.sort_by_key(|c| std::cmp::Reverse(c.1));
    let falling = r.events.iter().filter(|e| e.inside.bend_st < -1.0).count();
    let rising = r.events.iter().filter(|e| e.inside.bend_st > 1.0).count();
    let words: Vec<String> = counts.iter().map(|(s, c)| format!("{} {}%", s.word(), c * 100 / total)).collect();
    format!(
        "{}; {}% fall and {}% rise by more than a semitone",
        words.join(", "),
        falling * 100 / total,
        rising * 100 / total
    )
}

fn wobbles(r: &Report) -> Option<(f32, f32, usize)> {
    let w: Vec<(f32, f32)> = r.events.iter().filter_map(|e| e.inside.wobble).collect();
    let rate = spread(w.iter().map(|w| w.0))?;
    let depth = spread(w.iter().map(|w| w.1))?;
    Some((rate.median, depth.median, w.len()))
}

fn delay_words(r: &Report) -> String {
    match &r.delay {
        None => "none found".into(),
        Some(d) => {
            let mut s = format!("every {:.1} ms; the first {:.1} dB down", d.interval_ms, -d.first_db);
            if let Some(p) = d.per_repeat_db {
                let _ = write!(s, ", then {:.1} dB lost per repeat", -p);
            }
            let _ = write!(s, " (after {} of {} events)", d.confirmed, d.of);
            if d.first_db > -1.5 {
                s.push_str(": as loud as the event, so a pattern rather than an echo");
            }
            s
        }
    }
}

/// What `tatum events <wav>` prints.
pub fn describe(r: &Report, name: &str, first: usize) -> String {
    let mut o = String::new();
    let _ = writeln!(o, "{}", head(r, name));
    if !r.events.is_empty() {
        let _ = writeln!(o, "  {:<26} {:>10}   {:<20} {:>4}", "", "median", "25%..75%", "n");
        for me in MEASURES {
            if let Some(s) = population(r, me) {
                let _ = writeln!(
                    o,
                    "  {:<26} {:>10}   {:<20} {:>4}",
                    me.label,
                    with_unit(s.median, me.unit),
                    format!("{}..{}", n(s.low), n(s.high)),
                    s.n
                );
            }
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
        let clear = r.events.iter().filter(|e| e.inside.tail_clear).count();
        let _ = writeln!(
            o,
            "  {:<26} {}",
            "tail read on",
            if tails_clear(r) {
                format!("the {clear} events nothing near their pitch follows within 400 ms")
            } else {
                "every event, though others near their pitch start inside the 400 ms: read it as an upper bound".into()
            }
        );
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
        let dash = |v: Option<f32>| v.map_or("-".to_string(), n);
        for (i, e) in r.events.iter().take(first).enumerate() {
            let x = &e.inside;
            let _ = writeln!(
                o,
                "  {:>3} {:>8.3} {:>7.0} {:>+7.1} {:>7.0} {:>7} {:>6} {:>7} {:>6.0} {:>6.1} {:>6.1} {:>6.1} {:>+6.1} {:>6}",
                i + 1,
                e.start_s,
                x.start_hz,
                x.bend_st,
                (e.end_s - e.start_s) * 1000.0,
                n(x.attack_ms),
                dash(x.burst_db),
                dash(x.decay_db_ms),
                x.q,
                x.tonal_db,
                x.over_bed_db,
                x.in_mix_db,
                x.pan_db,
                dash(x.tail_db)
            );
        }
    }
    if let Some(s) = &r.sustained {
        o.push_str(&describe_sustained(s));
    } else if r.events.is_empty() {
        let _ = writeln!(o, "  no events, and nothing that holds: silence, or under half a second of sound");
    }
    let _ = writeln!(o, "clean");
    let found = r.clean.findings();
    for f in &found {
        let _ = writeln!(o, "  {f}");
    }
    if found.is_empty() {
        let _ = writeln!(
            o,
            "  nothing found: DC {:.0} dB under the signal, no steps, no zipper, nothing in the quiet{}",
            r.clean.dc_db,
            if r.clean.alias_unread.is_none() { ", nothing folded back" } else { "" }
        );
    }
    if let Some(why) = r.clean.alias_unread {
        let _ = writeln!(o, "  aliasing not looked for: {why}");
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
    pub a: Option<f32>,
    pub b: Option<f32>,
    /// B less A; for a ratio measure, B over A.
    pub difference: Option<f32>,
    pub ratio: bool,
    /// Outside A's own spread and by more than the measure's least.
    pub flagged: bool,
    /// How many times the least difference it is: what the advice is sorted by.
    pub weight: f32,
}

fn row(
    key: &str,
    label: &str,
    unit: &'static str,
    a: Option<Spread>,
    b: Option<Spread>,
    least: f32,
    ratio: bool,
) -> Row {
    let (av, bv) = (a.map(|s| s.median), b.map(|s| s.median));
    let (difference, weight, flagged) = match (a, b) {
        (Some(a), Some(b)) => {
            let outside = b.median < a.low || b.median > a.high;
            if ratio && a.median > 0.0 && b.median > 0.0 {
                let r = b.median / a.median;
                let w = r.ln().abs() / least.ln();
                (Some(r), w, outside && w >= 1.0)
            } else {
                let d = b.median - a.median;
                let w = d.abs() / least;
                (Some(d), w, outside && w >= 1.0)
            }
        }
        _ => (None, 0.0, false),
    };
    // Over silence both stand as far as the file's bit depth goes: 60 dB
    // against 40 says nothing about either sound.
    let alone = matches!(key, "over_bed_db" | "over_floor_db") && av.zip(bv).is_some_and(|(a, b)| a.min(b) > 35.0);
    let flagged = flagged && !alone;
    // With nothing on one side there is no ratio: the difference is shown.
    let ratio = ratio && av.zip(bv).is_some_and(|(a, b)| a > 0.0 && b > 0.0);
    Row { key: key.into(), label: label.into(), unit, a: av, b: bv, difference, ratio, flagged, weight }
}

fn one(v: Option<f32>) -> Option<Spread> {
    v.filter(|v| v.is_finite()).map(|v| Spread { median: v, low: v, high: v, n: 1 })
}

/// Every measure of both files. A is the one B is held against.
pub fn rows(a: &Report, b: &Report) -> Vec<Row> {
    let mut out = Vec::new();
    if !a.events.is_empty() || !b.events.is_empty() {
        let per = |r: &Report| one(Some(r.events.len() as f32 / r.seconds.max(1e-3)));
        out.push(row("per_second", "events a second", "", per(a), per(b), 1.3, true));
        for me in MEASURES {
            out.push(row(me.key, me.label, me.unit, population(a, me), population(b, me), me.least, me.ratio));
        }
        let (da, db) = (a.delay.as_ref(), b.delay.as_ref());
        out.push(row(
            "repeat_ms",
            "repeats every",
            "ms",
            one(da.map(|d| d.interval_ms)),
            one(db.map(|d| d.interval_ms)),
            1.03,
            true,
        ));
        out.push(row(
            "repeat_first_db",
            "first repeat",
            "dB",
            one(da.map(|d| d.first_db)),
            one(db.map(|d| d.first_db)),
            3.0,
            false,
        ));
        out.push(row(
            "repeat_loss_db",
            "each repeat after",
            "dB",
            one(da.and_then(|d| d.per_repeat_db)),
            one(db.and_then(|d| d.per_repeat_db)),
            1.5,
            false,
        ));
    }
    if let (Some(x), Some(y)) = (&a.sustained, &b.sustained) {
        let mut add = |key: &str, label: &str, unit: &'static str, p: f32, q: f32, least: f32, ratio: bool| {
            out.push(row(key, label, unit, one(Some(p)), one(Some(q)), least, ratio));
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

/// What `tatum events a.wav --against b.wav` prints. A is the reference:
/// its own spread is what a difference is held against, and the advice says
/// what to change in B.
pub fn against(a: &Report, b: &Report, name_a: &str, name_b: &str) -> String {
    let mut o = String::new();
    let _ = writeln!(o, "A  {}", head(a, name_a));
    let _ = writeln!(o, "B  {}", head(b, name_b));
    let rows = rows(a, b);
    let _ = writeln!(o, "{:<28} {:>12} {:>12} {:>12}", "", "A", "B", "B against A");
    let cell = |v: Option<f32>, unit: &str| v.map_or("-".into(), |v| with_unit(v, unit));
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
            cell(r.a, r.unit),
            cell(r.b, r.unit),
            d,
            if r.flagged { "  *" } else { "" }
        );
    }
    let _ = writeln!(o, "  * outside A's own middle half, and by enough to matter");
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
        let (Some(x), Some(y)) = (r.a, r.b) else { continue };
        if r.key.starts_with("side_") && sides.len() >= 3 {
            if !said_width {
                said_width = true;
                let mid =
                    |f: fn(&Row) -> Option<f32>| spread(sides.iter().filter_map(|r| f(r))).map_or(0.0, |s| s.median);
                let (wa, wb) = (mid(|r| r.a), mid(|r| r.b));
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
                "the first 5 ms carry {} dB above the resonance against {} dB: {}",
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
                "the partial stands {} dB over the rest of the event against {} dB: {}",
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
            "corr" => format!(
                "left/right correlation of an event {} against {}: {}",
                n(y),
                n(x),
                if more {
                    "B is narrower: more of it in a stereo reverb or chorus"
                } else {
                    "B is wider: less stereo effect on the events"
                }
            ),
            "tail_db" => {
                let share = |v: f32| 100.0 * 10f32.powf(v / 10.0);
                if more {
                    format!(
                        "after its events B keeps {:.0}% of an event's energy in the next 400 ms ({} dB), A {:.0}% ({} dB): less reverb or delay send, or a shorter one",
                        share(y), n(y), share(x), n(x)
                    )
                } else {
                    format!(
                        "little tail after B's events ({:.0}% of an event's energy, {} dB): A keeps {:.0}% in the 400 ms after ({} dB): more of the part in the reverb or delay",
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
                "the centroid moves {} st over an event against {} st: {}",
                n(y),
                n(x),
                if y < x {
                    "B's events darken more as they fall than A's"
                } else {
                    "A's events darken as they fall; a filter envelope"
                }
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
                if more { "narrower there" } else { "wider there" }
            ),
            _ => continue,
        };
        out.push(line);
        if out.len() >= 12 {
            break;
        }
    }
    match (&a.delay, &b.delay) {
        (Some(d), None) if !a.events.is_empty() && !b.events.is_empty() => out.push(format!(
            "A repeats its events every {:.1} ms ({} dB down); B has no repeats: a delay at that time",
            d.interval_ms,
            n(-d.first_db)
        )),
        (None, Some(d)) => {
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

fn json_report(r: &Report, name: &str) -> String {
    let mut o = String::from("{");
    let _ = write!(
        o,
        "\"file\":{},\"seconds\":{},\"band_hz\":[{},{}],\"band_given\":{},\"events_per_second\":{}",
        jstr(name),
        js(r.seconds),
        js(r.band.0),
        js(r.band.1),
        r.band_given,
        js(r.events.len() as f32 / r.seconds.max(1e-3))
    );
    o.push_str(",\"measures\":{");
    let mut first = true;
    for me in MEASURES {
        if let Some(s) = population(r, me) {
            let _ = write!(
                o,
                "{}{}:{{\"unit\":{},\"median\":{},\"p25\":{},\"p75\":{},\"n\":{}}}",
                if first { "" } else { "," },
                jstr(me.key),
                jstr(me.unit),
                js(s.median),
                js(s.low),
                js(s.high),
                s.n
            );
            first = false;
        }
    }
    o.push('}');
    let _ = write!(o, ",\"tail_read_on_clear_events\":{}", tails_clear(r));
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
                ",\"repeats\":{{\"interval_ms\":{},\"first_db\":{},\"per_repeat_db\":{},\"confirmed\":{},\"of\":{}}}",
                js(d.interval_ms),
                js(d.first_db),
                jo(d.per_repeat_db),
                d.confirmed,
                d.of
            );
        }
        None => o.push_str(",\"repeats\":null"),
    }
    o.push_str(",\"events\":[");
    for (i, e) in r.events.iter().enumerate() {
        let _ = write!(
            o,
            "{}{{\"start_s\":{},\"end_s\":{},\"shape\":{},\"tail_clear\":{}",
            if i == 0 { "" } else { "," },
            js(e.start_s),
            js(e.end_s),
            jstr(e.inside.shape.word()),
            e.inside.tail_clear
        );
        for me in MEASURES {
            let _ = write!(o, ",{}:{}", jstr(me.key), jo((me.get)(e)));
        }
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
    let _ = write!(o, ",\"clean\":{{\"dc_db\":{},\"aliasing_looked_for\":{}", js(c.dc_db), c.alias_unread.is_none());
    match &c.alias {
        Some(a) => {
            let _ = write!(
                o,
                ",\"aliasing\":{{\"db\":{},\"f0_hz\":{},\"at_s\":{},\"harmonic\":{},\"lands_hz\":{},\"harmonic_db\":{}}}",
                js(a.db),
                js(a.f0),
                js(a.at_s),
                a.worst.0,
                js(a.worst.1),
                js(a.worst.2)
            );
        }
        None => o.push_str(",\"aliasing\":null"),
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
    for (i, r) in rows.iter().enumerate() {
        let _ = write!(
            o,
            "{}{{\"key\":{},\"label\":{},\"unit\":{},\"a\":{},\"b\":{},\"{}\":{},\"flagged\":{}}}",
            if i == 0 { "" } else { "," },
            jstr(&r.key),
            jstr(&r.label),
            jstr(r.unit),
            jo(r.a),
            jo(r.b),
            if r.ratio { "b_over_a" } else { "b_less_a" },
            jo(r.difference),
            r.flagged
        );
    }
    let lines: Vec<String> = advice(a, b, &rows, name_of(name_b)).iter().map(|l| jstr(l)).collect();
    let _ = writeln!(o, "],\"advice\":[{}]}}", lines.join(","));
    o
}
