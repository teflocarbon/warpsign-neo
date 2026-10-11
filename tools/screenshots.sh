#!/bin/bash
# Screenshot every main screen of the app in the iOS simulator, light and
# dark, using its demo mode (sample data, see ios/WarpSign/Platform/Demo.swift).
#
# Run after tools/build_ios.sh. Writes PNGs to $OUT (default: screenshots/).
# Needs: macOS with Xcode and an iOS simulator runtime.

set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
OUT=${OUT:-$ROOT/screenshots}
BUNDLE_ID=com.example.warpsign
SCREENS=(library appDetail signOptions signing install sources sourceDetail settings installing signIn)

echo "== Building for the simulator"
xcodebuild -project "$ROOT/ios/WarpSign.xcodeproj" -scheme WarpSign -configuration Debug \
    -destination 'generic/platform=iOS Simulator' -derivedDataPath "$ROOT/ios/build" \
    CODE_SIGNING_ALLOWED=NO build -quiet
APP="$ROOT/ios/build/Build/Products/Debug-iphonesimulator/WarpSign.app"

echo "== Booting a simulator"
RUNTIME=$(xcrun simctl list runtimes iOS -j | python3 -c "import json,sys; r=[x for x in json.load(sys.stdin)['runtimes'] if x['isAvailable']]; print(r[-1]['identifier'])")
DEVICE_TYPE=$(xcrun simctl list devicetypes -j | python3 -c "
import json, sys
names = [d['identifier'] for d in json.load(sys.stdin)['devicetypes'] if d['name'].startswith('iPhone') and 'Pro' in d['name'] and 'Max' not in d['name']]
print(names[-1])")
echo "$RUNTIME on $DEVICE_TYPE"
UDID=$(xcrun simctl create warpsign-shots "$DEVICE_TYPE" "$RUNTIME")
trap 'xcrun simctl shutdown "$UDID" >/dev/null 2>&1; xcrun simctl delete "$UDID" >/dev/null 2>&1' EXIT
xcrun simctl boot "$UDID"
xcrun simctl bootstatus "$UDID" -b >/dev/null
xcrun simctl status_bar "$UDID" override --time 9:41 --batteryState charged --batteryLevel 100 \
    --cellularMode active --cellularBars 4 --wifiBars 3
xcrun simctl install "$UDID" "$APP"

mkdir -p "$OUT"
for appearance in light dark; do
    xcrun simctl ui "$UDID" appearance "$appearance"
    for screen in "${SCREENS[@]}"; do
        xcrun simctl launch --terminate-running-process "$UDID" "$BUNDLE_ID" -demo -demoScreen "$screen" >/dev/null
        sleep 4
        xcrun simctl io "$UDID" screenshot --type=png "$OUT/$screen-$appearance.png" >/dev/null 2>&1
        echo "  $screen ($appearance)"
    done
done

# Contact sheets, to look at everything at once
python3 - "$OUT" "${SCREENS[@]}" <<'PY' || echo "(no contact sheets: needs Pillow)"
import sys
from pathlib import Path
from PIL import Image
out, screens = Path(sys.argv[1]), sys.argv[2:]
for appearance in ("light", "dark"):
    shots = [Image.open(out / f"{s}-{appearance}.png") for s in screens]
    w, h = shots[0].size
    scale = 0.4
    tw, th = int(w * scale), int(h * scale)
    cols = 5
    rows = (len(shots) + cols - 1) // cols
    sheet = Image.new("RGB", (cols * tw + (cols + 1) * 20, rows * th + (rows + 1) * 20), (128, 128, 128))
    for i, shot in enumerate(shots):
        r, c = divmod(i, cols)
        sheet.paste(shot.resize((tw, th), Image.LANCZOS).convert("RGB"), (20 + c * (tw + 20), 20 + r * (th + 20)))
    sheet.save(out / f"contact-{appearance}.png")
PY
echo "Wrote screenshots to $OUT"
