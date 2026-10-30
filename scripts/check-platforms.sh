#!/usr/bin/env bash
# Type-check the workspace for Linux and Windows from this machine (compile, no link),
# to catch platform breakage before CI does. Uses rustup's toolchain and zig as the C
# compiler (Lua and other C code); Windows is checked through its GNU target, as the
# MSVC headers can't be had here (CI builds the MSVC version).
#
#   scripts/check-platforms.sh [linux] [windows]     # default: both
#
# Needs rustup and zig (brew install zig).
set -euo pipefail

cd "$(dirname "$0")/.."
command -v rustup >/dev/null || { echo "missing: rustup" >&2; exit 1; }
command -v zig >/dev/null || { echo "missing: zig (brew install zig)" >&2; exit 1; }

platforms=("$@")
[ ${#platforms[@]} -eq 0 ] && platforms=(linux windows)

tools="$(mktemp -d "${TMPDIR:-/tmp}/deckard-cross.XXXXXX")"
trap 'rm -rf "$tools"' EXIT
# A stand-in pkg-config entry for ALSA: checking compiles but doesn't link.
mkdir -p "$tools/pc"
printf 'Name: alsa\nDescription: stand-in for checks\nVersion: 1.2.0\nLibs: -lasound\nCflags:\n' > "$tools/pc/alsa.pc"

RUSTC="$(rustup which rustc --toolchain stable)"
export RUSTC
export PKG_CONFIG_ALLOW_CROSS=1 PKG_CONFIG_PATH="$tools/pc" PKG_CONFIG_LIBDIR="$tools/pc"
status=0
for platform in "${platforms[@]}"; do
    case "$platform" in
        linux) target=x86_64-unknown-linux-gnu zig_target=x86_64-linux-gnu ;;
        windows) target=x86_64-pc-windows-gnu zig_target=x86_64-windows-gnu ;;
        *) echo "unknown platform: $platform" >&2; exit 2 ;;
    esac
    rustup target add --toolchain stable "$target" >/dev/null
    # cc wants one executable per tool; its own --target (Rust's triple) is dropped for
    # zig's.
    for tool in cc c++; do
        cat > "$tools/$tool-$target" <<WRAP
#!/bin/sh
for a in "\$@"; do
    shift
    case "\$a" in --target=*) ;; *) set -- "\$@" "\$a" ;; esac
done
exec zig $tool -target $zig_target "\$@"
WRAP
    done
    mv "$tools/c++-$target" "$tools/cxx-$target"
    printf '#!/bin/sh\nexec zig ar "$@"\n' > "$tools/ar-$target"
    chmod +x "$tools"/*-"$target"
    var="${target//-/_}"
    export "CC_$var=$tools/cc-$target" "CXX_$var=$tools/cxx-$target" "AR_$var=$tools/ar-$target"
    echo "checking $platform ($target)"
    if ! rustup run stable cargo check --workspace --all-targets --target "$target" --target-dir target/check-platforms; then
        status=1
    fi
done
exit $status
