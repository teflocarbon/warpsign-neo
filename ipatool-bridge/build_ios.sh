#!/bin/bash
# Build ipatool-bridge as ios/Frameworks/IpatoolBridge.xcframework (device
# and arm64 simulator): ipatool's App Store client, as a Go c-archive.
#
# Without Unicorn, the emulator ipatool needs to sign in and download:
# Unicorn is GPLv2-only, so WarpSign loads it at run time instead
# (unicorn/README.md). ipatool finds it with dlsym(RTLD_DEFAULT), as it finds
# a statically linked one. The Rust core finds the bridge the same way, so
# the app links it with -force_load and -export_dynamic (ios/project.yml).
#
# Needs: macOS with Xcode and Go.

set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
DEPLOYMENT_TARGET=17.0
WORK=${WORK:-$ROOT/target/ipatool-bridge}
mkdir -p "$WORK"

cd "$HERE"
go mod download

# sdk, clang -target
build() {
    local sdk=$1 target=$2 out="$WORK/$1"
    local sysroot cc
    sysroot=$(xcrun --sdk "$sdk" --show-sdk-path)
    cc=$(xcrun --sdk "$sdk" --find clang)
    rm -rf "$out"
    mkdir -p "$out"

    echo "== ipatool-bridge for $sdk"
    GOOS=ios GOARCH=arm64 CGO_ENABLED=1 CC="$cc" \
        CGO_CFLAGS="-isysroot $sysroot -target $target" \
        CGO_LDFLAGS="-isysroot $sysroot -target $target" \
        go build -trimpath -buildmode=c-archive -o "$out/libIpatoolBridge.a" .
}

build iphoneos "arm64-apple-ios$DEPLOYMENT_TARGET"
# DEVICE_ONLY=1 (tools/build_ios.sh): no simulator slice
SLICES=(-library "$WORK/iphoneos/libIpatoolBridge.a" -headers "$WORK/headers")
if [ "${DEVICE_ONLY:-0}" != 1 ]; then
    build iphonesimulator "arm64-apple-ios$DEPLOYMENT_TARGET-simulator"
    SLICES+=(-library "$WORK/iphonesimulator/libIpatoolBridge.a" -headers "$WORK/headers")
fi

# The C API, without Go's generated header (its typedefs clash with Swift's)
mkdir -p "$WORK/headers"
cat > "$WORK/headers/ipatool_bridge.h" <<'H'
// JSON in, JSON out: see ipatool-bridge/bridge.go. Free replies with
// warpsign_ipatool_free.
char *warpsign_ipatool_call(const char *request);
void warpsign_ipatool_free(char *reply);
H

rm -rf "$ROOT/ios/Frameworks/IpatoolBridge.xcframework"
mkdir -p "$ROOT/ios/Frameworks"
xcodebuild -create-xcframework "${SLICES[@]}" \
    -output "$ROOT/ios/Frameworks/IpatoolBridge.xcframework"
echo "Done: ios/Frameworks/IpatoolBridge.xcframework"
