#!/usr/bin/env bash
# Cross-compile the reference-capture proxy d3d8.dll (32-bit, TH06 is i686).
set -euo pipefail
cd "$(dirname "$0")"
i686-w64-mingw32-gcc -O2 -Wall -shared -static-libgcc \
    -o d3d8.dll d3d8proxy.c d3d8proxy.def -Wl,--kill-at
echo "built $(pwd)/d3d8.dll"
