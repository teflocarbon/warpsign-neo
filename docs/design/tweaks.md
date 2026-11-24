# Design: tweaks (injection, extraction, Tweak Depot)

Proposed November 2026, decisions agreed (below), not built yet. This covers injecting tweaks into an IPA as it's signed, finding the tweaks already injected into an IPA and saving copies of them, and the Tweak Depot that keeps them. A later section sketches adding tweaks from GitHub releases. Decisions, what is still open and the order of work are at the end.

## Background

### How jailbreak-free injectors work

Every maintained injector uses the same recipe. They differ only in details.

| | cyan (pyzule-rw) | Feather | Impactor | Azule | LiveContainer |
|---|---|---|---|---|---|
| Licence | Unlicense | GPL-3.0 | MIT (Rust) | none (archived) | AGPL-3.0 |
| Hooking runtime | ElleKit, renamed `CydiaSubstrate.framework` | ElleKit, renamed `CydiaSubstrate.framework` | ElleKit .deb | saurik's Substrate, repackaged | ElleKit, renamed, `dlopen`ed |
| Load command | `LC_LOAD_WEAK_DYLIB` | weak by default | weak | strong by default | none: a loader `dlopen`s the tweaks |
| Path | `@rpath/X`, adds `LC_RPATH @executable_path/Frameworks` | `@executable_path/Frameworks/X` | `@rpath/X` | `@rpath/X` | n/a |
| Binaries patched | main executable | main, plus extensions as an option | main | main | n/a |
| Substrate path rewrite | any dependency containing `substrate.`, `orion.` or `cephei.` | only `/Library/Frameworks/CydiaSubstrate.framework/…` | same as Feather | Substrate and substitute | none needed |
| .deb payload | everything that looks like code or a bundle | `DynamicLibraries`, `Frameworks`, `Application Support`, with and without `var/jb/` | as Feather, plus `usr/lib` | `dpkg -x` | not implemented |
| Filter plists | ignored | ignored | ignored | honoured | n/a |
| List or remove | no | yes: `@rpath`/`@executable_path` loads in the main executable | yes | no | per-app folder |

Notes:

- **ElleKit** (BSD-3-Clause, evelyneee/ellekit) implements the Substrate and libhooker APIs. Every maintained injector ships it as `Frameworks/CydiaSubstrate.framework/CydiaSubstrate`, so tweaks linked against Substrate find it. Saurik's Substrate is proprietary, and libhooker's open source release is BSD-4 (its advertising clause conflicts with the GPL family), so neither is an option.
- **Objective-C hooks** (`MSHookMessageEx`, Logos `%hook` on classes) only swizzle methods, so they work on stock iOS.
- **C function hooks** (`MSHookFunction`) rewrite executable pages. On stock iOS that's only allowed when the process is `CS_DEBUGGED`, which means JIT (StikDebug) or an attached debugger. Without that, the hook fails or the app is killed. ElleKit's hardware-breakpoint fallback (6 hooks) is commented out. Tweaks built with Theos's `internal` generator, or with Orion's fishhook backend, avoid this.
- **The header-padding trick:** a load command is added by writing it into the free space between the end of the load commands (`sizeofcmds`) and the first section's file offset, then bumping `ncmds` and `sizeofcmds`. Linkers leave plenty of room there (`-headerpad`). Feather (via Zsign) and Impactor both do this, on every slice of a fat binary. Nothing else in the file moves.
- **iOS 26+:** Feather patches stale arm64e slices in Substrate-era frameworks, which otherwise crash on load. The patch is LiveContainer's `LCPatchMachOFixupARM64eSlice` (AGPL-3.0, so it can be followed with a NOTICE credit). We should ship arm64-only ElleKit and check injected binaries for this.
- **Encrypted binaries** have to be decrypted first. cyan and Python warpsign both refuse an encrypted binary. WarpSign has decryption already (`fairplay.rs`, and Pathway A on the device).

The ytkace release linked as an example (v1.1.9) has `…_rootless.deb` and `…_roothide.deb`, plus two prebuilt YouTube IPAs. There's no loose `.dylib`. So a GitHub source has to unpack rootless debs, and the prebuilt IPAs are exactly what extraction is for.

### What Python warpsign does

Python warpsign injects only its own three prebuilt dylibs, behind two flags, using LIEF:

- `--inject-warpsign-fix` adds WarpsignFix and WarpsignFixUI, from [teflocarbon/WarpsignFix](https://github.com/teflocarbon/WarpsignFix). WarpsignFix uses fishhook to rewrite keychain access groups, app groups and CloudKit containers to the new team's. WarpsignFixUI is its in-app settings and help screen.
- `--hide-home-indicator` adds ForceHideHomeIndicator, a separate small tweak whose source isn't in that repo. It has no `.deb` or user-tweak support. The README lists these flags as not ported yet, along with `--patch-liquid-glass`. The facts that matter for parity:

- The dylibs are copied to `<App>.app/Frameworks/` (always created), only if a file with the same name isn't there already.
- The load command is a strong `LC_LOAD_DYLIB` at `@loader_path/` + `../` × depth + `Frameworks/<name>`. So the main binary gets `@loader_path/Frameworks/WarpsignFix.dylib`, and `PlugIns/E.appex/E` gets `@loader_path/../../Frameworks/WarpsignFix.dylib`.
- WarpsignFix goes into every non-`.dylib` component executable. The other two go into the main binary only.
- A list of conflicting fix-up dylibs (`CONFLICTING_DYLIBS`: `sideloadFixerLol.dylib`, `zxPluginsInject.dylib`, …) is removed from `Frameworks/`. In each slice, the first load command whose path contains one of those names is removed.
- `remappings.json` (keychain groups, app groups, iCloud containers, team IDs) is written for WarpsignFix to read.

None of this changes the design below, but it does decide the order of work. The Mach-O editor that tweaks need is also what porting those flags needs. Porting them is pure parity work (fixtures from the generator, as for everything else), and it proves the editor on code we already know works on devices. WarpsignFix also has a second job here: it's how an app tells the user that a tweak didn't load (see Injection).

## Pieces

All the logic goes in `warpsign-core`, so the CLI and the app share it, and in time the depot format too. Proposed modules:

```
crates/warpsign-core/src/tweaks/
  macho_edit.rs  load-command editing: add, remove and change dylib paths, add rpaths, all slices
  deb.rs         ar + control.tar.* + data.tar.* (gz, xz, lzma, bz2, zst), control-file parsing
  scan.rs        find the tweaks in an extracted app (extraction)
  inject.rs      put depot tweaks into an extracted app (injection)
  depot.rs       the Tweak Depot: index and content-addressed storage
```

New pure-Rust dependencies, all MIT or Apache: `ar`, `tar`, `flate2` (rust backend), `lzma-rs` or `xz2` (prefer a pure-Rust one, so there's nothing native to cross-compile for iOS), `bzip2-rs`, `ruzstd`. Mach-O parsing stays hand-written, as in `fairplay.rs`. There's little to it, and we need write access, which goblin doesn't give.

### Mach-O editing (`macho_edit.rs`)

Thin and fat, `MH_MAGIC_64` only (arm64 and arm64e slices). Operations:

- `load_dylibs(binary)`: every `LC_LOAD_DYLIB`, `LC_LOAD_WEAK_DYLIB`, `LC_REEXPORT_DYLIB` and `LC_LAZY_LOAD_DYLIB`, with its index, ordinal, path and versions, plus `LC_ID_DYLIB` and `LC_RPATH`s.
- `add_dylib(path, weak)`: written into the header padding. If there isn't room, it fails rather than moving segments.
- `remove_dylib(path)`: removes the command and shifts the later ones down. Library ordinals of the commands after it change, so this refuses (or warns) when the binary imports symbols from a later library through two-level namespace binds. Injected tweaks are almost always the last loads, so in practice this rarely matters. Feather and Impactor don't check.
- `change_dylib(old, new)` (`install_name_tool -change`), `add_rpath`, `set_id`.
- Each operation runs on every slice, and is skipped or refused when a slice is still encrypted (`cryptid != 0`).

The code signature isn't touched: the Sign step replaces it. Testing builds Mach-O bytes in the tests themselves, as `fairplay.rs` does. No real binaries go in the repo.

### The tweak runtime

ElleKit is shipped as `CydiaSubstrate.framework`, taken from a pinned ElleKit release `.deb`, as Feather does:

- **In the app:** `ipa.yml` downloads the pinned `.deb`, checks its SHA-256, and unpacks the framework into WarpSign's bundle. The binary isn't committed.
- **In the CLI:** the CLI downloads the same pinned URL and checks the same hash the first time it injects, keeping the framework in the config directory.
- **Notice:** the BSD-3 notice goes in NOTICE and next to the framework.

At injection:

- **Add the runtime:** if any tweak depends on Substrate (or libhooker, substitute, or ElleKit itself) and the app has no `Frameworks/CydiaSubstrate.framework`, it's added.
- **Rewrite dependencies:** any dependency of an injected binary whose path contains `substrate` (case-insensitive: `/Library/Frameworks/CydiaSubstrate.framework/CydiaSubstrate`, `/usr/lib/libsubstrate.dylib`, `@rpath/CydiaSubstrate.dylib`, rootless `/var/jb/...`), and likewise libhooker and substitute, becomes `@rpath/CydiaSubstrate.framework/CydiaSubstrate`. This is cyan's broad rule, not Feather's one exact path.
- **Other libraries:** Orion, Cephei and AltList are tweak dependencies like any other. They're in the depot only if a deb or an extraction brought them. Injection checks that each injected binary's non-system dependencies will resolve, and says what's missing, instead of shipping an app that crashes at launch.

### Injection (`inject.rs`, a new sign step)

Injection takes `SignOptions::tweaks: Vec<TweakRef>` (depot entries, or loose files for the CLI). It's split in two, like Python's staging and patching:

- **Copying the files** goes in `Extract`, before the components are listed.
- **Editing the load commands** is a new `Step::InjectTweaks`, after `Decrypt` and before `PatchBinaries`, as load commands can only be edited in a decrypted binary.

Between them, they do this:

1. Refuses if the main executable is still encrypted and won't be decrypted. It says to decrypt first, or does so, given a credential or Pathway A.
2. Copies each tweak's files into place: dylibs and frameworks to `Frameworks/`, `.bundle`s to the app root, `.appex` to `PlugIns/`. A file with the same name already there is a conflict. Replacing it is the user's choice, shown before signing.
3. Rewrites the Substrate dependencies and adds ElleKit if needed.
4. Adds `LC_LOAD_WEAK_DYLIB @rpath/<name>` to the main executable for each dylib, or `@rpath/<X>.framework/<X>` for each injected framework the tweak itself loads. It also adds `LC_RPATH @executable_path/Frameworks` if the app doesn't resolve `@rpath` there already. Weak is the default, as in cyan, Feather and Impactor: a missing or broken tweak then doesn't stop the app launching. Strong is an option.
5. Extensions get the same load commands only when asked (Feather's "inject into extensions"), with `@executable_path/../../Frameworks`.
6. Writes the list of what it injected to `tweaks.json` in the app (name, file, depot ID), next to `remappings.json`.
7. Logs exactly what it did, so the Activity log shows it.

**Telling the user a tweak didn't load:** weak loading fails silently, so WarpsignFix checks for it. When an app has tweaks, WarpsignFix is injected too, strongly, as Python injects it. At launch it reads `tweaks.json` and compares it with the images dyld loaded (`_dyld_image_count`, `_dyld_get_image_name`). If one is missing, WarpsignFixUI shows an alert naming the tweak, and dyld's reason when it can be had (`dlopen` on the path, then `dlerror`). It does this once per launch, with "Don't show again" for that tweak. This needs a change in the WarpsignFix repo, and that repo needs a licence before CI can build it.

**Bundle IDs:** injected files are added before the components are listed, as Python stages its dylibs before analysis. So `PatchBinaries` bundle-ID-patches them like any other nested dylib. A tweak that compares the bundle ID with `com.google.ios.youtube` then compares it with the ID the app is really signed with, which is what it reads at runtime. Python warpsign does the same for every nested dylib except its own fixes. Injected files then sign like any other nested code: `sign_app_bundle` already signs every dylib and framework inside the app.

The tweaks a signed copy was made with are recorded with its signing options. Then `Updater` re-applies them when it re-signs an update, the same way it reuses the other options. That's the feature people will notice most: a tweaked YouTube that stays tweaked across updates.

### Extraction (`scan.rs`)

The input is an extracted app (Library IPA, or a file for the CLI). The output is a list of `FoundTweak`s with a confidence score and the reasons behind it.

**Candidates:** every app-relative load (`@rpath`, `@executable_path` or `@loader_path`) in the main executable, plus extensions, that resolves to a file in the app. System paths (`/System/…`, `/usr/lib/…`) are ignored, except Substrate paths that weren't rewritten, which show as "broken injection".

**Signals that a candidate was injected, not linked by the developer**, strongest first:

1. **The app imports no symbols from it.** For each library ordinal, count the binds and imports in `LC_DYLD_CHAINED_FIXUPS`, or in `LC_DYLD_INFO(_ONLY)` bind opcodes. A framework the developer linked supplies symbols. A tweak is loaded for its constructor alone, so nothing binds to it. This is the most reliable signal, and it can't be faked without relinking.
2. **It depends on a hooking runtime:** CydiaSubstrate, libsubstrate, ElleKit, libhooker, substitute, or Orion. It also imports `MSHookMessageEx`, `MSHookFunction`, `LHHookFunctions`, `fishhook`'s `rebind_symbols`, and so on.
3. **Its load command comes after the linker's trailing commands.** ld writes dylib loads in one run, before `LC_RPATH`, `LC_FUNCTION_STARTS`, `LC_DATA_IN_CODE` and `LC_CODE_SIGNATURE`. A load after those was appended later.
4. **It's weak-linked to an app-relative path.** Apps rarely weak-link their own embedded frameworks, and injectors default to weak.
5. **Known names:** the runtimes above, Python warpsign's `CONFLICTING_DYLIBS`, and common sideloading fix-ups.
6. **A different signer** from the main executable's, or ad-hoc. This only works on IPAs that haven't been re-signed as a whole, so it's weak on its own.

**What's in a tweak:** a tweak is the injected binary, plus the closure of its non-system dependencies that the original app doesn't use (frameworks the tweak shipped, such as Orion or Cephei), plus resources:

- **Bundles:** `.bundle`s at the app root whose name matches the tweak (`YTKACE.dylib` and `YTKACE.bundle`), or whose name appears in the tweak's `__cstring` section (`NSBundle bundleWithPath:` strings).
- **Runtimes:** ElleKit or Substrate is its own entry, "tweak runtime". It's never saved per tweak: injection adds WarpSign's own copy.

**Metadata we can recover**, since there's no control file inside an IPA:

- File name, install name (`LC_ID_DYLIB`), and that command's `current_version` (often 0 for Theos builds).
- `LC_SOURCE_VERSION`, and for frameworks the `Info.plist` (`CFBundleIdentifier`, `CFBundleShortVersionString`).
- Architectures, minimum iOS (`LC_BUILD_VERSION`), size, and SHA-256 (the depot's identity).
- Signer (team, or ad-hoc), and the dependencies it links.
- **What it hooks:** class names from `MSHookMessageEx` and `objc_getClass` call sites are hard to get statically. The cheap version lists the tweak's own ObjC classes (`__objc_classlist`), and the class names it references (`__objc_classrefs`) that aren't its own: usually the app classes it hooks. "Hooks YTPlayerViewController, YTMainAppVideoPlayerOverlayView…" is a useful summary.
- **Needs JIT:** imports `MSHookFunction`, `MSHookMemory` or `LHHookFunctions`. The tweak works fully only with JIT (StikDebug).
- **Filter plists:** not in IPAs (injectors drop them), so there's none to recover. The source app's bundle ID stands in for one.

**Extraction only copies.** It saves the tweaks to the depot so they can be used elsewhere, and never changes the IPA they came from.

**Removing tweaks** is a separate action, for an IPA that's wanted without them. The same scan finds them, then their load commands are dropped and their files deleted, writing a new IPA. In the app it's Remove Tweaks… on the Library app's menu, which asks first because it replaces the Library's copy. In the CLI it's `warpsign tweaks strip`.

Signal thresholds need testing on real tweaked IPAs (the ytkace IPAs are a good public sample), with results described in the doc, not committed.

### Tweak Depot (`depot.rs`)

Storage is content-addressed, so the same dylib extracted from three apps is stored once, with three origins:

```
TweakDepot/
  depot.json                 index (below)
  blobs/<sha256>/            one tweak's files, laid out as they go in an app:
    Frameworks/YTKACE.dylib
    YTKACE.bundle/…
```

- **On iOS:** `Application Support/TweakDepot/`. Excluded from iCloud backup, as the Library's IPAs are. Its size is shown in Settings > Storage.
- **For the CLI:** `<config dir>/tweak-depot/`. `--depot-dir` overrides it.

An entry in `depot.json` records:

| Field | What |
|---|---|
| `id` | SHA-256 of the main binary (identity, dedup) |
| `name`, `displayName` | file stem; user-editable name |
| `kind` | `dylib`, `framework`, `bundle-only`, or `package` (a whole deb) |
| `version` | from the deb's control file, framework Info.plist, or `LC_ID_DYLIB`; may be unknown |
| `files` | relative paths and sizes in the blob |
| `dependencies` | runtime needed (substrate/none), other libraries it links, which depot entries satisfy them |
| `facts` | archs, minimum iOS, signer, hooks, needs-JIT, as scanned |
| `origins` | one or more (below) |
| `addedAt`, `notes` | |

Origins say where a tweak came from. That's the "app it came from" part:

- `extracted`: the app's bundle ID, name, version, build, and icon (a copy, so it survives the Library app being deleted). Also the IPA's file name, the Library entry's ID if it's still there, and when.
- `deb`: package ID, name, version, author, description, depends, and section, all from `control`. Also the variant (rootless, rootful or roothide), and the file or URL it came from.
- `github`: repo, release tag and asset name, for updates (see below).
- `file`: a loose `.dylib`, `.framework` or `.bundle` the user imported.

Packages (debs) are entries of their own. Their payload is unpacked once at import, using the same placement rules as injection: strip a leading `var/jb/`, then take `Library/MobileSubstrate/DynamicLibraries/*.dylib`, `Library/Frameworks/*.framework`, `usr/lib/*.dylib` and `Library/Application Support/**.bundle`. The filter plist next to each dylib is kept. It's shown ("made for com.google.ios.youtube"), and used to warn when the tweak is injected into a different app, never to block.

The depot is defined in Rust (types, index, import, export) and exposed through `warpsign-ffi`. Then the app and the CLI read and write the same format, and the tests cover it. The Swift side is a thin `@Observable TweakDepotStore` over it, as `LibraryStore` is over the library.

## The iOS app

- **The Tweak Depot is part of the Library, not a tab of its own**, so the tab bar doesn't grow to five. A segmented control at the top of the Library switches between **Apps** and **Tweaks**, as Music switches Library views. Search and **+** follow the selection. For Tweaks, **+** offers Import File (`.deb`, `.dylib`, a zipped `.framework`). Apps' filters (Updates, Installed…) only show for Apps.
- **The Tweaks view** is a list grouped by where tweaks came from. Each extracted app is a section with its icon, name, bundle ID and version ("From YouTube 21.47.6"), with its tweaks below. Packages, GitHub tweaks and imported files have their own sections. Rows show name, version, size, and badges: needs JIT, needs Substrate, and update available. An empty depot explains where tweaks come from, with buttons for each.
- **The fallback** if the segmented control feels crowded beside the activity strip and downloads: a Tweak Depot row at the top of the Apps list, showing a count ("12 tweaks from 4 apps"), which opens the same view.
- **Tweak detail:** name (renameable), version, kind and size, the origin(s) with a link to the Library app if it's still there, and the files (a tree). Also what it hooks, its dependencies and whether they're satisfied, archs and minimum iOS, the filter's bundle IDs, and notes. Actions: Inject Into… (choose a Library app, open Signing with it selected), Export (share as `.dylib`, or a zip for multi-file tweaks), and Delete.
- **Library app page:** a Tweaks section from the scan, done once at import and kept with the entry like `encryption`: "3 tweaks injected: YTKACE, YouTubeHeader…, ElleKit (runtime)". Extract… opens a sheet listing what was found, each with confidence and reasons ("not used by the app · links CydiaSubstrate · appended load"), all tweaks preselected and the runtime unselected. Saving copies them to the depot; the IPA is untouched. Remove Tweaks… is a separate item in the app's menu.
- **Signing options:** a Tweaks row. Its picker is the depot, with tweaks extracted from this same bundle ID or filtered for it at the top. It also offers inject into extensions, and strong instead of weak. A warning shows for an encrypted app (with Decrypt), for unmet dependencies, and for tweaks that need JIT. The choice is saved with the signed copy, so updates keep it.

## The CLI

```
warpsign tweaks scan <ipa>                         # list injected tweaks, with reasons
warpsign tweaks extract <ipa> [--only NAME…] [--strip -o clean.ipa]
warpsign tweaks strip <ipa> [--only NAME…] -o <out.ipa>
warpsign depot list | show <id> | import <file.deb|.dylib|dir> | export <id> -o <path> | remove <id>
warpsign sign … --tweak <depot-id|name|path>… [--tweak-extensions] [--strong-tweaks]
```

`--json` on `scan` and `depot list`, as for `inspect`.

## Later: tweaks from GitHub releases

A **Tweak Repos** section in the Sources tab, below the AltStore sources. Both are places things come from, and the Sources tab's add flow can tell them apart from the link: a GitHub repo or release link is a tweak repo, and anything else is an AltStore source. Tweak repos list releases, not apps:

- **Adding a source:** add `github.com/owner/repo` (or a release URL, which is reduced to the repo). WarpSign reads `GET https://api.github.com/repos/{owner}/{repo}/releases`, unauthenticated, with `ETag`/`If-None-Match` (a 304 is free against the 60-requests-an-hour limit). Release notes are shown as Markdown.
- **Assets:** sorted into tweaks (`.deb`, `.dylib`, `.zip` holding a `.framework`) and apps (`.ipa`). When a release has several debs, the rootless variant is preferred (`iphoneos-arm64` in its control file), then rootful. Roothide debs are offered last, with a warning: their binaries can link roothide's own library.
- **Get:** downloads through `DownloadManager`, and imports into the depot with a `github` origin. An `.ipa` asset goes to the Library, with an offer to scan it for tweaks.
- **Updates:** the newest release by tag (using `is_newer_version`) against the version in the depot. An update replaces the depot entry's blob. Library apps signed with that tweak get it at their next update, or with a Re-sign With Updated Tweaks action.
- **Later still, same model:** APT repos (`Release` + `Packages.gz`, as Sileo reads them) and plain links to a `.deb`.

There's no token support at first, so nothing secret is stored. A personal token for the higher rate limit could later go in the Keychain, never in logs or the Network Log.

## Public-repo and licence notes

- No tweak, deb or IPA goes into the repo or CI logs. Tests build Mach-O, ar and tar bytes in code. Real-world checks are described in prose: "seen on a tweaked YouTube IPA in November 2026".
- ElleKit (BSD-3) comes from a pinned release `.deb`, downloaded and checked against its SHA-256 in CI and by the CLI, not committed. It's credited in NOTICE, and its licence text ships with the framework.
- Code may be followed from Impactor's `tweak.rs`/`macho.rs` (MIT), cyan (Unlicense), Feather (GPL-3.0), LiveContainer's arm64e fix-up (AGPL-3.0) and Zsign (MIT), each with a NOTICE credit. Azule (no licence), saurik's Substrate (proprietary) and libhooker (BSD-4) are ideas only, or nothing.
- **WarpsignFix and WarpsignFixUI** are the owner's ([teflocarbon/WarpsignFix](https://github.com/teflocarbon/WarpsignFix)), and fine to ship.
  - That repo has no licence yet. It needs one (AGPL-3.0, to match) before `ipa.yml` builds it from source.
  - Its tracked `xcuserdata/` folder should go before anything here builds from it.
  - ForceHideHomeIndicator's source still needs finding; until then it's the prebuilt binary from Python warpsign.

## Decisions (November 2026)

1. **Order:** the Python parity flags (`--inject-warpsign-fix`, `--hide-home-indicator`) come right after the Mach-O editor, before tweaks. They use the same editor and prove it.
2. **WarpsignFix** (and WarpsignFixUI) ship with WarpSign, built from the owner's repo once it has a licence. It also becomes the check that tells the user when a tweak didn't load.
3. **Tweaks load weakly** by default, with that check behind them. Strong is an option.
4. **Injected tweaks are bundle-ID-patched** like every other nested dylib, as in Python.
5. **The Tweak Depot is part of the Library** (Apps | Tweaks), and GitHub tweak repos are in the Sources tab. There's no fifth tab.
6. **ElleKit** comes from its pinned release `.deb`.
7. **Extraction only copies** to the depot. Removing tweaks from an IPA is a separate action.
8. **The depot is Rust**, in `warpsign-core`, shared by the app (through FFI) and the CLI.

## Still open

- ForceHideHomeIndicator's source: where it is, and whether it moves into the WarpsignFix repo.
- The WarpsignFix repo's licence (AGPL-3.0 proposed), and its missing-tweak alert (a change there).
- Whether the Apps | Tweaks control, or the fallback row, looks right once it's on a phone.

## Order of work

Core and CLI first. Each piece is testable on Windows and Linux without a device, and the Swift UI comes once the core holds up.

1. `macho_edit.rs`: read, add, remove and change load commands and rpaths, fat and thin, with byte-built tests.
2. **Parity:** port `--inject-warpsign-fix` and `--hide-home-indicator` (generator cases and fixtures first, then the port), with `remappings.json`. WarpsignFix's binaries come from Python warpsign until its repo has a licence and CI builds it.
3. `scan.rs` and `warpsign tweaks scan <ipa>`: extraction's detection with reasons, checked against the public ytkace IPAs and described in the doc.
4. `deb.rs` and `depot.rs`, with `warpsign depot import/list/show` and `warpsign tweaks extract`.

Then injection (the staging and `Step::InjectTweaks`, ElleKit from its `.deb`, `--tweak`, `tweaks.json` and the missing-tweak alert), tested on a device. Then the Library's Tweaks view, and the Library-app and Signing sections. GitHub tweak repos come last.
