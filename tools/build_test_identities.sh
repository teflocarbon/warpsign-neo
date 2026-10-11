#!/bin/bash
# Build extra signing identities for warpsign-core's identity tests:
# a self-signed "Apple Distribution" certificate exported as a modern
# (AES) .p12, complementing the legacy development test-cert.p12.
#
# Needs: openssl 3.

set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
DATA="$ROOT/crates/warpsign-core/tests/data"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

openssl req -x509 -newkey rsa:2048 -nodes -keyout "$WORK/key.pem" -out "$WORK/cert.pem" -days 36500 \
    -subj "/UID=ABCDE12345/CN=Apple Distribution: Test User (ZZZZZ99999)/OU=ZZZZZ99999/O=Test User/C=US" \
    -addext "keyUsage=critical,digitalSignature" -addext "extendedKeyUsage=critical,codeSigning" 2>/dev/null
# OpenSSL 3's default: PBES2 with AES-256, which apple-codesign alone can't read
openssl pkcs12 -export -inkey "$WORK/key.pem" -in "$WORK/cert.pem" \
    -out "$DATA/test-distribution-aes.p12" -passout pass:secret

echo "Rebuilt $DATA/test-distribution-aes.p12"
