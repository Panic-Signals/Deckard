#!/usr/bin/env bash
# Build DECKARD for Linux: the binary, a .desktop entry and the udev rule that lets
# users reach the Maschine MK3, in a tarball; and a .deb and an .rpm that install them
# where they belong (scripts/linux/nfpm.yaml).
#
#   scripts/build-linux.sh [version]          # version defaults to the crate's
#
# Output: dist/deckard-<version>-linux-<arch>.tar.gz
#         dist/deckard_<version>-1_<amd64|arm64>.deb
#         dist/deckard-<version>-1.<x86_64|aarch64>.rpm
#
# Needs the ALSA headers and pkg-config (audio); the script says how to get them. The
# packages need nfpm (https://nfpm.goreleaser.com): on the PATH, or mise fetches it
# (`mise run release:linux` has it); without it, only the tarball is built.
set -euo pipefail

cd "$(dirname "$0")/.."
VERSION="${1:-$(sed -n 's/^version = "\(.*\)"/\1/p' apps/viewer/Cargo.toml | head -1)}"
VERSION="${VERSION#v}"
export DECKARD_VERSION="$VERSION"
ARCH="$(uname -m)"
NAME="deckard-$VERSION-linux-$ARCH"
STAGE="dist/$NAME"
PKG="dist/pkg"
NFPM_VERSION="2.48.0"

missing=()
command -v cargo >/dev/null || missing+=("Rust (https://rustup.rs)")
command -v pkg-config >/dev/null || missing+=("pkg-config")
command -v cc >/dev/null || missing+=("a C compiler")
if command -v pkg-config >/dev/null && ! pkg-config --exists alsa; then
    missing+=("the ALSA headers")
fi
if [ ${#missing[@]} -gt 0 ]; then
    echo "missing: ${missing[*]}" >&2
    cat >&2 <<'HELP'
Install them with, for example:
  Debian/Ubuntu: sudo apt install build-essential pkg-config libasound2-dev
  Fedora:        sudo dnf install gcc pkgconf-pkg-config alsa-lib-devel
  Arch:          sudo pacman -S base-devel pkgconf alsa-lib
and Rust from https://rustup.rs
HELP
    exit 1
fi

case "$ARCH" in
    x86_64) NFPM_ARCH=amd64 ;;
    aarch64) NFPM_ARCH=arm64 ;;
    *) NFPM_ARCH="$ARCH" ;;
esac

echo "building $VERSION for $ARCH"
cargo build --release --locked -p deckard

rm -rf "$STAGE" "dist/$NAME.tar.gz"
mkdir -p "$STAGE"
install -m 755 target/release/deckard "$STAGE/deckard"
install -m 644 scripts/linux/deckard.desktop "$STAGE/deckard.desktop"
# The icon (DECKARD draws it), for the desktop entry.
target/release/deckard --write-icon "$STAGE/deckard.png" 256
install -m 644 scripts/linux/50-deckard-maschine-mk3.rules "$STAGE/50-deckard-maschine-mk3.rules"
install -m 755 scripts/install-demucs.sh "$STAGE/install-demucs.sh"
install -m 755 scripts/install-yabridge.sh "$STAGE/install-yabridge.sh"
cat > "$STAGE/README.txt" <<README
DECKARD $VERSION for Linux ($ARCH)

Run ./deckard, or open a .mxprj project with it.

Stem separation (Sampling > EDIT > STEMS) needs its model (84 MB), installed once:
  ./install-demucs.sh
(DECKARD offers to install it the first time it starts, and runs it the first time
you use STEMS.)

Windows plug-ins (NI's and others without Linux versions) run through Wine and
yabridge: Preferences > Plug-ins installs both, or
  sudo apt install wine   (or: sudo dnf install wine)
  ./install-yabridge.sh

Maschine MK3: to use the controller without root, install the udev rule once:
  sudo cp 50-deckard-maschine-mk3.rules /etc/udev/rules.d/
  sudo udevadm control --reload-rules && sudo udevadm trigger
then unplug and replug the controller.

Desktop entry: copy deckard.desktop to ~/.local/share/applications/ and set its
Exec= line to where you put deckard; copy deckard.png to
~/.local/share/icons/hicolor/256x256/apps/ for its icon.

The .deb and .rpm on the releases page do all of this for you:
  Debian/Ubuntu: sudo apt install ./deckard_${VERSION}-1_$NFPM_ARCH.deb
  Fedora:        sudo dnf install ./deckard-$VERSION-1.$ARCH.rpm
README

tar -C dist -czf "dist/$NAME.tar.gz" "$NAME"
rm -rf "$STAGE"
echo "wrote dist/$NAME.tar.gz"

# The .deb and .rpm: the same files, placed by the package manager (and the icon at
# the sizes the desktop looks for).
if command -v nfpm >/dev/null; then
    NFPM=(nfpm)
elif command -v mise >/dev/null; then
    NFPM=(mise x "nfpm@$NFPM_VERSION" -- nfpm)
else
    echo "nfpm isn't installed (https://nfpm.goreleaser.com): no .deb or .rpm" >&2
    exit 0
fi
rm -rf "$PKG" dist/deckard_*.deb dist/deckard-*.rpm
mkdir -p "$PKG/icons"
install -m 755 target/release/deckard "$PKG/deckard"
for size in 48 128 256 512; do
    target/release/deckard --write-icon "$PKG/icons/$size.png" "$size"
done
sed "s|@VERSION@|$VERSION|g; s|@ARCH@|$NFPM_ARCH|g" scripts/linux/nfpm.yaml > "$PKG/nfpm.yaml"
for format in deb rpm; do
    "${NFPM[@]}" package --config "$PKG/nfpm.yaml" --packager "$format" --target dist/ >/dev/null
done
rm -rf "$PKG"
printf 'wrote %s\n' dist/deckard_*.deb dist/deckard-*.rpm
