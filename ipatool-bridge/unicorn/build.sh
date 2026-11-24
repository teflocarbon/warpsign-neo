#!/bin/bash
# Build Unicorn, the emulator ipatool runs Apple's code in, as libunicorn.dylib
# for iOS devices and arm64 simulators, into target/unicorn/<sdk>/ (with
# MODE=interpreter, target/unicorn-interpreter/<sdk>/).
#
# Unicorn is GPLv2-only, which can't be combined with WarpSign (AGPL-3.0), so
# WarpSign doesn't ship it: the app downloads this library, signs it with
# the user's own certificate and loads it (Platform/UnicornRuntime.swift).
# See README.md here.
#
# Unicorn 2.1.4, with ipatool's iOS patch (tools/patches/unicorn-ios.patch in
# its Go module) and then one of:
#   MODE=jit (default): splitwx.patch, generated code in JIT memory, which
#     on iOS 26 and later needs StikDebug
#   MODE=interpreter: interpreter.patch, QEMU's interpreter, no JIT at all
# Only arm64 guests: ipatool's emulator runs arm64 code since 2.8.0.
#
# Needs: macOS with Xcode, Go (to fetch ipatool's patch), CMake.

set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
DEPLOYMENT_TARGET=17.0
UNICORN_VERSION=2.1.4
UNICORN_SHA256=ea8863f095a0136388694e5a6063afd9bb7650e30243dd6251af59c5ce5601f4
MODE=${MODE:-jit}
case "$MODE" in
    jit) PATCH=splitwx.patch INTERPRETER=OFF DEFAULT_WORK=$ROOT/target/unicorn ;;
    interpreter) PATCH=interpreter.patch INTERPRETER=ON DEFAULT_WORK=$ROOT/target/unicorn-interpreter ;;
    *) echo "MODE is jit or interpreter" >&2; exit 2 ;;
esac
WORK=${WORK:-$DEFAULT_WORK}
mkdir -p "$WORK"

# QEMU's configure stops without pkg-config, though Unicorn needs no
# packages: where there's none, stand in with one that finds nothing
if ! command -v pkg-config >/dev/null; then
    mkdir -p "$WORK/bin"
    printf '#!/bin/sh\nexit 1\n' > "$WORK/bin/pkg-config"
    chmod +x "$WORK/bin/pkg-config"
    export PATH="$WORK/bin:$PATH"
fi

IPATOOL_DIR=$(cd "$HERE/.." && go mod download && go list -m -f '{{.Dir}}' github.com/majd/ipatool/v2)

# Sources, patched afresh each time
SRC="$WORK/unicorn-$UNICORN_VERSION"
if [ ! -f "$WORK/unicorn.tar.gz" ]; then
    curl -fsSL --retry 3 "https://github.com/unicorn-engine/unicorn/archive/refs/tags/$UNICORN_VERSION.tar.gz" -o "$WORK/unicorn.tar.gz"
fi
echo "$UNICORN_SHA256  $WORK/unicorn.tar.gz" | shasum -a 256 -c -
rm -rf "$SRC"
tar -xzf "$WORK/unicorn.tar.gz" -C "$WORK"
patch -s -d "$SRC" -p1 < "$IPATOOL_DIR/tools/patches/unicorn-ios.patch"
patch -s -d "$SRC" -p1 < "$HERE/$PATCH"

# sdk, clang -target
build() {
    local sdk=$1 target=$2 out="$WORK/$1"
    local sysroot cc
    sysroot=$(xcrun --sdk "$sdk" --show-sdk-path)
    cc=$(xcrun --sdk "$sdk" --find clang)
    rm -rf "$out"
    mkdir -p "$out"

    echo "== Unicorn for $sdk"
    # QEMU's configure (run by Unicorn's CMake) takes the compiler and
    # target from the environment, not from CMake. No build paths in the
    # library (assert messages): they read unicorn-2.1.4/... instead.
    local cflags="-target $target -ffile-prefix-map=$WORK/="
    SDKROOT="$sysroot" CC="$cc" CFLAGS="$cflags" \
        IPHONEOS_DEPLOYMENT_TARGET=$DEPLOYMENT_TARGET \
        ARCHFLAGS='-arch arm64' cmake -S "$SRC" -B "$out/build" \
        -DCMAKE_SYSTEM_NAME=iOS -DCMAKE_OSX_SYSROOT="$sysroot" \
        -DCMAKE_OSX_ARCHITECTURES=arm64 -DCMAKE_OSX_DEPLOYMENT_TARGET=$DEPLOYMENT_TARGET \
        -DCMAKE_C_FLAGS="$cflags" -DCMAKE_BUILD_TYPE=Release \
        -DBUILD_SHARED_LIBS=ON -DUNICORN_ARCH=aarch64 -DUNICORN_INTERPRETER=$INTERPRETER -DUNICORN_BUILD_TESTS=OFF -DUNICORN_INSTALL=OFF >/dev/null
    SDKROOT="$sysroot" CC="$cc" IPHONEOS_DEPLOYMENT_TARGET=$DEPLOYMENT_TARGET \
        cmake --build "$out/build" --parallel "$(sysctl -n hw.ncpu)" >/dev/null

    # One plain file, unsigned: the app signs it with the user's certificate
    cp -L "$out/build/libunicorn.dylib" "$out/libunicorn.dylib"
    install_name_tool -id @rpath/libunicorn.dylib "$out/libunicorn.dylib"
    codesign --remove-signature "$out/libunicorn.dylib" 2>/dev/null || true
    shasum -a 256 "$out/libunicorn.dylib"
}

build iphoneos "arm64-apple-ios$DEPLOYMENT_TARGET"
build iphonesimulator "arm64-apple-ios$DEPLOYMENT_TARGET-simulator"
echo "Done: $WORK/{iphoneos,iphonesimulator}/libunicorn.dylib"
