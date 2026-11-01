#!/usr/bin/env bash
# Build yabridge from its source (Robbert van der Helm, GPL-3.0) and install it into
# ~/.local/share/yabridge, for Wine 9.22 and later: yabridge's last release, 5.1.1,
# predates them, and through it mouse clicks and typing never reach plug-in windows
# (yabridge issues #382 and #409). Its master branch has the fix but no release yet.
# DECKARD's Preferences ▸ Plug-ins runs this; so can you:
#
#   scripts/build-yabridge.sh          build and install
#   scripts/build-yabridge.sh --check  only say what's missing to build (exit 1 if anything)
#
# What it needs, which `--check` lists as your distribution's packages: git, meson,
# ninja, pkg-config, a C++ compiler, cargo (Rust, for yabridgectl), libxcb's and dbus's
# development files, and Wine's own development files (winegcc and its headers): on
# Debian/Ubuntu/Mint `wine-stable-dev` for WineHQ's packages (or `-staging-`/
# `-devel-`), `libwine-dev` for the distribution's Wine; `wine-devel` on Fedora; Arch's
# wine has them. The source is pinned to a commit of master that was built and used
# with Wine 11.0; set YABRIDGE_COMMIT to build another. The copy that was installed
# before is kept as ~/.local/share/yabridge.release.
set -euo pipefail

COMMIT="${YABRIDGE_COMMIT:-b580a9f7fc46509767ca156d4f92872552b9e571}"
REPO="https://github.com/robbert-vdh/yabridge.git"
DIR="${XDG_DATA_HOME:-$HOME/.local/share}/yabridge"
CACHE="${XDG_CACHE_HOME:-$HOME/.cache}/deckard/yabridge-src"

# Wine from WineHQ's packages lives in /opt/wine-<variant>; its winegcc and headers
# come first, before the distribution's (older) Wine.
for v in stable staging devel; do
    if [ -d "/opt/wine-$v/bin" ]; then
        export PATH="/opt/wine-$v/bin:$PATH"
    fi
done

family() {
    local id
    # shellcheck disable=SC1091
    id=$(. /etc/os-release 2>/dev/null && echo "${ID:-} ${ID_LIKE:-}")
    case " $id " in
        *" debian "*|*" ubuntu "*|*" linuxmint "*) echo debian ;;
        *" fedora "*|*" rhel "*|*" centos "*) echo fedora ;;
        *" arch "*) echo arch ;;
        *) echo other ;;
    esac
}

# Wine's development package for this system: WineHQ's variant if one is installed.
wine_dev_package() {
    case "$(family)" in
        debian)
            for v in stable staging devel; do
                if dpkg -s "winehq-$v" >/dev/null 2>&1; then
                    echo "wine-$v-dev"
                    return
                fi
            done
            echo libwine-dev ;;
        fedora) echo wine-devel ;;
        arch) echo wine ;;
        *) echo "" ;;
    esac
}

have_wine_headers() {
    for d in /opt/wine-stable /opt/wine-staging /opt/wine-devel /usr; do
        [ -f "$d/include/wine/windows/windows.h" ] && return 0
    done
    return 1
}

# What's missing, as this distribution's package names (stdout, one per line).
missing() {
    local fam; fam=$(family)
    pkg() { # name-on-debian name-on-fedora name-on-arch
        case "$fam" in debian) echo "$1" ;; fedora) echo "$2" ;; arch) echo "$3" ;; *) echo "$1" ;; esac
    }
    # `command -v` isn't enough for the tools: a version manager's shim can be on the
    # PATH with nothing behind it, so each must answer `--version` with a number.
    answers() { "$1" --version 2>/dev/null | grep -qE '[0-9]+\.[0-9]+'; }
    answers git || pkg git git git
    answers meson || pkg meson meson meson
    answers ninja || pkg ninja-build ninja-build ninja
    answers pkg-config || pkg pkg-config pkgconf-pkg-config pkgconf
    answers g++ || pkg g++ gcc-c++ gcc
    answers cargo || pkg cargo cargo rust
    pkg-config --exists xcb 2>/dev/null || pkg libxcb1-dev libxcb-devel libxcb
    pkg-config --exists dbus-1 2>/dev/null || pkg libdbus-1-dev dbus-devel dbus
    if ! have_wine_headers || ! command -v winegcc >/dev/null; then
        local w; w=$(wine_dev_package)
        [ -n "$w" ] && echo "$w"
    fi
    true
}

if [ "${1:-}" = "--check" ]; then
    MISSING=$(missing | sort -u | tr '\n' ' ' | sed 's/ $//')
    if [ -n "$MISSING" ]; then
        echo "missing: $MISSING"
        case "$(family)" in
            debian) echo "root: export DEBIAN_FRONTEND=noninteractive; apt-get update && apt-get install -y $MISSING" ;;
            fedora) echo "root: dnf install -y $MISSING" ;;
            arch) echo "root: pacman -S --noconfirm --needed $MISSING" ;;
            *) echo "root:" ;;
        esac
        exit 1
    fi
    echo "ready"
    exit 0
fi

MISSING=$(missing | sort -u | tr '\n' ' ' | sed 's/ $//')
if [ -n "$MISSING" ]; then
    echo "yabridge: can't build without $MISSING" >&2
    exit 1
fi

# The source, at the pinned commit.
mkdir -p "$(dirname "$CACHE")"
if [ ! -d "$CACHE/.git" ]; then
    rm -rf "$CACHE"
    git clone -q "$REPO" "$CACHE"
fi
cd "$CACHE"
git fetch -q origin "$COMMIT" 2>/dev/null || git fetch -q origin
git checkout -q "$COMMIT"
echo "yabridge: building $(git rev-parse --short HEAD) (this takes a few minutes)"

# A configure that stopped before writing build.ninja is started over.
[ -f build/build.ninja ] || rm -rf build
if [ ! -d build ]; then
    meson setup build --buildtype=release --cross-file=cross-wine.conf --unity=on --unity-size=1000
fi
ninja -C build
(cd tools/yabridgectl && cargo build --release)

NEW="$DIR.new"
rm -rf "$NEW"
mkdir -p "$NEW"
for f in libyabridge-vst3.so libyabridge-vst2.so libyabridge-clap.so \
         libyabridge-chainloader-vst3.so libyabridge-chainloader-vst2.so libyabridge-chainloader-clap.so \
         yabridge-host.exe yabridge-host.exe.so; do
    [ -f "build/$f" ] && cp "build/$f" "$NEW/"
done
[ -f "$NEW/libyabridge-vst3.so" ] && [ -f "$NEW/yabridge-host.exe.so" ] || { echo "yabridge: the build made no libyabridge-vst3.so" >&2; exit 1; }
cp tools/yabridgectl/target/release/yabridgectl "$NEW/"
cp CHANGELOG.md README.md "$NEW/"
WINE_VERSION=$(wine --version 2>/dev/null || echo "wine")
echo "yabridge master $(git rev-parse HEAD) built $(date -u +%Y-%m-%d) for $WINE_VERSION by DECKARD" > "$NEW/BUILD"

if [ -d "$DIR" ] && [ ! -f "$DIR/BUILD" ]; then
    rm -rf "$DIR.release"
    mv "$DIR" "$DIR.release"
else
    rm -rf "$DIR"
fi
mv "$NEW" "$DIR"
echo "yabridge: built $(git rev-parse --short HEAD) and installed ($DIR)"
