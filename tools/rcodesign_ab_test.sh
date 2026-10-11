#!/bin/bash
# A/B test rcodesign against Apple's codesign on macOS.
#
# Takes an IPA already signed by Python warpsign (so profiles, entitlements and
# patches are all in place), re-signs it with rcodesign using the same
# certificate and the entitlements already in each binary, then checks the
# result with Apple's codesign and diffs it against the original. The signer is
# the only variable. Install the output on a device to finish the test.
#
# Usage:
#   tools/rcodesign_ab_test.sh <warpsign-signed.ipa> <cert.p12> <cert_pass.txt>
#
# Needs: macOS, rcodesign (cargo install --locked apple-codesign).
# The .p12 must use legacy encryption (Keychain Access exports do); rcodesign
# 0.29 cannot read OpenSSL 3's default AES-encrypted .p12 files.

set -euo pipefail

if [ $# -ne 3 ]; then
    sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'
    exit 1
fi

IPA=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
P12=$(cd "$(dirname "$2")" && pwd)/$(basename "$2")
PASS_FILE=$(cd "$(dirname "$3")" && pwd)/$(basename "$3")
OUT_IPA="${IPA%.ipa}-rcodesign.ipa"

command -v rcodesign >/dev/null || { echo "rcodesign not found: cargo install --locked apple-codesign"; exit 1; }

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$WORK/original" "$WORK/resigned" "$WORK/entitlements"

ditto -x -k "$IPA" "$WORK/original"
ditto -x -k "$IPA" "$WORK/resigned"
APP=$(find "$WORK/resigned/Payload" -maxdepth 1 -name "*.app" | head -n 1)
ORIG_APP="$WORK/original/Payload/$(basename "$APP")"

# The main app ("") plus signable nested bundles, deepest first, relative to the .app
BUNDLES=("")
while IFS= read -r rel; do
    BUNDLES+=("$rel")
done < <(cd "$APP" && find . -mindepth 1 -type d \( -name "*.app" -o -name "*.appex" -o -name "*.framework" \) \
    | sed 's|^\./||' | awk -F/ '{print NF "\t" $0}' | sort -rn | cut -f2-)

bundle_executable() {
    /usr/libexec/PlistBuddy -c "Print :CFBundleExecutable" "$1/Info.plist" 2>/dev/null || true
}

TEAM_ID=$(codesign -dv "$ORIG_APP" 2>&1 | sed -n 's/^TeamIdentifier=//p')
echo "Team ID from original signature: $TEAM_ID"

ARGS=(sign --p12-file "$P12" --p12-password-file "$PASS_FILE" --timestamp-url none --team-name "$TEAM_ID")

# Entitlements of the main app (unscoped) and of each nested bundle (scoped by path)
for rel in "${BUNDLES[@]}"; do
    dir="$APP${rel:+/$rel}"
    exe=$(bundle_executable "$dir")
    [ -n "$exe" ] || continue
    chmod 755 "$dir/$exe"  # zip extraction can drop the exec bit; rcodesign keeps whatever it finds
    ents="$WORK/entitlements/$(echo "${rel:-main}" | tr '/ ' '__').plist"
    if codesign -d --entitlements - --xml "$dir" > "$ents" 2>/dev/null && [ -s "$ents" ]; then
        if [ -z "$rel" ]; then
            ARGS+=(--entitlements-xml-file "$ents")
        else
            ARGS+=(--entitlements-xml-file "$rel:$ents")
        fi
    fi
done

echo
echo "== rcodesign ${ARGS[*]} <app>"
rcodesign "${ARGS[@]}" "$APP"

echo
echo "== codesign --verify --deep --strict"
if codesign --verify --deep --strict --verbose=2 "$APP"; then
    echo "PASS: Apple's codesign accepts the rcodesign signature"
else
    echo "FAIL: Apple's codesign rejects the rcodesign signature"
fi

compare() {
    local rel=$1
    local orig="$ORIG_APP${rel:+/$rel}" new="$APP${rel:+/$rel}"
    local a b
    a=$(codesign -d --entitlements - --xml "$orig" 2>/dev/null | plutil -convert xml1 -o - - 2>/dev/null || true)
    b=$(codesign -d --entitlements - --xml "$new" 2>/dev/null | plutil -convert xml1 -o - - 2>/dev/null || true)
    if [ "$a" == "$b" ]; then echo "  entitlements: identical"; else echo "  entitlements: DIFFER"; diff <(echo "$a") <(echo "$b") | sed 's/^/    /'; fi

    a=$(codesign -dvvv "$orig" 2>&1 | grep -E "^(Identifier|Format|CodeDirectory|TeamIdentifier|Runtime|Sealed Resources)" | sed 's/hashes=.*//' || true)
    b=$(codesign -dvvv "$new" 2>&1 | grep -E "^(Identifier|Format|CodeDirectory|TeamIdentifier|Runtime|Sealed Resources)" | sed 's/hashes=.*//' || true)
    if [ "$a" == "$b" ]; then echo "  signature details: identical"; else echo "  signature details: DIFFER"; diff <(echo "$a") <(echo "$b") | sed 's/^/    /'; fi

    a=$(codesign -d -r- "$orig" 2>&1 | grep designated || true)
    b=$(codesign -d -r- "$new" 2>&1 | grep designated || true)
    if [ "$a" == "$b" ]; then echo "  designated requirement: identical"; else echo "  designated requirement: DIFFER"; echo "    codesign:  $a"; echo "    rcodesign: $b"; fi
}

echo
echo "== Comparing against the original codesign signature"
for rel in "${BUNDLES[@]}"; do
    echo "[${rel:-main app}]"
    compare "$rel"
done

(cd "$WORK/resigned" && ditto -c -k --sequesterRsrc --keepParent Payload "$OUT_IPA")
echo
echo "Wrote $OUT_IPA - install it on a device and launch it (and its extensions) to finish the test."
