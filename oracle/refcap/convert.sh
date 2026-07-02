#!/usr/bin/env bash
# Convert a captured BMP sequence to an mp4 (and optionally keep PNGs).
# Usage: ./convert.sh <capture_dir> <out.mp4>
set -euo pipefail
DIR="${1:?capture dir}"
OUT="${2:?out.mp4}"
ffmpeg -y -framerate 60 -start_number "$(ls "$DIR" | grep -o '[0-9]\{6\}' | sort -n | head -1)" \
    -i "$DIR/frame_%06d.bmp" -c:v libx264 -pix_fmt yuv420p -crf 18 "$OUT"
echo "wrote $OUT"
