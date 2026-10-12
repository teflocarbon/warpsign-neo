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

`serve::manifest` and the Worker produce the same bytes, both checked against `worker/test/manifest.plist` (`cargo test`, and `npm test` in `worker/`). The manifest server is set in Settings > Installing, defaulting to `serve::DEFAULT_MANIFEST_SERVER` (`https://warpsign-manifests.teflo.dev`, the Worker on teflo.dev).

### Deploying the Worker

In the Cloudflare dashboard: Workers & Pages > Create > Worker, name it, then Edit code, replace it with `worker/src/index.js` and Deploy. Or `npx wrangler deploy` in `worker/`. Check it with `https://<name>.<account>.workers.dev/manifest?slot=1&port=14310&id=com.example.app&title=Example&version=1.0`.

## With a pairing file: SideStore's way

- A pairing file, made once on a computer with `warpsign pair`, plus a loopback VPN that loops `10.7.0.1` back to the device itself: WarpSign's own (below), or LocalDevVPN.
- **Since iOS 26.6, lockdownd resets connections that arrive through the VPN** (first `QueryType`/`StartSession` gets "connection reset" or "broken pipe"; seen on a device in October 2026, and by SideStore and AirCard users). So `warpsign pair` makes a **RemotePairing** file by default (iOS 17.4+): idevice_pair's format, paired promptlessly over USB through `com.apple.dt.remotepairingdeviced.lockdown`, plus a `UDID` key other tools ignore. On the device, WarpSign pair-verifies on the remote pairing port (usually 49152; overridable in Settings, as some devices advertise another as `_remotepairing._tcp`), opens the TLS-PSK tunnel, runs idevice's userspace TCP stack through it, and reaches AFC and installation_proxy as RSD shim services. It only ever verifies: it never starts a pair-setup from the device. `warpsign pair --lockdown` still makes the old kind for earlier iOS.
- WarpSign copies the IPA into `PublicStaging` over AFC and asks `installation_proxy` to install it (`device.rs`, using the `idevice` crate). Lockdown connections keep a heartbeat.
- Clearer errors and real progress, straight from installd. The pairing file also gives WarpSign the device's UDID, so it can check, and fix, the device's registration on the team.
- Costs: a computer once.

### WarpSign's own loopback VPN

`WarpSignTunnel.appex` is a packet tunnel doing what LocalDevVPN does, following SideStore's StosVPN (MIT). It claims `10.7.0.0/24` and only that (no default route), and rewrites each IPv4 packet so one sent from the tunnel's address `10.7.0.0` to `10.7.0.1` arrives from `10.7.0.1` at `10.7.0.0`, and back. iOS takes that for a connection from another machine, which lockdownd and RemotePairing answer. The rewrite swaps values between the two address fields, so checksums stay correct. Nothing leaves the device.

- Installing and Test Connection turn it on (`LoopbackTunnel.connect`), adding the configuration first; iOS asks to allow that once. If it can't (WarpSign signed without Network Extensions, e.g. by another signer with a wildcard profile), LocalDevVPN still works, and the app says which.
- It needs the Network Extensions capability (`packet-tunnel-provider`), which paid teams have and free accounts don't, which is why SideStore needs a separate VPN app. CI ad-hoc signs the IPA with the app's and extension's entitlements so that signing WarpSign with WarpSign sees them and maps them like any other app's (`NETWORK_EXTENSIONS`, not the macOS `…_DIRECT_DISTRIBUTION`, which grants the same key).
- The app finds the extension's bundle ID in its PlugIns rather than assuming it, as signing changes it.
- Next: installing from the extension itself. It lives as long as the VPN is up, whatever happens to the app, so it could finish an install the app was suspended in. Its own sockets bypass its tunnel, so it would reach the device by writing packets into it from a third address (`10.7.0.2`) through idevice's userspace TCP stack (jktcp handles IPv4), within the extension's 50 MB.

The CLI uses the same code over USB: `warpsign install` and `warpsign sign --install`.

## Leaving WarpSign mid-job

On iOS 26 and later, signing and installing with a pairing file run as a continued processing task (`Platform/ContinuedWork.swift`): iOS keeps them going after WarpSign is left, and shows its own progress card in the Dynamic Island and on the Lock Screen, fed by the step events. The request is submitted with the `fail` strategy (now or not at all) and the task is always completed as successful, so iOS never draws its own failure card; the job's outcome is WarpSign's to show.

The identifier comes from Info.plist (`<bundle ID>.continued.*`, read back at runtime) rather than from the bundle ID, so the two agree however WarpSign was signed. Re-signing remaps it with the bundle ID when Patch IDs is on, as Python warpsign's Info.plist byte replacement already does for every mapped ID (pinned in `info_plist_patching.json`). Web installs keep the silent-audio hold (`KeepAlive.swift`), since iOS's installer does that work.

## Never over itself

WarpSign refuses to install a signed copy whose bundle ID is its own (`Bundle.main.bundleIdentifier`), by either method. iOS has to stop the running app to replace it, and with it whatever is feeding the install (the VPN extension, inside the same bundle, or the loopback web server); installd is left with a half-finished install and installs nothing more until the device restarts (seen on a device in October 2026). The Install button says so instead, and the install screen checks the IPA's own bundle ID before starting, as a backstop. A copy of WarpSign signed with another bundle ID installs as a separate app and is allowed; to update WarpSign itself, install from a computer (`warpsign install`) or another installer.
