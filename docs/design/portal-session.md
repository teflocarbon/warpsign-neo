# Design: keeping the Developer Portal session alive

Why WarpSign kept needing a new sign-in, what's been changed, and the bigger options. November 2026. The changes are unit-tested against a scripted stand-in for Apple, but they haven't been tried against a real account yet: see "To check with a real account" below.

## How a session is made of cookies

Signing in on the web (`idmsa.apple.com`, then `developer.apple.com`) leaves three kinds of cookie, which last very different lengths of time:

| Cookie | Set by | Lasts | What it is |
| --- | --- | --- | --- |
| `DES…` (quoted) | `2sv/trust`, after a code | `Max-Age` 30 days | The two-factor trust: a sign-in that sends it isn't asked for a code |
| `myacinfo` | `signin/complete` (`Domain=.apple.com`) | A browser-session cookie; Apple decides on its side how long it's valid. Reported anywhere from hours to about a month, longer with "remember me" | The Apple ID sign-in |
| `dqsid` | The portal (and App Store Connect) | `Max-Age` 1800 (30 minutes), renewed by each response | The portal's own session |

Sources: fastlane's session dumps and issues ([#21840](https://github.com/fastlane/fastlane/discussions/21840): sessions made of `myacinfo`, `DES…` and `dqsid`, and "reduced to just 30 minutes"), a 2020 `spaceauth` dump showing `dqsid` with `max_age: 1800` and the idmsa trust cookie with `2592000`, and CircleCI/Bitrise reports of sessions lasting "30 minutes" to "several hours". None of it is Apple documentation, and it may change.

`X-Apple-ID-Session-Id` and `scnt` only matter during sign-in and two-factor. The portal API doesn't use them. `csrf`/`csrf_ts` are needed for changes, and come back with portal responses.

## Why it seemed to sign out "in seconds"

WarpSign's cookie handling was already correct: every `Set-Cookie` is kept (`HttpClient::remember`), expiry is honoured, the session is saved after each portal call, and the iOS transport never touches cookies itself. The problem was what happened once the 30-minute `dqsid` had lapsed:

1. **A lapsed portal session was treated as a lapsed sign-in.** Any 401 led straight to a full password sign-in (SRP, hashcash and all), or to "sign in again" when the password wasn't remembered. A browser, or Python warpsign, instead opens the portal page again with the still-valid `myacinfo`, gets a new `dqsid`, and carries on. Python warpsign does this before every command (`validate_token`: the session check, then `GET /account/resources`). WarpSign only did it at app launch. So after half an hour of not signing anything, the next portal call failed, whenever that was. To the user that looks like being signed out right after opening the app.
2. **Repeated silent sign-ins have a cost.** Each one is a new Apple ID sign-in. Apple notices frequent ones: it can ask for a code again even with the trust cookie, and refused sign-ins can lock the account. When it asks for a code, silent sign-in pauses and the user has to sign in by hand. So the more often (1) happened, the sooner the user was asked to sign in.
3. **Cookies from failed calls were thrown away.** The session was only saved after a call succeeded, so cookies Apple renewed during a sign that failed part way through were lost.
4. **Being offline at launch looked like being signed out.** If the session check at launch couldn't reach Apple (no network yet), the app showed "Sign In", and nothing tried the saved session again until the next launch.

Not the cause, though checked: the User-Agent is constant per platform (CFNetwork's default in the app, `warpsign/<version>` in the CLI), and the Keychain items are `AfterFirstUnlockThisDeviceOnly`, so background updates can read them.

## What's changed (this branch)

- **Core**: `SessionState::portal_used_at` records when developer.apple.com last answered a request successfully (saved with the session). `Authenticator::keep_alive()` reopens the portal (`GET /account/resources`, then `getTeams`, the same as restoring a session) if it's been unused for 25 minutes (`PORTAL_IDLE_REFRESH`), and otherwise sends nothing. `SessionExpired` from it means `myacinfo` has ended too. Tests: `keeps_the_portal_open_without_a_password` and `portal_requests_mark_the_portal_used` in `tests/portal.rs`.
- **App** (`AccountStore`): every portal call first runs `keepAlive()`. When a call is turned away anyway, it reopens the portal (`checkSession`) before falling back to a password sign-in. The session is saved after every call, whether it worked or not. On coming to the foreground, `resume()` keeps the portal open, or tries the saved session again if the launch check was offline. FFI: `AppleAccount.keep_alive()`.
- **CLI**: commands that use the portal run `keep_alive()` first, as Python warpsign did, and say "run `warpsign login`" if the sign-in has ended.

None of this reaches Apple in a new way: it's the same portal page and `getTeams` request the app already sent at every launch, sent when it's needed instead of only then.

## Bigger options

1. **Keep the whole cookie jar when signing in again** (parity with Python warpsign and fastlane). Both reuse every saved cookie at a new sign-in, while WarpSign sends only the `DES…` trust cookie (`SessionState::trust`). Sending the old `myacinfo` along may make Apple treat it as the same browser coming back, and ask for codes less often. Small change, but it changes what reaches Apple's sign-in, so test it first.
2. **Keep alive in the background.** iOS background refresh runs at times iOS chooses, often hours apart, so it can't keep a 30-minute `dqsid` alive. It might keep `myacinfo` alive, if Apple's limit on it is time since last use rather than a fixed lifetime (not known). Cheap to try once the main fix has been checked, using the existing `BackgroundUpdates` refresh task.
3. **GrandSlam (GSA) tokens, as Xcode, AltStore, SideStore, xtool and Sideloader use.** Sign in once to `gsa.apple.com` (SRP plus two-factor, as now) with *anisette* headers identifying a trusted device. That gives a long-lived token (PET) that can mint Xcode service tokens without a password or code, and those call `developerservices2.apple.com/services/QH65B2/…` (Xcode's plist API) instead of the web portal. Benefits: sessions that last weeks, which is what keeps AltStore signed in. Costs: (a) anisette data. On iOS that means private AuthKit API, or a remote anisette server, which sees identifying device data. (b) A different API: every portal request would be rewritten away from Python warpsign's web endpoints, so the parity fixtures no longer apply, and some web-only features (capabilities by ID, the `.action` endpoints) would need Xcode-API equivalents. (c) Apple shows the sign-in as a new Mac/Xcode. That's a major change. Worth it only if the fixes above still leave sessions too short. Licences: SideStore is AGPL-3.0, AltStore/AltSign and xtool need checking; take ideas only unless the licence is compatible, and credit in NOTICE.
4. **App Store Connect API keys.** Apple's supported answer for automation, with no session at all. But only an Admin can make a key, and as far as known the API doesn't cover everything the signing pipeline does through the web portal (registering app groups and iCloud containers, for one). Worth a look for the CLI, not for the phone.

## To check with a real account

1. **Half an hour idle.** Sign in in the app, sign something, leave the app in the background for 40 minutes, then sign again. Expected: it works without asking for anything. The Network Log should show `GET /account/resources` and `getTeams` just before the signing requests, and **no** `idmsa.apple.com/appleauth/auth/signin` requests.
2. **Which cookie lapses.** In that log, check that the response to `/account/resources` sets a new `dqsid` (or whatever the portal's session cookie is called now; with the CLI, `--http-log` shows cookie names only). If it instead redirects to an `idmsa` sign-in page, `myacinfo` had already ended, and option 1 or 3 is the next step.
3. **How long `myacinfo` lasts.** Note when a reopen first fails with "signed out" (the app then signs in again with the password, or asks). That's Apple's real limit, and it decides whether option 2 is worth doing.
4. **Offline at launch.** Turn on Airplane Mode, launch the app, turn it off, and switch away and back. Expected: it shows signed in without the user doing anything.
5. **CLI.** `warpsign device list`, wait 30 minutes, run it again: expected to work, with the reopen visible in `--http-log`.

Read any log line by line before sharing it: it contains the account's team and devices.
