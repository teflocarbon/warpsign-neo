#!/bin/bash
# Rebuild the synthetic iOS app used by warpsign-core's signing tests.
#
# The binaries are real arm64 iOS Mach-O files built with clang and lld (no
# Apple SDK needed). They are committed, so tests don't need a cross-compiler;
# rerun this only to change them. Also regenerates the throwaway self-signed
# test certificate.
#
# Needs: clang and lld (Ubuntu: apt install clang lld), openssl.

set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
DATA="$ROOT/crates/warpsign-core/tests/data"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

# clang's Darwin driver looks for ld64.lld
mkdir -p "$WORK/bin"
ln -s "$(command -v lld)" "$WORK/bin/ld64.lld"
export PATH="$WORK/bin:$PATH"
TARGET=(-target arm64-apple-ios15.0 -nostdlib -fuse-ld=lld -O1)

cd "$WORK"
echo 'int helper(void) { return 7; }' > foo.c
echo 'int helper(void); int main(void) { return helper(); }' > main.c
echo 'int main(void) { return 42; }' > widget.c
clang "${TARGET[@]}" -dynamiclib -install_name @rpath/Foo.framework/Foo foo.c -o Foo
clang "${TARGET[@]}" main.c Foo -Wl,-e,_main -Wl,-fixup_chains -Wl,-rpath,@executable_path/Frameworks -o Test
clang "${TARGET[@]}" widget.c -Wl,-e,_main -o Widget

APP="$DATA/Test.app"
rm -rf "$APP"
mkdir -p "$APP/Frameworks/Foo.framework" "$APP/PlugIns/Widget.appex" "$APP/en.lproj"
cp Test "$APP/Test"
cp Foo "$APP/Frameworks/Foo.framework/Foo"
cp Widget "$APP/PlugIns/Widget.appex/Widget"

python3 - "$APP" <<'PY'
import plistlib, sys
app = sys.argv[1]
def info(path, bundle_id, executable, package_type):
    with open(f"{app}/{path}/Info.plist", "wb") as f:
        plistlib.dump({
            "CFBundleIdentifier": bundle_id,
            "CFBundleExecutable": executable,
            "CFBundlePackageType": package_type,
            "CFBundleName": executable,
            "CFBundleVersion": "1",
            "CFBundleShortVersionString": "1.0",
            "MinimumOSVersion": "15.0",
            "CFBundleSupportedPlatforms": ["iPhoneOS"],
        }, f)
info(".", "com.example.test", "Test", "APPL")
info("Frameworks/Foo.framework", "com.example.foo", "Foo", "FMWK")
info("PlugIns/Widget.appex", "com.example.test.widget", "Widget", "XPC!")
PY
printf 'placeholder profile for main app' > "$APP/embedded.mobileprovision"
printf 'placeholder profile for widget' > "$APP/PlugIns/Widget.appex/embedded.mobileprovision"
printf '"Hello" = "Hello";' > "$APP/en.lproj/Localizable.strings"

# Throwaway self-signed certificate shaped like an Apple Development one.
# Legacy .p12 encryption, as exported by Keychain Access.
openssl req -x509 -newkey rsa:2048 -nodes -keyout key.pem -out cert.pem -days 36500 \
    -subj "/UID=ABCDE12345/CN=Apple Development: Test User (ABCDE12345)/OU=ZZZZZ99999/O=Test User/C=US" \
    -addext "keyUsage=critical,digitalSignature" -addext "extendedKeyUsage=critical,codeSigning" 2>/dev/null
openssl pkcs12 -export -legacy -inkey key.pem -in cert.pem -out "$DATA/test-cert.p12" -passout pass:secret

echo "Rebuilt $APP and $DATA/test-cert.p12"
