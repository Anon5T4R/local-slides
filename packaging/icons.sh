#!/usr/bin/env bash
# Regenerate every platform icon from assets/app-icon/localslides.svg.
# Needs: resvg (brew install resvg / cargo install resvg). On macOS, iconutil also writes the
# .icns; on Windows (no iconutil) the committed .icns is packed from the PNG renders directly, so
# this script leaves that file alone there. The outputs are committed, so builds and packaging
# never need these tools.
#
#   packaging/icons.sh
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIR="$ROOT/assets/app-icon"
SVG="$DIR/localslides.svg"
ID="com.local.localslides"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

command -v resvg >/dev/null || { echo "error: resvg not found (brew install resvg)" >&2; exit 1; }

# The SVG is a full-bleed 1024 tile, used as-is on Windows and Linux. macOS icons follow Apple's
# grid: the tile is 824/1024 of the canvas with a transparent margin, so widen the viewBox around it.
MAC="$TMP/mac.svg"
sed 's/viewBox="0 0 1024 1024"/viewBox="-124.272 -124.272 1272.544 1272.544"/' "$SVG" >"$MAC"
grep -q 'viewBox="-124.272' "$MAC" || { echo "error: expected viewBox=\"0 0 1024 1024\" in $SVG" >&2; exit 1; }

render() { resvg -w "$2" -h "$2" --use-fonts-dir /System/Library/Fonts "$1" "$3" </dev/null; }

render "$SVG" 1024 "$DIR/localslides-1024.png"
render "$MAC" 512 "$DIR/localslides-macos-512.png"

# Linux hicolor theme (the 256 one is also the runtime icon on Windows and Linux).
for s in 16 24 32 48 64 128 256 512; do
  mkdir -p "$DIR/hicolor/${s}x${s}/apps"
  render "$SVG" "$s" "$DIR/hicolor/${s}x${s}/apps/$ID.png"
done
mkdir -p "$DIR/hicolor/scalable/apps"
cp "$SVG" "$DIR/hicolor/scalable/apps/$ID.svg"

# Windows .ico.
ICO_PNGS=()
for s in 16 20 24 32 40 48 64 128 256; do
  render "$SVG" "$s" "$TMP/ico-$s.png"
  ICO_PNGS+=("$TMP/ico-$s.png")
done
(cd "$ROOT" && cargo run -q -p xtask -- ico "$DIR/localslides.ico" "${ICO_PNGS[@]}")

# macOS .icns.
if command -v iconutil >/dev/null; then
  SET="$TMP/localslides.iconset"
  mkdir -p "$SET"
  for s in 16 32 128 256 512; do
    render "$MAC" "$s" "$SET/icon_${s}x${s}.png"
    render "$MAC" $((s * 2)) "$SET/icon_${s}x${s}@2x.png"
  done
  iconutil -c icns -o "$DIR/localslides.icns" "$SET"
else
  echo "warning: iconutil not found (macOS only); localslides.icns left as committed (on Windows it is packed from the PNG renders directly)" >&2
fi
echo "icons written to $DIR"
