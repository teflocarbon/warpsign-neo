# Fix dylibs

The dylibs `--inject-warpsign-fix` and `--hide-home-indicator` inject (`crates/warpsign-core/src/warpsign_fix.rs`), built from source here rather than taken prebuilt from Python warpsign:

- `WarpsignFix/` and `WarpsignFixUI/`: copied unchanged from [teflocarbon/WarpsignFix](https://github.com/teflocarbon/WarpsignFix) at commit 2c51879, by the same author, and licensed here under the AGPL-3.0 like the rest of this repository. WarpsignFix uses fishhook (Facebook, BSD-3-Clause, see NOTICE).
  - **WarpsignFix** remaps keychain access groups, app groups and iCloud containers to the new team at runtime.
  - **WarpsignFixUI** is its settings screen. It reads the `remappings.json` that signing writes.
  - `WarpsignFix/README.md` and `WarpsignFixUI/module.modulemap` are kept as they were, but aren't built.
- `ForceHideHomeIndicator/`: written here to do what Python warpsign's prebuilt `ForceHideHomeIndicator.dylib` does, as its source isn't published.

`ios/project.yml` builds each as a framework target, and WarpSign copies their binaries into `FixDylibs/` in its bundle (`WarpsignFix.dylib`, …), which it passes to signing. They're never linked into WarpSign itself. `.github/workflows/ipa.yml` also publishes them, unsigned, as `WarpsignFix-dylibs.zip` on the latest release, for the CLI (`--fix-dylibs`).
