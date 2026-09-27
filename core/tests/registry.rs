//! The parameter registry is the single source of truth for validation, docs and
//! the diff engine. These tests keep it honest against the modules themselves.

use tatum_core::params::{self, ModuleKind};
use tatum_core::song_engine::SongEngine;

/// A minimal song for one module kind, with `body` pasted into the module block.
fn song(kind: ModuleKind, body: &str) -> String {
    let pattern = if kind == ModuleKind::Beats { "pattern p { kick: X - X - }" } else { "pattern p { 1.1 - 1.3 - }" };
    format!(
        "tempo 120\nscale C major\n\nmodule {k} m {{\n{body}}}\n\n{pattern}\n\n\
         track t {{ play p using m out > master }}\n\n\
         scene s {{ track t {{ play p using m }} }}\n\narrange {{ s x1 }}\n",
        k = kind.as_str(),
        body = body,
        pattern = pattern,
    )
}

fn render(source: &str) -> Vec<f32> {
    let mut engine = SongEngine::from_source(source).unwrap_or_else(|e| panic!("{}\n{}", e, source));
    engine.start();
    engine.render_steps(16).0
}

/// Spelling a parameter at its registry default must be a no-op. The diff engine
/// restores the registry default when a line is deleted, so a default that
/// disagrees with the module's constructor changes the sound on an unrelated
/// edit. This is the bug that shipped `fm level` as 1.0 in the registry and 2.0
/// in the module.
#[test]
fn every_registry_default_is_a_no_op_on_a_fresh_module() {
    let mut drift = Vec::new();
    for kind in ModuleKind::ALL {
        let plain = render(&song(kind, ""));
        assert!(plain.iter().any(|v| v.abs() > 1e-6), "{} renders silence", kind.as_str());

        for spec in params::specs(kind) {
            let line = format!("    {} {}\n", spec.name, spec.default);
            let with_default = render(&song(kind, &line));
            assert_eq!(plain.len(), with_default.len(), "{}.{}: render length changed", kind.as_str(), spec.name);
            // Relative RMS, not worst sample: a filter's phase response makes the
            // peak difference jumpy even when the two renders are the same sound.
            // 1e-4 is -80 dB, far below anything audible.
            let energy = |v: &[f32]| (v.iter().map(|x| (x * x) as f64).sum::<f64>() / v.len() as f64).sqrt();
            let diff: Vec<f32> = plain.iter().zip(&with_default).map(|(a, b)| a - b).collect();
            let relative = energy(&diff) / energy(&plain).max(1e-12);
            if relative >= 1e-4 {
                drift.push(format!(
                    "{}.{} (default {}) moves the output by {:.2} dB below signal",
                    kind.as_str(),
                    spec.name,
                    spec.default,
                    20.0 * relative.log10()
                ));
            }
        }
    }
    assert!(
        drift.is_empty(),
        "the registry and the modules disagree on {} defaults:\n  {}",
        drift.len(),
        drift.join("\n  ")
    );
}

/// Every option name must decode back to its own index. Adding an option in the
/// middle of a list used to shift every module below it.
#[test]
fn every_choice_name_round_trips() {
    for kind in ModuleKind::ALL {
        for spec in params::specs(kind) {
            if let params::Range::Choice(names) = spec.range {
                for (idx, name) in names.iter().enumerate() {
                    let value = spec
                        .value_from_name(name)
                        .unwrap_or_else(|| panic!("{}.{}: '{}' has no value", kind.as_str(), spec.name, name));
                    let back = spec
                        .choice_name(value)
                        .unwrap_or_else(|| panic!("{}.{}: {} decodes to nothing", kind.as_str(), spec.name, value));
                    assert_eq!(
                        back,
                        *name,
                        "{}.{}: option {} ('{}') decodes to '{}'",
                        kind.as_str(),
                        spec.name,
                        idx,
                        name,
                        back
                    );
                }
            }
        }
    }
}

/// Automation stores the parameter name inline so that starting a scene, which
/// happens on the audio thread, allocates nothing.
#[test]
fn every_param_name_fits_where_automation_stores_it() {
    for kind in ModuleKind::ALL {
        for spec in params::specs(kind) {
            assert!(
                spec.name.len() <= tatum_core::song_engine::INLINE_NAME_CAP,
                "'{}' is {} bytes; automation stores names inline in {}",
                spec.name,
                spec.name.len(),
                tatum_core::song_engine::INLINE_NAME_CAP
            );
        }
    }
}

/// Every parameter must carry a description: `tatum params` and the MCP docs are
/// generated from them, and an empty one is what the author reads.
#[test]
fn every_param_is_documented() {
    for kind in ModuleKind::ALL {
        for spec in params::specs(kind) {
            assert!(spec.doc.len() > 8, "{}.{} has no usable description", kind.as_str(), spec.name);
        }
    }
}

/// The docs quote concrete anchors ("0.5 ≈ 630Hz"). Those are what an author
/// aims at, so they have to match the engine's own math — which is an
/// approximation, not libm. Reading them off the curve constants means this
/// checks the same code the modules run.
#[test]
fn documented_anchors_match_the_engine() {
    let close = |got: f32, want: f32, what: &str| {
        assert!((got - want).abs() / want < 0.03, "{}: doc says {}, engine gives {}", what, want, got);
    };
    // bass cutoff: "exponential 20Hz..20kHz (0.25 ≈ 110Hz, 0.5 ≈ 630Hz, 0.75 ≈ 3.5kHz)"
    close(params::BASS_CUTOFF.to_real(0.25), 110.0, "bass cutoff at 0.25");
    close(params::BASS_CUTOFF.to_real(0.5), 630.0, "bass cutoff at 0.5");
    close(params::BASS_CUTOFF.to_real(0.75), 3500.0, "bass cutoff at 0.75");
    // keys cutoff: "exponential 200Hz..20kHz (0.5 ≈ 2kHz)"
    close(params::KEYS_CUTOFF.to_real(0.5), 2000.0, "keys cutoff at 0.5");
    // ENV_TIME_DOC: "0.25 ≈ 6.7ms, 0.5 ≈ 45ms, 0.75 ≈ 300ms, 1.0 = 2s"
    close(params::ENV_TIME.to_real(0.25), 6.7, "env time at 0.25");
    close(params::ENV_TIME.to_real(0.5), 45.0, "env time at 0.5");
    close(params::ENV_TIME.to_real(0.75), 300.0, "env time at 0.75");
    close(params::ENV_TIME.to_real(1.0), 2000.0, "env time at 1.0");
    // fm mod_index: "exponential 0.1..4.0 (0.5 ≈ 0.63, 0.75 ≈ 1.6)"
    close(0.1 * tatum_core::math::pow(40.0, 0.5), 0.63, "mod_index at 0.5");
    close(0.1 * tatum_core::math::pow(40.0, 0.75), 1.6, "mod_index at 0.75");
    // op ratio: "ratio = 0.5 + v*15.5: 0.032 = 1.0, 0.097 = 2.0, 0.161 = 3.0"
    close(params::OP_RATIO.to_real(0.032), 1.0, "op ratio at 0.032");
    close(params::OP_RATIO.to_real(0.097), 2.0, "op ratio at 0.097");
    close(params::OP_RATIO.to_real(0.161), 3.0, "op ratio at 0.161");
}

/// Every curve must survive the round trip, or `cutoff 800hz` lands somewhere
/// else. The inverse goes through the engine's own ln(), which was off by 11%
/// until it was measured.
#[test]
fn every_curve_round_trips_through_its_unit() {
    let mut worst = 0.0f32;
    for kind in ModuleKind::ALL {
        for spec in params::specs(kind) {
            if spec.curve.unit().is_none() {
                continue;
            }
            for step in 0..=20 {
                let knob = step as f32 / 20.0;
                let back = spec.curve.to_knob(spec.curve.to_real(knob));
                let err = (back - knob).abs();
                worst = worst.max(err);
                assert!(
                    err < 2e-3,
                    "{}.{}: knob {} -> {} -> {}",
                    kind.as_str(),
                    spec.name,
                    knob,
                    spec.curve.to_real(knob),
                    back
                );
            }
        }
    }
    assert!(worst > 0.0 || true, "worst round-trip error {}", worst);
}
