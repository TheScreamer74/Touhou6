#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
OUT="${TH06_ORACLE_OUTPUT_DIR:-$(mktemp -d "${TMPDIR:-/tmp}/th06-projectiles.XXXXXX")}"
TH06_ORACLE_OUTPUT_DIR="$OUT" "$HERE/build.sh"
cd "$HERE/../.."
env -u TH06_GOD TH06_LASER_REFERENCE="$OUT/laser-reference.txt" cargo test -p th06 --lib --offline \
    stage::tests::original_laser_states_and_collisions_match_decompiled_source -- --ignored
env TH06_SHAKE_REFERENCE="$OUT/shakes-reference.txt" cargo test -p th06 --lib --offline \
    stage::tests::original_screen_shake_chains_match_source_with_overlap_and_time_stop -- --ignored
printf 'PASS: 864 laser cases (73,440 observations), 480 screen-shake observations. Evidence: %s\n' "$OUT"
