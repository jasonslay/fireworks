#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

mkdir -p docs/screenshots
cargo build --release

BIN=./target/release/fireworks

capture_png() {
  local scene=$1
  local out=$2
  local frame=$3
  FIREWORKS_SCENE="$scene" \
  FIREWORKS_SCREENSHOT="$out" \
  FIREWORKS_SCREENSHOT_FRAME="$frame" \
  "$BIN"
}

capture_gif() {
  local scene=$1
  local out=$2
  local end=$3
  local tmp
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' RETURN

  FIREWORKS_SCENE="$scene" \
  FIREWORKS_FRAME_DIR="$tmp" \
  FIREWORKS_FRAME_END="$end" \
  FIREWORKS_FRAME_STEP=3 \
  "$BIN"

  ffmpeg -y -loglevel error \
    -framerate 15 -i "$tmp/frame_%04d.png" \
    -vf "fps=12,scale=800:-1:flags=lanczos,split[s0][s1];[s0]palettegen=stats_mode=diff[p];[s1][p]paletteuse=dither=bayer" \
    -loop 0 "$out"
}

# Still PNG for the calm night-sky hero thumbnail in the table.
capture_png night docs/screenshots/night.png 90

# Animated GIFs for motion-heavy scenes (sim frames are 60 Hz ticks, step 3 ≈ 20 Hz capture).
capture_gif burst docs/screenshots/burst.gif 120
capture_gif finale docs/screenshots/finale.gif 165

echo "Screenshots written to docs/screenshots/"
echo "Note: GIF capture requires ffmpeg."
