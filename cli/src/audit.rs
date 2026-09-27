//! `tatum audit` — render every tonal track on its own and listen to each
//! note for the things that make a mix sound dirty.
//!
//! This exists because of a bug that took an hour to find by ear. An FM bell
//! with 18% chorus was heard as a small distortion on its first note. Every
//! whole-song measurement said the track was clean — averaged over 222
//! seconds it was the *tidiest* voice in the song.
//!
//! The metric is the same throughout: how much of a note's energy is NOT at a
//! harmonic of the note the pattern asked for. That is computable only
//! because the compiler knows what was written. What took three wrong
//! versions to learn is what it can and cannot be compared against.
//!
//! **It cannot be compared between voices.** A saturated saw has half its
//! energy off the harmonics by design; a bell has almost none. Any threshold
//! that catches the bell calls every saw broken. So the per-track number is a
//! fingerprint of a timbre, and the rows print alphabetically to make ranking
//! them awkward on purpose.
//!
//! **It can be compared between the notes of one voice.** One note far above
//! the rest of its own voice is worth going to listen to, and the bar is
//! printed so you can.
//!
//! **It can be compared between the same voice with and without one effect.**
//! Both renders are the same timbre, so whatever the difference is, the
//! effect caused it. This is the one that would have found the bell in a
//! single run, because that bug is not one bad note: the chorus raised every
//! note of the bell by 17 dB evenly, and the ear caught it on the first note
//! because a clean bell has nothing to hide it behind.
//!
//! Three more rules, each of which cost a wrong diagnosis:
//!
//!   - **Per note, not per song.** An average is the wrong instrument for a
//!     bug that lives in one note.
//!   - **Soloed and dry.** In the mix everything masks everything, and with
//!     the sends open a reverb tail puts the previous note inside this note's
//!     window — which made the tool report *more* dirty notes after the bug
//!     was fixed than before.
//!   - **Absolute, not percentage.** A window whose percentage of harsh
//!     energy is huge is usually just a quiet window; chasing that cost a
//!     round of edits before the absolute numbers showed it was nothing.
//!
//! The FFT (in `tatum-debug`, shared with `debug`) is hand-written because
//! `core` carries no dependencies and forty lines is cheaper than taking one.
//! Goertzel was tried first — the
//! harmonic frequencies are known in advance, so in principle only those bins
//! are needed — and it failed: normalising one bin against total power needs
//! Parseval over the whole spectrum anyway, and every track came back at
//! roughly 0 dB. Subtracting a fitted sinusoid failed too; note envelopes
//! smear a pure tone across neighbouring bins. What works is a windowed
//! spectrum with harmonic bands of finite width (±45 Hz), one window placed
//! inside each step so that what it sees is one note.

use std::collections::BTreeSet;
use std::process;

use tatum_core::dsl;
use tatum_core::dsl::ast::{NoteRef, Song, Step};
use tatum_core::song_engine::SongEngine;
use tatum_core::SAMPLE_RATE;
use tatum_debug::fft::fft;

/// How much of this window is NOT at a harmonic of `f0`, in dB.
///
/// A note is not a sum of steady sinusoids — it has an envelope and usually
/// vibrato — so fitting and subtracting sinusoids leaves the envelope behind
/// as "residual" and measures nothing. A windowed spectrum tolerates that:
/// the envelope widens each harmonic by a couple of bins, which is why the
/// harmonic bands below are ±3 bins wide rather than exact.
///
/// What this is actually looking for is a copy of the note detuned far enough
/// to sit outside those bands. That is what a chorus does, and on a high
/// note it is heard as dirt.
fn inharmonic_db(x: &[f32], f0: f32) -> f64 {
    let n = x.len();
    let mut re: Vec<f64> = x
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let w = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / n as f64).cos();
            *v as f64 * w
        })
        .collect();
    let mut im = vec![0.0f64; n];
    fft(&mut re, &mut im);
    let half = n / 2;
    let bin = SAMPLE_RATE as f64 / n as f64;
    let mut total = 0.0f64;
    let mut harmonic = 0.0f64;
    for k in 1..half {
        let p = re[k] * re[k] + im[k] * im[k];
        total += p;
        let f = k as f64 * bin;
        // within three bins of any harmonic of f0?
        let ratio = f / f0 as f64;
        let nearest = ratio.round();
        // ±45 Hz, not ±3 bins: an envelope plus vibrato plus a reverb tail
        // smears each harmonic wider than the window's resolution, and a
        // band that is too tight counts the note's own breathing as dirt.
        if (1.0..=16.0).contains(&nearest) && (f - nearest * f0 as f64).abs() <= 45.0 {
            harmonic += p;
        }
    }
    if total <= 1e-20 {
        return -200.0;
    }
    10.0 * ((total - harmonic).max(total * 1e-12) / total).log10()
}

/// Strongest written note in this window: the candidate whose first two
/// harmonics carry the most energy.
fn detect(x: &[f32], candidates: &[f32]) -> Option<f32> {
    let n = x.len();
    let mut re: Vec<f64> = x
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let w = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / n as f64).cos();
            *v as f64 * w
        })
        .collect();
    let mut im = vec![0.0f64; n];
    fft(&mut re, &mut im);
    let bin = SAMPLE_RATE as f64 / n as f64;
    let power = |f: f64| -> f64 {
        let k = (f / bin).round() as usize;
        (k.saturating_sub(2)..=(k + 2))
            .filter(|k| *k > 0 && *k < n / 2)
            .map(|k| re[k] * re[k] + im[k] * im[k])
            .sum()
    };
    candidates
        .iter()
        .map(|f| (*f, power(*f as f64) + 0.5 * power(*f as f64 * 2.0)))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(f, _)| f)
}

/// Every note a track can play, as frequencies.
///
/// Degrees resolve through the song's scale just like absolute names do.
/// Skipping them was a real hole: the repo's own arpeggiator example writes
/// its chord as `[1.4 3.4 5.4 7.4]`, so it had no candidate notes at all and
/// the audit printed no row for it — silently, which is the worst way for a
/// measuring tool to fail.
fn written_notes(song: &Song, track: &str) -> Vec<f32> {
    let mut patterns: BTreeSet<&str> = BTreeSet::new();
    if let Some(t) = song.tracks.iter().find(|t| t.name == track) {
        patterns.insert(&t.play);
    }
    for sc in &song.scenes {
        if let Some(t) = sc.tracks.iter().find(|t| t.name == track) {
            patterns.insert(&t.play);
        }
    }
    let mut midi: BTreeSet<u8> = BTreeSet::new();
    let (intervals, root) = dsl::compiler::scale_context(song);
    let take = |n: &NoteRef, out: &mut BTreeSet<u8>| {
        out.insert(dsl::compiler::resolve_note(n, &intervals, root));
    };
    for name in patterns {
        let Some(p) = song.patterns.iter().find(|p| p.name == name) else { continue };
        if !p.lane_labels.is_empty() {
            continue;
        }
        for step in p.rows.iter().flatten() {
            match step {
                Step::Note(ns) => take(&ns.note, &mut midi),
                Step::Chord(cs) => cs.notes.iter().for_each(|ns| take(&ns.note, &mut midi)),
                Step::Subdiv(subs) => subs.iter().for_each(|ns| take(&ns.note, &mut midi)),
                _ => {}
            }
        }
    }
    // An arp repeats the held notes in higher octaves, so those pitches
    // sound too and the detector has to know about them.
    let octs = song
        .tracks
        .iter()
        .find(|t| t.name == track)
        .and_then(|t| t.arp.as_ref())
        .filter(|a| a.mode != "off")
        .and_then(|a| a.octaves)
        .unwrap_or(1.0)
        .max(1.0) as u8;
    let lifted: Vec<u8> = midi
        .iter()
        .flat_map(|m| (0..octs).filter_map(move |o| m.checked_add(12 * o)))
        .collect();
    lifted
        .into_iter()
        .collect::<BTreeSet<u8>>()
        .into_iter()
        .map(|m| 440.0 * ((m as f32 - 69.0) / 12.0).exp2())
        .collect()
}

/// The source with every track but one muted and that one's sends closed, so
/// the render is that voice dry and alone.
///
/// The sends have to go. A reverb or a delay puts the tail of the note before
/// into the window of the note after, and "not at a harmonic of this note" is
/// exactly what the note before looks like. With them open the tool reported
/// MORE dirty notes on a bell after its chorus was removed than before, which
/// is the opposite of the truth: the chorus had raised the whole voice's
/// floor, and taking it out lowered the floor without touching the handful of
/// notes that sit next to a loud tail. Smear from an effect is deliberate;
/// the question here is whether the voice itself is clean.
/// What a solo render can have taken out of it, to find out how much of the
/// voice's dirt that thing accounts for.
#[derive(Clone, Copy, PartialEq)]
pub enum Suspect {
    Chorus,
    Drive,
}

impl Suspect {
    fn label(self) -> &'static str {
        match self { Suspect::Chorus => "chorus", Suspect::Drive => "drive" }
    }
}

/// `solo_source`, with one suspect switched off on the track being measured.
fn solo_without(song: &Song, keep: &str, off: Option<Suspect>) -> Song {
    let mut s = solo_base(song, keep);
    let Some(off) = off else { return s };
    if off == Suspect::Chorus {
        for m in s.module_defs.iter_mut() {
            for p in m.params.iter_mut() {
                if p.name == "chorus_mix" { p.value = 0.0; }
            }
        }
    }
    let strip = |routing: &mut Vec<dsl::ast::RoutingNode>| {
        routing.retain(|r| match off {
            Suspect::Chorus => r.kind != "chorus",
            Suspect::Drive => !matches!(r.kind.as_str(), "saturate" | "drive" | "distort" | "bitcrush"),
        });
    };
    for t in s.tracks.iter_mut().filter(|t| t.name == keep) { strip(&mut t.routing); }
    for sc in s.scenes.iter_mut() {
        for t in sc.tracks.iter_mut().filter(|t| t.name == keep) { strip(&mut t.routing); }
    }
    s
}

/// Why the per-note check cannot say anything about this voice, if it cannot.
///
/// All of these are one problem wearing different clothes: the measurement
/// needs a window holding one pitch that the window can resolve, and these
/// are the voices where it never gets one. Run across the corpus without
/// them, the check reported 30 "dirty" notes on a sub, 24 on a low organ and
/// 8 on an arpeggio -- always the same handful of pitches, over and over,
/// which is the signature of a measurement that is wrong about a pitch
/// rather than a note that is wrong.
///
/// The with-and-without comparison is unaffected by every one of these: it
/// is the same voice both times, so whatever differs, the effect caused it.
fn cannot_judge_notes(song: &Song, track: &str) -> Option<&'static str> {
    let mine = || {
        song.tracks.iter().chain(song.scenes.iter().flat_map(|s| s.tracks.iter()))
            .filter(move |t| t.name == track)
    };
    // An arp runs below the step, so a window that fits inside a step still
    // holds several of its notes.
    if mine().any(|t| t.arp.as_ref().is_some_and(|a| a.mode != "off")) {
        return Some("its arp puts several notes in every window");
    }
    // Vibrato deep enough to matter moves the upper harmonics out of their
    // bands: half a semitone at C#6 moves the eighth harmonic 107 Hz.
    if mine()
        .filter_map(|t| song.module_defs.iter().find(|m| m.name == t.using_instrument))
        .any(|m| m.params.iter().any(|p| p.name == "vibrato_depth" && p.value > 0.10))
    {
        return Some("it bends off the written pitch on purpose");
    }
    None
}


/// Is there anything for this suspect to switch off on this track? Rendering
/// a second time to remove something that is not there costs the same as
/// rendering to remove something that is.
fn has_suspect(song: &Song, track: &str, what: Suspect) -> bool {
    let tracks = || song.tracks.iter().chain(song.scenes.iter().flat_map(|s| s.tracks.iter()));
    let in_chain = tracks().filter(|t| t.name == track).any(|t| {
        t.routing.iter().any(|r| match what {
            Suspect::Chorus => r.kind == "chorus",
            Suspect::Drive => matches!(r.kind.as_str(), "saturate" | "drive" | "distort" | "bitcrush"),
        })
    });
    if in_chain { return true }
    if what != Suspect::Chorus { return false }
    tracks()
        .filter(|t| t.name == track)
        .filter_map(|t| song.module_defs.iter().find(|m| m.name == t.using_instrument))
        .any(|m| m.params.iter().any(|p| p.name == "chorus_mix" && p.value > 0.0))
}

fn solo_base(song: &Song, keep: &str) -> Song {
    let mut s = song.clone();
    for t in s.tracks.iter_mut() {
        if t.name != keep {
            t.level = Some(0.0);
        } else {
            t.delay_send = Some(0.0);
            t.reverb_send = Some(0.0);
        }
    }
    for sc in s.scenes.iter_mut() {
        for t in sc.tracks.iter_mut() {
            if t.name != keep {
                t.level = Some(0.0);
            } else {
                t.delay_send = Some(0.0);
                t.reverb_send = Some(0.0);
            }
        }
        // a level automation would put the muted track straight back
        sc.automations.retain(|a| !a.target.ends_with(".level"));
    }
    s
}

/// How far above its own voice's median a note has to sit before it is called
/// out. See the module header: the unit is dB above that track's median note,
/// not dB absolute, because absolute is not comparable between timbres.
const OUTLIER_DB: f64 = 8.0;

/// Above this, the window has no pitch in it that we recognise, so there is
/// nothing to say about how clean it is. Half the energy off the harmonics is
/// already a lot; past that the model does not apply.
const NO_PITCH_DB: f64 = -3.0;

/// How much of a voice's inharmonic content one effect has to account for
/// before it is worth naming. Measured on the bell: its 18% chorus accounted
/// for 16.7 dB. A chorus that only costs two or three is doing what a chorus
/// is for.
const BLAME_DB: f64 = 6.0;

/// A note this short gives one window, and one window at the start of a note
/// still carries the end of the note before it. It counts toward the voice's
/// median but is not worth sending someone to listen to.
const MIN_STEPS_TO_REPORT: usize = 2;

/// Below this the harmonic bands are wider than the gap between harmonics and
/// the measurement stops meaning anything: a sub at D1 is 37 Hz, and the
/// bands are +/-45. A window short enough to sit inside one step cannot
/// resolve any better, so this is a property of the method, not a setting.
const MIN_PITCH_HZ: f32 = 110.0;

/// One note: a run of consecutive steps that detected the same pitch.
struct Note {
    bar: f32,
    hz: f32,
    /// Median over the note's windows. The median and not the worst, because
    /// the window at a note's edge is mostly the note before it.
    db: f64,
    windows: usize,
}

/// One note that stands out from the other notes of its own voice.
struct Finding {
    track: String,
    bar: f32,
    note: String,
    steps: usize,
    db: f64,
    margin: f64,
    median: f64,
}

struct Row {
    track: String,
    notes: usize,
    median_db: f64,
    worst: Note,
    peak: f32,
    /// How many dB of this voice's inharmonic content each suspect accounts
    /// for: the voice's median as written, minus its median with that thing
    /// switched off.
    blame: Vec<(Suspect, f64)>,
}

fn median(xs: &mut [f64]) -> f64 {
    xs.sort_by(|a, b| a.total_cmp(b));
    xs[xs.len() / 2]
}

/// A frequency as a note name, so a finding says C#6 and not 1109.
fn hz_name(hz: f32) -> String {
    const N: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    let m = (12.0 * (hz / 440.0).log2() + 69.0).round() as i32;
    format!("{}{}", N[(m.rem_euclid(12)) as usize], m / 12 - 1)
}

/// Measure one soloed track, note by note.
///
/// A window is placed inside each step rather than on a free-running grid.
/// The grid version could not tell a dirty note from a window that happened
/// to straddle two notes -- and a straddling window is mostly "not at a
/// harmonic of f0" by construction, so it looked like the dirtiest thing in
/// every song. The old code dealt with that by throwing away everything above
/// -12 dB, which meant the reported number was the discard threshold on four
/// tracks out of five rather than anything about the sound.
fn measure(mono: &[f32], peak: f32, cands: &[f32], spb: f32) -> Vec<Note> {
    let step = spb / 16.0;
    // Skip the attack: the click at a note's start is broadband on purpose and
    // measuring it would call every percussive voice dirty.
    let skip = (0.02 * SAMPLE_RATE) as usize;
    // The largest window that still fits inside one step, so what it sees is
    // one note. 1024 samples is 43 Hz of resolution, which is the coarsest
    // that can still resolve the +/-45 Hz harmonic bands.
    let room = (step as usize).saturating_sub(skip);
    let n = (1..=13).map(|k| 1usize << k).rfind(|w| *w <= room).unwrap_or(1024).clamp(1024, 8192);
    let hann: Vec<f32> = (0..n)
        .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / n as f32).cos())
        .collect();

    let steps = ((mono.len() as f32 - skip as f32 - n as f32) / step).max(0.0) as usize;
    let mut per_step: Vec<Option<(f32, f64)>> = Vec::with_capacity(steps);
    for s in 0..steps {
        let start = (s as f32 * step) as usize + skip;
        let w: Vec<f32> = mono[start..start + n].iter().zip(&hann).map(|(x, h)| x * h).collect();
        // A rest, or a tail that has run out: nothing to say about it.
        if w.iter().fold(0.0f32, |a, v| a.max(v.abs())) < peak * 0.08 {
            per_step.push(None);
            continue;
        }
        match detect(&w, cands) {
            // A window where less than half the energy sits at harmonics of
            // the pitch we picked is not a dirty note, it is a window where
            // we did not find the note: a transition, a tail, or the
            // detector choosing the wrong candidate. Reporting those as the
            // dirtiest thing in the song is what the first version did.
            Some(f0) => {
                let db = inharmonic_db(&w, f0);
                per_step.push(if db > NO_PITCH_DB { None } else { Some((f0, db)) });
            }
            None => per_step.push(None),
        }
    }

    // Consecutive steps holding the same pitch are one note.
    let mut notes = Vec::new();
    let mut run: Vec<f64> = Vec::new();
    let mut run_f0 = 0.0f32;
    let mut run_at = 0usize;
    let flush = |notes: &mut Vec<Note>, run: &mut Vec<f64>, f0: f32, at: usize| {
        if run.is_empty() { return }
        notes.push(Note {
            bar: at as f32 / 16.0 + 1.0,
            hz: f0,
            db: median(run),
            windows: run.len(),
        });
        run.clear();
    };
    for (s, m) in per_step.iter().enumerate() {
        match m {
            Some((f0, db)) if run.is_empty() => { run_f0 = *f0; run_at = s; run.push(*db); }
            Some((f0, db)) if (*f0 - run_f0).abs() < 0.5 => run.push(*db),
            Some((f0, db)) => {
                flush(&mut notes, &mut run, run_f0, run_at);
                run_f0 = *f0; run_at = s; run.push(*db);
            }
            None => flush(&mut notes, &mut run, run_f0, run_at),
        }
    }
    flush(&mut notes, &mut run, run_f0, run_at);
    notes
}

/// What one track's audit found. At most one of `row` and `skipped` is set;
/// `unjudged` can come with a row.
#[derive(Default)]
struct Track {
    row: Option<Row>,
    findings: Vec<Finding>,
    skipped: Option<usize>,
    unjudged: Option<&'static str>,
}

fn audit_track(song: &Song, name: &str, bars: u32, spb: f32) -> Track {
    let mut out = Track::default();
    let cands = written_notes(song, name);
    if cands.is_empty() { return out }
    let run = |off: Option<Suspect>| -> Option<(Vec<Note>, f32)> {
        let solo = solo_without(song, name, off);
        let compiled = dsl::compiler::compile(&solo).ok()?;
        let mut eng = SongEngine::from_compiled(compiled);
        eng.start();
        let (l, r) = eng.render(bars);
        let mono: Vec<f32> = l.iter().zip(&r).map(|(a, b)| (a + b) * 0.5).collect();
        let peak = mono.iter().fold(0.0f32, |a, v| a.max(v.abs()));
        if peak < 1e-4 { return None }
        Some((measure(&mono, peak, &cands, spb), peak))
    };
    let blind = cannot_judge_notes(song, name);
    let Some((notes, peak)) = run(None) else { return out };
    // Under a handful of notes there is no "its own voice" to compare to.
    // A voice can also land here by being too dirty to measure at all --
    // a supersaw or a heavily chorused pad has most of its energy off the
    // harmonics by design, so every window fails the pitch test and the
    // model simply does not describe it. Saying so is better than
    // producing a number.
    if notes.len() < 8 {
        match blind {
            // The reason it cannot be checked note by note is usually
            // also the reason there are so few notes to check.
            Some(why) => out.unjudged = Some(why),
            None => out.skipped = Some(notes.len()),
        }
        return out;
    }
    let mut all: Vec<f64> = notes.iter().map(|n| n.db).collect();
    let med = median(&mut all);

    // Switching the suspect off and measuring the same voice again is the
    // one comparison that is both absolute and meaningful: it is the same
    // timbre either way, so the difference is caused by the thing that
    // was removed. It is also what would have found the bell bug in one
    // run -- that chorus does not make one note dirty, it raises the
    // whole voice's floor by 17 dB, and the ear notices on the first note
    // because a clean bell is where there is nothing to hide behind.
    let mut blame = Vec::new();
    for suspect in [Suspect::Chorus, Suspect::Drive] {
        if !has_suspect(song, name, suspect) { continue }
        let Some((clean, _)) = run(Some(suspect)) else { continue };
        if clean.len() < 8 { continue }
        let mut c: Vec<f64> = clean.iter().map(|n| n.db).collect();
        let delta = med - median(&mut c);
        if delta >= BLAME_DB {
            blame.push((suspect, delta));
        }
    }
    // A voice written to leave the written pitch cannot be judged by how
    // far it is from the written pitch. Half a semitone of vibrato at
    // C#6 moves the eighth harmonic 107 Hz, well outside the harmonic
    // band, so a guitar solo with bends reads as the dirtiest thing in
    // the song and always will. The comparison below it is unaffected:
    // the vibrato is in both renders, so whatever the difference is, the
    // vibrato is not it.
    out.unjudged = blind;
    for note in &notes {
        let margin = note.db - med;
        if blind.is_some() { break }
        // Below this the harmonics are closer together than the +/-45 Hz
        // band they are measured in, so the bands overlap and the number
        // is arithmetic rather than sound. A sub at D1 has its harmonics
        // 37 Hz apart.
        if note.hz < MIN_PITCH_HZ { continue }
        if margin >= OUTLIER_DB && note.windows >= MIN_STEPS_TO_REPORT {
            out.findings.push(Finding {
                track: name.to_string(),
                bar: note.bar,
                note: hz_name(note.hz),
                steps: note.windows,
                db: note.db,
                margin,
                median: med,
            });
        }
    }
    let worst = notes.into_iter().max_by(|a, b| a.db.total_cmp(&b.db)).unwrap();
    out.row = Some(Row { track: name.to_string(), notes: all.len(), median_db: med, worst, peak, blame });
    out
}

pub fn cmd(args: &[String]) {
    let strict = args.iter().any(|a| a == "--strict");
    let json = args.iter().any(|a| a == "--json");
    let limit: Option<u32> = args.iter().position(|a| a == "--bars")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok());
    let Some(path) = args.iter().enumerate()
        .filter(|(i, _)| *i == 0 || args[i - 1] != "--bars")
        .map(|(_, a)| a)
        .find(|a| !a.starts_with("--"))
    else {
        eprintln!("usage: tatum audit <file.synth> [--bars N] [--json] [--strict]");
        eprintln!("  --bars N  audit only the first N bars; a whole song is not needed to");
        eprintln!("            tell whether a voice is clean, and it is six times slower");
        eprintln!("  --json    the same numbers, for a script");
        eprintln!("  --strict  exit 1 if anything is reported");
        process::exit(2);
    };
    let src = match crate::include::Source::load(std::path::Path::new(path)) {
        Ok(s) => s.text,
        Err(e) => {
            eprintln!("error: {e}");
            process::exit(1);
        }
    };
    let song = match dsl::parse(&src) {
        Ok(s) => s,
        Err(errs) => {
            eprintln!("{path}: parse errors:");
            for e in &errs { eprintln!("  line {}: {}", e.line, e.message); }
            process::exit(1);
        }
    };
    let full = match SongEngine::from_source(&src) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("{path}: {e}");
            process::exit(1);
        }
    };
    let bars = limit.unwrap_or(u32::MAX).min(full.arrangement_bars().max(4));
    let spb = SAMPLE_RATE * 60.0 / full.tempo() * 4.0;
    let names: Vec<String> = (0..full.track_count())
        .filter(|i| full.track_kind(*i) != "beats")
        .map(|i| full.track_name(i).to_string())
        .collect();
    drop(full);

    if !json {
        eprintln!("auditing {} tonal tracks over {bars} bars...", names.len());
    }
    // Every track is its own render of its own solo song, so they run side
    // by side; merged in track order, the report is the same as one after
    // the other.
    let tracks: Vec<Track> = std::thread::scope(|s| {
        let running: Vec<_> = names.iter()
            .map(|name| s.spawn(|| audit_track(&song, name, bars, spb)))
            .collect();
        running.into_iter().map(|t| t.join().expect("audit thread")).collect()
    });
    let mut rows: Vec<Row> = Vec::new();
    let mut findings: Vec<Finding> = Vec::new();
    let mut skipped: Vec<(String, usize)> = Vec::new();
    let mut unjudged: Vec<(String, &'static str)> = Vec::new();
    for (name, t) in names.iter().zip(tracks) {
        if let Some(n) = t.skipped { skipped.push((name.clone(), n)) }
        if let Some(why) = t.unjudged { unjudged.push((name.clone(), why)) }
        findings.extend(t.findings);
        rows.extend(t.row);
    }

    // By name, NOT by score. The absolute number is not comparable between
    // timbres -- a saturated saw measures dirtier than a bell however clean
    // both are -- so ranking voices against each other points at the wrong
    // one. What IS comparable is a note against the other notes of the same
    // voice, which is what the findings below are.
    rows.sort_by(|a, b| a.track.cmp(&b.track));
    findings.sort_by(|a, b| b.margin.total_cmp(&a.margin));
    let blamed: Vec<&Row> = rows.iter().filter(|r| !r.blame.is_empty()).collect();

    if json {
        print_json(path, bars, &rows, &findings, &skipped, &unjudged);
    } else {
        print_table(&rows, &findings, &skipped, &unjudged, &blamed);
    }

    if strict && (!findings.is_empty() || !blamed.is_empty()) {
        process::exit(1);
    }
}

fn print_table(
    rows: &[Row],
    findings: &[Finding],
    skipped: &[(String, usize)],
    unjudged: &[(String, &'static str)],
    blamed: &[&Row],
) {
    println!();
    println!("{:<12} {:>6} {:>10} {:>10} {:>9} {:>6} {:>7}", "track", "notes", "median", "worst", "at bar", "note", "peak");
    for r in rows {
        let flag = if r.peak > 0.99 { "  <-- clipping" } else { "" };
        println!(
            "{:<12} {:>6} {:>7.1} dB {:>7.1} dB {:>9.2} {:>6} {:>7.3}{}",
            r.track, r.notes, r.median_db, r.worst.db, r.worst.bar, hz_name(r.worst.hz), r.peak, flag
        );
    }

    if !blamed.is_empty() {
        println!();
        println!("Switched off one at a time, on the same voice:");
        for r in blamed {
            for (what, delta) in &r.blame {
                println!(
                    "  the {} on `{}` accounts for {:.1} dB of its inharmonic content",
                    what.label(), r.track, delta
                );
            }
        }
    }

    if !unjudged.is_empty() {
        println!();
        println!("Measured, but not checked note by note:");
        for (t, why) in unjudged {
            println!("  {t}: {why}");
        }
    }

    if !skipped.is_empty() {
        println!();
        println!("Not judged, too little of the sound is at harmonics of the written note:");
        for (t, n) in skipped {
            println!("  {t} ({n} notes with a pitch the model recognises)");
        }
    }

    println!();
    if findings.is_empty() {
        println!("No note stands more than {OUTLIER_DB:.0} dB out of its own voice.");
    } else {
        println!("{} note(s) stand out from their own voice:", findings.len());
        for f in findings {
            println!(
                "  {:<12} bar {:<7.2} {:<5} held {:>2} step(s)  {:>6.1} dB, {:.1} dB above this voice's median of {:.1}",
                f.track, f.bar, f.note, f.steps, f.db, f.margin, f.median
            );
        }
    }

    println!();
    println!("Every number is the energy in a note that is NOT at a harmonic of the note the");
    println!("pattern asked for, measured on the voice alone and dry. It is not comparable");
    println!("between voices -- a saturated saw reads dirtier than a bell however clean both");
    println!("are -- so the `median` column is a fingerprint of a timbre, not a score, and the");
    println!("rows are in alphabetical order to make ranking them awkward on purpose.");
    println!();
    println!("Two things here are comparable, and they are the two the tool is for:");
    println!("  - a note against the other notes of the SAME voice, which catches one note");
    println!("    going wrong;");
    println!("  - the same voice with and without one effect, which catches an effect that");
    println!("    dirties all of it. That is the shape the chorus bug turned out to have: it");
    println!("    did not make one note bad, it raised the whole bell 17 dB, and the ear");
    println!("    caught it on the first note because a clean bell has nothing to hide it.");
}

/// Hand-written, like the rest of this crate's JSON: `core` takes no
/// dependencies and neither does the thing that reads it.
fn print_json(
    path: &str,
    bars: u32,
    rows: &[Row],
    findings: &[Finding],
    skipped: &[(String, usize)],
    unjudged: &[(String, &'static str)],
) {
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    println!("{{");
    println!("  \"song\": \"{}\",", esc(path));
    println!("  \"bars\": {bars},");
    println!("  \"tracks\": [");
    for (i, r) in rows.iter().enumerate() {
        let blame: Vec<String> = r.blame.iter()
            .map(|(w, d)| format!("{{\"what\": \"{}\", \"db\": {:.2}}}", w.label(), d))
            .collect();
        println!(
            "    {{\"track\": \"{}\", \"notes\": {}, \"median_db\": {:.2}, \"worst_db\": {:.2}, \"worst_bar\": {:.2}, \"worst_note\": \"{}\", \"peak\": {:.4}, \"blame\": [{}]}}{}",
            esc(&r.track), r.notes, r.median_db, r.worst.db, r.worst.bar, hz_name(r.worst.hz),
            r.peak, blame.join(", "),
            if i + 1 == rows.len() { "" } else { "," }
        );
    }
    println!("  ],");
    println!("  \"findings\": [");
    for (i, f) in findings.iter().enumerate() {
        println!(
            "    {{\"track\": \"{}\", \"bar\": {:.2}, \"note\": \"{}\", \"steps\": {}, \"db\": {:.2}, \"margin_db\": {:.2}, \"median_db\": {:.2}}}{}",
            esc(&f.track), f.bar, f.note, f.steps, f.db, f.margin, f.median,
            if i + 1 == findings.len() { "" } else { "," }
        );
    }
    println!("  ],");
    let list = |v: Vec<String>| v.join(", ");
    println!("  \"not_checked_note_by_note\": [{}],", list(
        unjudged.iter().map(|(t, w)| format!("{{\"track\": \"{}\", \"why\": \"{}\"}}", esc(t), w)).collect()
    ));
    println!("  \"not_measurable\": [{}]", list(
        skipped.iter().map(|(t, n)| format!("{{\"track\": \"{}\", \"notes\": {}}}", esc(t), n)).collect()
    ));
    println!("}}");
}
