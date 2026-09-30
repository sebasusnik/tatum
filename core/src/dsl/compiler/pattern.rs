//! Patterns: rows of steps become flat step arrays, or one lane per drum
//! when the rows are labelled.

use alloc::vec::Vec;
use alloc::format;

use crate::dsl::ast::*;
use crate::dsl::error::CompileError;

use super::compiled::{
    ChordNote, CompiledLane, CompiledPattern, CompiledStep, StepPLock, SubNote, MAX_CHORD_NOTES, MAX_SUBDIV,
};
use super::notes::{drum_name_to_midi, resolve_note};

// ── Pattern compilation ──

pub(super) fn compile_pattern(
    pat: &PatternDef,
    scale_intervals: &[u8],
    root_midi: u8,
) -> Result<CompiledPattern, CompileError> {
    // Multi-lane drum pattern
    if !pat.lane_labels.is_empty() {
        let mut lanes = Vec::new();
        for (i, row) in pat.rows.iter().enumerate() {
            let label = if i < pat.lane_labels.len() {
                &pat.lane_labels[i]
            } else {
                continue;
            };
            let midi_note = drum_name_to_midi(label);
            if midi_note == 0 {
                return Err(CompileError::new(format!(
                    "pattern '{}': unknown drum lane '{}' (kick, snare, clap, hat, openhat, tom, tom2, tom3, crash)",
                    pat.name, label
                )));
            }
            let steps: Vec<CompiledStep> = row
                .iter()
                .map(|step| {
                    match step {
                        Step::Subdiv(_) => CompiledStep::Rest,
                        Step::DrumSub(hits) => drum_sub(hits),
                        Step::DrumHit(ds) => {
                            let plock = lock(&ds.plock);
                            CompiledStep::DrumHit {
                                velocity: ds.velocity,
                                probability: ds.probability,
                                roll: ds.roll,
                                plock,
                            }
                        }
                        Step::Rest => CompiledStep::Rest,
                        Step::Tie => CompiledStep::Tie,
                        Step::Chord(_) => CompiledStep::Rest, // chords not supported in drum lanes
                        Step::Note(ns) => {
                            let midi = resolve_note(&ns.note, scale_intervals, root_midi);
                            let vel = ns.velocity.unwrap_or(0.8);
                            let plock = lock(&ns.plock);
                            CompiledStep::NoteOn { midi_note: midi, velocity: vel, plock, slide: ns.slide }
                        }
                    }
                })
                .collect();
            lanes.push(CompiledLane { midi_note, steps, swing_override: None, nudge: 0.0 });
        }
        let steps_per_row = lanes.first().map(|l| l.steps.len()).unwrap_or(4);
        return Ok(CompiledPattern { name: pat.name.clone(), steps: Vec::new(), steps_per_row, lanes });
    }

    // Sequential pattern (existing behavior)
    let mut steps = Vec::new();
    let mut max_row_len = 0;

    for row in &pat.rows {
        let row_len = row.len();
        if row_len > max_row_len {
            max_row_len = row_len;
        }
        for step in row {
            match step {
                Step::Note(ns) => {
                    let midi = resolve_note(&ns.note, scale_intervals, root_midi);
                    let vel = ns.velocity.unwrap_or(0.8);
                    let plock = lock(&ns.plock);
                    steps.push(CompiledStep::NoteOn { midi_note: midi, velocity: vel, plock, slide: ns.slide });
                }
                Step::Chord(cs) => {
                    let mut chord_notes = [ChordNote::default(); MAX_CHORD_NOTES];
                    let shared_vel = cs.velocity.unwrap_or(0.8);
                    let count = cs.notes.len().min(MAX_CHORD_NOTES);
                    for (i, ns) in cs.notes.iter().take(MAX_CHORD_NOTES).enumerate() {
                        chord_notes[i] = ChordNote {
                            midi_note: resolve_note(&ns.note, scale_intervals, root_midi),
                            velocity: ns.velocity.unwrap_or(shared_vel),
                        };
                    }
                    let plock = lock(&cs.plock);
                    steps.push(CompiledStep::Chord { notes: chord_notes, count: count as u8, plock });
                }
                Step::Subdiv(subs) => {
                    let mut notes = [SubNote::default(); MAX_SUBDIV];
                    let count = subs.len().min(MAX_SUBDIV);
                    for (i, ns) in subs.iter().take(MAX_SUBDIV).enumerate() {
                        notes[i] = SubNote {
                            midi_note: resolve_note(&ns.note, scale_intervals, root_midi),
                            velocity: ns.velocity.unwrap_or(0.8),
                            slide: ns.slide,
                        };
                    }
                    let plock = subs.first().map(|ns| lock(&ns.plock)).unwrap_or_default();
                    steps.push(CompiledStep::Subdiv { notes, count: count as u8, plock });
                }
                Step::DrumHit(ds) => {
                    let plock = lock(&ds.plock);
                    steps.push(CompiledStep::DrumHit {
                        velocity: ds.velocity,
                        probability: ds.probability,
                        roll: ds.roll,
                        plock,
                    });
                }
                Step::DrumSub(hits) => steps.push(drum_sub(hits)),
                Step::Rest => {
                    steps.push(CompiledStep::Rest);
                }
                Step::Tie => {
                    steps.push(CompiledStep::Tie);
                }
            }
        }
    }

    Ok(CompiledPattern {
        name: pat.name.clone(),
        steps,
        steps_per_row: if max_row_len > 0 { max_row_len } else { 4 },
        lanes: Vec::new(),
    })
}

/// A step's locks as the engine reads them.
pub(super) fn lock(p: &PLock) -> StepPLock {
    StepPLock {
        cutoff: p.cutoff,
        env_depth: p.env_depth,
        resonance: p.resonance,
        gate: p.gate,
        probability: p.probability,
    }
}

fn drum_sub(hits: &[f32]) -> CompiledStep {
    let mut out = [0.0; MAX_SUBDIV];
    let count = hits.len().min(MAX_SUBDIV);
    out[..count].copy_from_slice(&hits[..count]);
    CompiledStep::DrumSub { hits: out, count: count as u8, plock: StepPLock::default() }
}
