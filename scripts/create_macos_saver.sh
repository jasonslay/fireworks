#!/usr/bin/env bash
# Build an installable Fireworks.saver that embeds the WebAssembly web bundle.
#
# The legacyScreenSaver sandbox cannot launch the native fireworks binary, so
# the screensaver hosts the same web build used on jtslay.com inside WKWebView.
#
# Usage:
#   ./scripts/create_macos_saver.sh <web-dist-dir> [output-dir]
#
# Produces:
#   <output-dir>/Fireworks.saver
#   <output-dir>/fireworks-macos-screensaver.zip
#
# Optional signing / notarization (required to avoid Gatekeeper malware warnings
# when downloading from the internet):
#   MACOS_CODESIGN_IDENTITY   Developer ID Application: Name (TEAMID)
#   APPLE_API_KEY_PATH        path to AuthKey_XXX.p8  (or set APPLE_API_KEY inline)
#   APPLE_API_KEY_ID
#   APPLE_API_ISSUER_ID
#   APPLE_TEAM_ID
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/macos/screensaver"
WEB_DIST="${1:?Usage: $0 <web-dist-dir> [output-dir]}"
OUT_DIR="${2:-$ROOT/dist/screensaver}"
NAME="Fireworks"
BUNDLE="$OUT_DIR/$NAME.saver"
ZIP="$OUT_DIR/fireworks-macos-screensaver.zip"
DEPLOY="${MACOSX_DEPLOYMENT_TARGET:-14.0}"
SDK="$(xcrun --sdk macosx --show-sdk-path)"
IDENTITY="${MACOS_CODESIGN_IDENTITY:-}"

if [[ ! -d "$WEB_DIST" ]]; then
  echo "error: web dist dir not found: $WEB_DIST" >&2
  exit 1
fi
for required in index.html fireworks.js fireworks_bg.wasm; do
  if [[ ! -f "$WEB_DIST/$required" ]]; then
    echo "error: missing $required in $WEB_DIST (run ./scripts/build_web.sh first)" >&2
    exit 1
  fi
done
if [[ ! -f "$SRC/FireworksView.swift" || ! -f "$SRC/Info.plist" ]]; then
  echo "error: screensaver sources missing under $SRC" >&2
  exit 1
fi

WASM_SIZE=$(wc -c < "$WEB_DIST/fireworks_bg.wasm" | tr -d ' ')
if (( WASM_SIZE < 1000000 )); then
  echo "error: fireworks_bg.wasm looks too small (${WASM_SIZE} bytes)" >&2
  exit 1
fi

rm -rf "$BUNDLE" "$ZIP"
mkdir -p "$BUNDLE/Contents/MacOS" "$BUNDLE/Contents/Resources/web"

cp "$SRC/Info.plist" "$BUNDLE/Contents/Info.plist"

# Stamp version from Cargo.toml when available.
if VERSION=$(grep -m1 '^version' "$ROOT/Cargo.toml" | sed 's/version = "\(.*\)"/\1/'); then
  /usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $VERSION" "$BUNDLE/Contents/Info.plist" 2>/dev/null \
    || true
  /usr/libexec/PlistBuddy -c "Set :CFBundleVersion $VERSION" "$BUNDLE/Contents/Info.plist" 2>/dev/null \
    || true
fi

# Embed the web bundle (index.html + JS + WASM).
cp "$WEB_DIST/index.html" "$WEB_DIST/fireworks.js" "$WEB_DIST/fireworks_bg.wasm" \
  "$BUNDLE/Contents/Resources/web/"
# wasm-bindgen may emit a snippets dir or extra js; copy anything else present.
if [[ -d "$WEB_DIST/snippets" ]]; then
  cp -R "$WEB_DIST/snippets" "$BUNDLE/Contents/Resources/web/"
fi

COMMON=(
  -sdk "$SDK"
  -framework ScreenSaver
  -framework AppKit
  -framework WebKit
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

BUNDLE_WASM="$BUNDLE/Contents/Resources/web/fireworks_bg.wasm"
BUNDLE_WASM_SIZE=$(wc -c < "$BUNDLE_WASM" | tr -d ' ')
if (( BUNDLE_WASM_SIZE != WASM_SIZE )); then
  echo "error: wasm was altered during packaging (${BUNDLE_WASM_SIZE} vs ${WASM_SIZE} bytes)" >&2
  exit 1
fi

sign_bundle() {
  local plugin="$BUNDLE/Contents/MacOS/$NAME"

  if [[ -n "$IDENTITY" ]]; then
    echo "Signing with Developer ID: $IDENTITY"
    codesign --force --options runtime --timestamp \
      --sign "$IDENTITY" \
      "$plugin"
    codesign --force --options runtime --timestamp \
      --sign "$IDENTITY" \
      "$BUNDLE"
    codesign --verify --deep --strict --verbose=2 "$BUNDLE"
  else
    echo "warning: MACOS_CODESIGN_IDENTITY unset; ad-hoc signing only." >&2
    echo "warning: Gatekeeper will block downloads until Developer ID + notarization are configured." >&2
    codesign --force --deep --sign - --timestamp=none "$BUNDLE"
  fi
}

notarize_bundle() {
  if [[ -z "$IDENTITY" ]]; then
    return 0
  fi
  if [[ -z "${APPLE_API_KEY_ID:-}" || -z "${APPLE_API_ISSUER_ID:-}" || -z "${APPLE_TEAM_ID:-}" ]]; then
    echo "warning: skipping notarization (APPLE_API_KEY_ID / APPLE_API_ISSUER_ID / APPLE_TEAM_ID not set)." >&2
    return 0
  fi

  local key_path="${APPLE_API_KEY_PATH:-}"
  local tmp_key=""
  if [[ -z "$key_path" ]]; then
    if [[ -z "${APPLE_API_KEY:-}" ]]; then
      echo "warning: skipping notarization (APPLE_API_KEY_PATH or APPLE_API_KEY not set)." >&2
      return 0
    fi
    tmp_key="$(mktemp)"
    printf '%s\n' "$APPLE_API_KEY" >"$tmp_key"
    key_path="$tmp_key"
  fi

  local submit_zip
  submit_zip="$(mktemp -u).zip"
  echo "Submitting for notarization..."
  ditto -c -k --keepParent "$BUNDLE" "$submit_zip"
  xcrun notarytool submit "$submit_zip" \
    --key "$key_path" \
    --key-id "$APPLE_API_KEY_ID" \
    --issuer "$APPLE_API_ISSUER_ID" \
    --wait
  rm -f "$submit_zip"
  [[ -n "$tmp_key" ]] && rm -f "$tmp_key"

  echo "Stapling notarization ticket..."
  xcrun stapler staple "$BUNDLE"
  xcrun stapler validate "$BUNDLE"
}

sign_bundle
notarize_bundle

# Zip keeps the .saver as a single downloadable installable artifact.
# Include a double-clickable installer that clears Gatekeeper quarantine and
# re-signs locally (required for unsigned downloads on modern macOS).
STAGE="$OUT_DIR/stage"
rm -rf "$STAGE"
mkdir -p "$STAGE"
ditto "$BUNDLE" "$STAGE/$NAME.saver"
cat >"$STAGE/Install Fireworks Screensaver.command" <<'EOF'
#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"
xattr -cr "Fireworks.saver" 2>/dev/null || true
codesign --force --deep --sign - "Fireworks.saver"
DEST="${HOME}/Library/Screen Savers/Fireworks.saver"
mkdir -p "${HOME}/Library/Screen Savers"
rm -rf "$DEST"
ditto "Fireworks.saver" "$DEST"
xattr -cr "$DEST" 2>/dev/null || true
codesign --force --deep --sign - "$DEST"
killall -9 legacyScreenSaver 2>/dev/null || true
killall -9 WallpaperAgent 2>/dev/null || true
echo "Installed to $DEST"
echo "Open System Settings → Screen Saver and select Fireworks."
echo "If blocked: Privacy & Security → Open Anyway, then try again."
open /System/Applications/'System Settings.app' 2>/dev/null \
  || open /System/Applications/'System Preferences.app' 2>/dev/null \
  || true
read -r -p "Press Return to close…"
EOF
chmod +x "$STAGE/Install Fireworks Screensaver.command"

rm -f "$ZIP"
(
  cd "$STAGE"
  zip -qry "$ZIP" "Fireworks.saver" "Install Fireworks Screensaver.command"
)

ZIP_SIZE=$(wc -c < "$ZIP" | tr -d ' ')
if (( ZIP_SIZE < 1000000 )); then
  echo "error: screensaver zip is suspiciously small (${ZIP_SIZE} bytes; wasm was ${WASM_SIZE})" >&2
  exit 1
fi

ZIP_WASM_SIZE=$(unzip -l "$ZIP" | awk '/Resources\/web\/fireworks_bg\.wasm$/ { print $1; exit }')
if [[ -z "${ZIP_WASM_SIZE}" ]] || (( ZIP_WASM_SIZE != WASM_SIZE )); then
  echo "error: zip is missing the full wasm (found ${ZIP_WASM_SIZE:-0} bytes, expected ${WASM_SIZE})" >&2
  exit 1
fi

echo "Built: $BUNDLE"
echo "Zip:   $ZIP (${ZIP_SIZE} bytes; wasm ${WASM_SIZE} bytes)"
lipo -info "$BUNDLE/Contents/MacOS/$NAME"
ls -lh "$BUNDLE/Contents/MacOS/$NAME" "$BUNDLE/Contents/Resources/web/fireworks_bg.wasm" "$ZIP"
