//! `tatum debug`: a song taken apart, to find where a noise comes from.
//!
//! One render of the song with the engine's taps on ([`SongEngine::set_taps`]),
//! so every part is heard exactly as it sits in the mix -- the sidechain
//! pumping it, the bus compressor leaning on it -- rather than rendered again
//! alone, where it would behave differently. Out of that come, per part, a
//! WAV to listen to and a spectrogram to look at; a sheet with every part
//! stacked on one time axis and the mix at the bottom; and a report of what
//! the parts do that they should not.
//!
//! It exists because finding a noise by ear in a full mix took hours, and it
//! is built to be read by the model writing the song as much as by the
//! person listening: the sheet is a picture it can open, and the report says
//! where to look.

pub mod draw;
pub mod fft;
pub mod listen;
pub mod mixing;
pub mod picture;
pub mod spectrogram;
pub mod wav;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use tatum_core::dsl::compiler::CompiledSong;
pub use tatum_core::dsl::isolate::Isolation;
use tatum_core::song_engine::{SongEngine, Taps};
use tatum_core::{BLOCK_SIZE, SAMPLE_RATE};

use draw::{Marks, Ruler, Strip};
use listen::{Click, Floor, Frames, FRAME};
use spectrogram::Spectrogram;
use wav::WavWriter;

pub struct Options {
    /// Also keep each track before its insert chain, next to it.
    pub dry: bool,
    /// First and last bar, counted from 1, both included. `None` is the song.
    pub bars: Option<(u32, u32)>,
    pub out_dir: PathBuf,
    /// Width of the time axis in pixels.
    pub width: usize,
    /// The engine's output gain; see `tatum_core::dsl::isolate::output_gain`.
    /// The parts are taken before it, the mix after.
    pub gain: f32,
}

impl Options {
    pub fn new(out_dir: PathBuf) -> Options {
        Options { dry: false, bars: None, out_dir, width: 1400, gain: 1.0 }
    }
}

pub struct Outcome {
    pub report: String,
    pub sheet: PathBuf,
}

/// Where a part's signal comes from.
#[derive(Clone, Copy, PartialEq)]
enum Source {
    Track(usize),
    Dry(usize),
    Bus(usize),
    Delay,
    Reverb,
    Mix,
}

struct Part {
    /// As drawn and reported: `bass`, `bass dry`, `bus drums`, `mix`.
    label: String,
    /// File name without extension.
    file: String,
    source: Source,
    /// Not a drum track. Drums fire and forget, so "between notes" means
    /// nothing for them, and a hat is meant to be bright.
    tonal: bool,
    wav: Option<WavWriter>,
    spec: Spectrogram,
    frames: Frames,
    peak: f32,
    sum_sq: f64,
    sum: f64,
    samples: u64,
}

impl Part {
    fn push(&mut self, l: &[f32], r: &[f32]) -> Result<(), String> {
        if let Some(w) = self.wav.as_mut() { w.push(l, r)?; }
        for (a, b) in l.iter().zip(r) {
            let m = (a + b) * 0.5;
            self.spec.push(m);
            self.frames.push(m);
            self.peak = self.peak.max(a.abs()).max(b.abs());
            self.sum_sq += (m * m) as f64;
            self.sum += m as f64;
        }
        self.samples += l.len() as u64;
        Ok(())
    }

    fn silent(&self) -> bool { self.peak < 1e-5 }

    fn rms(&self) -> f32 {
        if self.samples == 0 { 0.0 } else { (self.sum_sq / self.samples as f64).sqrt() as f32 }
    }
}

/// Where the bars fall, in samples from the start of the song.
struct Clock {
    bar_starts: Vec<usize>,
    sections: Vec<(String, u32)>,
}

impl Clock {
    fn new(engine: &SongEngine) -> Clock {
        let beats_per_bar = engine.steps_per_bar() as f32 / 4.0;
        let bar_len = |bpm: f32| SAMPLE_RATE * 60.0 / bpm * beats_per_bar;
        let mut sections: Vec<(String, u32, f32)> = engine.sections().into_iter()
            .map(|(n, b, bpm)| (n.to_string(), b, bpm))
            .collect();
        if sections.is_empty() {
            // A rig without an arrangement plays its tracks; four bars shows them.
            sections.push((String::new(), 4, engine.tempo()));
        }
        let mut bar_starts = vec![0usize];
        let mut at = 0.0f64;
        let mut named = Vec::new();
        for (name, bars, bpm) in &sections {
            named.push((name.clone(), bar_starts.len() as u32));
            for _ in 0..*bars {
                at += bar_len(*bpm) as f64;
                bar_starts.push(at.round() as usize);
            }
        }
        Clock { bar_starts, sections: named }
    }

    fn bars(&self) -> u32 { self.bar_starts.len() as u32 - 1 }

    /// "bar 17.3 (0:32.4)" for a sample from the start of the song.
    fn at(&self, sample: usize) -> String {
        let k = self.bar_starts.partition_point(|&s| s <= sample).max(1) - 1;
        let k = k.min(self.bar_starts.len() - 2);
        let (a, b) = (self.bar_starts[k], self.bar_starts[k + 1]);
        let bar = k as f32 + 1.0 + (sample - a) as f32 / (b - a).max(1) as f32;
        let secs = sample as f32 / SAMPLE_RATE;
        format!("bar {:.1} ({}:{:04.1})", bar, (secs / 60.0) as u32, secs % 60.0)
    }
}

/// Render `song` with every part kept apart and write the pictures, the audio
/// and the report into `opts.out_dir`.
pub fn run(song: CompiledSong, title: &str, isolation: &Isolation, opts: &Options) -> Result<Outcome, String> {
    // Kept for the second pass, which renders the song again to cut out the
    // moments the report points at.
    let again = song.clone();
    let mut engine = SongEngine::from_compiled(song);
    engine.set_output_gain(opts.gain);
    let clock = Clock::new(&engine);
    let (first, last) = match opts.bars {
        Some((a, b)) => (a.clamp(1, clock.bars()), b.clamp(a.max(1), clock.bars())),
        None => (1, clock.bars()),
    };
    let start = clock.bar_starts[first as usize - 1];
    let end = clock.bar_starts[last as usize];
    let hop = (end - start).div_ceil(opts.width).max(16);
    let columns = (end - start).div_ceil(hop);

    prepare_dir(&opts.out_dir)?;
    let mut parts = plan_parts(&engine, opts.dry, hop, Some(&opts.out_dir))?;

    // For each block fed: where it ended, and which tracks held a note.
    let mut held_log: Vec<(usize, Vec<bool>)> = Vec::new();
    let reached = play(&mut engine, end, |pos, chunk, taps, bl, br| {
        if pos + chunk <= start { return Ok(()) }
        let from = start.saturating_sub(pos);
        for p in parts.iter_mut() {
            let (l, r) = p.source.pick(taps, bl, br);
            p.push(&l[from..chunk], &r[from..chunk])?;
        }
        held_log.push((pos + chunk - start, taps.held.clone()));
        Ok(())
    })?;
    let rendered = reached.saturating_sub(start);

    for p in parts.iter_mut() {
        p.spec.finish(columns);
        if let Some(w) = p.wav.take() { w.finish()?; }
        if p.silent() {
            let _ = std::fs::remove_file(opts.out_dir.join(format!("{}.wav", p.file)));
        }
    }
    let (silent, parts): (Vec<Part>, Vec<Part>) = parts.into_iter().partition(|p| p.silent());

    // Listen.
    let held = |ti: usize, frame: usize| -> bool {
        let s = frame * FRAME;
        let k = held_log.partition_point(|(e, _)| *e <= s);
        held_log.get(k).is_some_and(|(_, h)| h[ti])
    };
    // Not the mix: every click in it is in a part, where it can be told apart
    // from what surrounds it, and in the mix a drum hit landing on a loud bar
    // looks like one.
    let scans: Vec<Vec<Click>> = parts.iter()
        .map(|p| if p.source == Source::Mix { Vec::new() } else { listen::clicks(&p.frames) })
        .collect();
    let floors: Vec<Vec<Floor>> = parts.iter().map(|p| match p.source {
        Source::Track(ti) | Source::Dry(ti) if p.tonal => {
            listen::floors(&p.frames, &|f| held(ti, f))
        }
        _ => Vec::new(),
    }).collect();
    // A click within this of a section change is filed under it: a scene
    // taking over is the one moment many parts are cut at once.
    let section_starts: Vec<usize> = clock.sections.iter()
        .filter(|(_, b)| *b > 1)
        .map(|(_, b)| clock.bar_starts[*b as usize - 1])
        .collect();
    let at_change = |frame: usize| {
        let s = start + frame * FRAME;
        section_starts.iter().any(|&c| s.abs_diff(c) < SAMPLE_RATE as usize / 50)
    };

    // Zoom in on the worst of each part: its loudest click, its loudest floor.
    let mut zooms: Vec<Zoom> = Vec::new();
    for (i, p) in parts.iter().enumerate() {
        if p.source == Source::Mix { continue }
        if let Some(c) = scans[i].iter().max_by(|a, b| a.db.partial_cmp(&b.db).unwrap()) {
            zooms.push(Zoom::new(Zoom {
                part: i,
                centre: start + c.frame * FRAME + FRAME / 2,
                file: format!("zoom.{}.click.png", p.file),
                title: format!("{}: click at {}, {:.0} dB{}", p.label, clock.at(start + c.frame * FRAME), c.db,
                    if at_change(c.frame) { ", on a section change" } else { "" }),
                ..Default::default()
            }));
        }
        if let Some(f) = floors[i].iter().max_by(|a, b| a.db.partial_cmp(&b.db).unwrap()) {
            let mid = (f.start + f.end) / 2;
            zooms.push(Zoom::new(Zoom {
                part: i,
                centre: start + mid * FRAME,
                file: format!("zoom.{}.not-quiet.png", p.file),
                title: format!("{}: not quiet at {}, {:.0} dB", p.label, clock.at(start + mid * FRAME), f.db),
                ..Default::default()
            }));
        }
    }
    if !zooms.is_empty() {
        let mut engine = SongEngine::from_compiled(again);
        engine.set_output_gain(opts.gain);
        let last_end = zooms.iter().map(|z| z.centre + ZOOM_SPECTRUM / 2).max().unwrap_or(0).min(end);
        play(&mut engine, last_end, |pos, chunk, taps, bl, br| {
            for z in zooms.iter_mut() {
                let (l, r) = parts[z.part].source.pick(taps, bl, br);
                for k in 0..chunk {
                    let s = pos + k;
                    let m = (l[k] + r[k]) * 0.5;
                    if s + ZOOM_SPECTRUM / 2 >= z.centre && s < z.centre + ZOOM_SPECTRUM / 2 { z.spec.push(m) }
                    if s + ZOOM_WAVE / 2 >= z.centre && s < z.centre + ZOOM_WAVE / 2 { z.wave.push(m) }
                }
            }
            Ok(())
        })?;
    }

    // The mix, section by section. Only what a listener hears as a voice: the
    // tracks and the two returns, not the buses (their tracks are already
    // here) nor the dry copies.
    let spans: Vec<Span> = clock.sections.iter().enumerate().filter_map(|(i, (name, bar))| {
        let s = clock.bar_starts[*bar as usize - 1];
        let e = clock.sections.get(i + 1).map_or(*clock.bar_starts.last().unwrap(), |(_, nb)| clock.bar_starts[*nb as usize - 1]);
        let (s, e) = (s.max(start), e.min(start + rendered));
        if e <= s { return None }
        let last_bar = clock.bar_starts.partition_point(|&b| b < e) as u32;
        Some(Span {
            name: if name.is_empty() { "all".into() } else { name.clone() },
            bars: (clock.bar_starts.partition_point(|&b| b <= s) as u32, last_bar),
            frames: ((s - start) / FRAME, (e - start) / FRAME),
            columns: ((s - start) / hop, ((e - start) / hop).min(columns)),
        })
    }).collect();
    let voices: Vec<usize> = (0..parts.len())
        .filter(|&i| matches!(parts[i].source, Source::Track(_) | Source::Reverb | Source::Delay))
        .collect();
    let levels: Vec<Vec<[f32; 8]>> = voices.iter().map(|&i| mixing::band_levels(&parts[i].spec)).collect();
    let clashes = mixing::clashes(&levels, &spans.iter().map(|s| s.columns).collect::<Vec<_>>());
    let crowd = mixing::crowding(&levels, columns);

    // Draw.
    let ruler = Ruler {
        bars: (first..=last).map(|b| ((clock.bar_starts[b as usize - 1] - start) / hop, b)).collect(),
        sections: clock.sections.iter()
            .filter(|(n, _)| !n.is_empty())
            .filter_map(|(n, b)| {
                let s = clock.bar_starts[*b as usize - 1];
                let next = clock.sections.iter().find(|(_, nb)| nb > b)
                    .map_or(end, |(_, nb)| clock.bar_starts[*nb as usize - 1]);
                if next <= start || s >= end { return None }
                Some((s.saturating_sub(start) / hop, n.clone()))
            })
            .collect(),
    };
    let col = |frame: usize| (frame * FRAME / hop).min(columns.saturating_sub(1));
    let strips: Vec<Strip> = parts.iter().zip(&scans).zip(&floors).map(|((p, scan), fl)| Strip {
        name: &p.label,
        note: format!("peak {:.0} db", listen::db(p.peak)),
        spec: &p.spec,
        marks: Marks {
            clicks: scan.iter().map(|c| col(c.frame)).collect(),
            floors: fl.iter().map(|f| (col(f.start), col(f.end))).collect(),
        },
    }).collect();
    let sheet_path = opts.out_dir.join("sheet.png");
    draw::sheet(&strips, Some(&crowd), &ruler, columns).save(&sheet_path)?;
    for (s, p) in strips.iter().zip(&parts) {
        draw::part(s, &ruler, columns).save(&opts.out_dir.join(format!("{}.png", p.file)))?;
    }
    for z in &zooms {
        let zoom_hop = ZOOM_SPECTRUM.div_ceil(opts.width).max(16);
        let mut spec = Spectrogram::new(zoom_hop);
        for &x in &z.spec { spec.push(x) }
        let zoom_columns = z.spec.len().div_ceil(zoom_hop);
        spec.finish(zoom_columns);
        draw::zoom(&z.title, &z.wave, &spec, opts.width, zoom_columns)
            .save(&opts.out_dir.join(&z.file))?;
    }
    let zoom_of = |part: usize, kind: &str| zooms.iter()
        .find(|z| z.part == part && z.file.ends_with(kind))
        .map_or(String::new(), |z| format!("\n  {:<18} see {}", "", z.file));

    // Report.
    let mut r = String::new();
    let secs = rendered as f32 / SAMPLE_RATE;
    let _ = writeln!(r, "tatum debug: {title}, bars {first}-{last} of {} ({}:{:02})",
        clock.bars(), (secs / 60.0) as u32, secs as u32 % 60);
    if !isolation.solo.is_empty() { let _ = writeln!(r, "solo: {}", isolation.solo.join(", ")); }
    if !isolation.mute.is_empty() { let _ = writeln!(r, "muted: {}", isolation.mute.join(", ")); }
    let _ = writeln!(r, "\nin {}/", opts.out_dir.display());
    let _ = writeln!(r, "  sheet.png        every part stacked on one time axis, the mix at the bottom");
    let _ = writeln!(r, "  <part>.png/.wav  each part on its own, as it sits in the mix");
    if !zooms.is_empty() {
        let _ = writeln!(r, "  zoom.*.png       the worst moment of a part up close: the wave, and the spectrum around it");
    }
    if !silent.is_empty() {
        let names: Vec<&str> = silent.iter().map(|p| p.label.as_str()).collect();
        let _ = writeln!(r, "  silent, left out: {}", names.join(", "));
    }

    let _ = writeln!(r, "\n{:<20} {:>8} {:>8}", "part", "peak", "level");
    for p in &parts {
        let _ = writeln!(r, "{:<20} {:>5.1} dB {:>5.1} dB", p.label, listen::db(p.peak), listen::db(p.rms()));
    }

    let _ = writeln!(r, "\nlevels by section: each part against the loudest track there, in dB");
    for span in &spans {
        let (f0, f1) = span.frames;
        let mix = parts.iter().find(|p| p.source == Source::Mix).map_or(-200.0, |p| p.frames.db(f0, f1));
        let mut here: Vec<(&str, f32, bool)> = voices.iter()
            .map(|&i| (parts[i].label.as_str(), parts[i].frames.db(f0, f1), matches!(parts[i].source, Source::Track(_))))
            .filter(|(_, db, _)| *db > PLAYING_DB)
            .collect();
        here.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        let top = here.iter().filter(|h| h.2).map(|h| h.1).fold(f32::MIN, f32::max);
        let list: Vec<String> = here.iter().map(|(n, db, track)| {
            let rel = db - top;
            // A return is meant to sit under the tracks it carries.
            if *track && rel < -BURIED_DB { format!("{n} {rel:.0} (buried)") } else { format!("{n} {rel:.0}") }
        }).collect();
        let _ = writeln!(r, "  {:<14} bars {:>3}-{:<3} mix {:>4.0} dB | {}", span.name, span.bars.0, span.bars.1, mix,
            if list.is_empty() { "nothing playing".into() } else { list.join(", ") });
    }

    let _ = writeln!(r, "\nin each other's way: two parts level with each other and on top of the same range, most of a section");
    if clashes.is_empty() { let _ = writeln!(r, "  none"); }
    for c in clashes.iter().take(12) {
        let (a, b) = (&parts[voices[c.a]].label, &parts[voices[c.b]].label);
        let mut where_: Vec<String> = c.sections.iter().take(4)
            .map(|(si, share)| format!("{} {:.0}%", spans[*si].name, share * 100.0)).collect();
        if c.sections.len() > 4 { where_.push(format!("+{} more", c.sections.len() - 4)); }
        let _ = writeln!(r, "  {:<26} {:<13} {}", format!("{a} + {b}"), mixing::BANDS[c.band].2, where_.join(", "));
    }
    if clashes.len() > 12 { let _ = writeln!(r, "  (+{} more)", clashes.len() - 12); }

    let at = |frame: usize| clock.at(start + frame * FRAME);
    let _ = writeln!(r, "\nclicks: a sudden corner in the wave that is not a note starting");
    let mut any = false;
    for (i, (p, scan)) in parts.iter().zip(&scans).enumerate() {
        if scan.is_empty() { continue }
        any = true;
        let mut worst: Vec<&Click> = scan.iter().collect();
        worst.sort_by(|a, b| b.db.partial_cmp(&a.db).unwrap());
        let shown: Vec<String> = worst.iter().take(3)
            .map(|c| format!("{} at {:.0} dB", at(c.frame), c.db)).collect();
        let changes = scan.iter().filter(|c| at_change(c.frame)).count();
        let count = if changes > 0 {
            format!("{} ({} on a section change)", scan.len(), changes)
        } else {
            scan.len().to_string()
        };
        let _ = writeln!(r, "  {:<18} {}; loudest {}{}", p.label, count, shown.join(", "), zoom_of(i, ".click.png"));
    }
    if !any { let _ = writeln!(r, "  none"); }

    let _ = writeln!(r, "\nnot quiet between notes: a gap with no note held that ends still sounding, and not fading");
    any = false;
    for (i, (p, fl)) in parts.iter().zip(&floors).enumerate() {
        if fl.is_empty() { continue }
        any = true;
        let worst = fl.iter().max_by(|a, b| a.db.partial_cmp(&b.db).unwrap()).unwrap();
        let under = listen::playing_db(&p.frames) - worst.db;
        let what = floor_character(&parts[i], worst, hop);
        let places: Vec<String> = fl.iter().take(3).map(|f| at(f.start)).collect();
        let more = if fl.len() > 3 { format!(" (+{} more)", fl.len() - 3) } else { String::new() };
        let _ = writeln!(r, "  {:<18} {:.0} dB, {:.0} dB under its notes, {}; {}{}{}",
            p.label, worst.db, under, what, places.join(", "), more, zoom_of(i, ".not-quiet.png"));
    }
    if !any { let _ = writeln!(r, "  none"); }

    // The two ends of the range. Only reported past a share no instrument
    // gets to by playing what it is meant to; the numbers were set from the
    // examples, where a sub pitched at 22 Hz sat at 40% and every hat under 10%.
    let edge = |band: &dyn Fn(&Frames) -> &Vec<f32>, judged: &dyn Fn(&Part) -> bool, over: f32| {
        let mut lines = Vec::new();
        for p in parts.iter().filter(|p| judged(p)) {
            let share = listen::share_db(&p.frames, band(&p.frames));
            if share < over { continue }
            let where_ = loudest_second(band(&p.frames)).map(|f| format!("; most at {}", at(f))).unwrap_or_default();
            lines.push(format!("  {:<18} {:.0}% of its energy{}", p.label, 10f32.powf(share / 10.0) * 100.0, where_));
        }
        lines
    };
    let low = edge(&|f| &f.low, &|p| p.source != Source::Mix, RUMBLE_SHARE_DB);
    let _ = writeln!(r, "\nbelow {:.0} Hz: under what speakers play -- a note pitched too low, or an FM ratio under 1", listen::RUMBLE_HZ);
    if low.is_empty() { let _ = writeln!(r, "  none"); } else { for l in low { let _ = writeln!(r, "{l}"); } }
    let high = edge(&|f| &f.high, &|p| p.tonal && matches!(p.source, Source::Track(_) | Source::Dry(_)), FIZZ_SHARE_DB);
    let _ = writeln!(r, "\nabove {:.0} kHz on a tonal track: fizz, usually overtones folding back (aliasing) or clipping", listen::FIZZ_HZ / 1000.0);
    if high.is_empty() { let _ = writeln!(r, "  none"); } else { for l in high { let _ = writeln!(r, "{l}"); } }

    let off: Vec<String> = parts.iter()
        .filter(|p| p.samples > 0 && (p.sum / p.samples as f64).abs() > 0.01)
        .map(|p| format!("{} ({:+.3})", p.label, p.sum / p.samples as f64))
        .collect();
    if !off.is_empty() {
        let _ = writeln!(r, "\noff centre: the wave sits above or below zero on average (DC)\n  {}", off.join(", "));
    }

    std::fs::write(opts.out_dir.join("report.txt"), &r)
        .map_err(|e| format!("cannot write report: {e}"))?;
    Ok(Outcome { report: r, sheet: sheet_path })
}

/// Listens to a render as it is made, for `tatum render` and `tatum_render`:
/// the clicks, the noise between notes and the two ends of the range that
/// [`run`] reports, found in the same pass that writes the file and written
/// nowhere. Until this, they turned up only for whoever thought to run
/// `tatum debug`. Two parts in each other's way need the fine spectrograms,
/// which would make every render slow, so they stay in `run`.
pub struct Listener {
    parts: Vec<Part>,
    held_log: Vec<(usize, Vec<bool>)>,
    clock: Clock,
    pos: usize,
}

/// A spectrogram column every tenth of a second: enough to tell hiss from a
/// tone, and nearly free.
const LISTEN_HOP: usize = SAMPLE_RATE as usize / 10;

impl Listener {
    /// Turns the engine's taps on; hand it every block from then on.
    pub fn new(engine: &mut SongEngine) -> Listener {
        engine.set_taps(true);
        let parts = plan_parts(engine, false, LISTEN_HOP, None).expect("no files to open");
        Listener { parts, held_log: Vec::new(), clock: Clock::new(engine), pos: 0 }
    }

    /// One block of the render: the mix, and the engine for its taps.
    pub fn feed(&mut self, engine: &SongEngine, l: &[f32], r: &[f32]) {
        let Some(taps) = engine.taps() else { return };
        let n = l.len();
        for p in self.parts.iter_mut() {
            let (pl, pr) = p.source.pick(taps, l, r);
            let _ = p.push(&pl[..n], &pr[..n]);
        }
        self.pos += n;
        self.held_log.push((self.pos, taps.held.clone()));
    }

    /// One line per kind of problem heard, each naming the parts and where.
    /// Empty when there is nothing to say.
    pub fn findings(self) -> Vec<String> {
        let Listener { parts, held_log, clock, .. } = self;
        let at = |frame: usize| clock.at(frame * FRAME);
        let mut out = Vec::new();

        let clicks: Vec<String> = parts.iter()
            .filter(|p| p.source != Source::Mix && !p.silent())
            .filter_map(|p| {
                let c = listen::clicks(&p.frames);
                let first = c.first()?;
                let more = if c.len() > 1 { format!(", {} in all", c.len()) } else { String::new() };
                Some(format!("{} at {}{}", p.label, at(first.frame), more))
            })
            .collect();
        if !clicks.is_empty() { out.push(format!("clicks: {}", clicks.join("; "))); }

        let held = |ti: usize, frame: usize| -> bool {
            let s = frame * FRAME;
            let k = held_log.partition_point(|(e, _)| *e <= s);
            held_log.get(k).is_some_and(|(_, h)| h[ti])
        };
        let floors: Vec<String> = parts.iter()
            .filter_map(|p| {
                let Source::Track(ti) = p.source else { return None };
                if !p.tonal { return None }
                let fl = listen::floors(&p.frames, &|f| held(ti, f));
                let worst = fl.iter().max_by(|a, b| a.db.partial_cmp(&b.db).unwrap())?;
                Some(format!("{} at {}, {:.0} dB, {}", p.label, at(worst.start), worst.db, floor_character(p, worst, LISTEN_HOP)))
            })
            .collect();
        if !floors.is_empty() { out.push(format!("not quiet between notes: {}", floors.join("; "))); }

        let share = |p: &Part, band: &Vec<f32>, over: f32| {
            let s = listen::share_db(&p.frames, band);
            (s >= over).then(|| format!("{} {:.0}%", p.label, 10f32.powf(s / 10.0) * 100.0))
        };
        let low: Vec<String> = parts.iter()
            .filter(|p| p.source != Source::Mix)
            .filter_map(|p| share(p, &p.frames.low, RUMBLE_SHARE_DB))
            .collect();
        if !low.is_empty() {
            out.push(format!("under {:.0} Hz, where no speaker plays: {}", listen::RUMBLE_HZ, low.join(", ")));
        }
        let high: Vec<String> = parts.iter()
            .filter(|p| p.tonal && matches!(p.source, Source::Track(_)))
            .filter_map(|p| share(p, &p.frames.high, FIZZ_SHARE_DB))
            .collect();
        if !high.is_empty() {
            out.push(format!("over {:.0} kHz on a tonal track: {}", listen::FIZZ_HZ / 1000.0, high.join(", ")));
        }

        let off: Vec<String> = parts.iter()
            .filter(|p| p.samples > 0 && (p.sum / p.samples as f64).abs() > 0.01)
            .map(|p| format!("{} ({:+.3})", p.label, p.sum / p.samples as f64))
            .collect();
        if !off.is_empty() { out.push(format!("off centre (DC): {}", off.join(", "))); }
        out
    }
}

/// A part under this level in a section is not playing in it.
const PLAYING_DB: f32 = -70.0;
/// A track this far under the loudest one in its section is buried: there,
/// but not heard as a part.
const BURIED_DB: f32 = 25.0;

/// A section of the song, as far as it falls inside what was rendered.
struct Span {
    name: String,
    /// First and last bar, counted from 1.
    bars: (u32, u32),
    frames: (usize, usize),
    columns: (usize, usize),
}

/// Share of a part's energy below `RUMBLE_HZ` past which it is reported.
const RUMBLE_SHARE_DB: f32 = -10.0; // 10%
/// Share of a tonal part's energy above `FIZZ_HZ` past which it is reported.
const FIZZ_SHARE_DB: f32 = -15.0; // 3%

/// Half a second of spectrum and 80 ms of wave around a finding.
const ZOOM_SPECTRUM: usize = SAMPLE_RATE as usize / 2;
const ZOOM_WAVE: usize = SAMPLE_RATE as usize * 80 / 1000;

/// A moment to look at up close, filled in by the second render.
#[derive(Default)]
struct Zoom {
    part: usize,
    centre: usize,
    file: String,
    title: String,
    wave: Vec<f32>,
    spec: Vec<f32>,
}

impl Zoom {
    /// Pad the front with silence when the moment is too near the start of
    /// the song for a whole window, so it stays in the middle of the picture.
    fn new(z: Zoom) -> Zoom {
        Zoom {
            wave: vec![0.0; (ZOOM_WAVE / 2).saturating_sub(z.centre)],
            spec: vec![0.0; (ZOOM_SPECTRUM / 2).saturating_sub(z.centre)],
            ..z
        }
    }
}

impl Source {
    fn pick<'a>(self, taps: &'a Taps, bl: &'a [f32], br: &'a [f32]) -> (&'a [f32], &'a [f32]) {
        match self {
            Source::Track(i) => (&taps.tracks[i].l, &taps.tracks[i].r),
            Source::Dry(i) => (&taps.dry[i].l, &taps.dry[i].r),
            Source::Bus(i) => (&taps.buses[i].l, &taps.buses[i].r),
            Source::Delay => (&taps.delay.l, &taps.delay.r),
            Source::Reverb => (&taps.reverb.l, &taps.reverb.r),
            Source::Mix => (bl, br),
        }
    }
}

/// Render from the top to `end` with the taps on, handing each block over as
/// (where it starts, its length, the taps, the mix). Returns where it stopped:
/// the arrangement can end before `end`.
fn play(
    engine: &mut SongEngine,
    end: usize,
    mut each: impl FnMut(usize, usize, &Taps, &[f32], &[f32]) -> Result<(), String>,
) -> Result<usize, String> {
    engine.set_taps(true);
    engine.start();
    let mut pos = 0usize;
    let (mut bl, mut br) = ([0.0f32; BLOCK_SIZE], [0.0f32; BLOCK_SIZE]);
    while pos < end && engine.running() {
        let chunk = (end - pos).min(BLOCK_SIZE);
        engine.process_block_stereo(&mut bl[..chunk], &mut br[..chunk]);
        each(pos, chunk, engine.taps().expect("taps are on"), &bl[..chunk], &br[..chunk])?;
        pos += chunk;
    }
    Ok(pos)
}

/// The frame starting the loudest second of a band, if it has any energy.
fn loudest_second(band: &[f32]) -> Option<usize> {
    const SECOND: usize = 1000;
    if band.len() < SECOND { return band.iter().any(|&v| v > 0.0).then_some(0) }
    let mut sum: f64 = band[..SECOND].iter().map(|&v| v as f64).sum();
    let (mut best, mut at) = (sum, 0);
    for i in SECOND..band.len() {
        sum += band[i] as f64 - band[i - SECOND] as f64;
        if sum > best { best = sum; at = i + 1 - SECOND; }
    }
    (best > 0.0).then_some(at)
}

/// Hiss or a tone, from the spectrogram columns under a floor.
fn floor_character(p: &Part, f: &Floor, hop: usize) -> String {
    use spectrogram::{cell_db, row_hz, ROWS};
    let (a, b) = (f.start * FRAME / hop, (f.end * FRAME / hop).max(f.start * FRAME / hop + 1));
    let b = b.min(p.spec.columns());
    if a >= b { return "?".into() }
    let rows: Vec<f32> = (0..ROWS)
        .map(|r| (a..b).map(|c| cell_db(p.spec.cell(c, r))).sum::<f32>() / (b - a) as f32)
        .collect();
    let (top, loudest) = rows.iter().enumerate()
        .fold((0, f32::MIN), |acc, (i, &v)| if v > acc.1 { (i, v) } else { acc });
    let wide = rows.iter().filter(|&&v| v > loudest - 12.0).count();
    if wide * 3 > ROWS {
        "spread over the whole range, like hiss".into()
    } else {
        format!("a tone near {:.0} Hz", row_hz(top as f32 + 0.5))
    }
}

fn prepare_dir(dir: &Path) -> Result<(), String> {
    // Only a directory this command made is cleared: a stale part from the
    // last run, with other tracks soloed, would be read as part of this one.
    if dir.join("report.txt").exists() {
        for entry in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?.flatten() {
            let path = entry.path();
            if matches!(path.extension().and_then(|e| e.to_str()), Some("png" | "wav" | "txt")) {
                let _ = std::fs::remove_file(path);
            }
        }
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))
}

/// Every part of the song, each with its own listening state; with a WAV
/// file for each in `dir`, when there is one.
fn plan_parts(engine: &SongEngine, dry: bool, hop: usize, dir: Option<&Path>) -> Result<Vec<Part>, String> {
    let mut specs: Vec<(String, String, Source, bool)> = Vec::new();
    for i in 0..engine.track_count() {
        let name = engine.track_name(i);
        let tonal = engine.track_kind(i) != "beats";
        specs.push((name.to_string(), file_name(name), Source::Track(i), tonal));
        if dry {
            specs.push((format!("{name} dry"), format!("{}.dry", file_name(name)), Source::Dry(i), tonal));
        }
    }
    for i in 0..engine.bus_count() {
        let name = engine.bus_name(i);
        specs.push((format!("bus {name}"), format!("bus.{}", file_name(name)), Source::Bus(i), false));
    }
    specs.push(("reverb return".into(), "send.reverb".into(), Source::Reverb, false));
    specs.push(("delay return".into(), "send.delay".into(), Source::Delay, false));
    specs.push(("mix".into(), "mix".into(), Source::Mix, false));
    specs.into_iter().map(|(label, file, source, tonal)| {
        Ok(Part {
            wav: match dir {
                Some(dir) => Some(WavWriter::create(&dir.join(format!("{file}.wav")))?),
                None => None,
            },
            label, file, source, tonal,
            spec: Spectrogram::new(hop),
            frames: Frames::default(),
            peak: 0.0, sum_sq: 0.0, sum: 0.0, samples: 0,
        })
    }).collect()
}

fn file_name(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect()
}
