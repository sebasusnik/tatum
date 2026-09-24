# tatum-mcp

A Model Context Protocol server that lets an AI write, validate and render `.synth`
songs with the compiler in the loop. Transport is stdio; nothing is sent anywhere.

## Tools

| tool | input | returns |
|------|-------|---------|
| `tatum_docs` | none | the DSL reference (`docs/DSL.md`) |
| `tatum_params` | `module?`, `format?` (markdown or json) | the parameter registry, the only source of valid names |
| `tatum_examples` | `name?` | the example list, or one example's source |
| `tatum_check` | `source` | `ok` plus a summary, or every error with line, column and suggestion |
| `tatum_render` | `source`, `output?`, `bars?` | the WAV path plus the mix report below |

### The mix report

`tatum_render` returns more than a loudness number, because balancing a track is
where the time actually goes:

- `sections`: per scene, RMS, peak, crest factor, the four-band split
  (low / mid / harsh 2-5 kHz / air) and whether it sits on the limiter ceiling.
- `tracks`: per track, peak **and** RMS with their own dB-below-loudest columns.
  They rank differently — a sparse bass reads far under a continuous pad on RMS
  while peaking above it — plus `crest` and the band the track mostly occupies.
- `buses`: the same after each bus chain, so a track that meters fine but
  arrives quiet at master is visible.
- `master`: crest factor entering and leaving the master chain. A large drop
  means the limiter is eating the transients rather than the mix getting louder.
- `hint`: plain-language warnings — tracks buried or silent, boxy midrange,
  fatiguing 2-5 kHz, thin low end, two tracks masking each other in the same
  band, and crest lost to limiting.

Resources mirror the same content: `tatum://docs/dsl`, `tatum://docs/params`,
`tatum://examples/<name>`.

The server's `instructions` tell the model the intended loop: read the docs once, pick an
example for the genre, write, `tatum_check` until clean, `tatum_render`, read the section
report and adjust the arrangement.

## Claude Code

The repo ships a `.mcp.json`, so inside this directory the server is available as `tatum`
after approving it. Elsewhere:

```
cargo build --release -p tatum-mcp
claude mcp add tatum -- /path/to/tatum/target/release/tatum-mcp
```

Environment variables:

- `SYNTH_EXAMPLES_DIR`: where `tatum_examples` looks (default: the repo's `examples/`).
- `SYNTH_RENDER_DIR`: where `tatum_render` writes when no `output` is given
  (default: `<tmp>/tatum-renders`).

## Claude Desktop

```json
{
  "mcpServers": {
    "tatum": { "command": "/path/to/tatum/target/release/tatum-mcp" }
  }
}
```

## Protocol notes

Newline-delimited JSON-RPC 2.0. Implements `initialize`, `ping`, `tools/list`,
`tools/call`, `resources/list`, `resources/read`. Protocol versions 2024-11-05,
2025-03-26 and 2025-06-18 are accepted and echoed back. Logs go to stderr only.
