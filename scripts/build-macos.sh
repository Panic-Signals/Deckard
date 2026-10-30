#!/usr/bin/env bash
# Build DECKARD for macOS: a universal (Apple Silicon + Intel) DECKARD.app, zipped.
#
#   scripts/build-macos.sh [version]          # version defaults to the crate's
#
# Output: dist/DECKARD-<version>-macos-universal.zip
#
# Signing (optional; without it the app is signed ad hoc and runs on this Mac):
#   MACOS_SIGN_IDENTITY   a "Developer ID Application: …" identity in the keychain
# Notarizing (optional, needs the identity):
#   APPLE_ID, APPLE_TEAM_ID, APPLE_APP_PASSWORD (an app-specific password)
set -euo pipefail

cd "$(dirname "$0")/.."
VERSION="${1:-$(sed -n 's/^version = "\(.*\)"/\1/p' apps/viewer/Cargo.toml | head -1)}"
VERSION="${VERSION#v}"
export DECKARD_VERSION="$VERSION"
APP="dist/DECKARD.app"
OUT="dist/DECKARD-$VERSION-macos-universal.zip"

need() { command -v "$1" >/dev/null || { echo "missing: $1 ($2)" >&2; exit 1; }; }
need cargo "install Rust from https://rustup.rs"
need lipo "install the Xcode command line tools: xcode-select --install"

# Both architectures. With rustup, its toolchain builds them (a Homebrew Rust earlier
# in PATH can't add targets).
if command -v rustup >/dev/null; then
    rustup target add --toolchain stable aarch64-apple-darwin x86_64-apple-darwin >/dev/null
    RUSTC="$(rustup which rustc --toolchain stable)"
    export RUSTC
    cargo() { rustup run stable cargo "$@"; }
fi
for target in aarch64-apple-darwin x86_64-apple-darwin; do
    echo "building $VERSION for $target"
    MACOSX_DEPLOYMENT_TARGET=12.0 cargo build --release --locked -p deckard --target "$target"
done

rm -rf "$APP" "$OUT"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create -output "$APP/Contents/MacOS/DECKARD" \
    target/aarch64-apple-darwin/release/deckard target/x86_64-apple-darwin/release/deckard
# The icon: DECKARD draws it (the television of its MK3 goodbye screens), at each size
# macOS asks for, made into AppIcon.icns.
need iconutil "install the Xcode command line tools: xcode-select --install"
HOST_BIN="target/$( [ "$(uname -m)" = arm64 ] && echo aarch64 || echo x86_64 )-apple-darwin/release/deckard"
ICONSET="$(mktemp -d -t deckard-icon)/AppIcon.iconset"
mkdir -p "$ICONSET"
for px in 16 32 128 256 512; do
    "$HOST_BIN" --write-icon "$ICONSET/icon_${px}x${px}.png" "$px"
    "$HOST_BIN" --write-icon "$ICONSET/icon_${px}x${px}@2x.png" "$((px * 2))"
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"
rm -rf "$(dirname "$ICONSET")"
# The stem separation model's installer: DECKARD runs it the first time STEMS is used.
install -m 755 scripts/install-demucs.sh "$APP/Contents/Resources/install-demucs.sh"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>DECKARD</string>
    <key>CFBundleDisplayName</key><string>DECKARD</string>
    <key>CFBundleIdentifier</key><string>io.github.summitt.deckard</string>
    <key>CFBundleExecutable</key><string>DECKARD</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundleVersion</key><string>$VERSION</string>
    <key>LSMinimumSystemVersion</key><string>12.0</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSMicrophoneUsageDescription</key><string>DECKARD records audio from your inputs when you sample.</string>
    <key>CFBundleDocumentTypes</key>
    <array>
        <dict>
            <key>CFBundleTypeName</key><string>Maschine Project</string>
            <key>CFBundleTypeExtensions</key><array><string>mxprj</string></array>
            <key>CFBundleTypeRole</key><string>Editor</string>
        </dict>
    </array>
</dict>
</plist>
PLIST

# Plug-ins are other vendors' code: the hardened runtime must let them load, and audio
# input needs its entitlement.
ENTITLEMENTS="$(mktemp -t deckard-entitlements).plist"
cat > "$ENTITLEMENTS" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>com.apple.security.cs.disable-library-validation</key><true/>
    <key>com.apple.security.cs.allow-jit</key><true/>
    <key>com.apple.security.device.audio-input</key><true/>
    <key>com.apple.security.device.usb</key><true/>
</dict>
</plist>
PLIST

if [ -n "${MACOS_SIGN_IDENTITY:-}" ]; then
    echo "signing as $MACOS_SIGN_IDENTITY"
    codesign --force --options runtime --timestamp --entitlements "$ENTITLEMENTS" \
        --sign "$MACOS_SIGN_IDENTITY" "$APP"
else
    echo "signing ad hoc (set MACOS_SIGN_IDENTITY for a Developer ID)"
    codesign --force --sign - --entitlements "$ENTITLEMENTS" "$APP"
fi
rm -f "$ENTITLEMENTS"

ditto -c -k --keepParent "$APP" "$OUT"

if [ -n "${MACOS_SIGN_IDENTITY:-}" ] && [ -n "${APPLE_ID:-}" ]; then
    echo "notarizing"
    xcrun notarytool submit "$OUT" --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" \
        --password "$APPLE_APP_PASSWORD" --wait
    xcrun stapler staple "$APP"
    rm -f "$OUT"
    ditto -c -k --keepParent "$APP" "$OUT"
fi

echo "wrote $OUT"
