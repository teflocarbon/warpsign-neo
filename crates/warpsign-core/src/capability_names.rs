//! People-friendly names for entitlement keys, e.g. "Associated Domains"
//! for `com.apple.developer.associated-domains`.
//!
//! Not part of Python warpsign: this is only for showing entitlements.
//! Names come from the Developer Portal's capabilities response, which
//! names every capability and lists the entitlement keys it grants. Only
//! keys that are never portal capabilities have names written here.

use std::collections::BTreeMap;

use serde_json::Value as Json;

/// What an entitlement key is called
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntitlementName {
    /// e.g. "Push Notifications"
    pub name: String,
    pub description: Option<String>,
    /// The portal capability that grants it, e.g. "PUSH_NOTIFICATIONS".
    /// None for keys that aren't capabilities.
    pub capability: Option<String>,
}

/// Keys every signed app has (or may have) that no portal capability
/// grants, so the capabilities response never names them
const NOT_CAPABILITIES: &[(&str, &str, &str)] = &[
    (
        "application-identifier",
        "App ID",
        "The team and bundle ID the app is signed for",
    ),
    (
        "com.apple.developer.team-identifier",
        "Team ID",
        "The team the app is signed by",
    ),
    (
        "keychain-access-groups",
        "Keychain Groups",
        "Keychain items the app can read and write",
    ),
    (
        "get-task-allow",
        "Debugging",
        "Lets debuggers attach to the app",
    ),
    (
        "beta-reports-active",
        "TestFlight Reports",
        "Set by App Store distribution profiles",
    ),
];

fn not_capability(key: &str) -> Option<EntitlementName> {
    NOT_CAPABILITIES
        .iter()
        .find(|(known, _, _)| *known == key)
        .map(|(_, name, description)| EntitlementName {
            name: (*name).to_string(),
            description: Some((*description).to_string()),
            capability: None,
        })
}

/// One capability in the portal's capabilities response
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalCapability {
    /// e.g. "PUSH_NOTIFICATIONS"
    pub id: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    /// The entitlement keys it grants: those in `attributes.entitlements`,
    /// then those in its settings' options' entitlements (where iCloud,
    /// Data Protection and others list theirs), without repeats
    pub entitlement_keys: Vec<String>,
}

/// Every capability in a `/services-account/v1/capabilities` response, in
/// order. Anything missing, null or of an unexpected type (attributes,
/// settings, options, entitlements, keys) counts as absent.
pub fn capabilities(response: &Json) -> Vec<PortalCapability> {
    fn array<'a>(value: &'a Json, key: &str) -> &'a [Json] {
        value
            .get(key)
            .and_then(Json::as_array)
            .map_or(&[], Vec::as_slice)
    }
    fn string(value: &Json, key: &str) -> Option<String> {
        value
            .get(key)
            .and_then(Json::as_str)
            .filter(|value| !value.is_empty())
            .map(String::from)
    }

    array(response, "data")
        .iter()
        .filter(|capability| capability.is_object())
        .map(|capability| {
            let attributes = capability.get("attributes").unwrap_or(&Json::Null);
            let direct = array(attributes, "entitlements").iter();
            let in_settings = array(attributes, "settings")
                .iter()
                .flat_map(|setting| array(setting, "options"))
                .flat_map(|option| array(option, "entitlements"));
            let mut entitlement_keys: Vec<String> = Vec::new();
            for key in direct
                .chain(in_settings)
                .filter_map(|entitlement| string(entitlement, "profileKey"))
            {
                if !entitlement_keys.contains(&key) {
                    entitlement_keys.push(key);
                }
            }
            PortalCapability {
                id: string(capability, "id"),
                name: string(attributes, "name"),
                description: string(attributes, "description"),
                entitlement_keys,
            }
        })
        .collect()
}

/// Names for entitlement keys, from a portal capabilities response
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EntitlementNames {
    from_portal: BTreeMap<String, EntitlementName>,
}

impl EntitlementNames {
    /// No capabilities response to hand: only keys that are never
    /// capabilities have names
    pub fn without_portal() -> Self {
        Self::default()
    }

    /// `response` is the body of `/services-account/v1/capabilities` (see
    /// [`capabilities`]). When several capabilities list a key, the first
    /// one names it. Capabilities without a name name nothing.
    pub fn from_capabilities(response: &Json) -> Self {
        let mut from_portal = BTreeMap::new();
        for capability in capabilities(response) {
            let Some(name) = capability.name else {
                continue;
            };
            for key in capability.entitlement_keys {
                from_portal.entry(key).or_insert_with(|| EntitlementName {
                    name: name.clone(),
                    description: capability.description.clone(),
                    capability: capability.id.clone(),
                });
            }
        }
        Self { from_portal }
    }

    /// The name for `key`, if the portal (or this module, for keys that
    /// are never capabilities) gives it one
    pub fn get(&self, key: &str) -> Option<EntitlementName> {
        self.from_portal
            .get(key)
            .cloned()
            .or_else(|| not_capability(key))
    }

    /// Every key with a name, sorted by key
    pub fn all(&self) -> Vec<(String, EntitlementName)> {
        let mut all = self.from_portal.clone();
        for (key, _, _) in NOT_CAPABILITIES {
            if let Some(name) = not_capability(key) {
                all.entry((*key).to_string()).or_insert(name);
            }
        }
        all.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Json {
        serde_json::from_str(include_str!("../../../fixtures/capabilities_response.json")).unwrap()
    }

    #[test]
    fn names_keys_from_the_portals_capabilities() {
        let names = EntitlementNames::from_capabilities(&fixture());
        let push = names.get("aps-environment").unwrap();
        assert_eq!(push.name, "Push Notifications");
        assert_eq!(push.capability.as_deref(), Some("PUSH_NOTIFICATIONS"));
        assert_eq!(
            push.description.as_deref(),
            Some("Push Notifications capability")
        );
        assert_eq!(
            names
                .get("com.apple.security.application-groups")
                .unwrap()
                .name,
            "App Groups"
        );
        // Listed by two capabilities: the first names it
        assert_eq!(
            names
                .get("com.apple.developer.associated-domains")
                .unwrap()
                .name,
            "Associated Domains"
        );
        // Only under iCloud's settings options
        let kvstore = names
            .get("com.apple.developer.ubiquity-kvstore-identifier")
            .unwrap();
        assert_eq!(kvstore.name, "iCloud");
        assert_eq!(kvstore.capability.as_deref(), Some("ICLOUD"));
        assert_eq!(
            names
                .get("com.apple.developer.icloud-services")
                .unwrap()
                .capability
                .as_deref(),
            Some("ICLOUD")
        );
        assert_eq!(names.get("com.example.unknown"), None);
    }

    #[test]
    fn lists_capabilities_whatever_their_shape() {
        let listed = capabilities(&fixture());
        assert_eq!(listed.len(), 9);
        assert_eq!(
            listed[2].entitlement_keys,
            [
                "com.apple.developer.icloud-container-identifiers",
                "com.apple.developer.ubiquity-container-identifiers",
                "com.apple.developer.icloud-services",
                "com.apple.developer.ubiquity-kvstore-identifier",
            ]
        );
        assert!(listed[8].entitlement_keys.is_empty());

        // Missing, null, empty and odd pieces are skipped, not fatal
        let odd = serde_json::json!({"data": [
            {"id": "A", "attributes": {"name": "A", "settings": null, "entitlements": null}},
            {"id": "B", "attributes": {"name": "B", "settings": [
                {"options": null},
                {"options": [{"entitlements": null}, {}, {"entitlements": [
                    {"profileKey": "b.key"}, {"profileKey": null}, {}, 7
                ]}]},
                {}
            ]}},
            {"id": "C"},
            {"attributes": {"name": "No ID", "entitlements": [{"profileKey": "d.key"}]}},
            "not a capability",
            null
        ]});
        let listed = capabilities(&odd);
        assert_eq!(listed.len(), 4);
        assert!(listed[0].entitlement_keys.is_empty());
        assert_eq!(listed[1].entitlement_keys, ["b.key"]);
        assert_eq!(listed[2].name, None);
        let names = EntitlementNames::from_capabilities(&odd);
        assert_eq!(names.get("b.key").unwrap().name, "B");
        assert_eq!(names.get("d.key").unwrap().capability, None);
        assert!(capabilities(&serde_json::json!({"data": null})).is_empty());
        assert!(capabilities(&Json::Null).is_empty());
    }

    #[test]
    fn names_keys_that_are_never_capabilities() {
        for names in [
            EntitlementNames::from_capabilities(&fixture()),
            EntitlementNames::without_portal(),
        ] {
            let debugging = names.get("get-task-allow").unwrap();
            assert_eq!(debugging.name, "Debugging");
            assert_eq!(debugging.capability, None);
            assert_eq!(names.get("application-identifier").unwrap().name, "App ID");
        }
        // Nothing the portal names is known without it
        let offline = EntitlementNames::without_portal();
        assert_eq!(offline.get("aps-environment"), None);
        assert_eq!(offline.all().len(), NOT_CAPABILITIES.len());

        let all = EntitlementNames::from_capabilities(&fixture()).all();
        assert!(all.iter().any(|(key, _)| key == "aps-environment"));
        assert!(all.iter().any(|(key, _)| key == "keychain-access-groups"));
        assert!(all.windows(2).all(|pair| pair[0].0 < pair[1].0));
    }
}
