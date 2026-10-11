#!/bin/bash
# Build a throwaway pairing file (jitterbug/SideStore format) for
# warpsign-core's device tests: fake certificates, a made-up UDID. It pairs
# with nothing.
#
# Needs: openssl, python3.

set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
OUT="$ROOT/crates/warpsign-core/tests/data/test.mobiledevicepairing"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

for name in root host device; do
    openssl req -x509 -newkey rsa:2048 -nodes -keyout "$WORK/$name.key" -out "$WORK/$name.crt" \
        -days 36500 -subj "/CN=WarpSign test $name" 2>/dev/null
done

python3 - "$WORK" "$OUT" <<'PY'
import plistlib, sys
work, out = sys.argv[1], sys.argv[2]
read = lambda name: open(f"{work}/{name}", "rb").read()
with open(out, "wb") as f:
    plistlib.dump({
        "DeviceCertificate": read("device.crt"),
        "HostCertificate": read("host.crt"),
        "HostPrivateKey": read("host.key"),
        "RootCertificate": read("root.crt"),
        "RootPrivateKey": read("root.key"),
        "SystemBUID": "00000000-0000-0000-0000-000000000000",
        "HostID": "11111111-2222-4333-8444-555555555555",
        "EscrowBag": b"\0" * 32,
        "WiFiMACAddress": "00:00:00:00:00:00",
        "UDID": "00008110-000A0B0C0D0E0F10",
    }, f)
PY
echo "Rebuilt $OUT"
