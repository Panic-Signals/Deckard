#!/usr/bin/env bash
# Install the stem separation model (EDIT ▸ STEMS): Meta's Demucs v4 `htdemucs` weights
# (MIT), as demucs-rs reads them, into DECKARD's settings folder's `models`. It isn't
# bundled with DECKARD; `mise install` runs this, and so can you:
#
#   scripts/install-demucs.sh
#
# The folder is the app's: $DECKARD_SETTINGS, else Application Support (macOS) or
# $XDG_CONFIG_HOME / ~/.config (Linux), then `deckard/models`. The file is checked
# against its published SHA-256 before it's put there; an intact copy is kept.
set -euo pipefail

# CI builds and tests DECKARD without it.
if [ -n "${CI:-}" ] && [ -z "${DECKARD_INSTALL_DEMUCS:-}" ]; then
    echo "stem separation model: skipped on CI (set DECKARD_INSTALL_DEMUCS=1 to install)"
    exit 0
fi

FILE="htdemucs.safetensors"
URL="https://huggingface.co/set-soft/audio_separation/resolve/main/Demucs/$FILE"
SHA256="8193504cdfb3943adaf039b8acb524a46e87ebf232c383ac7a32c80a6578423e"

if [ -n "${DECKARD_SETTINGS:-}" ]; then
    SETTINGS="$DECKARD_SETTINGS"
elif [ "$(uname -s)" = "Darwin" ]; then
    SETTINGS="$HOME/Library/Application Support/deckard"
else
    SETTINGS="${XDG_CONFIG_HOME:-$HOME/.config}/deckard"
fi
DIR="$SETTINGS/models"

sha256() {
    if command -v shasum >/dev/null; then
        shasum -a 256 "$1" | cut -d' ' -f1
    else
        sha256sum "$1" | cut -d' ' -f1
    fi
}

if [ -f "$DIR/$FILE" ] && [ "$(sha256 "$DIR/$FILE")" = "$SHA256" ]; then
    echo "stem separation model: installed ($DIR/$FILE)"
    exit 0
fi

command -v curl >/dev/null || { echo "missing: curl" >&2; exit 1; }
mkdir -p "$DIR"
TMP="$(mktemp "$DIR/.$FILE.XXXXXX")"
trap 'rm -f "$TMP"' EXIT
echo "stem separation model: downloading $FILE (84 MB)"
curl -fL --retry 3 --progress-bar -o "$TMP" "$URL"
GOT="$(sha256 "$TMP")"
if [ "$GOT" != "$SHA256" ]; then
    echo "stem separation model: the download isn't the expected file (SHA-256 $GOT)" >&2
    exit 1
fi
chmod 644 "$TMP"
mv "$TMP" "$DIR/$FILE"
trap - EXIT
echo "stem separation model: installed ($DIR/$FILE)"
