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
| `tatum_debug` | `source`, `solo?`, `mute?`, `bars?` (`"17-24"`), `dry?`, `output?` | the `tatum debug` report and the path of `sheet.png`, every part's spectrogram stacked over the mix, to open and look at; the report names a `zoom.*.png` close-up for the worst moment of each part |

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
- `master`: crest factor entering the master chain and leaving the engine. A
  large drop means the master chain or the output limiter is eating the
  transients.
- `heard`: what `tatum_debug` would find, listened for while rendering:
  clicks, noise between notes, energy under 25 Hz or over 16 kHz, a wave off
  centre. Each line names the parts and the bar; run `tatum_debug` for the
  pictures.
- `output_gain_db`: what the engine added or took off to bring the song to
  -18 LUFS. Every song plays at that loudness whatever its mix, so this is not
  a number to chase.
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

- `SYNTH_EXAMPLES_DIR`: a directory of `.synth` files for `tatum_examples` to serve
  instead of the ones compiled into the server (the repo's `examples/` at build
  time, so an installed server has them too).
- `SYNTH_RENDER_DIR`: where `tatum_render` and `tatum_debug` write (default:
  `<tmp>/tatum-renders`). Their `output` is a name inside it (`song.wav`,
  `drafts/v2.wav`, `debug/take2`); absolute paths and `..` are refused, so a
  model calling the server cannot write anywhere else on the machine.

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
