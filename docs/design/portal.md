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
- **Port `CAPABILITY_SETTINGS` as-is.** Some capabilities need extra settings, and the API doesn't say which option to choose. If a capability has settings that aren't in the table, skip just that capability with a clear warning, rather than failing the whole update.
- **Surface every portal failure as an error with Apple's message.** Python warpsign returned `None`/`[]` and carried on.
- **Reuse provisioning profiles by name** (regenerate them) instead of creating a new one on every sign.
- **Keep the bundle ID that registration returns** instead of looking it up again in a list limited to 1000 entries.
- **Capture a real `/v1/capabilities` response** (with identifying details removed) as a fixture once there's an account to test with.
