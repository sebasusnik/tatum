//! `tatum fmt --units`: rewrite every module parameter written as a knob
//! position (`cutoff 0.1`) in its unit (`cutoff 179.35hz`), in place. Each
//! value is written with as few decimals as read back to the same knob, so
//! the song sounds the same afterwards.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process;

use tatum_core::params::{self, ModuleKind};

use crate::include::Source;

const USAGE: &str = "usage: tatum fmt --units <song.synth>";

pub fn cmd(args: &[String]) {
    let units = args.iter().any(|a| a == "--units");
    let Some(path) = args.iter().find(|a| !a.starts_with('-')) else {
        eprintln!("{USAGE}");
        process::exit(1);
    };
    if !units {
        eprintln!("error: `fmt` only knows `--units` for now\n{USAGE}");
        process::exit(1);
    }
    let source = match Source::load(Path::new(path)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            process::exit(1);
        }
    };
    let ast = match tatum_core::dsl::parse(&source.text) {
        Ok(ast) => ast,
        Err(errs) => {
            source.print_errors(&tatum_core::song_engine::DslError::Parse(errs));
            process::exit(1);
        }
    };

    // file -> line -> [(param, new text)]
    let mut edits: BTreeMap<PathBuf, BTreeMap<usize, Vec<(String, String)>>> = BTreeMap::new();
    for m in &ast.module_defs {
        let Some(kind) = ModuleKind::from_str(&m.module_type) else { continue };
        for p in m.params.iter().filter(|p| p.bare) {
            let Some(spec) = params::lookup(kind, &p.name) else { continue };
            let Some(text) = spec.write_in_units(p.value) else { continue };
            let (file, line) = source.locate(p.line);
            edits.entry(file.to_path_buf()).or_default().entry(line).or_default().push((p.name.clone(), text));
        }
    }
    if edits.is_empty() {
        eprintln!("nothing to rewrite: every parameter with a unit is written in it");
        return;
    }
    for (file, lines) in &edits {
        let text = std::fs::read_to_string(file).unwrap_or_else(|e| {
            eprintln!("error: cannot read {}: {e}", file.display());
            process::exit(1);
        });
        let mut out: Vec<String> = text.split('\n').map(String::from).collect();
        let mut count = 0;
        for (&line, params) in lines {
            let Some(l) = out.get_mut(line.wrapping_sub(1)) else { continue };
            for (name, new) in params {
                if let Some(rewritten) = replace_value(l, name, new) {
                    *l = rewritten;
                    count += 1;
                }
            }
        }
        std::fs::write(file, out.join("\n")).unwrap_or_else(|e| {
            eprintln!("error: cannot write {}: {e}", file.display());
            process::exit(1);
        });
        eprintln!("{}: {count} rewritten in units", file.display());
    }
}

/// `name <number>` on `line` with the number replaced by `new`, or `None`
/// when the line does not have it as a whole word followed by a number.
fn replace_value(line: &str, name: &str, new: &str) -> Option<String> {
    let bytes = line.as_bytes();
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut from = 0;
    while let Some(i) = line[from..].find(name).map(|i| i + from) {
        let end = i + name.len();
        let alone = (i == 0 || !word(bytes[i - 1])) && (end >= bytes.len() || !word(bytes[end]));
        if alone {
            let rest = &line[end..];
            let spaces = rest.len() - rest.trim_start().len();
            let num_start = end + spaces;
            let num_len = line[num_start..].bytes()
                .take_while(|c| c.is_ascii_digit() || *c == b'.' || *c == b'-')
                .count();
            let after = line.as_bytes().get(num_start + num_len).copied();
            if spaces > 0 && num_len > 0 && !after.is_some_and(|c| c.is_ascii_alphabetic() || c == b'%') {
                return Some(format!("{}{}{}", &line[..num_start], new, &line[num_start + num_len..]));
            }
        }
        from = end;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::replace_value;

    #[test]
    fn only_the_named_parameter_and_only_its_number() {
        assert_eq!(replace_value("    cutoff 0.10", "cutoff", "179hz").unwrap(), "    cutoff 179hz");
        assert_eq!(replace_value("module k { cutoff_env 0.2 cutoff 0.3 }", "cutoff", "2khz").unwrap(),
            "module k { cutoff_env 0.2 cutoff 2khz }");
        assert!(replace_value("    cutoff 800hz", "cutoff", "800hz").is_none());
        assert_eq!(replace_value("    osc2_pitch -0.25   # an octave down", "osc2_pitch", "-12st").unwrap(),
            "    osc2_pitch -12st   # an octave down");
    }
}
