# warpsign-neo

Sign iOS apps with your paid Apple Developer account, on the device itself or from any desktop OS, with provisioning handled automatically through the Developer Portal.

This is the successor to [warpsign](https://github.com/teflocarbon/warpsign). Python warpsign does the provisioning work well (dedicated App IDs per app and extension, only the capabilities an app actually uses, app groups and iCloud containers registered and linked, deterministic ID encoding), but it needs macOS for signing, and its "no Mac" path is a GitHub Actions workflow. warpsign-neo keeps that behaviour and drops the Mac.

> Status: early. The ID mapping and entitlement processing are ported and verified against Python warpsign. Bundles can be signed in-process on any OS with apple-codesign (not yet verified on a device, see `docs/spikes/rcodesign.md`). Nothing talks to Apple yet.

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

## Compatibility with Python warpsign

Encoded bundle IDs must stay identical between the two implementations. Otherwise an app re-signed with warpsign-neo gets a new bundle ID, which iOS treats as a different app, so updates lose their data.

Python warpsign is the reference implementation. `tools/gen_fixtures.py` runs it over a set of cases and records its exact outputs in `fixtures/`. The tests in `crates/warpsign-core/tests/python_parity.rs` require the Rust code to reproduce them. That includes a bit-for-bit port of CPython's Mersenne Twister, which the ID encoding is built on.

Regenerate the fixtures with:

```bash
python tools/gen_fixtures.py /path/to/warpsign
```

## Layout

```
crates/warpsign-core/   Shared logic (library)
docs/spikes/            Findings from experiments (e.g. signing with rcodesign)
fixtures/               Reference outputs from Python warpsign
tools/                  Fixture generator, test app builder, macOS A/B test for rcodesign
```

## Development

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```
