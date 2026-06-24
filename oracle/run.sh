#!/usr/bin/env bash
# Reference oracle: compile the decomp's exact bullet/RNG math (bullet_oracle.cpp)
# and the port's real spawn_bullet_pattern (examples/oracle_dump.rs), run the same
# battery + RNG seed, and diff. PASS == the port matches the decomp byte-for-byte.
set -euo pipefail
cd "$(dirname "$0")/.."

clang++ -O2 -o /tmp/bullet_oracle oracle/bullet_oracle.cpp
/tmp/bullet_oracle > /tmp/oracle_cpp.txt

source "$HOME/.cargo/env" 2>/dev/null || true
cargo run -p th06 --example oracle_dump --release 2>/dev/null | grep -E '^-?[0-9]' > /tmp/oracle_rust.txt

if diff -q /tmp/oracle_cpp.txt /tmp/oracle_rust.txt >/dev/null; then
    echo "PASS: port bullet math matches the decomp ($(wc -l < /tmp/oracle_cpp.txt | tr -d ' ') bullets)"
else
    echo "FAIL: divergence (decomp left, port right):"
    diff /tmp/oracle_cpp.txt /tmp/oracle_rust.txt | head -40
    exit 1
fi
