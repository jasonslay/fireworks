#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

mkdir -p docs/screenshots
cargo build --release

BIN=./target/release/fireworks
OUT=docs/screenshots/demo.gif
CAPTURE_WIDTH=1280
CAPTURE_HEIGHT=800
FPS=24
# Capture every 2 sim frames (~30 Hz at 60 FPS) then assemble at 24 FPS.
FRAME_STEP=2
FRAME_END=240

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

FIREWORKS_FRAME_DIR="$tmp" \
FIREWORKS_FRAME_END="$FRAME_END" \
FIREWORKS_FRAME_STEP="$FRAME_STEP" \
"$BIN"

if command -v gifski >/dev/null 2>&1; then
  gifski --output "$OUT" --fps "$FPS" --width "$CAPTURE_WIDTH" --height "$CAPTURE_HEIGHT" \
    --quality 100 "$tmp"/frame_*.png
else
  echo "gifski not found; falling back to ffmpeg (install gifski for better quality)" >&2
  ffmpeg -y -loglevel error \
    -framerate "$FPS" -i "$tmp/frame_%04d.png" \
    -vf "scale=${CAPTURE_WIDTH}:${CAPTURE_HEIGHT}:flags=lanczos,split[s0][s1];[s0]palettegen=stats_mode=diff[p];[s1][p]paletteuse=dither=sierra2_4a:diff_mode=rectangle" \
    -gifflags -offsetting \
    -loop 0 "$OUT"
fi

echo "Wrote ${OUT} (${CAPTURE_WIDTH}x${CAPTURE_HEIGHT}, ${FPS} fps)"
