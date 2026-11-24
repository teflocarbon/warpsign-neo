# Anisette on the device — spike

**Question:** can the WarpSign iOS app get anisette data on the iPhone
itself? That is the first step towards signing in to GrandSlam
(`gsa.apple.com`) for long-lived tokens, as Xcode, AltStore and SideStore
do, instead of the 30-minute web portal session (`docs/design/portal-session.md`
on the keep-alive branch).

**Short answer:**

- **AuthKit (route 1):** probably not. Every public tool that gets anisette
  from AuthKit on iOS holds `com.apple.authkit.client.private`, which a
  development profile can't carry. Nobody has published a test from a stock
  app, though, so this branch adds a probe to settle it on a real device.
- **The Android ADI libraries in-process (route 2):** the machinery works —
  this branch has a full Rust port that loads the libraries, runs them and
  provisions against Apple — **but since about 20 November 2026 Apple
  rejects the last provisioning step for every Android-ADI identity**, our
  own included, with error −29003 "environment mismatch". So it can't make
  usable anisette right now. It needs no JIT (Unicorn's interpreter), and we
  wrote our own loader rather than copy anyone's GPL/LGPL code.
- **A remote anisette v3 server (route 3):** the only route that works
  today, because the servers that still pass Apple's gate derive anisette
  from a Mac. The server never sees the Apple ID, but it holds this
  "device's" ADI identity.

**Recommendation:** run the probe first. Given Apple's −29003 block on
Android-ADI, ship GrandSlam on an opt-in **Mac-based** remote v3 server the
user chooses, and keep the in-process port (route 2) ready for if or when
Apple reopens that path.

Researched and built November 2026. Links are to the sources; no code was
copied from them.

## What anisette is

GrandSlam requests, and the Xcode-style developer services requests made
with its tokens, carry device headers produced by Apple's ADI (CoreADI):

| Header | What it is |
|---|---|
| `X-Apple-I-MD` | one-time password, changes per request |
| `X-Apple-I-MD-M` | machine ID from ADI provisioning |
| `X-Apple-I-MD-RINFO` | routing info |
| `X-Apple-I-MD-LU`, `X-Mme-Device-Id` | local user and device IDs (v3 clients derive these themselves) |
| `X-Mme-Client-Info`, `X-Apple-I-SRL-NO`, client time, locale | device description |

ADI must first be *provisioned* (a round trip with Apple that produces
`adi.pb`). After that, each request needs a fresh OTP. So anisette is needed
whenever the app talks to Apple, not just at sign-in.

## Route 1 — Apple's own frameworks on the device

On iOS, anisette comes from `akd`, the AuthKit daemon, which owns ADI.
Clients reach it over XPC through AuthKit.framework:

- `-[AKAppleIDSession appleIDHeadersForRequest:]`: the whole header set for a
  request. AltServer's Mail plugin uses it on macOS with identifier
  `com.apple.gs.xcode.auth`
  ([ALTPluginService.m](https://github.com/altstoreio/AltStore/blob/master/AltPlugin/ALTPluginService.m)).
- `-[AKAnisetteProvisioningController anisetteDataWithCompletion:]`: returns
  `AKAnisetteData` (machine ID, OTP, routing info)
  ([header](https://www.developer.limneos.net/headers/14.4/AuthKit.framework/Headers/AKAnisetteProvisioningController.h)).
- `AKDevice.currentDevice`: the device description. This one is local.
- CoreADI.framework holds the ADI code itself. In-process calls into it are
  obfuscated and need ADI's storage, which belongs to `akd`/`adid`.
- On macOS, `AOSUtilities retrieveOTPHeadersForDSID:` (AOSKit) is the easy
  path. It's used by AltServer
  ([AnisetteDataManager.swift](https://github.com/altstoreio/AltStore/blob/master/AltServer/Anisette%20Data/AnisetteDataManager.swift)),
  AltServer for Windows through iCloud for Windows' `AOSKit.dll`
  ([AnisetteDataManager.cpp](https://github.com/rileytestut/AltServer-Windows/blob/master/AltServer/AnisetteDataManager.cpp)),
  and omnisette's macOS provider
  ([aos_kit.rs](https://github.com/SideStore/apple-private-apis/blob/master/omnisette/src/aos_kit.rs)).
  AOSKit isn't on iOS.

What other tools found:

- **AltStore (iOS app)** never makes anisette on the phone. It asks
  AltServer on the computer
  ([FetchAnisetteDataOperation.swift](https://github.com/altstoreio/AltStore/blob/master/AltStore/Operations/FetchAnisetteDataOperation.swift)).
- **AltDaemon** (AltStore for jailbroken devices) does call `AKAppleIDSession`
  and `AKDevice` in-process
  ([AnisetteDataManager.swift](https://github.com/altstoreio/AltStore/blob/master/AltDaemon/AnisetteDataManager.swift)).
  It is signed with `com.apple.authkit.client.private` and
  `platform-application`
  ([AltDaemon.entitlements](https://github.com/altstoreio/AltStore/blob/master/AltDaemon/AltDaemon.entitlements)).
- **SideStore** never used AuthKit on the device. It went from remote
  servers to in-process ADI (route 2).
- **ReProvision** (jailbreak) uses AuthKit with private entitlements
  ([Login Notes](https://github.com/Matchstic/ReProvision/wiki/Login-Notes)).
- **No public source** shows a development-signed app getting anisette from
  AuthKit, and none shows exactly which check `akd` makes. The only evidence
  is indirect: every working client carries `com.apple.authkit.client.private`,
  and `com.apple.private.*` / `com.apple.authkit.*` entitlements can't go in a
  development profile. AMFI kills an app that claims entitlements its profile
  doesn't allow, so we can't simply add them.
- **What JIT changes:** StikDebug's JIT (debugger attached, `CS_DEBUGGED`)
  doesn't change our entitlements. It doesn't help route 1.

**Expected probe result:** AuthKit loads, `AKDevice` answers, and both
anisette calls fail with an XPC or AuthKit error, or time out. If they do
return headers, route 1 is the answer and the rest of this doc is the
fallback.

## Route 2 — Apple's Android ADI library in-process

Apple Music for Android ships ADI as `libstoreservicescore.so` (with
`libCoreADI.so`) for arm64-v8a, armeabi-v7a, x86 and x86_64. Several tools
load these libraries with their own ELF loader and call the obfuscated
exports (provision, OTP):

- **Provision** (Dadoum, LGPL-2.1,
  [repo](https://github.com/Dadoum/Provision)): loads them natively on Linux
  (x86, x86_64, AArch64), with `mprotect(PROT_EXEC)` on the mapped segments.
- **anisette-v3-server** (Dadoum, no licence, so all rights reserved;
  [repo](https://github.com/Dadoum/anisette-v3-server)): a server built on the
  same approach.
- **Sideloader** (Dadoum, GPL-3.0, [repo](https://github.com/Dadoum/Sideloader)):
  the same approach in a sideloading tool, on Linux and Windows.
- **omnisette** (SideStore/apple-private-apis, MPL-2.0,
  [lib.rs](https://github.com/SideStore/apple-private-apis/blob/master/omnisette/src/lib.rs)):
  Rust, with a `store_services_core` provider. Its loader,
  [android-loader](https://github.com/Dadoum/android-loader), has no licence.
- **AnisetteKit** ([repo](https://github.com/mahee96/AnisetteKit)): runs both
  libraries inside **Unicorn's TCI interpreter**, so it needs no JIT, on iOS,
  tvOS, visionOS and macOS. SideStore 0.6.4 (September 2026) made it the
  primary anisette source
  ([release](https://github.com/SideStore/SideStore/releases/tag/0.6.4)). So
  in-process ADI on a stock iPhone is proven to run — though Apple now
  rejects what it produces at provisioning (see below).

On iOS, there are two ways to run the arm64-v8a library.

**(a) Natively, under JIT.** StikDebug gives us executable memory on
iOS 27.2. arm64-v8a is the iPhone's own instruction set, so this would be
fast. What it costs:

- Our own ELF loader and a bionic shim: libc, pthread,
  `__system_property_get`, file I/O redirected to our container.
- An ABI bridge between Android's AAPCS64 and Apple's arm64 ABI. Variadic
  arguments are passed differently, and Apple reserves `x18`. If the Android
  code uses `x18` as scratch, the kernel may zero it under it. Unverified:
  it needs a disassembly check.
- StikDebug attached each time the app needs anisette, which is every
  GrandSlam or developer services call. That's a usability regression next
  to the FairPlay helper, which needs no JIT (`fairplay-ondevice.md`).

**(b) In an interpreter, without JIT.** This is the AnisetteKit approach.
ADI's work per request is small, so interpreter speed is fine. The blocker is
licensing: Unicorn bundles `qemu/target/arm/neon_helper.c`, which is
**GPLv2-only** ([AnisetteKit#1](https://github.com/mahee96/AnisetteKit/issues/1)).
We can't link it into this AGPL-3.0 app (see the existing Unicorn note for
ipatool). We'd need an AGPL-compatible arm64 emulator: a different engine, or
a purpose-built interpreter for the subset these two libraries use. Also,
AnisetteKit's README says AGPL-3.0 but GitHub can't detect a licence, and its
Unicorn issue is open, so we shouldn't reuse it as it stands.

Constraints for both:

- **Apple's binaries can't be shipped.** The app would download Apple
  Music's APK from Apple at first use and extract the two `.so` files, as
  Provision does. They'd never go in the repo, IPA or releases.
- Provisioning runs against `gsa.apple.com` with our own identifier, and
  `adi.pb` stays in the app's Keychain or container. That file is the
  device's trusted-machine identity: never log it or include it in a network
  log or support bundle (Provision's README gives the same warning).
- The ADI identity is a fake "Android device". Apple sees a new trusted
  machine, not this iPhone.

### What this branch built, and what Apple does with it

This branch has a from-scratch Rust port of route 2(b) in
`crates/warpsign-core/src/anisette/`, with no code taken from the projects
above:

- `elf.rs` — a minimal AArch64 `.so` loader (segments, dynamic symbols,
  RELA relocations).
- `machine.rs` — the libraries loaded into Unicorn, with a small emulated
  libc (files kept in memory, `malloc`, the pthread calls as no-ops, the
  same ~30 functions Provision provides) reached through a code hook on a
  page of `ret`s, so the Android code never touches the real system.
- `adi.rs` — the obfuscated ADI exports wrapped as `start_provisioning`,
  `end_provisioning`, `request_otp` and the rest.
- `provisioning.rs` — the GrandSlam provisioning round trips.
- `apk.rs` — fetching the two libraries from Apple's Apple Music APK with
  HTTP range requests (a few MB, not the 142 MB APK), so Apple's binaries
  are never shipped.
- `mod.rs` — `AnisetteState` (the identity and `adi.pb`, kept secret) and
  the headers.

`warpsign anisette --unicorn <lib>` runs the whole thing. On a Windows PC
on 24 November 2026, with Unicorn 2.1.4's interpreter and the arm64-v8a
libraries, it:

1. loaded both libraries and set up ADI in ~40 ms total;
2. fetched a fresh `lookup` url-bag from `gsa.apple.com`;
3. ran `startMachineProvisioning` — Apple **accepted** our emulated ADI's
   `cpim` (HTTP 200);
4. ran `finishMachineProvisioning` — Apple returned **HTTP 401, error
   −29003 "environment mismatch"**.

So the loader, the emulated libc, the ADI calls and two of the three Apple
round trips all work; the emulator is running Apple's real ADI code and
Apple's own server took its output through the first step. The wall is at
the final step, and it isn't ours: SideStore #1745 and its comments
(November 2026) report that since about 20 November Apple returns −29003 at
`finishMachineProvisioning` for **every** Android-ADI identity — Dadoum's
`anisette-v3-server`, `ani.sidestore.zip`, `ani.npeg.us` and self-hosted
ones alike — while servers that derive anisette from a **Mac** still pass.
This is the same campaign as the September `com.apple.dt.Xcode` block and
the 20 November `macOS;`→`Mac OS X;` switch (see "Current GrandSlam quirks"):
Apple is fingerprinting non-Apple provisioning. Changing the client-info
string doesn't move −29003; the gate is on the Android ADI identity itself.

Bottom line: in-process ADI is **built and proven up to Apple's gate**, but
can't mint usable anisette while that gate is closed. The code is worth
keeping — if Apple reopens the path (as it has wobbled before) it works
immediately, and it's the only route with no third-party server — but it is
not shippable today.

## Route 3 — a remote anisette v3 server

This is what SideStore did until 0.6.4 (server list:
[SideStore/anisette-servers](https://github.com/SideStore/anisette-servers),
AGPL-3.0). It's also what xtool does today (MIT, defaults to
`mac-ani.jkcoxson.com`;
[OmnisetteADIProvider.swift](https://github.com/xtool-org/xtool/blob/main/Sources/XKit/GrandSlam/Anisette/OmnisetteADIProvider.swift)).
The v3 protocol is best described by its client code
([remote_anisette_v3.rs](https://github.com/SideStore/apple-private-apis/blob/master/omnisette/src/remote_anisette_v3.rs),
[ADIDataProvider.swift](https://github.com/xtool-org/xtool/blob/main/Sources/XKit/GrandSlam/Anisette/ADIDataProvider.swift)).

**What the client does.** It talks to Apple directly for the provisioning
round trips (`lookup`, `midStartProvisioning`, `midFinishProvisioning`). It
derives `X-Mme-Device-Id` and `X-Apple-I-MD-LU` itself, from a random
16-byte identifier, and keeps `adi.pb`.

**What the server sees:**

- the identifier;
- the provisioning blobs (`spim`, `ptm`, `tk`), sent over the
  `/v3/provisioning_session` websocket;
- the identifier and `adi.pb` again on every `/v3/get_headers` call;
- the timing of every Apple request we make, since each one needs fresh
  headers.

**What the server never sees:** the Apple ID, the password, 2FA codes,
GrandSlam tokens, or the requests themselves.

**The trade-off:**

- The operator can mint valid anisette for our device identity at will and
  link it to our IP address.
- A server that is malicious or overloaded can break sign-in.
- Apple sees many accounts behind a few server-made identities. The point of
  v3 is that each client gets its own identity, but the server still sees
  them all. v3 was a response to lockouts on shared v1 identities
  ([SideStore#191](https://github.com/SideStore/SideStore/issues/191)).
- Public servers come and go.
- For a public, AGPL app: the server should be the user's choice, opt-in,
  and ideally self-hosted. It shouldn't be a default the app contacts
  silently.

## Current GrandSlam quirks to remember

These apply to any GrandSlam client we write:

- **Since September 2026,** GrandSlam answers 503 when `X-MMe-Client-Info`
  names `com.apple.dt.Xcode`. Clients now report `com.apple.akd`
  ([anisette-v3-server#59](https://github.com/Dadoum/anisette-v3-server/issues/59),
  [AltStore#1790](https://github.com/altstoreio/AltStore/pull/1790)).
- **Since about 20 November 2026,** it answers 503 for an OS label of
  `macOS;` and needs `Mac OS X;`, sometimes with a current OS version
  string ([SideStore#1745](https://github.com/SideStore/SideStore/issues/1745)).
- Expect more of this. Keep the device description data-driven, not
  hard-coded across the code.

## Recommendation

1. **Run the probe now** (below). It costs nothing and settles route 1. If
   AuthKit returns `X-Apple-I-MD` and `X-Apple-I-MD-M`, use it. Those are
   real device headers for this iPhone, the best possible identity.
2. **If it's refused** (expected), build the GrandSlam client in
   `warpsign-core` against an `AnisetteProvider` trait, with the in-process
   ADI already written as one implementation. Ship **remote v3** first,
   because Apple's −29003 block means that's the only one that works today:
   - opt-in, with a server URL the user enters;
   - a **Mac-based** server (e.g. the jkcoxson family): Android-ADI servers
     are blocked by the same −29003 as our in-process port;
   - client-side provisioning, as xtool does. xtool is MIT, so its client
     can be followed with credit in NOTICE;
   - `adi.pb` kept in the Keychain.
   The CLI can use the same trait. On macOS, omnisette's AOSKit approach
   (MPL-2.0) is available. Its `store_services_core` provider is not,
   because android-loader is unlicensed.
3. **Keep the in-process ADI port (route 2) warm.** It's built and passes
   Apple's first provisioning step; retry `finishMachineProvisioning`
   periodically (the `warpsign anisette` probe is the check) and wire it in
   as the preferred provider the moment Apple's −29003 gate lifts, since it
   needs no third-party server. A note: licensing still rules out *shipping*
   Unicorn (GPLv2-only, see the Unicorn note), so the iOS app would reuse
   the run-time-downloaded Unicorn it already loads for ipatool, or we build
   a GPL-free interpreter for the small instruction subset these libraries
   use.
4. **Keep the web portal session**, with its keep-alive, as the default
   until a GrandSlam route has been tested against a real account and
   device (AGENTS.md: behaviour that reaches Apple needs that).

## The probe

**Where:** Settings → Developer → **Probe Anisette from AuthKit**
(`ios/WarpSign/Platform/AnisetteProbe.swift`).

**What it does.** It only asks; it never provisions, erases or syncs ADI,
and sends nothing over the network itself. It:

1. `dlopen`s AuthKit, and CoreADI just to see whether it loads (it isn't
   called).
2. Reads `AKDevice.currentDevice` (local) and reports which properties came
   back.
3. Calls `-[AKAnisetteProvisioningController anisetteDataWithCompletion:]`
   and reports whether `machineID`, `oneTimePassword` and `routingInfo` came
   back, or the error's domain and code.
4. Calls `-[AKAppleIDSession appleIDHeadersForRequest:]` (identifier
   `com.apple.gs.xcode.auth`) on a GrandSlam request it never sends. It
   reports each anisette header as present (with its length) or absent,
   plus the *names* of any other headers.

Each XPC call times out after 15 seconds.

**What it never shows:** header values, device IDs or serials, or error
descriptions (they can quote account details). It shows lengths, domains
and codes only. Even so, check a screenshot before sharing it.

**Reading the result:**

| Outcome | Meaning |
|---|---|
| `X-Apple-I-MD` and `X-Apple-I-MD-M` present | Route 1 works from a development-signed app. |
| AuthKit error, XPC error (`NSCocoaErrorDomain` 4097/4099), or a timeout | `akd` refused us, as expected: routes 3 and 2. |
| Selectors or classes missing | AuthKit's interface changed on this iOS version. Note the version and look again. |

**Not verified yet.** It isn't built here (no Swift compiler outside macOS):
`ios.yml` CI builds it. Nobody has run it on a device yet.
