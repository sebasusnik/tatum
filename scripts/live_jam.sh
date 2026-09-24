#!/bin/zsh
# Plays a copy of examples/live_set.synth with `tatum watch` and edits it
# every two seconds, the way the five-minute soak in docs/LIVE.md did:
# kick level, bass cutoff, pad reverb send, and the bass line with its first
# note doubled every other round. Ctrl-C stops it.
#
#   ./scripts/live_jam.sh            # 150 edits, five minutes
#   ./scripts/live_jam.sh 30         # 30 edits, one minute

set -e
cd "$(dirname "$0")/.."
cargo build -q --release -p tatum-cli
f=/tmp/live_jam.synth
cp examples/live_set.synth $f
edits=${1:-150}

(
  n=0
  while (( n < edits )); do
    sleep 2
    n=$((n+1))
    case $((n % 4)) in
      1) lvl=$(( (n % 5) + 3 )); sed -i '' "s/track kick { play beat using kit level 0\.[0-9]/track kick { play beat using kit level 0.$lvl/" $f ;;
      2) if (( n % 8 == 2 )); then sed -i '' 's/pattern line { 1.1 - 1.3 -/pattern line { 1.1 1.1 1.3 -/' $f; else sed -i '' 's/pattern line { 1.1 1.1 1.3 -/pattern line { 1.1 - 1.3 -/' $f; fi ;;
      3) c=$(( (n % 6) + 2 )); sed -i '' "s/module bass low { cutoff 0\.[0-9]/module bass low { cutoff 0.$c/" $f ;;
      0) sed -i '' 's/reverb_send 0\.[0-9]/reverb_send 0.'$(( (n % 4) + 4 ))'/' $f ;;
    esac
  done
  sleep 3
  echo q
) | ./target/release/tatum watch $f
