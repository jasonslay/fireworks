#!/usr/bin/env bash
# Build an installable Fireworks.saver bundle around a macOS fireworks binary.
#
# Usage:
#   ./scripts/create_macos_saver.sh <path-to-fireworks-binary> [output-dir]
#
# Produces:
#   <output-dir>/Fireworks.saver
#   <output-dir>/fireworks-macos-screensaver.zip
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/macos/screensaver"
BINARY="${1:?Usage: $0 <path-to-fireworks-binary> [output-dir]}"
OUT_DIR="${2:-$ROOT/dist/screensaver}"
NAME="Fireworks"
BUNDLE="$OUT_DIR/$NAME.saver"
ZIP="$OUT_DIR/fireworks-macos-screensaver.zip"
DEPLOY="${MACOSX_DEPLOYMENT_TARGET:-14.0}"
SDK="$(xcrun --sdk macosx --show-sdk-path)"

if [[ ! -f "$BINARY" ]]; then
  echo "error: binary not found: $BINARY" >&2
  exit 1
fi
if [[ ! -f "$SRC/FireworksView.swift" || ! -f "$SRC/Info.plist" ]]; then
  echo "error: screensaver sources missing under $SRC" >&2
  exit 1
fi

rm -rf "$BUNDLE" "$ZIP"
mkdir -p "$BUNDLE/Contents/MacOS"

cp "$SRC/Info.plist" "$BUNDLE/Contents/Info.plist"

# Stamp version from Cargo.toml when available.
if VERSION=$(grep -m1 '^version' "$ROOT/Cargo.toml" | sed 's/version = "\(.*\)"/\1/'); then
  /usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $VERSION" "$BUNDLE/Contents/Info.plist" 2>/dev/null \
    || true
  /usr/libexec/PlistBuddy -c "Set :CFBundleVersion $VERSION" "$BUNDLE/Contents/Info.plist" 2>/dev/null \
    || true
fi

cp "$BINARY" "$BUNDLE/Contents/MacOS/fireworks"
chmod +x "$BUNDLE/Contents/MacOS/fireworks"

COMMON=(
  -sdk "$SDK"
  -framework ScreenSaver
  -framework AppKit
  -emit-library
  -Xlinker -bundle
  -module-name "$NAME"
  -O
)

swiftc -target "arm64-apple-macos${DEPLOY}" "${COMMON[@]}" \
  -o "$BUNDLE/Contents/MacOS/arm64.bin" \
  "$SRC/FireworksView.swift"

swiftc -target "x86_64-apple-macos${DEPLOY}" "${COMMON[@]}" \
  -o "$BUNDLE/Contents/MacOS/x86_64.bin" \
  "$SRC/FireworksView.swift"

lipo -create \
  "$BUNDLE/Contents/MacOS/arm64.bin" \
  "$BUNDLE/Contents/MacOS/x86_64.bin" \
  -output "$BUNDLE/Contents/MacOS/$NAME"
rm -f "$BUNDLE/Contents/MacOS/arm64.bin" "$BUNDLE/Contents/MacOS/x86_64.bin"

# Ad-hoc sign so Gatekeeper will load the bundle on the build machine / CI.
codesign --force --deep --sign - --timestamp=none "$BUNDLE"

# Zip keeps the .saver as a single downloadable installable artifact.
ditto -c -k --keepParent "$BUNDLE" "$ZIP"

echo "Built: $BUNDLE"
echo "Zip:   $ZIP"
lipo -info "$BUNDLE/Contents/MacOS/$NAME"
lipo -info "$BUNDLE/Contents/MacOS/fireworks"
