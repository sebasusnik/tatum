//! `synth-mcp` — a Model Context Protocol server over stdio.
//!
//! Gives an AI author everything it needs to write valid `.synth` songs:
//! the DSL reference, the parameter registry, the example library, a strict
//! `check` with line-numbered errors, and `render` with a loudness report per
//! section so it can judge its own arrangement.
//!
//! Wire format: newline-delimited JSON-RPC 2.0 on stdin/stdout. Logs go to
//! stderr only. Register in Claude Code with the repo's `.mcp.json`, or:
//!
//! ```text
//! claude mcp add synth -- cargo run -q --release -p synth-mcp
//! ```

use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use synth_core::dsl::{self, compiler, lint};
use synth_core::params::{self, ModuleKind};
use synth_core::song_engine::{DslError, SongEngine};
use synth_core::SAMPLE_RATE;

const DSL_DOC: &str = include_str!("../../docs/DSL.md");
const PROTOCOL_VERSIONS: &[&str] = &["2024-11-05", "2025-03-26", "2025-06-18"];
const MAX_RENDER_BARS: u32 = 512;
/// Default threshold of the `limiter` node; a section peaking here is being limited.
const LIMITER_CEILING: f32 = 0.95;

const INSTRUCTIONS: &str = "\
synth-core writes music as `.synth` files: modules (instruments), patterns, tracks, \
scenes and an arrangement. Workflow: read `synth_docs` once (it ends with sound-design \
recipes), look at one example from `synth_examples` for the target genre, write the file, \
run `synth_check` and fix every error it reports (they carry line numbers and suggestions), \
act on its design warnings, then `synth_render` and read the per-section loudness report to \
judge the arrangement. Parameter names and ranges come from `synth_params`; never invent a \
parameter. Nothing that sustains should stay static: pads and leads get an LFO on the \
filter, vibrato, an `auto` sweep or an arp, plus sends and sidechain against the kick.";

fn main() {
    let ctx = Ctx::from_env();
    let stdin = io::stdin();
    let stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => handle_message(&ctx, &msg),
            Err(e) => Some(error_response(Value::Null, -32700, &format!("parse error: {}", e))),
        };
        if let Some(resp) = response {
            let mut out = stdout.lock();
            let _ = writeln!(out, "{}", resp);
            let _ = out.flush();
        }
    }
}

/// Where to find examples and where to put renders.
struct Ctx {
    examples_dir: PathBuf,
    render_dir: PathBuf,
}

impl Ctx {
    fn from_env() -> Self {
        let examples_dir = std::env::var("SYNTH_EXAMPLES_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples"));
        let render_dir = std::env::var("SYNTH_RENDER_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("synth-renders"));
        Self { examples_dir, render_dir }
    }
}

// ── JSON-RPC plumbing ──

fn handle_message(ctx: &Ctx, msg: &Value) -> Option<Value> {
    let id = msg.get("id").cloned();
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let params = msg.get("params").cloned().unwrap_or(Value::Null);

    // Notifications carry no id and get no response.
    let id = match id {
        Some(id) if !id.is_null() => id,
        _ => {
            if method == "notifications/initialized" || method.starts_with("notifications/") {
                return None;
            }
            return Some(error_response(Value::Null, -32600, "requests need an id"));
        }
    };

    let result = match method {
        "initialize" => Ok(initialize(&params)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tool_definitions() })),
        "tools/call" => call_tool(ctx, &params),
        "resources/list" => Ok(json!({ "resources": resource_list(ctx) })),
        "resources/read" => read_resource(ctx, &params),
        "resources/templates/list" => Ok(json!({ "resourceTemplates": [] })),
        "prompts/list" => Ok(json!({ "prompts": [] })),
        other => Err((-32601, format!("method not found: {}", other))),
    };

    Some(match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => error_response(id, code, &message),
    })
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn initialize(params: &Value) -> Value {
    let requested = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("");
    let version = if PROTOCOL_VERSIONS.contains(&requested) { requested } else { PROTOCOL_VERSIONS[PROTOCOL_VERSIONS.len() - 1] };
    json!({
        "protocolVersion": version,
        "capabilities": {
            "tools": { "listChanged": false },
            "resources": { "subscribe": false, "listChanged": false }
        },
        "serverInfo": { "name": "synth-mcp", "version": env!("CARGO_PKG_VERSION") },
        "instructions": INSTRUCTIONS
    })
}

// ── Tools ──

fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "synth_docs",
            "description": "The .synth language reference: globals, modules, patterns (ties, slides, chords, drum lanes), tracks, arpeggiator, scenes, automation, arrangement and livecoding semantics. Read this once before writing a song.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        }),
        json!({
            "name": "synth_params",
            "description": "Parameter registry. module = bass|fm|keys|beats gives every valid module parameter with range, default, option names and meaning; module = track gives the track options (level, pan, gate, sends, sidechain, arp, out); module = fx gives every effect and graph node with its arguments and options. This is the only source of valid names.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "module": { "type": "string", "enum": ["bass", "fm", "keys", "beats", "track", "fx"], "description": "One module type, track for track options, or fx for effects and nodes. Omit for all modules." },
                    "format": { "type": "string", "enum": ["markdown", "json"], "description": "Default markdown." }
                },
                "additionalProperties": false
            }
        }),
        json!({
            "name": "synth_examples",
            "description": "Example songs. Without a name: the list of examples with their one-line description. With a name: the full .synth source, useful as a template for a genre.",
            "inputSchema": {
                "type": "object",
                "properties": { "name": { "type": "string", "description": "Example name without extension, e.g. acid_arp" } },
                "additionalProperties": false
            }
        }),
        json!({
            "name": "synth_check",
            "description": "Parse and compile .synth source without rendering. Returns ok=true with a summary (tempo, bars, duration, counts) plus design warnings (static_pad, dry_mix, no_sidechain, single_scene, no_limiter, unused_*) with a hint each, or ok=false with every error, each carrying a line number and often a suggestion. Fix all errors before rendering; treat warnings as things a producer would fix.",
            "inputSchema": {
                "type": "object",
                "properties": { "source": { "type": "string", "description": "Full .synth source text" } },
                "required": ["source"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "synth_render",
            "description": "Compile and render .synth source to a 16-bit stereo WAV file. Returns the file path plus a mix report: overall peak, RMS and clipped samples; per arrangement section the RMS, peak, dB, crest factor and low/mid/high energy balance; per track its peak, RMS and dB below the loudest track; and hints about limiting, buried or silent tracks and boxy midrange. Use it to check that builds rise, drops hit hardest, nothing clips, and every track is audible. For quick level checks pass bars (e.g. 8) to render only the start.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source": { "type": "string", "description": "Full .synth source text" },
                    "output": { "type": "string", "description": "Path for the WAV file. Default: a file in the render directory, name derived from the first comment line or 'song'." },
                    "bars": { "type": "integer", "minimum": 1, "maximum": MAX_RENDER_BARS, "description": "Render only the first N bars (default: the whole arrangement)." }
                },
                "required": ["source"],
                "additionalProperties": false
            }
        }),
    ]
}

fn call_tool(ctx: &Ctx, params: &Value) -> Result<Value, (i64, String)> {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
    let outcome = match name {
        "synth_docs" => Ok(DSL_DOC.to_string()),
        "synth_params" => tool_params(&args),
        "synth_examples" => tool_examples(ctx, &args),
        "synth_check" => tool_check(&args),
        "synth_render" => tool_render(ctx, &args),
        other => return Err((-32602, format!("unknown tool: {}", other))),
    };
    Ok(match outcome {
        Ok(text) => json!({ "content": [{ "type": "text", "text": text }], "isError": false }),
        Err(text) => json!({ "content": [{ "type": "text", "text": text }], "isError": true }),
    })
}

fn tool_params(args: &Value) -> Result<String, String> {
    if matches!(args.get("module").and_then(Value::as_str), Some("track")) {
        return Ok(match args.get("format").and_then(Value::as_str) {
            Some("json") => params::track_json(),
            _ => params::track_markdown(),
        });
    }
    if matches!(args.get("module").and_then(Value::as_str), Some("fx") | Some("nodes")) {
        return Ok(match args.get("format").and_then(Value::as_str) {
            Some("json") => synth_core::nodes::json(),
            _ => synth_core::nodes::markdown(),
        });
    }
    let kinds: Vec<ModuleKind> = match args.get("module").and_then(Value::as_str) {
        Some(m) => vec![ModuleKind::from_str(m).ok_or_else(|| format!("unknown module '{}' (bass, fm, keys, beats)", m))?],
        None => ModuleKind::ALL.to_vec(),
    };
    Ok(match args.get("format").and_then(Value::as_str) {
        Some("json") => params::json(&kinds),
        _ => params::markdown(&kinds),
    })
}

fn list_examples(ctx: &Ctx) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&ctx.examples_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("synth") {
                continue;
            }
            let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
            // First comment line that has words in it (skips box-drawing banners).
            let first_line = std::fs::read_to_string(&path)
                .ok()
                .and_then(|s| {
                    s.lines()
                        .take(8)
                        .map(|l| l.trim_start_matches('#').trim().trim_matches(|c: char| !c.is_alphanumeric()).trim().to_string())
                        .find(|l| l.chars().filter(|c| c.is_alphabetic()).count() >= 4)
                })
                .unwrap_or_default();
            out.push((name, first_line));
        }
    }
    out.sort();
    out
}

fn tool_examples(ctx: &Ctx, args: &Value) -> Result<String, String> {
    match args.get("name").and_then(Value::as_str) {
        Some(name) => {
            if name.contains('/') || name.contains("..") {
                return Err("invalid example name".into());
            }
            let path = ctx.examples_dir.join(format!("{}.synth", name));
            std::fs::read_to_string(&path).map_err(|_| {
                let names: Vec<String> = list_examples(ctx).into_iter().map(|(n, _)| n).collect();
                format!("no example '{}'. Available: {}", name, names.join(", "))
            })
        }
        None => {
            let list = list_examples(ctx);
            if list.is_empty() {
                return Err(format!("no examples found in {} (set SYNTH_EXAMPLES_DIR)", ctx.examples_dir.display()));
            }
            Ok(list.iter().map(|(n, d)| format!("- {}: {}", n, d)).collect::<Vec<_>>().join("\n"))
        }
    }
}

/// Parse + compile, returning the compiled song or a JSON error report.
fn compile_source(source: &str) -> Result<compiler::CompiledSong, String> {
    let ast = dsl::parse(source).map_err(|errs| DslError::Parse(errs).to_json())?;
    compiler::compile(&ast).map_err(|errs| DslError::Compile(errs).to_json())
}

fn summary(song: &compiler::CompiledSong) -> Value {
    let bars: u32 = song.arrangement.iter().map(|(_, r)| *r).sum();
    let meter_beats = song.globals.meter.0 as f32;
    let duration = bars as f32 * meter_beats * 60.0 / song.globals.tempo;
    json!({
        "tempo": song.globals.tempo,
        "bars": bars,
        "duration_s": (duration * 10.0).round() / 10.0,
        "instruments": song.instruments.len(),
        "patterns": song.patterns.len(),
        "tracks": song.tracks.len(),
        "scenes": song.scenes.len(),
        "arrangement": song.arrangement.iter().map(|(si, r)| {
            json!({ "scene": song.scenes.get(*si).map(|s| s.name.as_str()).unwrap_or("?"), "bars": r })
        }).collect::<Vec<_>>()
    })
}

fn tool_check(args: &Value) -> Result<String, String> {
    let source = args.get("source").and_then(Value::as_str).ok_or("missing 'source'")?;
    match compile_source(source) {
        Ok(song) => {
            let warnings: Vec<Value> = dsl::parse(source)
                .map(|ast| lint::lint_song(&ast))
                .unwrap_or_default()
                .into_iter()
                .map(|l| json!({ "code": l.code, "message": l.message, "hint": l.hint }))
                .collect();
            Ok(serde_json::to_string_pretty(&json!({ "ok": true, "summary": summary(&song), "warnings": warnings })).unwrap())
        }
        Err(err_json) => Err(pretty(&err_json)),
    }
}

fn pretty(json_text: &str) -> String {
    serde_json::from_str::<Value>(json_text)
        .map(|v| serde_json::to_string_pretty(&v).unwrap())
        .unwrap_or_else(|_| json_text.to_string())
}

fn tool_render(ctx: &Ctx, args: &Value) -> Result<String, String> {
    let source = args.get("source").and_then(Value::as_str).ok_or("missing 'source'")?;
    let song = compile_source(source).map_err(|e| pretty(&e))?;

    let sections: Vec<(String, u32)> = song.arrangement.iter()
        .map(|(si, r)| (song.scenes.get(*si).map(|s| s.name.clone()).unwrap_or_default(), *r))
        .collect();
    let total_bars: u32 = sections.iter().map(|(_, r)| *r).sum();
    if total_bars == 0 {
        return Err("nothing to render: the arrangement is empty (add `arrange { scene xN }`)".into());
    }
    let bars = args.get("bars").and_then(Value::as_u64).map(|b| b as u32).unwrap_or(total_bars).min(total_bars).min(MAX_RENDER_BARS);
    let tempo = song.globals.tempo;
    let steps_per_bar = song.globals.meter.0 as usize * 4;

    let mut engine = SongEngine::from_compiled(song);
    engine.reset_meters();
    let (l, r) = engine.render(bars);
    // Per-track levels: post level and pan, before the master chain.
    let mut track_report = Vec::new();
    let mut loudest = 0.0f32;
    for i in 0..engine.track_count() {
        loudest = loudest.max(engine.track_rms(i));
    }
    for i in 0..engine.track_count() {
        let (p, rr) = (engine.track_peak(i), engine.track_rms(i));
        track_report.push(json!({
            "track": engine.track_name(i),
            "peak": round3(p),
            "rms": round3(rr),
            "db": round1(20.0 * rr.max(1e-6).log10()),
            "vs_loudest_db": round1(20.0 * (rr.max(1e-6) / loudest.max(1e-6)).log10()),
        }));
    }
    let bad = l.iter().chain(r.iter()).filter(|v| !v.is_finite()).count();
    if bad > 0 {
        return Err(format!(
            "render produced {} non-finite samples: an effect is unstable. Check reverb sizes, delay/phaser feedback and compressor makeup, then re-render.",
            bad
        ));
    }

    // Output path
    let output = match args.get("output").and_then(Value::as_str) {
        Some(p) => PathBuf::from(p),
        None => {
            std::fs::create_dir_all(&ctx.render_dir).map_err(|e| format!("cannot create {}: {}", ctx.render_dir.display(), e))?;
            ctx.render_dir.join(format!("{}.wav", slug_from_source(source)))
        }
    };
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {}", parent.display(), e))?;
        }
    }
    std::fs::write(&output, synth_core::wav::encode_stereo_16(&l, &r, SAMPLE_RATE as u32))
        .map_err(|e| format!("cannot write {}: {}", output.display(), e))?;

    // Loudness report per section
    let samples_per_bar = (SAMPLE_RATE * 60.0 / tempo / 4.0 * steps_per_bar as f32) as usize;
    let mut report = Vec::new();
    let mut at_ceiling_sections: Vec<String> = Vec::new();
    let mut bar_cursor = 0u32;
    for (name, count) in &sections {
        if bar_cursor >= bars { break; }
        let count = (*count).min(bars - bar_cursor);
        let a = (bar_cursor as usize * samples_per_bar).min(l.len());
        let b = ((bar_cursor + count) as usize * samples_per_bar).min(l.len());
        let (rms, peak) = stats(&l[a..b], &r[a..b]);
        let at_ceiling = peak >= LIMITER_CEILING - 0.005;
        if at_ceiling { at_ceiling_sections.push(name.clone()); }
        let (lo, mid, harsh, air) = balance(&l[a..b], &r[a..b]);
        report.push(json!({
            "scene": name, "bars": count,
            "rms": round3(rms), "peak": round3(peak),
            "db": round1(20.0 * rms.max(1e-6).log10()),
            "crest": round1(peak / rms.max(1e-6)),
            "balance_pct": { "low": lo, "mid": mid, "harsh": harsh, "air": air },
            "at_limiter_ceiling": at_ceiling
        }));
        bar_cursor += count;
    }
    let (rms, peak) = stats(&l, &r);
    let clipped = l.iter().chain(r.iter()).filter(|s| s.abs() >= 0.999).count();
    let mut hints: Vec<String> = Vec::new();
    if !at_ceiling_sections.is_empty() {
        hints.push(format!(
            "peak sits at the master limiter ceiling ({}) in: {}. The limiter is flattening dynamics there; lower track levels or the compressor makeup, or raise the sections that should be quieter instead.",
            LIMITER_CEILING, at_ceiling_sections.join(", ")
        ));
    }
    // Tracks buried more than 30 dB under the loudest are effectively inaudible;
    // a chain that silently reroutes or a missing make-up gain looks like this.
    let buried: Vec<String> = track_report.iter()
        .filter(|t| t["vs_loudest_db"].as_f64().unwrap_or(0.0) < -30.0 && t["rms"].as_f64().unwrap_or(0.0) > 0.0)
        .map(|t| t["track"].as_str().unwrap_or("").to_string())
        .collect();
    if !buried.is_empty() {
        hints.push(format!(
            "more than 30 dB below the loudest track, so effectively inaudible: {}. Raise their level, add gain() in their chain, or check that the chain reaches master.",
            buried.join(", ")
        ));
    }
    let silent: Vec<String> = track_report.iter()
        .filter(|t| t["rms"].as_f64().unwrap_or(1.0) <= 0.0)
        .map(|t| t["track"].as_str().unwrap_or("").to_string())
        .collect();
    if !silent.is_empty() {
        hints.push(format!("silent for the whole render: {}. They play in no scene, or their pattern is all rests.", silent.join(", ")));
    }
    if let Some(w) = report.iter().find(|s| s["balance_pct"]["mid"].as_f64().unwrap_or(0.0) >= 70.0) {
        hints.push(format!(
            "scene '{}' is {}% midrange (250 Hz-2 kHz): it will sound boxy and small. Give the bass room below 250 Hz and let something live above 5 kHz.",
            w["scene"].as_str().unwrap_or(""), w["balance_pct"]["mid"]
        ));
    }
    if let Some(w) = report.iter().find(|s| s["balance_pct"]["harsh"].as_f64().unwrap_or(0.0) >= 20.0) {
        hints.push(format!(
            "scene '{}' puts {}% of its energy in 2-5 kHz, where the ear is most sensitive: it will sound fatiguing. Usual causes are distortion with no filter after it, bright hats, or a positive master tilt.",
            w["scene"].as_str().unwrap_or(""), w["balance_pct"]["harsh"]
        ));
    }
    if let Some(w) = report.iter().find(|s| s["balance_pct"]["low"].as_f64().unwrap_or(100.0) <= 15.0) {
        hints.push(format!(
            "scene '{}' has only {}% of its energy below 250 Hz: it will sound thin. Heavy distortion trades a fundamental for harmonics, so a distorted bass usually needs a clean sub layer under it.",
            w["scene"].as_str().unwrap_or(""), w["balance_pct"]["low"]
        ));
    }
    let hint = if hints.is_empty() { Value::Null } else { json!(hints.join(" | ")) };

    Ok(serde_json::to_string_pretty(&json!({
        "ok": true,
        "path": output.display().to_string(),
        "bars": bars,
        "duration_s": round1(l.len() as f32 / SAMPLE_RATE),
        "peak": round3(peak),
        "rms": round3(rms),
        "clipped_samples": clipped,
        "gain_compensation": round3(engine.gain_compensation()),
        "sections": report,
        "tracks": track_report,
        "hint": hint
    })).unwrap())
}

/// Energy split into low (<250 Hz), mid (250 Hz..2 kHz), harsh (2..5 kHz,
/// where the ear is most sensitive) and air (>5 kHz), as percentages.
///
/// Uses three cascaded one-pole sections per edge. A single pole rolls off so
/// gently that midrange leaks into the "high" bucket and a dark mix reads as
/// bright, which is worse than no number at all.
fn balance(l: &[f32], r: &[f32]) -> (f64, f64, f64, f64) {
    const POLES: usize = 3;
    // Cascading N one-poles drags the overall -3 dB point down; scale each
    // section up by 1/sqrt(2^(1/N) - 1) so the stated edge is the real one.
    const CASCADE_FIX: f32 = 1.9615;
    let coeff = |hz: f32| (-2.0 * core::f32::consts::PI * hz * CASCADE_FIX / SAMPLE_RATE).exp();
    let (c_lo, c_mid, c_hi) = (coeff(250.0), coeff(2000.0), coeff(5000.0));
    let (mut s_lo, mut s_mid, mut s_hi) = ([0.0f32; POLES], [0.0f32; POLES], [0.0f32; POLES]);
    let cascade = |x: f32, state: &mut [f32; POLES], c: f32| {
        let mut v = x;
        for s in state.iter_mut() {
            *s = v * (1.0 - c) + *s * c;
            v = *s;
        }
        v
    };
    let (mut lo, mut mid, mut harsh, mut air) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for (a, b) in l.iter().zip(r) {
        let x = (a + b) * 0.5;
        let v_lo = cascade(x, &mut s_lo, c_lo);
        let v_mid = cascade(x, &mut s_mid, c_mid);
        let v_hi = cascade(x, &mut s_hi, c_hi);
        lo += (v_lo * v_lo) as f64;
        mid += ((v_mid - v_lo) * (v_mid - v_lo)) as f64;
        harsh += ((v_hi - v_mid) * (v_hi - v_mid)) as f64;
        air += ((x - v_hi) * (x - v_hi)) as f64;
    }
    let total = lo + mid + harsh + air;
    if total <= 0.0 { return (0.0, 0.0, 0.0, 0.0); }
    let pct = |v: f64| (v / total * 1000.0).round() / 10.0;
    (pct(lo), pct(mid), pct(harsh), pct(air))
}

fn stats(l: &[f32], r: &[f32]) -> (f32, f32) {
    let n = l.len().max(1);
    let mut sum = 0.0f64;
    let mut peak = 0.0f32;
    for (a, b) in l.iter().zip(r) {
        sum += (a * a + b * b) as f64;
        peak = peak.max(a.abs()).max(b.abs());
    }
    ((sum / (2 * n) as f64).sqrt() as f32, peak)
}

fn round3(v: f32) -> f64 { ((v as f64) * 1000.0).round() / 1000.0 }
fn round1(v: f32) -> f64 { ((v as f64) * 10.0).round() / 10.0 }

/// File name from the first `# Title` comment, else "song".
fn slug_from_source(source: &str) -> String {
    let title = source.lines().next().unwrap_or("").trim_start_matches('#').trim();
    let title = title.split(|c| c == '—' || c == '-' || c == ':').next().unwrap_or("").trim();
    let slug: String = title.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
        .collect::<String>()
        .trim_matches('_')
        .to_string();
    if slug.is_empty() { "song".into() } else { slug }
}

// ── Resources ──

fn resource_list(ctx: &Ctx) -> Vec<Value> {
    let mut out = vec![
        json!({ "uri": "synth://docs/dsl", "name": "DSL reference", "mimeType": "text/markdown", "description": "The .synth language reference" }),
        json!({ "uri": "synth://docs/params", "name": "Parameter registry", "mimeType": "text/markdown", "description": "Every module parameter with range, default and meaning" }),
    ];
    for (name, desc) in list_examples(ctx) {
        out.push(json!({ "uri": format!("synth://examples/{}", name), "name": name, "mimeType": "text/plain", "description": desc }));
    }
    out
}

fn read_resource(ctx: &Ctx, params: &Value) -> Result<Value, (i64, String)> {
    let uri = params.get("uri").and_then(Value::as_str).unwrap_or("");
    let (mime, text) = match uri {
        "synth://docs/dsl" => ("text/markdown", DSL_DOC.to_string()),
        "synth://docs/params" => ("text/markdown", params::markdown(&ModuleKind::ALL) + &params::track_markdown() + &synth_core::nodes::markdown()),
        _ => match uri.strip_prefix("synth://examples/") {
            Some(name) => ("text/plain", tool_examples(ctx, &json!({ "name": name })).map_err(|e| (-32002, e))?),
            None => return Err((-32002, format!("unknown resource: {}", uri))),
        },
    };
    Ok(json!({ "contents": [{ "uri": uri, "mimeType": mime, "text": text }] }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Ctx {
        Ctx {
            examples_dir: Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples"),
            render_dir: std::env::temp_dir().join("synth-mcp-tests"),
        }
    }

    fn call(ctx: &Ctx, msg: Value) -> Value {
        handle_message(ctx, &msg).expect("response")
    }

    #[test]
    fn initialize_and_list_tools() {
        let c = ctx();
        let r = call(&c, json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-03-26" } }));
        assert_eq!(r["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(r["result"]["serverInfo"]["name"], "synth-mcp");
        let r = call(&c, json!({ "jsonrpc": "2.0", "id": "x", "method": "tools/list" }));
        let names: Vec<&str> = r["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, vec!["synth_docs", "synth_params", "synth_examples", "synth_check", "synth_render"]);
        assert_eq!(r["id"], "x");
    }

    #[test]
    fn notifications_get_no_response() {
        let c = ctx();
        assert!(handle_message(&c, &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).is_none());
    }

    #[test]
    fn unknown_method_is_an_error() {
        let c = ctx();
        let r = call(&c, json!({ "jsonrpc": "2.0", "id": 2, "method": "nope" }));
        assert_eq!(r["error"]["code"], -32601);
    }

    #[test]
    fn check_reports_errors_with_lines() {
        let c = ctx();
        let src = "tempo 120\nmodule bass b { cutof 0.3 }\n";
        let r = call(&c, json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "synth_check", "arguments": { "source": src } } }));
        assert_eq!(r["result"]["isError"], true);
        let text = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("Did you mean 'cutoff'"), "{}", text);
        assert!(text.contains("\"line\": 2"), "{}", text);
    }

    #[test]
    fn check_and_render_an_example() {
        let c = ctx();
        let src = std::fs::read_to_string(c.examples_dir.join("acid_arp.synth")).unwrap();
        let r = call(&c, json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": { "name": "synth_check", "arguments": { "source": src } } }));
        assert_eq!(r["result"]["isError"], false, "{}", r);
        let text = r["result"]["content"][0]["text"].as_str().unwrap();
        let v: Value = serde_json::from_str(text).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["summary"]["bars"], 12);

        let out = c.render_dir.join("acid_arp_test.wav");
        let r = call(&c, json!({ "jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": { "name": "synth_render", "arguments": { "source": src, "output": out.to_str().unwrap(), "bars": 4 } } }));
        assert_eq!(r["result"]["isError"], false, "{}", r);
        let v: Value = serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(v["bars"], 4);
        assert!(v["peak"].as_f64().unwrap() > 0.01);
        let sections = v["sections"].as_array().unwrap();
        assert_eq!(sections[0]["scene"], "intro");
        assert_eq!(sections[0]["bars"], 2);
        assert_eq!(sections[1]["scene"], "groove");
        assert!(out.exists());
    }

    #[test]
    fn examples_and_resources() {
        let c = ctx();
        let r = call(&c, json!({ "jsonrpc": "2.0", "id": 6, "method": "tools/call", "params": { "name": "synth_examples", "arguments": {} } }));
        let text = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("- acid_arp:"), "{}", text);
        let r = call(&c, json!({ "jsonrpc": "2.0", "id": 7, "method": "resources/read", "params": { "uri": "synth://examples/acid_arp" } }));
        assert!(r["result"]["contents"][0]["text"].as_str().unwrap().contains("tempo 126"));
        let r = call(&c, json!({ "jsonrpc": "2.0", "id": 8, "method": "resources/read", "params": { "uri": "synth://docs/params" } }));
        assert!(r["result"]["contents"][0]["text"].as_str().unwrap().contains("| `cutoff` |"));
        let r = call(&c, json!({ "jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": { "name": "synth_examples", "arguments": { "name": "../secret" } } }));
        assert_eq!(r["result"]["isError"], true);
    }

    #[test]
    fn slug() {
        assert_eq!(slug_from_source("# Acid Arp — a showcase\ntempo 1"), "acid_arp");
        assert_eq!(slug_from_source("tempo 120"), "tempo_120");
        assert_eq!(slug_from_source(""), "song");
    }
}

#[cfg(test)]
mod band_tests {
    use super::*;

    fn tone(hz: f32, secs: f32) -> Vec<f32> {
        (0..(SAMPLE_RATE * secs) as usize)
            .map(|i| (i as f32 / SAMPLE_RATE * hz * core::f32::consts::TAU).sin() * 0.5)
            .collect()
    }

    /// A pure tone must land in the band its frequency belongs to. Without the
    /// cascade correction a 1 kHz tone leaked most of its energy into "air".
    #[test]
    fn each_band_catches_its_own_tone() {
        let cases = [
            (80.0, 0usize, "low"),
            (1000.0, 1, "mid"),
            (3000.0, 2, "harsh"),
            (9000.0, 3, "air"),
        ];
        for (hz, idx, name) in cases {
            let x = tone(hz, 1.0);
            let (lo, mid, harsh, air) = balance(&x, &x);
            let pct = [lo, mid, harsh, air];
            // The bands overlap (one-pole cascades are gentle and 2-5 kHz is
            // barely more than an octave wide), so the bar is that the right
            // band dominates, not that it takes everything.
            let winner = pct.iter().cloned().fold(f64::MIN, f64::max);
            assert!(
                (pct[idx] - winner).abs() < 1e-9 && pct[idx] > 50.0,
                "a {} Hz tone should read mostly '{}', got low {} mid {} harsh {} air {}",
                hz, name, lo, mid, harsh, air
            );
        }
    }
}
