# Design: Developer Portal login and API

Decisions and known pitfalls for porting Python warpsign's portal client. Agreed October 2026.

## Login

- **Use the web portal flow** (idmsa.apple.com SRP sign-in, then developer.apple.com), as Python warpsign does. No anisette data is needed.
- **Send the `X-APPLE-HC` hashcash header.** Apple has required it on sign-in since February 2023. Python warpsign never sent it, which is the likely cause of its 403s, and repeated refused sign-ins can lock the Apple ID. Flow: GET the sign-in endpoint, read `X-Apple-HC-Bits` and `X-Apple-HC-Challenge`, find the counter whose `1:<bits>:<YYYYMMDDHHMMSS>:<challenge>::<counter>` SHA-1 starts with `<bits>` zero bits, and send that string. Test vector (from fastlane): bits 11, challenge `4d74fb15eb23f465f1f6fcbf534e5877`, date `20230223170600` → `1:11:20230223170600:4d74fb15eb23f465f1f6fcbf534e5877::6373`.
- **A 403 from sign-in is final.** Show Apple's `serviceErrors` message (e.g. `-20209`: account locked) and never retry it. fastlane fixed exactly this in 2.241.0.
- **409 just means 2FA is required.** Support trusted-device codes, and add SMS as a fallback.
- **Fetch the CSRF tokens after 2FA too.** Python warpsign only worked because it logged in twice, and the second login (from the saved session) is what fetched them.
- **Reject free accounts at login**, with a clear message. They aren't supported (see README).

## API

- **Capabilities come from `/v1/capabilities` every time**; never hardcode the entitlement → capability mapping.
- **Port `CAPABILITY_SETTINGS` as-is.** Some capabilities need extra settings, and the API doesn't say which option to choose. As built: neo first sends exactly Python warpsign's request. Only if the portal rejects it, and some requested capabilities have settings that aren't in the table, does it retry once without those, reporting them as skipped. So nothing changes wherever Python's request worked.
- **Surface every portal failure as an error with Apple's message.** Python warpsign returned `None`/`[]` and carried on.
- **Reuse provisioning profiles by name** (regenerate them) instead of creating a new one on every sign.
- **Keep the bundle ID that registration returns** instead of looking it up again in a list limited to 1000 entries.
- **Capture a real `/v1/capabilities` response** (with identifying details removed) as a fixture once there's an account to test with.

## As built (October 2026)

`crates/warpsign-core/src/portal/`, untested against Apple so far:

- **`http.rs`**: the platform supplies a dumb `Transport` (URLSession on iOS). Cookies and redirects are handled in Rust, so a session behaves the same everywhere and serialises to JSON (`SessionState`) for the Keychain. The transport must not follow redirects or touch cookies.
- **`auth.rs`**: widget key (from the `appstoreconnect.apple.com/logout` redirect, as fastlane 2.240 does, since Apple retired Python warpsign's Olympus `app/config` endpoint in September 2026; Olympus stays as a fallback), hashcash, SRP `signin/init` and `signin/complete`, 2FA (device code, SMS), `2sv/trust`, then the portal page for CSRF tokens and `getTeams`. Also takes fresh `csrf`/`csrf_ts` from any response header, as fastlane does.
- **`api.rs`**: the `DeveloperPortalAPI` port. `tests/portal_parity.rs` replays Python's calls (captured by `tools/gen_fixtures.py` into `fixtures/portal_requests.json`) and checks every request's method, URL, headers and body match. Deliberate differences: errors instead of `None`; no `Host`/`Connection`/bogus `Accept-Encoding` headers (the transport owns those); CSRF headers left out when unknown (Python sent `csrf_ts: None`).
- **New, with no Python reference**: certificate creation from a CSR (`POST v1/certificates`, App Store Connect API shape), verified only against mocks. Revoking is deliberately not implemented: the app sends you to developer.apple.com for that.
- **Free accounts**: rejected when `getTeams` returns no teams. Whether a free account gets an empty list or something else is unverified, so a 403 on the first portal request is also reported with Apple's message.
