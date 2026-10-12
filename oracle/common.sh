#!/usr/bin/env bash
# Paths and explicit source profile; no downloads or reference math rewriting.
ORACLE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DECOMP="${TH06_DECOMP_SRC:-$ORACLE_ROOT/../refs/th06-decomp/src}"
ST_DAT="${TH06_ST_DAT:-${TH06_RES_DIR:-$ORACLE_ROOT/../res}/ST.DAT}"
oracle_check_source() {
    local name
    for name in diffbuild.hpp Rng.cpp ZunTimer.cpp AnmManager.cpp; do
        if [ ! -f "$DECOMP/$name" ]; then
            printf '%s\n' "Oracle source profile unavailable: $DECOMP/$name" \
              'Use TH06_DECOMP_SRC=<source>/src with happyhavoc/th06 commit bd90685fa40eaf2e16789dcf788843b304e6e02d.' \
              'Later upstream revisions merged these files; see oracle/README.md.' >&2
            return 1
        fi
    done
}
oracle_check_assets() {
    [ -f "$ST_DAT" ] || { printf 'ST.DAT not found: %s (set TH06_ST_DAT)\n' "$ST_DAT" >&2; return 1; }
}
oracle_output_dir() {
    if [ -n "${TH06_ORACLE_OUTPUT_DIR:-}" ]; then
        mkdir -p "$TH06_ORACLE_OUTPUT_DIR"
        printf '%s\n' "$TH06_ORACLE_OUTPUT_DIR"
    else
        mktemp -d "${TMPDIR:-/tmp}/th06-oracle.XXXXXX"
    fi
}
oracle_check_stage() {
    case "$1" in 1|2|3|4|5|6|7) ;; *) printf 'Stage must be 1..7: %s\n' "$1" >&2; return 1 ;; esac
}
oracle_check_frames() {
    case "$1" in ''|*[!0-9]*) printf 'Frame count must be a positive integer: %s\n' "$1" >&2; return 1 ;; esac
    [ "$1" -gt 0 ] || { printf 'Frame count must be positive: %s\n' "$1" >&2; return 1; }
}
