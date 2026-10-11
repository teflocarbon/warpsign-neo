#!/bin/bash
# Build the Rust core for iOS and set up the Xcode project.
#
# Produces (all git-ignored):
#   ios/Frameworks/WarpsignFFI.xcframework   static library for device + simulator
#   ios/WarpSign/Generated/warpsign_ffi.swift Swift bindings from uniffi
#   ios/WarpSign.xcodeproj                   generated from ios/project.yml
#
# Usage: tools/build_ios.sh            (release build)
#        PROFILE=dev tools/build_ios.sh (faster debug build)
#
# Needs: macOS with Xcode, rustup, xcodegen (brew install xcodegen).
# The simulator slice is arm64 only (Apple silicon Macs).

set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
PROFILE=${PROFILE:-release}
case "$PROFILE" in
    dev) PROFILE_DIR=debug ;;
    *) PROFILE_DIR=$PROFILE ;;
esac
TARGETS=(aarch64-apple-ios aarch64-apple-ios-sim)

cd "$ROOT"
rustup target add "${TARGETS[@]}"

for target in "${TARGETS[@]}"; do
    echo "== Building warpsign-ffi for $target ($PROFILE)"
    cargo build -p warpsign-ffi --lib --profile "$PROFILE" --target "$target"
done

echo "== Generating Swift bindings"
cargo build -p warpsign-ffi --lib --profile "$PROFILE"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
cargo run -q -p uniffi-bindgen -- generate \
    --library "target/$PROFILE_DIR/libwarpsign_ffi.dylib" \
    --language swift --out-dir "$WORK/bindings"

mkdir -p ios/WarpSign/Generated
cp "$WORK/bindings/warpsign_ffi.swift" ios/WarpSign/Generated/

# The XCFramework's headers carry the C module the Swift bindings import
mkdir -p "$WORK/headers"
cp "$WORK/bindings/warpsign_ffiFFI.h" "$WORK/headers/"
cp "$WORK/bindings/warpsign_ffiFFI.modulemap" "$WORK/headers/module.modulemap"

echo "== Creating WarpsignFFI.xcframework"
rm -rf ios/Frameworks/WarpsignFFI.xcframework
mkdir -p ios/Frameworks
LIBRARIES=()
for target in "${TARGETS[@]}"; do
    LIBRARIES+=(-library "target/$target/$PROFILE_DIR/libwarpsign_ffi.a" -headers "$WORK/headers")
done
xcodebuild -create-xcframework "${LIBRARIES[@]}" -output ios/Frameworks/WarpsignFFI.xcframework

if command -v xcodegen >/dev/null; then
    echo "== Generating WarpSign.xcodeproj"
    (cd ios && xcodegen generate)
else
    echo "xcodegen not found: brew install xcodegen, then run it in ios/"
fi

echo "Done. Open ios/WarpSign.xcodeproj in Xcode."
