#!/usr/bin/env bash
# Diff decomp Stage::OnUpdate bg state vs the port's background.rs.
# Usage: ./compare.sh <stage 1-6> <frames>
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; cd "$HERE"
source "$HERE/../common.sh"
oracle_check_assets
N="${1:-5}"; FRAMES="${2:-1200}"; OUT="$(oracle_output_dir)"
oracle_check_stage "$N"
oracle_check_frames "$FRAMES"
if [ "${TH06_SKIP_ORACLE_BUILD:-0}" != 1 ]; then "$HERE/build.sh"; fi
cd "$ORACLE_ROOT"
cargo build -q -p th06 --release --example extract_ecl --example bg_dump
target/release/examples/extract_ecl "$ST_DAT" "stage$N.std" "$OUT/stage$N.std" >/dev/null
/tmp/oracle_bg "$OUT/stage$N.std" "$FRAMES" > "$OUT/bg_stage${N}_decomp.txt"
target/release/examples/bg_dump "$ST_DAT" "$N" "$FRAMES" > "$OUT/bg_stage${N}_port.txt"
if diff -q "$OUT/bg_stage${N}_decomp.txt" "$OUT/bg_stage${N}_port.txt" >/dev/null; then
    echo "IDENTICAL across $FRAMES frames (decomp == port) stage $N"
else
    echo "divergence stage $N (decomp left / port right):"
    diff "$OUT/bg_stage${N}_decomp.txt" "$OUT/bg_stage${N}_port.txt" | sed -n '1,12p' || true
    echo "Evidence: $OUT"
    exit 1
fi
echo "Evidence: $OUT"
