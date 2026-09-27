//! Reading a node's parameters: by position, by name, as a rhythm division,
//! and the filter envelope and LFO options every filter shares.

use crate::dsl::ast::*;

// ── Param helpers ──

pub(super) fn first_float_param(params: &[Param]) -> Option<f32> {
    for p in params {
        match p {
            Param::Float(v) => return Some(*v),
            Param::Expr(e) => return Some(e.eval()),
            _ => {}
        }
    }
    None
}

pub(super) fn float_param_at(params: &[Param], index: usize) -> Option<f32> {
    let mut count = 0;
    for p in params {
        match p {
            Param::Float(v) => {
                if count == index {
                    return Some(*v);
                }
                count += 1;
            }
            Param::Expr(e) => {
                if count == index {
                    return Some(e.eval());
                }
                count += 1;
            }
            Param::Waveform(_) => {} // skip waveforms in positional count
            Param::Named(_, _) => {} // skip named
            Param::RhythmDiv(_, _) => {
                if count == index {
                    // Convert rhythm div to float (not directly useful as freq)
                    return None;
                }
                count += 1;
            }
        }
    }
    None
}

pub(super) fn named_param(params: &[Param], name: &str) -> Option<f32> {
    for p in params {
        if let Param::Named(n, v) = p {
            if n == name {
                return Some(*v);
            }
        }
    }
    None
}

pub(super) fn rhythm_div_param(params: &[Param]) -> Option<f32> {
    for p in params {
        if let Param::RhythmDiv(num, den) = p {
            return Some(*num as f32 / *den as f32);
        }
    }
    None
}

/// Filter LFO options: lfo_hz / lfo_bars (cycle length) and lfo_depth (Hz).
pub(super) fn filter_lfo_params(params: &[Param]) -> crate::graph::node::FilterLfo {
    let bars = named_param(params, "lfo_bars").unwrap_or(0.0);
    let hz = named_param(params, "lfo_hz").unwrap_or(0.0);
    let depth = named_param(params, "lfo_depth").unwrap_or(0.0);
    crate::graph::node::FilterLfo { hz, bars, depth }
}

/// Extract filter envelope named params: ea, ed, es, er, edepth.
/// Returns (attack, decay, sustain, release, depth) with defaults.
pub(super) fn filter_env_params(params: &[Param]) -> (f32, f32, f32, f32, f32) {
    let ea = named_param(params, "ea").unwrap_or(0.005);
    let ed = named_param(params, "ed").unwrap_or(0.2);
    let es = named_param(params, "es").unwrap_or(0.0);
    let er = named_param(params, "er").unwrap_or(0.1);
    let edepth = named_param(params, "edepth").unwrap_or(0.0);
    (ea, ed, es, er, edepth)
}
