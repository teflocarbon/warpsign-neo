"""Write WarpSign's own AltStore-format source, for the latest release.

    python3 tools/warpsign_source.py WarpSign.ipa notes.txt source.json

Run by .github/workflows/ipa.yml after building. The version, build,
bundle ID and minimum iOS come from the IPA's Info.plist, so the source
always matches the IPA it points at; notes.txt holds the release notes
(one change per line). The download URL is the latest release's
WarpSign.ipa, replaced by every build, so the source lists one version.
"""
import datetime
import json
import os
import plistlib
import re
import sys
import zipfile

ipa, notes_file, output = sys.argv[1:4]
repo = os.environ.get("GITHUB_REPOSITORY", "teflocarbon/warpsign-neo")
sha = os.environ.get("GITHUB_SHA", "main")

with zipfile.ZipFile(ipa) as archive:
    name = next(n for n in archive.namelist() if re.fullmatch(r"Payload/[^/]+\.app/Info\.plist", n))
    info = plistlib.loads(archive.read(name))

with open(notes_file, encoding="utf-8") as f:
    lines = [line.strip() for line in f if line.strip()]
notes = "\n".join(f"- {line}" for line in lines) or "Build from main."

icon = f"https://raw.githubusercontent.com/{repo}/{sha}/ios/WarpSign/Assets.xcassets/AppIcon.appiconset/AppIcon.png"
version = {
    "version": info["CFBundleShortVersionString"],
    "buildVersion": info["CFBundleVersion"],
    "date": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "localizedDescription": notes,
    "downloadURL": f"https://github.com/{repo}/releases/download/latest/WarpSign.ipa",
    "size": os.path.getsize(ipa),
}
if "MinimumOSVersion" in info:
    version["minOSVersion"] = info["MinimumOSVersion"]

source = {
    "name": "WarpSign",
    "identifier": f"io.github.{repo.replace('/', '.')}",
    "subtitle": "WarpSign's own builds",
    "description": "Every build of WarpSign from GitHub, unsigned. WarpSign signs it, and WarpSign Companion installs it.",
    "iconURL": icon,
    "website": f"https://github.com/{repo}",
    "tintColor": "#5B3FD6",
    "featuredApps": [info["CFBundleIdentifier"]],
    "apps": [
        {
            "name": "WarpSign",
            "bundleIdentifier": info["CFBundleIdentifier"],
            "developerName": repo.split("/")[0],
            "subtitle": "Sign apps with your own developer account",
            "localizedDescription": "Signs apps on your iPhone with your own Apple Developer account, installs them, and keeps them updated.",
            "iconURL": icon,
            "tintColor": "#5B3FD6",
            "category": "developer",
            "versions": [version],
        }
    ],
}
with open(output, "w", encoding="utf-8") as f:
    f.write(json.dumps(source, indent=2, ensure_ascii=False) + "\n")
print(f"{output}: WarpSign {version['version']} ({version['buildVersion']})")
