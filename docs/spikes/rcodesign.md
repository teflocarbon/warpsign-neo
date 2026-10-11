# Spike: signing iOS apps with rcodesign

**Question:** can rcodesign (apple-codesign) replace Apple's `codesign` for warpsign, including different entitlements per extension, without a Mac?

**Status:** the signing works on Linux and the output looks right when inspected. It still needs `codesign --verify` on a Mac and a real install on a device (see "Still to prove").

## Setup

- apple-codesign 0.29.0 built from crates.io (`cargo install --locked apple-codesign`), on Linux.
- A synthetic app built with clang and lld for `arm64-apple-ios15.0`:
  - `Test.app`: main executable, linked against the framework below
  - `Frameworks/Foo.framework`
  - `PlugIns/Widget.appex`, with its own `embedded.mobileprovision`
  - resources: an icon and `en.lproj/Localizable.strings`
- A self-signed certificate shaped like an Apple Development certificate (`CN=Apple Development: …`, `OU=<team ID>`).

Signing command:

```bash
rcodesign sign \
  --p12-file cert.p12 --p12-password-file pass.txt \
  --timestamp-url none --team-name ZZZZZ99999 \
  --entitlements-xml-file main.plist \
  --entitlements-xml-file PlugIns/Widget.appex:widget.plist \
  Test.app
```

## Findings

**What works**

- One call signs the whole bundle. rcodesign signs nested bundles itself, deepest first (framework, then widget, then app).
- Per-component entitlements use scoped settings, `<path relative to the bundle>:<file>`. An unscoped value applies to the main app.
- Entitlements never pass down to nested code. Nested bundles and Mach-Os only inherit digest and runtime-version settings (`ScopedSetting::inherit_nested_bundle()` in `signing_settings.rs`).
- What ended up in each signature:
  - App and widget: exactly their own entitlements, in both XML and DER form. iOS 15+ checks the DER form.
  - Framework: no entitlement slots at all.
  - Identifiers come from each `CFBundleIdentifier`.
  - Main executables are flagged `MAIN_BINARY`, plus `ALLOW_UNSIGNED` from `get-task-allow`.
  - Hashes are SHA-256.
- `CodeResources` seals both profiles, the localisation, the icon and the nested bundles' files. It uses Apple's iOS rule set (no "nested code" rule), so nested bundles are sealed file by file and there's a v1 `files` dict, which is the layout I expect `codesign` to produce for iOS.

**Gotchas warpsign-neo must handle**

1. **Exec bits are not fixed.** Executables that arrive without `+x` (zip extraction drops it) stay that way. nab138's isideload fork patches the signer to force 755. Neo should set 755 on executables itself.
2. **Team ID is only taken from Apple-chained certificates.** rcodesign copies the team ID from the certificate's OU only when the certificate chains to Apple's root CA. Without a team ID, dyld refuses to load frameworks on iOS ("mapped file has no Team ID"). Real Apple certificates are fine, but neo should always pass the team ID explicitly.
3. **Timestamping is on by default.** It contacts `timestamp.apple.com`, which `codesign` doesn't do for development or ad hoc signing. Use `--timestamp-url none`.
4. **Modern `.p12` files fail.** OpenSSL 3's default AES/PBKDF2 encryption fails with a misleading "incorrect password given when decrypting PFX data". Legacy 3DES `.p12` files (what Keychain Access exports) work. Only matters if neo imports existing `.p12` files rather than generating its own key.
5. **Scoped paths use the OS's separator.** apple-codesign matches scoped settings against nested bundles' *native* relative paths, so on Windows `PlugIns/Widget.appex` never matches `PlugIns\Widget.appex`. Unmatched scopes are ignored silently, and the extension gets signed with no entitlements. Neo converts scope paths to the native separator and reads every bundle's entitlements back after signing, failing if they don't match (found by Windows CI).
6. **`rcodesign verify` only takes individual Mach-O files**, not bundles, and says it doesn't check everything Apple does. It isn't a substitute for `codesign --verify`.

**Maintenance**

- Upstream apple-codesign hasn't released since 0.29.0 (November 2024).
- `isideload-apple-codesign` (nab138, MPL-2.0) is maintained (0.29.11, May 2026). Comparing the crate sources, its changes are a virtual-filesystem layer for WebAssembly, the 755 fix, removal of timestamp-server support, and formatting. The signing logic is the same as upstream. isideload installs apps on real devices with it, which is good evidence the signatures are accepted.

## Still to prove (needs macOS and a device)

1. Run `tools/rcodesign_ab_test.sh` on a Mac. It takes an IPA already signed by Python warpsign and re-signs it with rcodesign using the same certificate and entitlements. Then it runs `codesign --verify --deep --strict` and diffs entitlements, signature details and the designated requirement against the original.
2. Install the output IPA on a device and launch the app and its extensions.
3. The designated requirement with a real Apple certificate. With the self-signed test certificate it was pinned to the root certificate's hash. Apple-chained certificates take a different code path, which the A/B test will show.
