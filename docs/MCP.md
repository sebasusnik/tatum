# synth-mcp

A Model Context Protocol server that lets an AI write, validate and render `.synth`
songs with the compiler in the loop. Transport is stdio; nothing is sent anywhere.

## Tools

| tool | input | returns |
|------|-------|---------|
| `synth_docs` | none | the DSL reference (`docs/DSL.md`) |
| `synth_params` | `module?`, `format?` (markdown or json) | the parameter registry, the only source of valid names |
| `synth_examples` | `name?` | the example list, or one example's source |
| `synth_check` | `source` | `ok` plus a summary, or every error with line, column and suggestion |
| `synth_render` | `source`, `output?`, `bars?` | the WAV path plus peak, RMS, clipped samples and a per-section loudness report |

Resources mirror the same content: `synth://docs/dsl`, `synth://docs/params`,
`synth://examples/<name>`.

The server's `instructions` tell the model the intended loop: read the docs once, pick an
example for the genre, write, `synth_check` until clean, `synth_render`, read the section
report and adjust the arrangement.

## Claude Code

The repo ships a `.mcp.json`, so inside this directory the server is available as `synth`
after approving it. Elsewhere:

```
cargo build --release -p synth-mcp
claude mcp add synth -- /path/to/synth-core/target/release/synth-mcp
```

Environment variables:

- `SYNTH_EXAMPLES_DIR`: where `synth_examples` looks (default: the repo's `examples/`).
- `SYNTH_RENDER_DIR`: where `synth_render` writes when no `output` is given
  (default: `<tmp>/synth-renders`).

## Claude Desktop

```json
{
  "mcpServers": {
    "synth": { "command": "/path/to/synth-core/target/release/synth-mcp" }
  }
}
```

## Protocol notes

Newline-delimited JSON-RPC 2.0. Implements `initialize`, `ping`, `tools/list`,
`tools/call`, `resources/list`, `resources/read`. Protocol versions 2024-11-05,
2025-03-26 and 2025-06-18 are accepted and echoed back. Logs go to stderr only.
