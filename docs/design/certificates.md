# Design: certificates

Goal: you never have to think about certificates. Agreed October 2026.

## Decisions

- **Neo creates its own certificates** on first setup: one Apple Development and one Apple Distribution. The private keys are generated on the device (or the desktop CLI's machine), a CSR goes through the Developer Portal, and the certificate comes back. Neo never asks you to make a certificate yourself.
- **Neo never revokes a certificate without asking.** Revoking breaks every app signed with it, including your own Xcode projects. If the account has no free slots, neo stops and shows your certificates (marking the ones neo made). You can either import one, or explicitly revoke one after a clear warning about what that breaks.
- **Distribution is best-effort.** Teams get 3 distribution certificates. If they're all in use, neo carries on with development only. Distribution-only features (production push, keeping original bundle IDs) are then unavailable, and neo says why.
- **Back up once, restore anywhere.** Right after creating the certificates, setup shows a "save your signing identity" step: one `.p12` containing both identities, protected by a long random passphrase neo generates. Save it via the share sheet: into a password manager that takes attachments, or to Files/iCloud Drive with the passphrase in the password manager. The screen says plainly that the file is the keys to your team's signing and must not be shared.
- **Restoring is "import identity"**: a new phone, a reinstalled app, or the desktop CLI sharing the phone's identity instead of using up another slot. On import, neo checks that each certificate belongs to the logged-in team and still exists (not revoked) on the portal.
- **Day to day, keys live in the iOS Keychain** (this device only, not synced), or in a file under the CLI's config directory on desktop. The exported `.p12` is the backup.
- **Reuse, don't churn.** Neo remembers its certificates by serial number and reuses them until they expire. Before signing, it checks with the portal that the certificate still exists and hasn't been revoked. Only then would it make a new one.
- **Same logic for the CLI and the app.** Everything lives in warpsign-core; only key storage differs per platform.

## Notes

- **Exported `.p12` files use legacy 3DES encryption**, which Keychain Access, Python warpsign and apple-codesign 0.29 can all read. On import, neo should accept both legacy and modern (AES) files. apple-codesign alone can't read AES ones (see `docs/spikes/rcodesign.md`).
- **Later:** keep the private key inside the Keychain and have it perform the signing operation, so the key never enters app memory. apple-codesign's signer interface allows a custom signer.
- **PPQ:** the first profile created for the team shows whether PPQ checking is on (see `profile::check_ppq_risk`). Cache that per team.
