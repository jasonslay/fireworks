#!/usr/bin/env bash
# Build the fireworks simulator for the web (WebAssembly + JS glue).
set -euo pipefail

export PATH="${HOME}/.cargo/bin:${PATH}"

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

PROFILE="${PROFILE:-wasm-release}"
TARGET="wasm32-unknown-unknown"
OUT="$ROOT/dist"
WASM="$ROOT/target/$TARGET/$PROFILE/fireworks.wasm"

echo "Building for $TARGET (profile: $PROFILE)…"
cargo build --profile "$PROFILE" --target "$TARGET" --no-default-features

if ! command -v wasm-bindgen >/dev/null; then
  echo "wasm-bindgen not found. Install with: cargo install wasm-bindgen-cli"
  exit 1
fi

if ! command -v wasm-opt >/dev/null; then
  echo "wasm-opt not found. Install binaryen (e.g. pacman -S binaryen)."
  exit 1
fi

rm -rf "$OUT"
mkdir -p "$OUT"

echo "Running wasm-bindgen…"
wasm-bindgen --no-typescript --target web \
  --out-dir "$OUT" \
  --out-name fireworks \
  "$WASM"

BEFORE=$(stat -c%s "$OUT/fireworks_bg.wasm")
echo "Optimizing with wasm-opt (-Oz)…"
wasm-opt \
  -Oz \
  --strip-debug \
  --strip-dwarf \
  --strip-producers \
  --strip-target-features \
  --converge \
  --enable-bulk-memory \
  --enable-nontrapping-float-to-int \
  -o "$OUT/fireworks_bg.wasm" \
  "$OUT/fireworks_bg.wasm"
AFTER=$(stat -c%s "$OUT/fireworks_bg.wasm")
SAVED=$((BEFORE - AFTER))

cp "$ROOT/web/index.html" "$OUT/index.html"

human() { numfmt --to=iec-i --suffix=B "$1" 2>/dev/null || echo "$1 bytes"; }

echo ""
echo "Web bundle ready in dist/"
echo "  wasm: $(human "$AFTER") (wasm-opt saved $(human "$SAVED"))"
echo "  cd dist && python -m http.server 8080"
echo "  Then open http://127.0.0.1:8080"
