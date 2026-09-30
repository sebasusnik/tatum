//! What a `play` line does to a pattern: `rev`, `fast 2`, `every 4 rev`...
//!
//! Every transform here runs at compile time and makes a new pattern; the
//! audio thread only ever reads steps. Transforms that apply on some loops
//! and not others (`every`, `sometimes`, `iter`) compile each version of the
//! pattern and a `PlayPlan` that says which one a loop plays.
//!
//! They work on events -- a note and the ties after it, a chord, a hit --
//! rather than on steps, so a tied note is never cut in two. Where the grid
//! cannot hold what a transform asks for (a note starting half a step in),
//! it keeps as much as a step can: the subdivisions the engine already has.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::dsl::ast::Transform;
use crate::dsl::error::CompileError;

use super::compiled::{CompiledLane, CompiledPattern, CompiledStep, LoopCondition, PlayPlan, SubNote, MAX_SUBDIV};

/// More variants than this is a `play` line nobody could follow.
pub const MAX_VARIANTS: usize = 64;

/// What a transform needs from the song: its scale, for `up N` in degrees.
pub struct Ctx<'a> {
    pub intervals: &'a [u8],
    pub root: u8,
}

/// Patterns for a track's `play` line: the pattern index to loop when it is
/// one fixed pattern, or the plan when loops differ. New patterns are pushed
/// onto `patterns`.
pub fn plan(
    track: &str,
    bases: &[usize],
    transforms: &[Transform],
    text: String,
    patterns: &mut Vec<CompiledPattern>,
    ctx: &Ctx,
) -> Result<(usize, Option<PlayPlan>), CompileError> {
    let conditions: Vec<(LoopCondition, &Transform)> = transforms
        .iter()
        .filter_map(|t| match t {
            Transform::Every(n, inner) => Some((LoopCondition::Every(*n), &**inner)),
            Transform::Sometimes(p, inner) => Some((LoopCondition::Chance(*p), &**inner)),
            Transform::Iter(n) => Some((LoopCondition::Iter(*n), t)),
            _ => None,
        })
        .collect();
    let states: Vec<usize> = conditions.iter().map(|(c, _)| c.states()).collect();
    let total = bases.len() * states.iter().product::<usize>();
    if total > MAX_VARIANTS {
        return Err(CompileError::new(format!(
            "track '{}': `{}` makes {} versions of the pattern; the most is {}",
            track, text, total, MAX_VARIANTS
        )));
    }

    let mut variants = Vec::with_capacity(total);
    for &base in bases {
        // Mixed radix over the conditions' states, the first one slowest.
        for combo in 0..total / bases.len() {
            let mut picks = vec![0usize; states.len()];
            let mut rest = combo;
            for (i, s) in states.iter().enumerate().rev() {
                picks[i] = rest % s;
                rest /= s;
            }
            let mut pat = patterns[base].clone();
            let mut cond = 0;
            for t in transforms {
                match t {
                    Transform::Every(_, inner) | Transform::Sometimes(_, inner) => {
                        if picks[cond] == 1 {
                            let len = pat.len();
                            pat = fit(apply(&pat, inner, ctx)?, len);
                        }
                        cond += 1;
                    }
                    Transform::Iter(n) => {
                        let k = picks[cond] as i32;
                        let len = pat.len() as i32;
                        pat = apply(&pat, &Transform::Shift(-(k * len / *n as i32)), ctx)?;
                        cond += 1;
                    }
                    t => pat = apply(&pat, t, ctx)?,
                }
            }
            pat.name = format!("{} [{} #{}]", patterns[base].name, track, variants.len());
            patterns.push(pat);
            variants.push(patterns.len() - 1);
        }
    }
    if variants.len() == 1 {
        return Ok((variants[0], None));
    }
    let conditions = conditions.into_iter().map(|(c, _)| c).collect();
    Ok((variants[0], Some(PlayPlan { variants, alternatives: bases.len(), conditions, text })))
}

/// One transform on a whole pattern, every lane of a drum pattern alike.
pub fn apply(pat: &CompiledPattern, t: &Transform, ctx: &Ctx) -> Result<CompiledPattern, CompileError> {
    let mut out = pat.clone();
    if pat.lanes.is_empty() {
        out.steps = steps(&pat.steps, t, ctx, false);
    } else {
        out.lanes =
            pat.lanes.iter().map(|l| CompiledLane { steps: steps(&l.steps, t, ctx, true), ..l.clone() }).collect();
    }
    Ok(out)
}

/// A pattern cut or padded to `len` steps: what `every` and `sometimes` do,
/// so a loop that is transformed takes as long as one that is not.
fn fit(mut pat: CompiledPattern, len: usize) -> CompiledPattern {
    let fit_steps = |s: &mut Vec<CompiledStep>| s.resize(len, CompiledStep::Rest);
    if pat.lanes.is_empty() {
        fit_steps(&mut pat.steps);
    } else {
        for l in pat.lanes.iter_mut() {
            fit_steps(&mut l.steps);
        }
    }
    pat
}

impl CompiledPattern {
    /// Steps in one loop.
    pub fn len(&self) -> usize {
        match self.lanes.first() {
            Some(l) => l.steps.len(),
            None => self.steps.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn steps(s: &[CompiledStep], t: &Transform, ctx: &Ctx, drums: bool) -> Vec<CompiledStep> {
    if s.is_empty() {
        return Vec::new();
    }
    match t {
        Transform::Rev => rev(s),
        Transform::Shift(n) => shift(s, *n),
        Transform::Fast(n) => fast(s, *n as usize, drums),
        Transform::Slow(n) => slow(s, *n as usize, drums),
        Transform::Ply(n) => s.iter().map(|st| ply(st, *n as usize)).collect(),
        Transform::Up { amount, semitones } => s
            .iter()
            .map(|st| pitch(st, |m| if *semitones { m as i32 + amount } else { degree_up(m, *amount, ctx) }))
            .collect(),
        Transform::Octave(n) => s.iter().map(|st| pitch(st, |m| m as i32 + 12 * n)).collect(),
        Transform::Degrade(p) => s.iter().map(|st| degrade(st, *p)).collect(),
        // Conditions are handled by `plan`; alone they are a plain loop.
        Transform::Every(..) | Transform::Sometimes(..) | Transform::Iter(_) => s.to_vec(),
    }
}

/// Where each event starts and how many steps it lasts (itself and its ties).
fn events(s: &[CompiledStep]) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    for (i, st) in s.iter().enumerate() {
        if st.is_onset() {
            out.push((i, 1));
        } else if matches!(st, CompiledStep::Tie) {
            if let Some(last) = out.last_mut() {
                if last.0 + last.1 == i {
                    last.1 += 1;
                }
            }
        }
    }
    out
}

fn rev(s: &[CompiledStep]) -> Vec<CompiledStep> {
    let len = s.len();
    let mut out = vec![CompiledStep::Rest; len];
    for (start, dur) in events(s) {
        let at = len - (start + dur);
        out[at] = reverse_inside(s[start]);
        for o in out.iter_mut().skip(at + 1).take(dur - 1) {
            *o = CompiledStep::Tie;
        }
    }
    out
}

/// A run inside a step, backwards: the whole pattern read the other way.
fn reverse_inside(st: CompiledStep) -> CompiledStep {
    match st {
        CompiledStep::Subdiv { mut notes, count, plock } => {
            notes[..count as usize].reverse();
            CompiledStep::Subdiv { notes, count, plock }
        }
        CompiledStep::DrumSub { mut hits, count, plock } => {
            hits[..count as usize].reverse();
            CompiledStep::DrumSub { hits, count, plock }
        }
        other => other,
    }
}

fn shift(s: &[CompiledStep], n: i32) -> Vec<CompiledStep> {
    let len = s.len() as i32;
    let n = n.rem_euclid(len) as usize;
    let mut out = s.to_vec();
    out.rotate_right(n);
    out
}

/// The pattern `n` times inside its own length: each group of `n` steps
/// becomes one, a subdivision when more than one thing starts in it.
fn fast(s: &[CompiledStep], n: usize, drums: bool) -> Vec<CompiledStep> {
    let len = s.len();
    let mut out = Vec::with_capacity(len);
    // The note sounding at the end of the last step, for slots that hold it.
    let mut held: Option<SubNote> = None;
    for j in 0..len {
        let group: Vec<CompiledStep> = (0..n).map(|k| s[(j * n + k) % len]).collect();
        let onsets = group.iter().filter(|g| g.is_onset()).count();
        let step = if onsets == 0 {
            match group[0] {
                CompiledStep::Tie => CompiledStep::Tie,
                _ => CompiledStep::Rest,
            }
        } else if onsets == 1 && group[0].is_onset() {
            group[0]
        } else if drums {
            let mut hits = [0.0; MAX_SUBDIV];
            for (k, g) in group.iter().enumerate().take(MAX_SUBDIV) {
                hits[k] = hit_velocity(g);
            }
            CompiledStep::DrumSub { hits, count: n.min(MAX_SUBDIV) as u8, plock: plock_of(&group) }
        } else {
            let mut notes = [SubNote::default(); MAX_SUBDIV];
            let mut count = 0usize;
            for g in &group {
                if count == MAX_SUBDIV {
                    break;
                }
                notes[count] = match g {
                    // A slot that holds the note: glide to where it already
                    // is, so a bass does not strike it again.
                    CompiledStep::Tie => held.map(|h| SubNote { slide: true, ..h }).unwrap_or_default(),
                    // A slot of silence: a note at velocity 0, which the
                    // sequencer plays as a release.
                    CompiledStep::Rest => SubNote { midi_note: 0, velocity: 0.0, slide: false },
                    other => first_note(other),
                };
                count += 1;
            }
            CompiledStep::Subdiv { notes, count: count as u8, plock: plock_of(&group) }
        };
        if let Some(last) = last_note(&step) {
            held = Some(last);
        } else if matches!(step, CompiledStep::Rest) {
            held = None;
        }
        out.push(step);
    }
    out
}

/// Every step `n` steps long.
fn slow(s: &[CompiledStep], n: usize, drums: bool) -> Vec<CompiledStep> {
    let mut out = Vec::with_capacity(s.len() * n);
    let hold = if drums { CompiledStep::Rest } else { CompiledStep::Tie };
    for st in s {
        match *st {
            CompiledStep::Subdiv { notes, count, plock } => {
                let count = count as usize;
                let mut cells: Vec<Vec<SubNote>> = vec![Vec::new(); n];
                for (k, note) in notes.iter().take(count).enumerate() {
                    cells[k * n / count].push(*note);
                }
                for cell in cells {
                    out.push(match cell.len() {
                        0 => hold,
                        1 if cell[0].velocity == 0.0 => CompiledStep::Rest,
                        1 => CompiledStep::NoteOn {
                            midi_note: cell[0].midi_note,
                            velocity: cell[0].velocity,
                            plock,
                            slide: cell[0].slide,
                        },
                        _ => {
                            let mut sub = [SubNote::default(); MAX_SUBDIV];
                            sub[..cell.len()].copy_from_slice(&cell);
                            CompiledStep::Subdiv { notes: sub, count: cell.len() as u8, plock }
                        }
                    });
                }
            }
            CompiledStep::DrumSub { hits, count, plock } => {
                let count = count as usize;
                let mut cells: Vec<Vec<f32>> = vec![Vec::new(); n];
                for (k, v) in hits.iter().take(count).enumerate() {
                    cells[k * n / count].push(*v);
                }
                for cell in cells {
                    out.push(match cell.as_slice() {
                        [] => CompiledStep::Rest,
                        [v] if *v <= 0.0 => CompiledStep::Rest,
                        [v] => CompiledStep::DrumHit { velocity: *v, probability: 1.0, roll: 1, plock },
                        many => {
                            let mut h = [0.0; MAX_SUBDIV];
                            h[..many.len()].copy_from_slice(many);
                            CompiledStep::DrumSub { hits: h, count: many.len() as u8, plock }
                        }
                    });
                }
            }
            CompiledStep::Tie => out.extend(core::iter::repeat_n(CompiledStep::Tie, n)),
            CompiledStep::Rest => out.extend(core::iter::repeat_n(CompiledStep::Rest, n)),
            other => {
                out.push(other);
                let after = if matches!(other, CompiledStep::DrumHit { .. }) { CompiledStep::Rest } else { hold };
                out.extend(core::iter::repeat_n(after, n - 1));
            }
        }
    }
    out
}

/// Each event `n` times inside its step.
fn ply(st: &CompiledStep, n: usize) -> CompiledStep {
    match *st {
        CompiledStep::NoteOn { midi_note, velocity, plock, slide } => {
            let mut notes = [SubNote::default(); MAX_SUBDIV];
            for (k, note) in notes.iter_mut().take(n).enumerate() {
                *note = SubNote { midi_note, velocity, slide: slide && k == 0 };
            }
            CompiledStep::Subdiv { notes, count: n as u8, plock }
        }
        CompiledStep::Subdiv { notes, count, plock } if count as usize * n <= MAX_SUBDIV => {
            let mut out = [SubNote::default(); MAX_SUBDIV];
            for (k, o) in out.iter_mut().take(count as usize * n).enumerate() {
                *o = SubNote { slide: false, ..notes[k / n] };
            }
            CompiledStep::Subdiv { notes: out, count: count * n as u8, plock }
        }
        CompiledStep::DrumHit { velocity, plock, .. } => {
            let mut hits = [0.0; MAX_SUBDIV];
            hits[..n].fill(velocity);
            CompiledStep::DrumSub { hits, count: n as u8, plock }
        }
        CompiledStep::DrumSub { hits, count, plock } if count as usize * n <= MAX_SUBDIV => {
            let mut out = [0.0; MAX_SUBDIV];
            for (k, o) in out.iter_mut().take(count as usize * n).enumerate() {
                *o = hits[k / n];
            }
            CompiledStep::DrumSub { hits: out, count: count * n as u8, plock }
        }
        other => other,
    }
}

fn pitch(st: &CompiledStep, f: impl Fn(u8) -> i32) -> CompiledStep {
    let clamp = |m: u8| f(m).clamp(0, 127) as u8;
    match *st {
        CompiledStep::NoteOn { midi_note, velocity, plock, slide } => {
            CompiledStep::NoteOn { midi_note: clamp(midi_note), velocity, plock, slide }
        }
        CompiledStep::Chord { mut notes, count, plock } => {
            for n in notes.iter_mut().take(count as usize) {
                n.midi_note = clamp(n.midi_note);
            }
            CompiledStep::Chord { notes, count, plock }
        }
        CompiledStep::Subdiv { mut notes, count, plock } => {
            for n in notes.iter_mut().take(count as usize) {
                if n.velocity > 0.0 {
                    n.midi_note = clamp(n.midi_note);
                }
            }
            CompiledStep::Subdiv { notes, count, plock }
        }
        other => other,
    }
}

/// `n` degrees of the scale from `midi`. A note outside the scale moves with
/// the scale note below it, keeping its distance from it.
fn degree_up(midi: u8, n: i32, ctx: &Ctx) -> i32 {
    let iv = ctx.intervals;
    if iv.is_empty() {
        return midi as i32;
    }
    let rel = (midi as i32 - ctx.root as i32).rem_euclid(12);
    let tonic = midi as i32 - rel;
    let idx = iv.iter().rposition(|&i| i as i32 <= rel).unwrap_or(0);
    let offset = rel - iv[idx] as i32;
    let target = idx as i32 + n;
    let octave = target.div_euclid(iv.len() as i32);
    let degree = target.rem_euclid(iv.len() as i32) as usize;
    tonic + 12 * octave + iv[degree] as i32 + offset
}

fn degrade(st: &CompiledStep, p: f32) -> CompiledStep {
    let keep = 1.0 - p;
    let scaled = |q: Option<f32>| Some(q.unwrap_or(1.0) * keep);
    match *st {
        CompiledStep::DrumHit { velocity, probability, roll, plock } => {
            CompiledStep::DrumHit { velocity, probability: probability * keep, roll, plock }
        }
        CompiledStep::NoteOn { midi_note, velocity, mut plock, slide } => {
            plock.probability = scaled(plock.probability);
            CompiledStep::NoteOn { midi_note, velocity, plock, slide }
        }
        CompiledStep::Chord { notes, count, mut plock } => {
            plock.probability = scaled(plock.probability);
            CompiledStep::Chord { notes, count, plock }
        }
        CompiledStep::Subdiv { notes, count, mut plock } => {
            plock.probability = scaled(plock.probability);
            CompiledStep::Subdiv { notes, count, plock }
        }
        CompiledStep::DrumSub { hits, count, mut plock } => {
            plock.probability = scaled(plock.probability);
            CompiledStep::DrumSub { hits, count, plock }
        }
        other => other,
    }
}

fn hit_velocity(st: &CompiledStep) -> f32 {
    match st {
        CompiledStep::DrumHit { velocity, .. } => *velocity,
        CompiledStep::DrumSub { hits, .. } => hits[0],
        CompiledStep::NoteOn { velocity, .. } => *velocity,
        _ => 0.0,
    }
}

fn first_note(st: &CompiledStep) -> SubNote {
    match *st {
        CompiledStep::NoteOn { midi_note, velocity, slide, .. } => SubNote { midi_note, velocity, slide },
        CompiledStep::Chord { notes, .. } => {
            SubNote { midi_note: notes[0].midi_note, velocity: notes[0].velocity, slide: false }
        }
        CompiledStep::Subdiv { notes, .. } => notes[0],
        _ => SubNote::default(),
    }
}

fn last_note(st: &CompiledStep) -> Option<SubNote> {
    match *st {
        CompiledStep::NoteOn { midi_note, velocity, slide, .. } => Some(SubNote { midi_note, velocity, slide }),
        CompiledStep::Chord { notes, .. } => {
            Some(SubNote { midi_note: notes[0].midi_note, velocity: notes[0].velocity, slide: false })
        }
        CompiledStep::Subdiv { notes, count, .. } => {
            notes[..count as usize].iter().rev().find(|n| n.velocity > 0.0).copied()
        }
        _ => None,
    }
}

fn plock_of(group: &[CompiledStep]) -> super::compiled::StepPLock {
    group
        .iter()
        .find_map(|g| match g {
            CompiledStep::NoteOn { plock, .. }
            | CompiledStep::Chord { plock, .. }
            | CompiledStep::Subdiv { plock, .. }
            | CompiledStep::DrumHit { plock, .. }
            | CompiledStep::DrumSub { plock, .. } => Some(*plock),
            _ => None,
        })
        .unwrap_or_default()
}

/// The `play` line as written, for the screen: `acid every 4 rev`.
pub fn describe(play: &str, also: &[String], transforms: &[Transform]) -> String {
    let mut s = String::from(play);
    for a in also {
        s += ", ";
        s += a;
    }
    for t in transforms {
        s += " ";
        s += &word(t);
    }
    s
}

fn word(t: &Transform) -> String {
    match t {
        Transform::Rev => "rev".into(),
        Transform::Fast(n) => format!("fast {}", n),
        Transform::Slow(n) => format!("slow {}", n),
        Transform::Shift(n) => format!("shift {}", n),
        Transform::Up { amount, semitones: true } => format!("up {}st", amount),
        Transform::Up { amount, .. } => format!("up {}", amount),
        Transform::Octave(n) => format!("octave {}", n),
        Transform::Degrade(p) => format!("degrade {}%", (p * 100.0 + 0.5) as u32),
        Transform::Ply(n) => format!("ply {}", n),
        Transform::Iter(n) => format!("iter {}", n),
        Transform::Every(n, inner) => format!("every {} {}", n, word(inner)),
        Transform::Sometimes(p, inner) => format!("sometimes {}% {}", (p * 100.0 + 0.5) as u32, word(inner)),
    }
}
