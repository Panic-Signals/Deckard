#!/usr/bin/env bash
# Install yabridge, which runs Windows VST plug-ins through Wine (Robbert van der
# Helm, GPL-3.0), the way its README says: unpacked into ~/.local/share/yabridge.
# DECKARD's Preferences ▸ Plug-ins runs this; so can you:
#
#   scripts/install-yabridge.sh
#
# Wine itself comes from your distribution's packages (DECKARD can install it for
# you, or: sudo apt install wine / sudo dnf install wine). The archive is checked
# against its SHA-256 before it's unpacked; a copy that's already there is kept.
set -euo pipefail

VERSION="5.1.1"
URL="https://github.com/robbert-vdh/yabridge/releases/download/$VERSION/yabridge-$VERSION.tar.gz"
SHA256="d11f0307412b5566b09a75ed4aa191266db56d22f4de4d20e4fbde6296cd1a47"
DIR="${XDG_DATA_HOME:-$HOME/.local/share}/yabridge"

if [ -x "$DIR/yabridgectl" ] && [ -f "$DIR/libyabridge-vst3.so" ]; then
    echo "yabridge: installed ($DIR)"
    exit 0
fi

command -v curl >/dev/null || { echo "missing: curl" >&2; exit 1; }
command -v tar >/dev/null || { echo "missing: tar" >&2; exit 1; }

TMP="$(mktemp -d "${TMPDIR:-/tmp}/yabridge.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT
echo "yabridge: downloading $VERSION"
curl -fL --retry 3 --progress-bar -o "$TMP/yabridge.tar.gz" "$URL"
GOT="$(sha256sum "$TMP/yabridge.tar.gz" | cut -d' ' -f1)"
if [ "$GOT" != "$SHA256" ]; then
    echo "yabridge: the download isn't the expected archive (SHA-256 $GOT)" >&2
    exit 1
fi
tar xzf "$TMP/yabridge.tar.gz" -C "$TMP"
[ -x "$TMP/yabridge/yabridgectl" ] || { echo "yabridge: the archive has no yabridgectl" >&2; exit 1; }
mkdir -p "$(dirname "$DIR")"
rm -rf "$DIR.new"
mv "$TMP/yabridge" "$DIR.new"
rm -rf "$DIR"
mv "$DIR.new" "$DIR"
echo "yabridge: installed ($DIR)"
