#!/usr/bin/env bash
# Install Fireworks.saver for the current user.
#
# Unsigned downloads are blocked by Gatekeeper; this script clears quarantine
# and applies a *local* ad-hoc signature so macOS will load the plugin.
# Developer ID + notarization is still required for a double-click install
# with no Terminal steps.
#
# Usage:
#   ./scripts/install_macos_saver.sh [Fireworks.saver|fireworks-macos-screensaver.zip]
set -euo pipefail

SRC="${1:-}"
DEST_DIR="${HOME}/Library/Screen Savers"
DEST="${DEST_DIR}/Fireworks.saver"
TMP=""

cleanup() {
  [[ -n "$TMP" && -d "$TMP" ]] && rm -rf "$TMP"
}
trap cleanup EXIT

if [[ -z "$SRC" ]]; then
  for candidate in \
    ./Fireworks.saver \
    ./fireworks-macos-screensaver.zip \
    ./dist/screensaver/Fireworks.saver \
    ./dist/screensaver/fireworks-macos-screensaver.zip
  do
    if [[ -e "$candidate" ]]; then
      SRC="$candidate"
      break
    fi
  done
fi

if [[ -z "$SRC" || ! -e "$SRC" ]]; then
  echo "Usage: $0 <Fireworks.saver|fireworks-macos-screensaver.zip>" >&2
  exit 1
fi

if [[ "$SRC" == *.zip ]]; then
  TMP="$(mktemp -d)"
  ditto -x -k "$SRC" "$TMP"
  SRC="$(find "$TMP" -name 'Fireworks.saver' -print -quit || true)"
  if [[ -z "$SRC" || ! -d "$SRC" ]]; then
    echo "error: zip does not contain Fireworks.saver" >&2
    exit 1
  fi
fi

if [[ ! -d "$SRC" ]]; then
  echo "error: not a .saver bundle: $SRC" >&2
  exit 1
fi

echo "Preparing $SRC"
# Clear every xattr (quarantine alone is often not enough on Sonoma/Sequoia).
xattr -cr "$SRC" 2>/dev/null || true
# Re-sign locally so legacyScreenSaver will load the plugin.
codesign --force --deep --sign - "$SRC"
codesign --verify --deep --verbose=2 "$SRC" || true

echo "Installing to $DEST"
mkdir -p "$DEST_DIR"
rm -rf "$DEST"
ditto "$SRC" "$DEST"
xattr -cr "$DEST" 2>/dev/null || true
codesign --force --deep --sign - "$DEST"

# Force the screensaver host to reload plugins.
killall -9 legacyScreenSaver 2>/dev/null || true
killall -9 WallpaperAgent 2>/dev/null || true

echo
echo "Installed Fireworks.saver."
echo "Open System Settings → Screen Saver and select Fireworks."
echo
echo "If macOS still blocks it:"
echo "  1. Try to open/select Fireworks once (dismiss the warning)."
echo "  2. System Settings → Privacy & Security → scroll to Security → Open Anyway."
echo "  3. Re-run: open '$DEST'"
