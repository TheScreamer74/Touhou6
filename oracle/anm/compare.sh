#!/usr/bin/env bash
# Diff decomp AnmManager::ExecuteScript vs the port's BgQuadVm for every bg quad
# script of a stage. Usage: ./compare.sh <stage 1-6> <frames>
set -euo pipefail
cd "$(dirname "$0")"
N="${1:-5}"; FRAMES="${2:-600}"; ROOT=../..
./build.sh >/dev/null 2>&1
[ -f "/tmp/stg${N}bg.anm" ] || (cd "$ROOT" && cargo run -q -p th06 --example extract_ecl --release -- ../res/ST.DAT "stg${N}bg.anm" "/tmp/stg${N}bg.anm" >/dev/null)
# script ids present in this anm
IDS=$(cd "$ROOT" && cargo run -q -p th06 --example bg_quad_ids --release -- ../res/ST.DAT "$N" 2>/dev/null)
fail=0
for sid in $IDS; do
    /tmp/oracle_anm "/tmp/stg${N}bg.anm" "$sid" "$FRAMES" > /tmp/anm_decomp.txt
    (cd "$ROOT" && cargo run -q -p th06 --example bg_quad_dump --release -- ../res/ST.DAT "$N" "$sid" "$FRAMES" 2>/dev/null) > /tmp/anm_port.txt
    if ! diff -q /tmp/anm_decomp.txt /tmp/anm_port.txt >/dev/null; then
        echo "stage $N script $sid DIVERGES:"; diff /tmp/anm_decomp.txt /tmp/anm_port.txt | head -6; fail=1
    fi
done
[ $fail -eq 0 ] && echo "stage $N: all bg quad scripts IDENTICAL across $FRAMES frames"
