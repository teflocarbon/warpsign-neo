# Unicorn for WarpSign

ipatool signs App Store requests by running Apple's own arm64 code in
[Unicorn](https://github.com/unicorn-engine/unicorn), a CPU emulator.

**This directory is licensed under the GNU General Public License, version 2
only**, like Unicorn itself, not under WarpSign's AGPL-3.0. The two licences
can't be combined in one program, so WarpSign doesn't contain Unicorn:

- The app has no Unicorn code. When the App Store first needs it, the app
  downloads `libunicorn-iphoneos.dylib` from this repository's
  `unicorn-<version>` release, checks it against the SHA-256 pinned in
  `ios/WarpSign/Platform/UnicornRuntime.swift`, signs it with the user's
  development certificate (iOS only loads code signed by the app's own team)
  and loads it with `dlopen`. ipatool finds it with `dlsym`.
- The library is built from the sources here, and published with them.

## What's here

- `build.sh`: builds `target/unicorn/{iphoneos,iphonesimulator}/libunicorn.dylib`
  from Unicorn 2.1.4 (checked by hash), ipatool's iOS patch
  (`tools/patches/unicorn-ios.patch` in ipatool's Go module, MIT-licensed
  changes to GPLv2 code) and `splitwx.patch`. Only arm64 guests are built:
  ipatool's emulator runs arm64 code since 2.8.0.
- `splitwx.patch`: split W^X for iOS 26 and later, below.

## Split W^X

Unicorn translates guest code into host code as it runs (QEMU's TCG), so it
needs JIT. ipatool's iOS patch flips its code buffer between writable and
executable with `mprotect`, which iOS 26's Trusted Execution Monitor (TXM)
no longer allows: executable memory comes only from regions a debugger
prepared (StikDebug's universal script), and is written through a second,
writable mapping of the same memory.

`splitwx.patch` adds that mode, after the approach of QEMU 6's
`--enable-splitwx` (QEMU is GPLv2-or-later), in a form small enough for
Unicorn's QEMU 5:

- `uc_splitwx_configure(rw, rx, size, slice)`, called once before any
  `uc_open`, gives Unicorn one region mapped twice. Each engine takes a slice
  of it as its code buffer and gives it back on `uc_close`.
- TCG keeps every code pointer in its writable form. Only what executes
  sees executable addresses: the call into generated code, the pointers
  `lookup_tb_ptr` returns, PC-relative displacements (computed from where
  the code runs) and literal code addresses. Return addresses coming back
  from generated code (`GETPC()`) are turned back into writable ones before
  they're looked up.
- Without `uc_splitwx_configure`, nothing changes.

Only the aarch64 host backend is converted, and only POSIX hosts.

WarpSign's side, asking the debugger for the region and mapping it again
writable, is `crates/warpsign-ffi/src/emulator.rs`.

### Tested

On macOS (Apple silicon), with a region from `mach_vm_allocate`, a
`mach_vm_remap` alias made read-and-execute only, and Unicorn's own unit
tests run without forking (the alias isn't inherited): all of `test_arm64`
(aarch64 build), and `test_x86` and `test_ctl` (x86 build) pass; `test_mem`
passes except `test_mem_read_and_write_large_memory_block`, which crashes the
same way without split W^X. In the iOS simulator, WarpSign loads the
library, sets up the region and emulates a short arm64 loop.

## Publishing a build

Releases publish the files `build.sh` made, unsigned, renamed
`libunicorn-iphoneos.dylib` and `libunicorn-iphonesimulator.dylib`, in a
release named `unicorn-<version>` (`UnicornRuntime.version`) with these
sources. Any new build changes the hashes: update them in
`UnicornRuntime.swift`, and bump the version so apps don't fetch a file
that doesn't match.
