#!/usr/bin/env bash
#
# One-command build of UnfairPlay with an `export-credential` subcommand added,
# so you can get the 16-byte FairPlay account credential that `warpsign decrypt
# --credential` consumes. Run on an Apple Silicon Mac (UnfairPlay's `setup`
# needs an installed Mac App Store app). Requires `go` and `git`.
#
#   ./build.sh                      # clones (pinned), patches, builds ./unfairplay
#   ./unfairplay export-credential  # after `sudo ./unfairplay setup ...` -> credential.bin
#
# This never hand-edits UnfairPlay's sources: it drops in two new files and
# registers the subcommand by rewriting the AddCommand() line programmatically.
set -euo pipefail

# UnfairPlay commit this addition is written against. Bump to build a newer
# version; if the registration step then fails, re-check the anchor below.
PIN="c1753f4076fceccbf3e97625ea88c0609869e978"
REPO="https://github.com/DrWhax/unfairplay.git"

cd "$(dirname "$0")"
SRC="src"

if [ ! -d "$SRC/.git" ]; then
  echo "== Cloning UnfairPlay"
  git clone "$REPO" "$SRC"
fi
echo "== Pinning UnfairPlay @ $PIN"
git -C "$SRC" fetch --quiet origin
git -C "$SRC" checkout --quiet -- .      # discard any edits from a previous run
git -C "$SRC" checkout --quiet "$PIN"

echo "== Adding export-credential"
cp patch/export_credential.go "$SRC/pkg/fairplay/export_credential.go"
cp patch/export_credential_cmd.go "$SRC/cmd/unfairplay/export_credential.go"

# Register the subcommand without hand-editing: append it to the existing
# AddCommand(...) call, anchored on the last command in it. Idempotent.
MAIN="$SRC/cmd/unfairplay/main.go"
if ! grep -q "newExportCredentialCommand" "$MAIN"; then
  perl -i -pe 's/newDoctorCommand\(\)\)/newDoctorCommand(), newExportCredentialCommand())/' "$MAIN"
fi
grep -q "newExportCredentialCommand" "$MAIN" || {
  echo "!! Could not register the subcommand (UnfairPlay's AddCommand line changed?)." >&2
  echo "   Pin a known commit or update the anchor in build.sh." >&2
  exit 1
}

echo "== Building"
( cd "$SRC" && go build -o ../unfairplay ./cmd/unfairplay )

cat <<'DONE'

Built ./unfairplay

Next, on this Mac (signed into the Apple ID that owns the apps):
  1. Install the target app from the Mac App Store and open it once.
  2. Download its encrypted IPA with the same account (e.g. ipatool).
  3. sudo ./unfairplay setup --app /Applications/App.app --ipa App.ipa
  4. ./unfairplay export-credential            # -> credential.bin (0600)

Then in warpsign:
  warpsign decrypt App.ipa App-decrypted.ipa --credential credential.bin
DONE
