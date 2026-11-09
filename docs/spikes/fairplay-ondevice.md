# On-device FairPlay decryption (Pathway A) — works on iOS 27.2

**Verdict: confirmed working on iOS 27.2** (iPhone, arm64), producing output
byte-identical to the offline credential decryptor (Pathway B). The trick is
*not* patched — an earlier "fails on 27.2" conclusion was a staging bug on our
side (see "The gotcha").

This is the no-credential path: ask the kernel to run an owned App Store app's
encrypted pages through FairPlay via `mremap_encrypted` with `cryptid=2`, on the
device itself. Technique described by
[FairPlayed](https://github.com/morzelowski/FairPlayed) (unlicensed — we wrote
our own from its README, copied no code). Spike done against a ~21 MB arm64
App Store binary the test device's account owns.

## What works (minimal requirements)

A plain **development-signed** iOS app that:

1. Embeds the target app in its installed bundle as a **proper app-bundle
   structure** — the main binary, its `Info.plist`, and the complete `SC_Info/`
   (`<name>.sinf`, `Manifest.plist`, the `.supf/.supp/.supx` records). `fairplayd`
   resolves the licence by path: `<dir>/SC_Info/<name>.sinf` next to the binary.
2. At runtime:
   - parse the arm64 slice's `LC_ENCRYPTION_INFO_64` → `cryptoff`, `cryptsize`,
     `cpusubtype`, slice offset;
   - `mmap(len = cryptoff + cryptsize, offset = slice_off, PROT_READ, MAP_PRIVATE)`
     — a **tight** mapping; the encrypted region is its tail;
   - `mremap_encrypted(base + cryptoff, cryptsize, /*cryptid=*/2, CPU_TYPE_ARM64,
     cpusubtype)`;
   - read the range back with `vm_read_overwrite` (a bad page is a
     `kern_return_t`, not a SIGBUS);
   - write `cryptid = 0` and repack.

`mremap_encrypted` is a syscall (`SYS_mremap_encrypted`, in the iOS SDK's
`sys/syscall.h`); no libc prototype, so declare
`extern int mremap_encrypted(void*, size_t, uint32_t, uint32_t, uint32_t)`.

### Not required (tested, all PASS without)

- **JIT / `CS_DEBUGGED`** — passes with no debugger attached.
- **`get-task-allow`** — passes signed with the minimum entitlements.
- Any private entitlement. Just a normal dev signature and the owning Apple ID
  signed in on the device.

## The gotcha (why it first failed)

Embedding a **loose binary** (a loose `<App>` binary next to just its `.sinf`)
makes `mremap_encrypted(cryptid=2)` return **ENOMEM** — the kernel refuses to
create the FairPlay-protected mapping. The fix is the full bundle structure
(binary + `Info.plist` + complete `SC_Info`). ENOMEM happens *before* the licence
check (licence/account failures return EPERM / `-42004` / `-42110`), so it reads
like a VM error; it's really "this isn't a recognizable app bundle." Adding JIT
and `get-task-allow` did **not** help while the binary was loose — only the
bundle structure did.

## Verification (oracle)

Pathway B gives ground truth: decrypting the same binary offline with the
account's 16-byte credential. The on-device `cryptid=2` output for the encrypted
region matched **byte-for-byte**:

```
Pathway B (offline, credential):  9310570f…49fa1
Pathway A (on-device, cryptid=2): 9310570f…49fa1   (identical)
entropy 8.000 → 6.763 · 61256 arm64 ret/retab/pacibsp markers · 1345/1345 pages
```

## Implications for integration (Pathway A, Part 2)

Viable, and WarpSign already has the machinery:

- **Stage** the encrypted app into a prebuilt helper bundle as FairPlayed does —
  full app contents with the `.app` suffix dropped (so code-signing treats it as
  resources, not a nested bundle to re-sign), `.sinf` replicated per
  `SinfReplicationPaths`. The **full bundle structure is mandatory**, not just the
  `.sinf`.
- **Sign** the helper for the user's team (`AppleAccount.sign_ipa`), **install**
  over the loopback tunnel (`install_ipa`), **launch**, and **hand the decrypted
  IPA back** via the existing `CompanionHandoff`/`LoopbackShare` + a URL scheme.
- No JIT/debugserver plumbing needed — simplifies the helper considerably.

## Caveats

- One helper build per target app (the target must be embedded at install time).
- arm64 slices; the device must be signed into the Apple ID that owns the app.
- Confirmed on **iOS 27.2** only. The `cryptid=2` path could still change in a
  future release; re-run this spike when it does.

## Reproduction

Spike sources (throwaway, built on a Mac with Xcode 27): a minimal SwiftUI app +
a C decrypt core (`decrypt_test.c`) embedding one owned encrypted binary as a
bundle, built unsigned with XcodeGen/`xcodebuild`, signed with `warpsign sign`,
installed with `xcrun devicectl`, and auto-running on launch (writing a report to
its Documents container, pulled back with `devicectl device copy from`).
