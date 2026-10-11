# First run against a real account

Everything up to Apple has been tested against recorded or scripted responses, never against Apple itself. This is the order to try it in, so each step only depends on ones already shown to work. The CLI is quicker to debug than the app, so start there.

Keep a log of every run: `--http-log` writes each request and response, with cookies, tokens, passwords, codes and sign-in values redacted. Attach it to bug reports.

```bash
cargo install --path crates/warpsign-cli
export WARPSIGN_HTTP_LOG=~/warpsign-http.log
```

## 1. Sign in (once!)

```bash
warpsign login
```

- **403 means stop.** Apple refused the sign-in. Retrying can lock the Apple ID. Send the log instead. The likely suspects are the hashcash header and the headers on `signin/complete`.
- **401** means the password was wrong, or the SRP maths doesn't match Apple's. If you're sure of the password, look in the log at `signin/init`'s `protocol`: `s2k_fo` is the path Python warpsign never handled.
- **409, then a code prompt**: two-factor works. Try `sms` once too.
- Afterwards, `warpsign status` should list your team without asking for a password. That's the saved session working.

## 2. Read-only portal calls

```bash
warpsign status            # teams, and identities if any
warpsign device list       # should show your iPhone
```

These are the same requests as Python warpsign (`tests/portal_parity.rs` checks it), so failures here are most likely session or CSRF problems.

## 3. Certificates (uses slots)

```bash
warpsign identity create
warpsign identity export ~/warpsign-identity.p12   # save the printed passphrase
```

Creating certificates is new: Python warpsign never did it. If Apple rejects the request, the log shows its reason. A full distribution slot is reported, not fatal.

Then open the `.p12` in Keychain Access on a Mac to check it's readable.

## 4. Sign something small

```bash
warpsign sign Small.ipa
```

This registers App IDs, app groups and containers, sets capabilities, makes profiles and signs. Check the portal afterwards: the App IDs should be named `WS App ...` like Python warpsign's, and profiles `TS <id> Development`. A second run should regenerate the same profiles, not make new ones.

Then install the signed IPA (Apple Configurator, or `xcrun devicectl device install app`) and launch it. **This is the first real test of rcodesign's signatures on a device.** If it won't install, compare with `tools/rcodesign_ab_test.sh`.

## 5. PPQ

```bash
warpsign profile ~/Library/MobileDevice/Provisioning\ Profiles/*.mobileprovision
```

Your account predates PPQ, so profiles should say `PPQ off`. With a newer team, `sign --force-original-id` must refuse.

## 6. The app

Build with `tools/build_ios.sh`, set your bundle ID and team, and run it on the phone. Then:

1. Settings → Sign In.
2. Settings → Signing Identity → Restore from Backup, with the `.p12` from step 3. This avoids using more certificate slots.
3. Library → import an IPA → Sign.

If something fails, Settings → Diagnostics → Network Log shows what Apple said, and each signing step's log can be copied.
