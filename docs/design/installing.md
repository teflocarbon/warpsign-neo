# Design: installing on the device

Agreed October 2026. WarpSign installs signed apps on the device it runs on in two ways. You pick one in Settings > Installing.

## Over the web (default): ESign's way

No setup. Found by static analysis of ESign 5.0.2, which you described as the most reliable installer you'd used (Feather's local server install has been flaky).

- iOS installs apps from `itms-services://?action=download-manifest&url=<manifest>`. It insists the **manifest** comes over trusted HTTPS, but not the **IPA** the manifest points to.
- **iOS 27 checks the manifest's `bundle-identifier` against the app** and refuses with "Unable to Install" when they differ (found on a real device, October 2026: the same IPA installed once the manifest named it). So static placeholder manifests, which ESign pioneered and WarpSign first copied (the now-unused teflocarbon/warpsign-manifests repo), no longer work. ESign itself now uses its `install_remote` route on iOS 27: its own server `i.yyyue.xyz` writes each manifest, so it sees every app installed through it.
- WarpSign does the same with a server you own: a Cloudflare Worker (`worker/`) that answers `GET /manifest?slot=&port=&id=&title=&version=` with a manifest naming the app, whose assets are always `http://127.0.0.1:<port>/app<slot>.ipa` and `.png`. It checks every value, stores and logs nothing, and only ever points at loopback, so it can't be used to offer apps hosted anywhere else. The app's ID, name and version are read from the signed IPA (`ManifestApp::from_ipa`), so the manifest always matches it.
- WarpSign serves the IPA (and an icon for iOS's progress display) from loopback only (`serve.rs`), opens the link, and iOS asks the user to confirm. It logs each request iOS makes to the install step, so a failed install shows how far iOS got.
- Slots rotate (1000 of them), so each install has its own URLs.
- A silent audio loop plus a background task keep WarpSign alive while iOS downloads (`KeepAlive.swift`, the `audio` background mode). ESign does the same, and does it unconditionally. A suspended server is the likely cause of other installers' flakiness.
- No TLS key ships in the app. Feather uses backloop.dev's published key, and ESign used to ship a real certificate for `i.yyyue.xyz` that expired in 2024: both break when the certificate is revoked or expires.
- Costs: the manifest server must be reachable when an install starts, and sees each installed app's ID, name and version (never the app); port 14310 must be free (close other signing apps); failures are iOS's own alert with little detail.

`serve::manifest` and the Worker produce the same bytes, both checked against `worker/test/manifest.plist` (`cargo test`, and `npm test` in `worker/`). The manifest server is set in Settings > Installing, defaulting to `serve::DEFAULT_MANIFEST_SERVER`.

### Deploying the Worker

In the Cloudflare dashboard: Workers & Pages > Create > Worker, name it, then Edit code, replace it with `worker/src/index.js` and Deploy. Or `npx wrangler deploy` in `worker/`. Check it with `https://<name>.<account>.workers.dev/manifest?slot=1&port=14310&id=com.example.app&title=Example&version=1.0`.

## Through LocalDevVPN: SideStore's way

- A pairing file, made once on a computer with `warpsign pair`, plus LocalDevVPN, which loops `10.7.0.1` back to the device's own lockdownd.
- WarpSign copies the IPA into `PublicStaging` over AFC and asks `installation_proxy` to install it (`device.rs`, using the `idevice` crate). It keeps a heartbeat on the connection.
- Clearer errors and real progress, straight from installd. The pairing file also gives WarpSign the device's UDID, so it can check, and fix, the device's registration on the team.
- Costs: a computer once, and a VPN app.

The CLI uses the same code over USB: `warpsign install` and `warpsign sign --install`.
