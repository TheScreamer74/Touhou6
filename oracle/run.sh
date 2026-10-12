#!/usr/bin/env bash
# Reference oracle: compile the decomp's exact bullet/RNG math (bullet_oracle.cpp)
# and the port's real spawn_bullet_pattern (examples/oracle_dump.rs), run the same
# battery + RNG seed, and diff. PASS == the port matches the decomp byte-for-byte.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
source "$HERE/common.sh"
OUT="$(oracle_output_dir)"
cd "$ORACLE_ROOT"

clang++ -O2 -o /tmp/bullet_oracle oracle/bullet_oracle.cpp
/tmp/bullet_oracle > "$OUT/bullets_decomp.txt"

cargo run -p th06 --example oracle_dump --release > "$OUT/bullets_port.txt"
[ -s "$OUT/bullets_decomp.txt" ] && [ -s "$OUT/bullets_port.txt" ] || { echo "No bullet observations emitted" >&2; exit 1; }

if diff -q "$OUT/bullets_decomp.txt" "$OUT/bullets_port.txt" >/dev/null; then
    echo "PASS: port bullet math matches the decomp ($(wc -l < "$OUT/bullets_decomp.txt" | tr -d ' ') bullets)"
else
    echo "FAIL: divergence (decomp left, port right):"
    diff "$OUT/bullets_decomp.txt" "$OUT/bullets_port.txt" | sed -n '1,40p' || true
    exit 1
fi
echo "Evidence: $OUT"
