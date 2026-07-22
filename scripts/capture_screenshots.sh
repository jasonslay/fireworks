#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

mkdir -p docs/screenshots
cargo build --release

BIN=./target/release/fireworks

FIREWORKS_SCENE=night \
FIREWORKS_SCREENSHOT=docs/screenshots/night.png \
FIREWORKS_SCREENSHOT_FRAME=90 \
"$BIN"

FIREWORKS_SCENE=burst \
FIREWORKS_SCREENSHOT=docs/screenshots/burst.png \
FIREWORKS_SCREENSHOT_FRAME=105 \
"$BIN"

FIREWORKS_SCENE=finale \
FIREWORKS_SCREENSHOT=docs/screenshots/finale.png \
FIREWORKS_SCREENSHOT_FRAME=120 \
"$BIN"

echo "Screenshots written to docs/screenshots/"
