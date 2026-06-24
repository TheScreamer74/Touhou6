#!/usr/bin/env bash
# Runs the decomp oracle and the Rust port over the same stage ECL (fixed seed +
# fixed player, no input) and reports the first divergent frame.
# Usage: ./compare.sh <stage 1-6> <frames> [ecldata.ecl]
set -euo pipefail
cd "$(dirname "$0")"
STAGE="${1:-5}"; FRAMES="${2:-6000}"; ECL="${3:-/tmp/ecldata$STAGE.ecl}"
ROOT=../..
./build.sh >/dev/null
/tmp/oracle_vm "$ECL" "$FRAMES" > /tmp/oracle_out.txt 2>/dev/null
(cd "$ROOT" && ./target/release/th06 --scene stage --ecl-dump --stage "$STAGE" --char 0 --frames "$FRAMES" >/tmp/rust_out.txt 2>/dev/null)
if diff -q /tmp/oracle_out.txt /tmp/rust_out.txt >/dev/null; then
    echo "IDENTICAL across $FRAMES frames (decomp == port)"
else
    L=$(diff /tmp/oracle_out.txt /tmp/rust_out.txt | grep -m1 -oE '^[0-9]+' )
    echo "first divergence near output line $L:"
    diff /tmp/oracle_out.txt /tmp/rust_out.txt | head -8
fi
