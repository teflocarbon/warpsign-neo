#!/bin/bash
# Rebuild the synthetic provisioning profiles used by warpsign-core's tests.
#
# Real profiles are CMS-signed plists from Apple. These are signed with the
# throwaway test certificate instead, which is enough for parsing tests.
#
# Needs: openssl, python3.

set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
DATA="$ROOT/crates/warpsign-core/tests/data"
OUT="$DATA/profiles"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

openssl pkcs12 -legacy -in "$DATA/test-cert.p12" -passin pass:secret -nokeys -out "$WORK/cert.pem" 2>/dev/null
openssl pkcs12 -legacy -in "$DATA/test-cert.p12" -passin pass:secret -nocerts -nodes -out "$WORK/key.pem" 2>/dev/null

python3 - "$WORK" <<'PY'
import plistlib, sys
work = sys.argv[1]
base = {
    "AppIDName": "WS App",
    "ApplicationIdentifierPrefix": ["ZZZZZ99999"],
    "TeamIdentifier": ["ZZZZZ99999"],
    "TeamName": "Test User",
    "Platform": ["iOS"],
    "ProvisionedDevices": ["00008110-000000000000001E", "00008030-000000000000002E"],
    "Entitlements": {
        "application-identifier": "ZZZZZ99999.com.example.test",
        "com.apple.developer.team-identifier": "ZZZZZ99999",
        "get-task-allow": True,
        "keychain-access-groups": ["ZZZZZ99999.*"],
    },
    "Version": 1,
}
profiles = {
    "ppq": {**base, "Name": "WS ppq Development", "UUID": "11111111-1111-1111-1111-111111111111", "PPQCheck": True},
    "no-ppq": {**base, "Name": "WS no-ppq Development", "UUID": "22222222-2222-2222-2222-222222222222", "PPQCheck": False},
    "legacy": {**base, "Name": "WS legacy Development", "UUID": "33333333-3333-3333-3333-333333333333"},
}
for name, profile in profiles.items():
    with open(f"{work}/{name}.plist", "wb") as f:
        plistlib.dump(profile, f)
PY

mkdir -p "$OUT"
for name in ppq no-ppq legacy; do
    openssl cms -sign -nodetach -binary -outform DER \
        -signer "$WORK/cert.pem" -inkey "$WORK/key.pem" \
        -in "$WORK/$name.plist" -out "$OUT/$name.mobileprovision"
done
echo "Rebuilt profiles in $OUT"
