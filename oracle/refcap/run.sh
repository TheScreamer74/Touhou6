#!/usr/bin/env bash
# Deploy the proxy into the game dir and run TH06 under Wine.
# Usage: ./run.sh [game_dir]
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
GAME="${1:-$HERE/../../../TH06 ~ The Embodiment of Scarlet Devil/kouma}"
[ -f "$GAME/102h.exe" ] || { echo "102h.exe not found in: $GAME"; exit 1; }
[ -f "$HERE/d3d8.dll" ] || "$HERE/build.sh"

cp "$HERE/d3d8.dll" "$GAME/d3d8.dll"
[ -f "$GAME/th06cap.txt" ] || cp "$HERE/th06cap.example.txt" "$GAME/th06cap.txt"

export WINEPREFIX="${WINEPREFIX:-$HOME/.wine-th06}"
export WINEDLLOVERRIDES="d3d8=n,b;mscoree,mshtml="
export WINEDEBUG="${WINEDEBUG:--all}"
WINE="${WINE:-/opt/homebrew/bin/wine}"

[ -d "$WINEPREFIX" ] || "$WINE" wineboot --init

cd "$GAME"
exec "$WINE" 102h.exe
