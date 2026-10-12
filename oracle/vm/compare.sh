#!/usr/bin/env bash
# Fixed seed0x1234, Normal rank16, ReimuA, player192,384; no input/damage/dialogue.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; cd "$HERE"
source "$HERE/../common.sh"
oracle_check_assets
STAGE="${1:-5}"; FRAMES="${2:-6000}"; OUT="$(oracle_output_dir)"
oracle_check_stage "$STAGE"
oracle_check_frames "$FRAMES"
RES_DIR="${TH06_RES_DIR:-$(dirname "$ST_DAT")}"
cd "$ORACLE_ROOT"
# Hash the actual working-tree inputs before building, then verify that concurrent
# edits did not change them during this run. The target binary is fingerprinted too.
source_manifest() {
    printf '%s\n' Cargo.toml Cargo.lock
    rg --files crates -g '*.rs' -g Cargo.toml -g '*.wgsl'
}
source_manifest | sort | while IFS= read -r file; do shasum -a 256 "$file"; done > "$OUT/vm_stage${STAGE}_sources_before.sha256"
cargo build -q -p th06 --release --bin th06 --example extract_ecl --example enemy_anm_sizes
if [ -n "${3:-}" ]; then ECL="$3"; else
    ECL="$OUT/ecldata$STAGE.ecl"
    target/release/examples/extract_ecl "$ST_DAT" "ecldata$STAGE.ecl" "$ECL" >/dev/null
fi
target/release/examples/enemy_anm_sizes "$ST_DAT" "$STAGE" > "$HERE/enemy_sizes.h"
"$HERE/build.sh"
/tmp/oracle_vm "$ECL" "$FRAMES" "$STAGE" > "$OUT/vm_stage${STAGE}_decomp.txt" 2> "$OUT/vm_stage${STAGE}_decomp.log"
target/release/th06 --game-dir "$RES_DIR" --scene stage --ecl-dump --stage "$STAGE" --char 0 --frames "$FRAMES" > "$OUT/vm_stage${STAGE}_port.txt" 2> "$OUT/vm_stage${STAGE}_port.log"
source_manifest | sort | while IFS= read -r file; do shasum -a 256 "$file"; done > "$OUT/vm_stage${STAGE}_sources_after.sha256"
shasum -a 256 target/release/th06 /tmp/oracle_vm "$ECL" > "$OUT/vm_stage${STAGE}_artifacts.sha256"
if ! diff -q "$OUT/vm_stage${STAGE}_sources_before.sha256" "$OUT/vm_stage${STAGE}_sources_after.sha256" >/dev/null; then
    echo "Source files changed during comparison; results cannot identify one source snapshot: $OUT" >&2
    exit 1
fi
if diff -q "$OUT/vm_stage${STAGE}_decomp.txt" "$OUT/vm_stage${STAGE}_port.txt" >/dev/null; then
    echo "PASS stage $STAGE: IDENTICAL across $FRAMES frames (decomp == port)"
else
    echo "DIVERGENCE stage $STAGE (decomp left / port right):"
    diff "$OUT/vm_stage${STAGE}_decomp.txt" "$OUT/vm_stage${STAGE}_port.txt" | sed -n '1,16p' || true
    echo "Evidence: $OUT"
    exit 1
fi
echo "Evidence: $OUT"
