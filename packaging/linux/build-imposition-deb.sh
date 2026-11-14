#!/usr/bin/env bash
# Build a side-by-side DEB for the Imposition edition.
# Usage: build-imposition-deb.sh <version> <release-dir> <output-dir>
set -euo pipefail
VERSION="$1"
BIN="$(realpath "$2")"
OUT="$(realpath -m "$3")"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PKG="tracedraw-imposition"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

install -d "$STAGE/DEBIAN" \
  "$STAGE/usr/bin" \
  "$STAGE/usr/lib/$PKG" \
  "$STAGE/usr/share/applications" \
  "$STAGE/usr/share/icons/hicolor/256x256/apps" \
  "$STAGE/usr/share/doc/$PKG"

install -m 0755 "$ROOT/packaging/linux/imposition-launcher" "$STAGE/usr/bin/tracedraw-imposition"
install -m 0755 "$BIN/tracedraw" "$STAGE/usr/lib/$PKG/tracedraw"
install -m 0755 "$BIN/tracedraw-cli" "$STAGE/usr/lib/$PKG/tracedraw-cli"
install -m 0644 "$ROOT/packaging/linux/tracedraw-imposition.desktop" "$STAGE/usr/share/applications/tracedraw-imposition.desktop"
install -m 0644 "$ROOT/assets/icon/tracedraw-256.png" "$STAGE/usr/share/icons/hicolor/256x256/apps/tracedraw-imposition.png"
install -m 0644 "$ROOT/README.md" "$STAGE/usr/share/doc/$PKG/README"
install -m 0644 "$ROOT/LICENSE-MIT" "$STAGE/usr/share/doc/$PKG/LICENSE-MIT"
install -m 0644 "$ROOT/LICENSE-APACHE" "$STAGE/usr/share/doc/$PKG/LICENSE-APACHE"

cat > "$STAGE/DEBIAN/control" <<CONTROL
Package: $PKG
Version: $VERSION
Section: graphics
Priority: optional
Architecture: amd64
Maintainer: TraceDraw contributors <contact@tracedraw.org>
Depends: libc6 (>= 2.35)
Description: TraceDraw editor with object imposition
 A separately installed TraceDraw build with object-based imposition.
 It uses its own launcher, application shortcut and user settings.
CONTROL

mkdir -p "$OUT"
dpkg-deb --build --root-owner-group "$STAGE" "$OUT/tracedraw-imposition_${VERSION}_amd64.deb"
