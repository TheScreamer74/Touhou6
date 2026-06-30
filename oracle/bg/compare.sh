#!/usr/bin/env bash
# Diff decomp Stage::OnUpdate bg state vs the port's background.rs.
# Usage: ./compare.sh <stage 1-6> <frames>
set -euo pipefail
cd "$(dirname "$0")"
N="${1:-5}"; FRAMES="${2:-1200}"; ROOT=../..
./build.sh >/dev/null
[ -f "/tmp/stage$N.std" ] || (cd "$ROOT" && cargo run -q -p th06 --example extract_ecl --release -- ../res/ST.DAT "stage$N.std" "/tmp/stage$N.std" >/dev/null)
/tmp/oracle_bg "/tmp/stage$N.std" "$FRAMES" > /tmp/bg_decomp.txt
(cd "$ROOT" && cargo run -q -p th06 --example bg_dump --release -- ../res/ST.DAT "$N" "$FRAMES" 2>/dev/null) > /tmp/bg_port.txt
if diff -q /tmp/bg_decomp.txt /tmp/bg_port.txt >/dev/null; then
    echo "IDENTICAL across $FRAMES frames (decomp == port) stage $N"
else
    echo "divergence stage $N (decomp left / port right):"
    diff /tmp/bg_decomp.txt /tmp/bg_port.txt | head -12
fi
