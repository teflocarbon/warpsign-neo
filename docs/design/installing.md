# Design: installing on the device

Agreed October 2026. WarpSign installs signed apps on the device it runs on in two ways. You pick one in Settings > Installing.

## Over the web (default): ESign's way

No setup. Found by static analysis of ESign 5.0.2, which you described as the most reliable installer you'd used (Feather's local server install has been flaky).

- iOS installs apps from `itms-services://?action=download-manifest&url=<manifest>`. It insists the **manifest** comes over trusted HTTPS, but not the **IPA** the manifest points to.
- So the manifests are static and the same for everyone, published in the public [teflocarbon/warpsign-manifests](https://github.com/teflocarbon/warpsign-manifests) repo. Slot N's says the app is at `http://127.0.0.1:14310/appN.ipa`, with a placeholder bundle ID and title. ESign does exactly this with its own GitHub/Gitee repos.
- WarpSign serves the IPA (and an icon for iOS's progress display) from loopback only (`serve.rs`), opens the link, and iOS asks the user to confirm.
- Slots rotate (1000 of them), so iOS never sees the same manifest URL twice in a row.
- A silent audio loop plus a background task keep WarpSign alive while iOS downloads (`KeepAlive.swift`, the `audio` background mode). ESign does the same, and does it unconditionally. A suspended server is the likely cause of other installers' flakiness.
- No TLS key ships in the app. Feather uses backloop.dev's published key, and ESign used to ship a real certificate for `i.yyyue.xyz` that expired in 2024: both break when the certificate is revoked or expires.
- Costs: GitHub must be reachable when an install starts, port 14310 must be free (close other signing apps), and failures are iOS's own alert with little detail.

`crates/warpsign-core/src/serve.rs` is the single source for the manifests: `warpsign manifests <dir>` regenerates them. Changing the port or slot count means republishing them.

## Through LocalDevVPN: SideStore's way

- A pairing file, made once on a computer with `warpsign pair`, plus LocalDevVPN, which loops `10.7.0.1` back to the device's own lockdownd.
- WarpSign copies the IPA into `PublicStaging` over AFC and asks `installation_proxy` to install it (`device.rs`, using the `idevice` crate). It keeps a heartbeat on the connection.
- Clearer errors and real progress, straight from installd. The pairing file also gives WarpSign the device's UDID, so it can check, and fix, the device's registration on the team.
- Costs: a computer once, and a VPN app.

The CLI uses the same code over USB: `warpsign install` and `warpsign sign --install`.
