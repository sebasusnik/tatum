#!/usr/bin/env python3
"""Call one synth-mcp tool over stdio and print its text result.

Usage:
  scripts/mcp-call.py synth_docs
  scripts/mcp-call.py synth_params '{"module":"fm"}'
  scripts/mcp-call.py synth_examples '{"name":"acid_arp"}'
  scripts/mcp-call.py synth_check --source-file song.synth
  scripts/mcp-call.py synth_render --source-file song.synth '{"output":"out.wav"}'
"""
import json, os, subprocess, sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "synth-mcp")

def main():
    args = sys.argv[1:]
    if not args:
        print(__doc__); sys.exit(2)
    tool = args.pop(0)
    arguments = {}
    if args and args[0] == "--source-file":
        args.pop(0)
        with open(args.pop(0)) as f:
            arguments["source"] = f.read()
    if args:
        arguments.update(json.loads(args.pop(0)))
    msgs = [
        {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "mcp-call", "version": "0"}}},
        {"jsonrpc": "2.0", "method": "notifications/initialized"},
        {"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": tool, "arguments": arguments}},
    ]
    proc = subprocess.run([BIN], input="\n".join(json.dumps(m) for m in msgs) + "\n", capture_output=True, text=True)
    for line in proc.stdout.splitlines():
        m = json.loads(line)
        if m.get("id") != 2:
            continue
        if "error" in m:
            print("RPC ERROR:", m["error"]); sys.exit(1)
        r = m["result"]
        print(r["content"][0]["text"])
        sys.exit(1 if r.get("isError") else 0)
    print("no response; stderr:", proc.stderr[-2000:]); sys.exit(1)

if __name__ == "__main__":
    main()
