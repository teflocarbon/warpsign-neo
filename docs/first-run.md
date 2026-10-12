# First run against a real account

Everything up to Apple has been tested against recorded or scripted responses, never against Apple itself. This is the order to try it in, so each step only depends on ones already shown to work. The CLI is quicker to debug than the app, so start there.

Keep a log of every run: `--http-log` writes each request and response, with cookies, tokens, passwords, codes and sign-in values redacted. Attach it to bug reports.

```bash
cargo install --path crates/warpsign-cli
export WARPSIGN_HTTP_LOG=~/warpsign-http.log
```

### On Windows

No Rust needed: Actions → CLI binaries → the latest run → download `warpsign-windows-x64` and unzip `warpsign.exe` somewhere handy. Then, in PowerShell from that folder:

```powershell
$env:WARPSIGN_HTTP_LOG = "$HOME\warpsign-http.log"
.\warpsign.exe login
```

For USB (steps 4 and 6) install Apple's **Apple Devices** app from the Microsoft Store, or iTunes. Either brings the Apple Mobile Device Service, which is how WarpSign finds the phone. Plug the phone in and tap Trust once, so the service can see it, before running `pair` or `install`.

The commands below are the same, with `.\warpsign.exe` for `warpsign` and Windows paths for `~/...`. Settings and the saved session live in `%APPDATA%\warpsign`. Windows Defender may scan or warn about an unsigned download; that's expected.

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

Then check it's readable: open the `.p12` in Keychain Access on a Mac, or on Windows double-click it (the Certificate Import Wizard asks for the passphrase; cancel before importing).

## 4. Sign something small

```bash
warpsign sign Small.ipa
```

This registers App IDs, app groups and containers, sets capabilities, makes profiles and signs. Check the portal afterwards: the App IDs should be named `WS App ...` like Python warpsign's, and profiles `TS <id> Development`. A second run should regenerate the same profiles, not make new ones.

Then install it over USB with `warpsign install Small-signed.ipa` (or `warpsign sign Small.ipa --install`) and launch it. **This is the first real test of rcodesign's signatures on a device.** If installd rejects it, its message says why. Compare with `tools/rcodesign_ab_test.sh`.

## 5. PPQ

```bash
warpsign profile ~/Library/MobileDevice/Provisioning\ Profiles/*.mobileprovision
```

(That path is the Mac's; on Windows, point it at any `.mobileprovision`, e.g. the one inside a signed IPA.) Your account predates PPQ, so profiles should say `PPQ off`. With a newer team, `sign --force-original-id` must refuse.

## 6. Pairing file

```bash
warpsign pair     # phone on USB and unlocked
```

This writes `<UDID>.rppairing.plist`, a RemotePairing file (iOS 17.4+). It pairs over the trust USB already has, so there's no prompt. iOS 26.6 and later refuse the older lockdown files through the VPN; `warpsign pair --lockdown` still makes one for older iOS. If it fails, idevice_pair's RemotePairing files work in WarpSign too, which tells a WarpSign problem from a device one.

## 7. The app

Build with `tools/build_ios.sh`, set your bundle ID and team, and run it on the phone. Or, without a Mac: run the **iOS IPA** workflow, download `WarpSign.ipa` from the repo's `latest` release (a plain file, unlike the artifact zip, which iOS Files can fail to open), and sign it with the CLI from step 4. Then:

1. Settings → Sign In.
2. Settings → Signing Identity → Restore from Backup, with the `.p12` from step 3. This avoids using more certificate slots.
3. Settings → Installing: get the pairing file to the phone (AirDrop from a Mac; from Windows, iCloud Drive or anything that lands it in Files) and open it in WarpSign, install LocalDevVPN and connect it, then Test Connection. The Team section should say this phone is registered.
4. Library → import an IPA → Sign → Install on This Device.

Installing goes through LocalDevVPN to the phone's own installer, as SideStore does. If Test Connection can't connect, check the VPN is on. If it says the pairing file was refused, make a new one.

If something fails, Settings → Diagnostics → Network Log shows what Apple said, and each signing step's log can be copied.
