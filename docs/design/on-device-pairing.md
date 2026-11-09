# Pairing on the device, without a computer

Written November 2026. Until now, installing with a pairing file has needed a computer once: `warpsign pair` over USB, then the file carried to the phone. On iOS 27 and later, WarpSign can now make that file on the phone itself, as StikDebug does. This note covers how that works, where it fits in WarpSign, and what is still unknown.

## How StikDebug does it

StikDebug (https://github.com/StikDebug/StikDebug, AGPL-3.0) added on-device pairing in late 2026 (Settings → Pair a Device in StikDebug 3.2.x; `StikDebug/Services/OnDevicePairingService.swift`). It isn't the same idea as Tetherless. An earlier look at Tetherless suggested an app starting RemotePairing *towards* the device. StikDebug does it the other way round.

1. **iOS 27 added device-initiated pairing.** Settings → Privacy & Security → Developer Mode lists the computers on the local network that advertise the DNS-SD service `_remotepairing-pairable-host._tcp`, with a "Pair with <name>" button for each. Tapping one makes the *device* connect to that computer and drive the RemotePairing conversation. Normally that's Xcode on a Mac.
2. **The app plays the Mac.** StikDebug listens on a TCP port on all IPv4 interfaces and advertises itself with `NetService` as `_remotepairing-pairable-host._tcp`, model `Mac17,7`, plus a TXT record (`name`, `identifier`, `authTag`, `model`, `flags`, `ver`, `minVer`). The phone finds it, as it would any Mac on the Wi-Fi, and connects to it. It's connecting to its own Wi-Fi address, so the connection never leaves the device. StikDebug tells "this device" from "another device on the network" by comparing the connection's peer address with its own local address.
3. **Pair-setup with the roles swapped.** The device sends the handshake (`originatedBy: host`) and runs SRP pair-setup (M1–M6, 3072-bit group, SHA-512, username `Pair-Setup`). The app answers as the accessory. It makes a random six-digit setup code and shows it, in the app and in a local notification, and the user types it into Settings. After SRP, the two exchange Ed25519 long-term identities and altIRKs in the encrypted M5/M6.
4. **The result** is an ordinary **RemotePairing file**: the host's identifier, its Ed25519 key pair, and the device's altIRK. It's the same format idevice_pair and `warpsign pair` make. It is *not* a lockdown `.mobiledevicepairing` file. From then on it's used the normal way: pair-verify on the device's remote pairing port, then the TLS-PSK tunnel and RSD.
5. **Keeping alive.** The user is in Settings while the app waits, so StikDebug keeps running in the background with silent audio and background location.

**What the user is asked:** a six-digit code, shown by the app and typed into Settings. No Trust prompt.

**iOS versions:** 27 and later. Earlier versions have no "Pair with …" in Settings. StikDebug gates both the feature and its documentation on 27. The owner's device runs 27.2.

## Libraries and licences

| What | Licence | How WarpSign uses it |
|---|---|---|
| `idevice` crate (jkcoxson/idevice), `remote_pairing::responder` (`PairableHost`, `PairableHostInfo`) | MIT | Already a dependency (0.1.68, feature `remote_pairing`). WarpSign calls it, it isn't copied. The protocol work all lives here. |
| idevice's C FFI (`ffi/src/pairable_host.rs`, `pairing_host.rs`) | MIT | Not used: WarpSign calls the Rust API through its own uniffi bindings. Read only to see how the pieces fit. |
| StikDebug | AGPL-3.0 (compatible) | Ideas only: the flow, the Mac model, comparing addresses, keeping alive and showing the code in a notification. No code is copied, so it needs no NOTICE entry; it's credited here and in `pairable_host.rs`. |

## How it fits into WarpSign

What WarpSign already had:

- `crates/warpsign-core/src/device.rs` reads both kinds of pairing file. For RemotePairing it pair-verifies on `10.7.0.1:49152` through the loopback VPN, opens the tunnel, and reaches AFC and installation_proxy over RSD. It never ran a pair-setup on the device: that only happened over USB (`create_pairing_file`, through `RemotePairingLockdownService`).
- `crates/warpsign-ffi/src/install.rs` exposes this to Swift. `DeviceStore` keeps the file in the Keychain.
- The loopback VPN (`Platform/LoopbackTunnel.swift`, `WarpSignTunnel`) routes `10.7.0.1` back to the device. **Pairing doesn't need it**: the device reaches WarpSign at its own Wi-Fi address. Only the installs afterwards use the VPN, exactly as they do with an imported file.

What's new:

- `crates/warpsign-core/src/pairable_host.rs`: `PairableHost` binds `0.0.0.0:0`, generates the host's `RpPairingFile` and `PairableHostInfo` (model `Mac17,7`), and provides the service name (the host identifier), the TXT records and the port. `accept` waits for the first connection *from this device* (peer address = local address); connections from other devices are closed without an answer. It then runs idevice's `PairableHost::accept`, passes the code to a callback, and returns the pairing file's bytes and the device's name and model. `cancel` works from any thread, before or during the wait.
- `crates/warpsign-ffi/src/install.rs`: an `OnDevicePairing` object (`service_type`, `service_name`, `port`, `txt_record`, `accept(observer)`, `cancel`), a `PairingCodeObserver` callback, a `PairedDevice` record, and a `WarpsignError.PairingCancelled` case.
- `ios/WarpSign/Platform/OnDevicePairer.swift`: advertises with `NetService` (through mDNSResponder; sending multicast directly would need the restricted multicast entitlement, which is also why idevice's own FFI, which uses `mdns-sd`, isn't used). It runs `accept` off the main thread, holds `KeepAlive` (silent audio, which WarpSign already has) while waiting, posts the code as a notification, and keeps the result with `DeviceStore.keepPairingFile`, as if it had been imported.
- `ios/WarpSign/Views/Settings/PairThisDeviceView.swift`: the sheet, opened from Settings → Installing → 1. Pairing File → **Pair This Device…**. It only shows on iOS 27+, and only before a pairing file has been added.
- `ios/project.yml`: `NSLocalNetworkUsageDescription`, and `NSBonjourServices` listing `_remotepairing-pairable-host._tcp`.

The file made here has no `UDID` key (`warpsign pair` adds one). `DeviceStore.check()` already fills in the UDID from the first connection, as it does for idevice_pair files.

## Open questions

- **Not yet tried on a device.** The Rust side is unit-tested (advertising data, cancelling, answering a local connection), but the whole flow needs an iOS 27 device and a macOS build (CI). Things to check: that Settings lists "Pair with WarpSign"; that the code is accepted; that the resulting file pair-verifies through the loopback VPN; and that installs work.
- **Wi-Fi.** The device reaches the app at its Wi-Fi address. Whether pairing works with no Wi-Fi (cellular only, or Wi-Fi off) is unknown. The sheet asks for Wi-Fi.
- **IPv4 only**, as StikDebug. If iOS ever connects over IPv6 (link-local) instead, the listener needs to be dual-stack, and the same-device check has to compare IPv6 addresses.
- **Developer Mode.** StikDebug's steps go through the Developer Mode page. Apps WarpSign installs are development-signed, so Developer Mode is normally already on, but where "Pair with …" sits when it's off is unverified.
- **Pinless pairing.** `PairableHostInfo::allows_pinless_pairing` advertises Apple's all-zero code. If iOS honours it, no code would need typing. That's untested, and it would let anything on the network pose as WarpSign without a code, so it stays off.
- **altIRK.** The host's altIRK isn't kept. It only matters for re-advertising the same host so an already-paired device recognises it, which WarpSign doesn't do: afterwards it connects to the device, not the other way round.
- **The CLI.** The same `PairableHost` could let `warpsign pair --wifi` pair a phone with a computer over Wi-Fi, with no USB. It would need mDNS advertising from Rust (for example `mdns-sd`, which idevice's FFI uses). Not done here.
- **Pairing other devices** (StikDebug can make a file for another iPhone or an Apple TV) is out of scope: WarpSign only pairs the device it runs on, and ignores connections from anything else.
