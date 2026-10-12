#!/usr/bin/env bash
# Exact formatted-state comparison; no numerical tolerance is introduced.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
source "$HERE/../common.sh"
oracle_check_assets
N="${1:-5}"; FRAMES="${2:-600}"; OUT="$(oracle_output_dir)"
oracle_check_stage "$N"
oracle_check_frames "$FRAMES"
if [ "${TH06_SKIP_ORACLE_BUILD:-0}" != 1 ]; then "$HERE/build.sh"; fi
cd "$ORACLE_ROOT"
cargo build -q -p th06 --release --example extract_ecl --example bg_quad_ids --example bg_quad_dump
target/release/examples/extract_ecl "$ST_DAT" "stg${N}bg.anm" "$OUT/stg${N}bg.anm" >/dev/null
IDS=$(target/release/examples/bg_quad_ids "$ST_DAT" "$N")
fail=0; count=0
for sid in $IDS; do
    count=$((count+1))
    /tmp/oracle_anm "$OUT/stg${N}bg.anm" "$sid" "$FRAMES" > "$OUT/anm_stage${N}_script${sid}_decomp.txt"
    target/release/examples/bg_quad_dump "$ST_DAT" "$N" "$sid" "$FRAMES" > "$OUT/anm_stage${N}_script${sid}_port.txt"
    if ! diff -q "$OUT/anm_stage${N}_script${sid}_decomp.txt" "$OUT/anm_stage${N}_script${sid}_port.txt" >/dev/null; then
        echo "stage $N script $sid DIVERGES:"
        diff "$OUT/anm_stage${N}_script${sid}_decomp.txt" "$OUT/anm_stage${N}_script${sid}_port.txt" | sed -n '1,12p' || true
        fail=1
    fi
done
if [ "$count" -eq 0 ]; then echo "No scripts compared for stage $N" >&2; exit 1; fi
if [ "$fail" -eq 0 ]; then echo "PASS stage $N: $count bg quad scripts IDENTICAL across $FRAMES frames"; fi
echo "Evidence: $OUT"
exit "$fail"
