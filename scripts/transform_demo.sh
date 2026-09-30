#!/usr/bin/env bash
# Live coding by transforming patterns, written as a script.
#
# In one half of the terminal:
#     tatum watch examples/transform_demo.synth --tui
# in the other:
#     scripts/transform_demo.sh
#
# The script edits only the lines at the bottom of the file, one word at a
# time, the way a person would, and says what each change does. The file is
# put back the way it was when the script ends or is interrupted.
#
#     scripts/transform_demo.sh --render out.wav
#
# plays the same changes offline instead: each stage becomes a step of a set
# in a temporary directory, and `tatum set render` walks them with the same
# hot swaps a live session makes.

set -euo pipefail

FILE="${FILE:-examples/transform_demo.synth}"
TATUM="${TATUM:-./target/release/tatum}"
BPM=128
MODE=live
OUT=""
if [[ "${1:-}" == "--render" ]]; then
    MODE=render
    OUT="${2:-transform_demo.wav}"
fi

[[ -f "$FILE" ]] || { echo "no $FILE here: run this from the repository root" >&2; exit 1; }

BASE="$(mktemp)"
cp "$FILE" "$BASE"
WORK="$(mktemp)"
cp "$FILE" "$WORK"
SET_DIR=""
STAGE=0
restore() {
    [[ "$MODE" == live ]] && cp "$BASE" "$FILE"
    rm -f "$BASE" "$WORK"
    [[ -n "$SET_DIR" ]] && rm -rf "$SET_DIR"
    true
}
trap restore EXIT
trap 'echo; echo "stopped: $FILE is back as it was"; exit 0' INT

bar_seconds() { echo "scale=3; $1 * 4 * 60 / $BPM" | bc; }

# `set_play TRACK "clause"`: rewrite that track's line below the marker.
set_play() {
    local track="$1" clause="$2"
    TRACK="$track" CLAUSE="$clause" perl -pi -e '
        $live = 1 if /Below this line/;
        if ($live && /^track\s+\Q$ENV{TRACK}\E\s*\{/) {
            $_ = sprintf("track %-5s { play %s }\n", $ENV{TRACK}, $ENV{CLAUSE});
        }' "$WORK"
}

# `stage BARS "what it does" TRACK "clause" [TRACK "clause" ...]`
stage() {
    local bars="$1" say="$2"
    shift 2
    while (($#)); do
        set_play "$1" "$2"
        shift 2
    done
    STAGE=$((STAGE + 1))
    if [[ "$MODE" == live ]]; then
        cp "$WORK" "$FILE"
        printf '\n\033[1;33m%2d\033[0m  %s\n' "$STAGE" "$say"
        grep -A20 'Below this line' "$FILE" | tail -n +2 | sed 's/^/      /'
        sleep "$(bar_seconds "$bars")"
    else
        local name
        name="$(printf '%02d-etapa.synth' "$STAGE")"
        { echo "# set: bars=$bars phase=etapa$STAGE"; echo "# set-note: $say"; cat "$WORK"; } > "$SET_DIR/$name"
        printf '%2d  %3d compases  %s\n' "$STAGE" "$bars" "$say"
    fi
}

if [[ "$MODE" == render ]]; then
    SET_DIR="$(mktemp -d)"
else
    echo "Editing $FILE. Run 'tatum watch $FILE --tui' in the other half, then press Enter."
    read -r
fi

stage 4 "Arranca: bombo y hats, nada más." \
    kick "kick4" hats "hats"
stage 4 "Entra el bajo rodando." \
    bass "roll"
stage 4 "Entra la línea ácida, tal cual está escrita." \
    acid "acid_line"
stage 8 "every 4 rev: la cuarta vuelta de cada cuatro, al revés. Es el remate de la frase." \
    acid "acid_line every 4 rev"
stage 4 "Hats a semicorcheas, y degrade 30%: se comen notas al azar, siempre las mismas en cada render." \
    hats "hats16 degrade 30%"
stage 4 "Entra el clap." \
    clap "clap"
stage 4 "fast 2: la línea ácida dos veces en el mismo compás." \
    acid "acid_line fast 2"
stage 4 "shift 3: la misma, corrida tres semicorcheas. Cambia dónde acentúa." \
    acid "acid_line fast 2 shift 3"
stage 8 "Los stabs alternan dos patrones, una vuelta cada uno: play stab_a, stab_b." \
    stab "stab_a, stab_b" acid "acid_line every 4 rev"
stage 8 "La ácida alterna con su respuesta, dos grados más arriba; el bajo sube una octava cada dos vueltas." \
    acid "acid_line, acid_answer up 2" bass "roll every 2 octave 1"
stage 8 "iter 4 en los hats: cada vuelta arranca un cuarto más adelante. sometimes 50% rev en el metal." \
    hats "hats16 iter 4" metal "metal_hit sometimes 50% rev"
stage 4 "every 4 fast 2 en el bombo: un redoble en el último compás de cada cuatro." \
    kick "kick4 every 4 fast 2"
stage 8 "Respiro: sin bombo ni clap, la ácida a media velocidad (slow 2)." \
    kick "rest" clap "rest" acid "acid_line slow 2" stab "rest"
stage 8 "Vuelve todo, la ácida con every 4 rev y degrade 20%." \
    kick "kick4" clap "clap" acid "acid_line every 4 rev degrade 20%" stab "stab_a, stab_b"
stage 4 "Salida: las transformaciones afuera, cada pista como está escrita." \
    kick "kick4" hats "hats" clap "rest" bass "roll" acid "acid_line" stab "rest" metal "rest"

if [[ "$MODE" == render ]]; then
    "$TATUM" set render "$SET_DIR" -o "$OUT"
else
    echo
    echo "Done. $FILE goes back to how it was."
fi
