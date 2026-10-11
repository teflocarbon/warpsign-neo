//! People-friendly names for entitlement keys, e.g. "Associated Domains"
//! for `com.apple.developer.associated-domains`.
//!
//! Not part of Python warpsign: this is only for showing entitlements. When
//! signing, names come from the portal's capabilities response; otherwise
//! (and for keys that aren't portal capabilities) from a built-in table.

use std::collections::HashMap;

use serde_json::Value as Json;

/// What an entitlement key is called
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntitlementName {
    /// e.g. "Push Notifications"
    pub name: String,
    pub description: Option<String>,
    /// The portal capability that grants it, e.g. "PUSH_NOTIFICATIONS",
    /// when known from the portal
    pub capability: Option<String>,
}

/// Keys that aren't portal capabilities, and common ones that are, for when
/// there's no portal response to hand
const BUILTIN: &[(&str, &str, Option<&str>)] = &[
    (
        "application-identifier",
        "App ID",
        Some("The team and bundle ID the app is signed for"),
    ),
    (
        "com.apple.developer.team-identifier",
        "Team ID",
        Some("The team the app is signed by"),
    ),
    (
        "keychain-access-groups",
        "Keychain Sharing",
        Some("Keychain items the app can read and write"),
    ),
    (
        "get-task-allow",
        "Debugging",
        Some("Lets debuggers attach to the app"),
    ),
    (
        "beta-reports-active",
        "TestFlight",
        Some("Beta feedback reports"),
    ),
    ("aps-environment", "Push Notifications", None),
    ("com.apple.security.application-groups", "App Groups", None),
    ("application-groups", "App Groups", None),
    (
        "com.apple.developer.icloud-container-identifiers",
        "iCloud",
        None,
    ),
    (
        "com.apple.developer.icloud-container-development-container-identifiers",
        "iCloud",
        None,
    ),
    (
        "com.apple.developer.icloud-container-environment",
        "iCloud",
        None,
    ),
    (
        "com.apple.developer.ubiquity-container-identifiers",
        "iCloud",
        None,
    ),
    ("com.apple.developer.icloud-services", "iCloud", None),
    (
        "com.apple.developer.ubiquity-kvstore-identifier",
        "iCloud",
        Some("iCloud key-value storage"),
    ),
    (
        "com.apple.developer.associated-domains",
        "Associated Domains",
        None,
    ),
    ("com.apple.developer.in-app-payments", "Apple Pay", None),
    ("com.apple.developer.healthkit", "HealthKit", None),
    ("com.apple.developer.healthkit.access", "HealthKit", None),
    (
        "com.apple.developer.healthkit.background-delivery",
        "HealthKit",
        Some("HealthKit background delivery"),
    ),
    ("com.apple.developer.homekit", "HomeKit", None),
    ("com.apple.developer.siri", "SiriKit", None),
    ("com.apple.developer.pass-type-identifiers", "Wallet", None),
    (
        "com.apple.developer.nfc.readersession.formats",
        "NFC Tag Reading",
        None,
    ),
    (
        "com.apple.developer.applesignin",
        "Sign in with Apple",
        None,
    ),
    (
        "com.apple.developer.networking.networkextension",
        "Network Extensions",
        None,
    ),
    (
        "com.apple.developer.networking.vpn.api",
        "Personal VPN",
        None,
    ),
    (
        "com.apple.developer.networking.wifi-info",
        "Access Wi-Fi Information",
        None,
    ),
    (
        "com.apple.developer.networking.HotspotConfiguration",
        "Hotspot",
        None,
    ),
    (
        "com.apple.developer.networking.multipath",
        "Multipath",
        None,
    ),
    (
        "com.apple.developer.default-data-protection",
        "Data Protection",
        None,
    ),
    ("inter-app-audio", "Inter-App Audio", None),
    ("com.apple.developer.ClassKit-environment", "ClassKit", None),
    (
        "com.apple.developer.authentication-services.autofill-credential-provider",
        "AutoFill Credential Provider",
        None,
    ),
    ("com.apple.developer.user-fonts", "Fonts", None),
    (
        "com.apple.developer.devicecheck.appattest-environment",
        "App Attest",
        None,
    ),
    (
        "com.apple.developer.family-controls",
        "Family Controls",
        None,
    ),
    ("com.apple.developer.weatherkit", "WeatherKit", None),
    (
        "com.apple.developer.kernel.extended-virtual-addressing",
        "Extended Virtual Address Space",
        None,
    ),
    (
        "com.apple.developer.kernel.increased-memory-limit",
        "Increased Memory Limit",
        None,
    ),
    (
        "com.apple.developer.group-session",
        "Group Activities",
        None,
    ),
    ("com.apple.developer.push-to-talk", "Push to Talk", None),
    (
        "com.apple.developer.usernotifications.time-sensitive",
        "Time Sensitive Notifications",
        None,
    ),
    (
        "com.apple.developer.usernotifications.communication",
        "Communication Notifications",
        None,
    ),
    ("com.apple.developer.game-center", "Game Center", None),
    (
        "com.apple.developer.coremedia.hls.low-latency",
        "Low Latency HLS",
        None,
    ),
    (
        "com.apple.developer.proximity-reader.payment.acceptance",
        "Tap to Pay on iPhone",
        None,
    ),
];

/// The built-in name for an entitlement key, if there is one
pub fn builtin_name(key: &str) -> Option<EntitlementName> {
    BUILTIN
        .iter()
        .find(|(builtin, _, _)| *builtin == key)
        .map(|(_, name, description)| EntitlementName {
            name: (*name).to_string(),
            description: description.map(String::from),
            capability: None,
        })
}

/// Names for entitlement keys, from a portal capabilities response with the
/// built-in table as a fallback
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EntitlementNames {
    from_portal: HashMap<String, EntitlementName>,
}

impl EntitlementNames {
    /// Built-in names only
    pub fn builtin() -> Self {
        Self::default()
    }

    /// `response` is the body of `/services-account/v1/capabilities`. Each
    /// capability names the keys it grants (`attributes.entitlements` and
    /// its settings' options' entitlements); the first capability listed
    /// for a key names it.
    pub fn from_capabilities(response: &Json) -> Self {
        fn array<'a>(value: &'a Json, key: &str) -> &'a [Json] {
            value
                .get(key)
                .and_then(Json::as_array)
                .map_or(&[], Vec::as_slice)
        }
        fn profile_key(entitlement: &Json) -> Option<&str> {
            entitlement
                .get("profileKey")
                .and_then(Json::as_str)
                .filter(|key| !key.is_empty())
        }

        let mut from_portal = HashMap::new();
        for capability in array(response, "data") {
            let attributes = capability.get("attributes").unwrap_or(&Json::Null);
            let Some(name) = attributes
                .get("name")
                .and_then(Json::as_str)
                .filter(|name| !name.is_empty())
            else {
                continue;
            };
            let description = attributes
                .get("description")
                .and_then(Json::as_str)
                .filter(|description| !description.is_empty());
            let id = capability.get("id").and_then(Json::as_str);
            let direct_keys = array(attributes, "entitlements")
                .iter()
                .filter_map(profile_key);
            let setting_keys = array(attributes, "settings")
                .iter()
                .flat_map(|setting| array(setting, "options"))
                .flat_map(|option| array(option, "entitlements"))
                .filter_map(profile_key);
            for key in direct_keys.chain(setting_keys) {
                from_portal
                    .entry(key.to_string())
                    .or_insert_with(|| EntitlementName {
                        name: name.to_string(),
                        description: description.map(String::from),
                        capability: id.map(String::from),
                    });
            }
        }
        Self { from_portal }
    }

    /// The name for `key`: the portal's, then the built-in one
    pub fn get(&self, key: &str) -> Option<EntitlementName> {
        self.from_portal
            .get(key)
            .cloned()
            .or_else(|| builtin_name(key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_keys_from_the_portal_then_the_builtin_table() {
        let response: Json =
            serde_json::from_str(include_str!("../../../fixtures/capabilities_response.json"))
                .unwrap();
        let names = EntitlementNames::from_capabilities(&response);
        let push = names.get("aps-environment").unwrap();
        assert_eq!(push.name, "Push Notifications");
        assert_eq!(push.capability.as_deref(), Some("PUSH_NOTIFICATIONS"));
        assert!(push.description.is_some());
        // The first capability listed for a key names it
        assert_eq!(
            names
                .get("com.apple.developer.associated-domains")
                .unwrap()
                .name,
            "Associated Domains"
        );
        assert_eq!(
            names
                .get("com.apple.developer.ubiquity-kvstore-identifier")
                .unwrap()
                .capability
                .as_deref(),
            Some("ICLOUD")
        );
        // Not a portal capability
        let keychain = names.get("keychain-access-groups").unwrap();
        assert_eq!(keychain.name, "Keychain Sharing");
        assert_eq!(keychain.capability, None);
        assert_eq!(names.get("com.example.unknown"), None);

        let builtin = EntitlementNames::builtin();
        assert_eq!(builtin.get("aps-environment").unwrap().capability, None);
        assert_eq!(builtin.get("get-task-allow").unwrap().name, "Debugging");
    }
}
