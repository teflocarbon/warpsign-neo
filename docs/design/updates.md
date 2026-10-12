# Design: automatic updates

Agreed October 2026. Apps downloaded from a source update by themselves, the way the App Store does: overnight while the phone charges, with a notification only when an update needs the user. Each app can opt out, and the whole thing can be turned off in Settings > Automatic Updates.

## What has an update

- A library app remembers the source it was downloaded from (`sourceURL`) and the bundle ID the source lists it under (`sourceAppId`). Apps downloaded before this was recorded are linked on the next check, by the source's name (the app's `origin`) and bundle ID.
- A check refreshes each linked source and compares its newest version with the library's, using `is_newer_version` in `source.rs`: dotted numeric versions compared part by part (a leading `v` ignored, missing parts as 0), a release above its own pre-release, and builds breaking ties. A version it can't read is never treated as newer, so a source with odd version strings doesn't update anything.
- Checks run when WarpSign comes to the foreground (at most hourly), from Check Now, and from a background app refresh every few hours (`BGAppRefreshTask`).

## Making an update

`Updater.update` does what you'd do by hand:

1. Download the new IPA and replace the library's copy of it (`replaceIPA`). The library entry, its signed copies, its custom icon and its history stay; a different bundle ID is refused.
2. Sign it with the options the app's newest signed copy was made with, and its custom icon (`SignJob`, shared with the Signing screen). An app that has never been signed stops here: it's downloaded, and the user is told to sign it once. The app's older signed copies are removed: they're of a version the library no longer has, and each is a whole IPA.
3. Install it with the pairing file. Over the web, iOS asks before every install, so it depends who started the update: an automatic one stops at signed and waits in the library (as with Install Updates off, and not as a problem); one the user started shows the install screen (`PendingInstallSheet`, one app at a time for Update All) with iOS's prompt.

Which apps can update is shown in the library, under each app's name (`UpdateOriginLabel`): its source, "updates off", a source WarpSign hasn't found in Sources, bundled with WarpSign (Companion), or imported from a file, which can't update. The app's page has an Updates section saying the same, with a link to its listing in the source, the newest version there, whether it's up to date, and when sources were last checked; the source's listing has an In Your Library section going the other way.

WarpSign itself is never updated this way: installing over the running app locks up installd until the phone restarts. It updates through WarpSign Companion.

## Downloads

Every download (from sources, from a link the user gives with + → Download from Link, and updates) goes through `DownloadManager`, a background URLSession: downloads carry on while WarpSign is in the background or suspended, and if iOS ends WarpSign, it finishes them and relaunches WarpSign to hand them over (`handleEventsForBackgroundURLSession`). Force-quitting cancels them; on the next launch they're shown as interrupted, and resume where they stopped when the server allows it. The list is saved (`Application Support/Downloads/downloads.json`), so a download never just disappears.

The library shows each at the top: size, speed (smoothed) and time left, "Waiting for the network" when nothing has arrived for a few seconds, and pause, resume, cancel and retry. Failures are explained: the server's status, a link that opened a web page instead of an app, no connection, an untrusted certificate. Links are https only. An interrupted update download isn't kept: the next check starts it again.

## When it runs

- When the user taps Update or Update All: in the foreground, with the progress in the Dynamic Island (`BGContinuedProcessingTask`).
- By itself: when a background check finds updates for apps set to update automatically, it asks for a `BGProcessingTask` that needs external power and a network connection. iOS runs that when the phone is idle and charging, usually overnight. Nothing is shown while it runs; a notification is posted for each update that failed or stopped short of installed.
- iOS decides exactly when either runs, and runs neither for an app that was force-quit or with Background App Refresh off.
- The session, signing identities and pairing file are Keychain items readable after first unlock, so updates work with the phone locked (not after a restart before it's unlocked, when iOS doesn't run them anyway).

## Settings

Settings > Automatic Updates, beyond the main switch:

- **Only While Charging** (on): the processing task's `requiresExternalPower`. Off, iOS runs it whenever it likes, which can close an app you're using while it installs.
- **Wi-Fi Only** (on): iOS can require a network but not Wi-Fi, so an automatic run checks first (`NWPathMonitor`) and waits for another chance on cellular or a network iOS counts as expensive. Low Data Mode always holds automatic updates back.
- **Install Updates** (on): off, automatic updates stop at signed and wait in the library. Updates started by hand always install. Over the web it's off and greyed out, with a link to set up a pairing file.
- **New Apps Update Automatically** (on): the per-app switch's starting value for apps downloaded (or linked to their source) from now on.
- **Notify When Apps Update** (off): one notification for what updated. Problems always notify.
- **Skip Betas** (on): a source's pre-releases (letters after the numbers, `is_pre_release`) aren't offered unless the library's version is one; the newest release is offered instead. Applies to updates started by hand too.

Per app, besides its switch: **Skip This Version** (on the app's page, or swiping its update in the library) stops offering that version; later versions are offered as usual.

The background task identifiers are read from Info.plist (`$(PRODUCT_BUNDLE_IDENTIFIER).refresh` and `.updates`), so they match however WarpSign was signed, as for the continued-processing task.
