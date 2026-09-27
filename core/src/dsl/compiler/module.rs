//! Module instruments: `module bass { ... }` and friends become presets,
//! every parameter checked against the registry on the way.

use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;

use crate::dsl::ast::*;
use crate::dsl::error::CompileError;
use crate::params::{self, ModuleKind};

use super::compiled::{CompiledInstrumentKind, FmPreset, ModulePreset};


// ── Module compilation ──

pub(super) fn compile_module_def(mod_def: &ModuleDef) -> Result<CompiledInstrumentKind, Vec<CompileError>> {
    let kind = match ModuleKind::from_str(&mod_def.module_type) {
        Some(k) => k,
        None => return Err(vec![CompileError::new(format!(
            "module '{}': unknown module type '{}' (expected bass, fm, keys, or beats)",
            mod_def.name, mod_def.module_type
        ))]),
    };

    let mut errors = Vec::new();
    let mut valid: Vec<(String, f32)> = Vec::new();
    for p in &mod_def.params {
        match params::lookup(kind, &p.name) {
            Some(spec) => match spec.validate(p.value) {
                Ok(()) => valid.push((p.name.clone(), p.value)),
                Err(msg) => errors.push(CompileError::at(p.line, format!("module '{}': {}", mod_def.name, msg))),
            },
            None => errors.push(CompileError::at(p.line, unknown_param_message(kind, &mod_def.name, &p.name))),
        }
    }
    for env in &mod_def.op_envelopes {
        if kind != ModuleKind::Fm {
            errors.push(CompileError::new(format!(
                "module '{}': op{}_envelope is only valid on fm modules", mod_def.name, env.op_index
            )));
        } else if env.op_index > 3 {
            errors.push(CompileError::new(format!(
                "module '{}': op{}_envelope — operators are op0..op3", mod_def.name, env.op_index
            )));
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }

    Ok(match kind {
        ModuleKind::Bass => {
            let preset = ModulePreset { params: valid };
            CompiledInstrumentKind::Bass(preset)
        }
        ModuleKind::Keys => {
            let preset = ModulePreset { params: valid };
            CompiledInstrumentKind::Keys(preset)
        }
        ModuleKind::Beats => {
            let preset = ModulePreset { params: valid };
            CompiledInstrumentKind::Beats(preset)
        }
        ModuleKind::Fm => {
            let mut preset = FmPreset { params: valid, ..Default::default() };
            for env in &mod_def.op_envelopes {
                preset.op_envelopes.push((env.op_index, (env.a, env.d, env.s, env.r)));
            }
            CompiledInstrumentKind::Fm(preset)
        }
    })
}

/// Explain an unknown parameter name: typo suggestion or wrong module type.
pub(super) fn unknown_param_message(kind: ModuleKind, module_name: &str, param: &str) -> String {
    let base = format!("module '{}' ({}): unknown parameter '{}'", module_name, kind.as_str(), param);
    if let Some(sugg) = params::suggest(kind, param) {
        return format!("{}. Did you mean '{}'?", base, sugg);
    }
    let others = params::kinds_with_param(param);
    if !others.is_empty() {
        let names: Vec<&str> = others.iter().map(|k| k.as_str()).collect();
        return format!("{}. It exists on: {}", base, names.join(", "));
    }
    format!("{}. Run `tatum params {}` for the list", base, kind.as_str())
}
