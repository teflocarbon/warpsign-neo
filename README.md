# warpsign-neo

Sign iOS apps with your paid Apple Developer account, on the device itself or from any desktop OS, with provisioning handled automatically through the Developer Portal.

This is the successor to [warpsign](https://github.com/teflocarbon/warpsign). Python warpsign does the provisioning work well (dedicated App IDs per app and extension, only the capabilities an app actually uses, app groups and iCloud containers registered and linked, deterministic ID encoding), but it needs macOS for signing, and its "no Mac" path is a GitHub Actions workflow. warpsign-neo keeps that behaviour and drops the Mac.

> Status: everything except talking to the real Apple is built and tested. Inspection, ID mapping, entitlement processing, Info.plist and binary patching are ported and checked against Python warpsign's outputs. The Developer Portal client is checked request-for-request against Python's. Signing, profiles and the sign pipeline run end to end against a fake portal. Not yet tried: signing in to a real Apple ID, and installing a neo-signed app on a device. `docs/first-run.md` is the order to try those in.

## Plan

**One core, two front-ends.**

- `warpsign-core` (Rust) holds the logic:
  - Developer Portal login (SRP and 2FA on the web flow, so no anisette)
  - certificates, App IDs, capabilities, app groups, iCloud containers and profiles
  - ID mapping and entitlement filtering
  - Info.plist and Mach-O patching
  - signing
- A **CLI** built from the core, for automation on Linux, Windows and macOS. It replaces the Python CLI and the GitHub Actions workflow.
- An **iOS app** (SwiftUI, with Swift bindings to the core via uniffi). You open an IPA on the phone and get a properly provisioned, signed app.

**Signing** will go through a backend interface. rcodesign is the first candidate because it's a Rust library, so signing happens in-process. zsign (proven on iOS) is the fallback. Apple's `codesign` stays available on macOS as the reference to compare output against.

**Getting the app onto a phone** only needs a basic sign with anything. On first launch it logs in, creates its own certificate through the portal and registers the device. After that it can provision itself, and everything else.

## Not supported

warpsign-neo is for your own **paid** Apple Developer account, with provisioning done through the Developer Portal. These are deliberately out of scope:

- **Free Apple accounts.** They can't create the explicit App IDs and capabilities warpsign relies on.
- **Wildcard provisioning profiles.** Every app and extension gets its own explicit App ID.
- **Bringing your own certificate and profile** to sign with, Feather-style. Neo creates and manages them through the portal.

## Compatibility with Python warpsign

Encoded bundle IDs must stay identical between the two implementations. Otherwise an app re-signed with warpsign-neo gets a new bundle ID, which iOS treats as a different app, so updates lose their data.

Python warpsign is the reference implementation. `tools/gen_fixtures.py` runs it over a set of cases and records its exact outputs in `fixtures/`. The tests in `crates/warpsign-core/tests/python_parity.rs` require the Rust code to reproduce them. That includes a bit-for-bit port of CPython's Mersenne Twister, which the ID encoding is built on.

Each fixture file records the warpsign commit it was generated from. Regenerate them with:

```bash
python tools/gen_fixtures.py /path/to/warpsign
```

## Layout

```
crates/warpsign-core/   Shared logic (library)
crates/warpsign-cli/    The `warpsign` command
crates/warpsign-ffi/    Swift bindings for the iOS app (uniffi)
crates/uniffi-bindgen/  Generates those bindings
ios/                    SwiftUI app (XcodeGen spec + sources)
docs/design/            Agreed designs (certificates, portal login and API)
docs/spikes/            Findings from experiments (e.g. signing with rcodesign)
fixtures/               Reference outputs from Python warpsign
tools/                  Fixture generator, test data builders, iOS build, macOS A/B test for rcodesign
```

## iOS app

- **Library**: import IPAs from Files or the share sheet, or download them from sources. Shows icons, versions and sizes without unpacking, and flags IPAs that are still App Store encrypted (they install but crash).
- **Signing**: Python warpsign's options (name, icon, identifiers, device and display tweaks, debugging), remembered between signs. Signing runs as ten named steps shown live; each step expands to its log, and the log can be copied for bug reports. The result is shared from the app.
- **Sources**: AltStore-format repos, with search, screenshots, versions and downloads.
- **Settings**: sign in with your Apple ID (device or SMS two-factor codes; the session lives in the Keychain), pick a team, create the development and distribution certificates, back them up as one passphrase-protected `.p12`, restore a backup, and check certificates with Apple. Diagnostics check provisioning profiles for PPQ.

To build it on a Mac (Xcode, rustup and `brew install xcodegen`):

```bash
tools/build_ios.sh        # builds the Rust core for iOS, the Swift bindings and the Xcode project
open ios/WarpSign.xcodeproj
```

Set your bundle ID in `ios/project.yml` (`PRODUCT_BUNDLE_IDENTIFIER`) and your team in Xcode to run it on a device. The **iOS** workflow in GitHub Actions does the same build without signing; it only runs when triggered by hand, as macOS runners are expensive.

## CLI

```bash
cargo install --path crates/warpsign-cli   # installs `warpsign`

warpsign login                     # Apple ID, password, two-factor code; picks a team
warpsign identity create           # development + distribution certificates, keys made locally
warpsign identity export id.p12    # back them up (prints a generated passphrase)
warpsign sign App.ipa              # -> App-signed.ipa; same options as Python warpsign
warpsign sign App.ipa --distribution --force-original-id
warpsign inspect App.ipa
warpsign profile *.mobileprovision # PPQ check
warpsign status
warpsign device list               # the devices signed apps install on
```

Not ported yet: `--patch-liquid-glass`, `--hide-home-indicator` and `--inject-warpsign-fix`, which edit Mach-O load commands (Python warpsign used LIEF).

The session and identities live in the config directory (`~/.config/warpsign`, `~/Library/Application Support/warpsign` or `%APPDATA%\warpsign`; override with `--config-dir` or `WARPSIGN_CONFIG_DIR`), readable only by you. `identity import` takes a phone's backup, so the CLI and the app can share certificates. For automation, `WARPSIGN_P12_PASSPHRASE` supplies the import passphrase.

## Development

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```
