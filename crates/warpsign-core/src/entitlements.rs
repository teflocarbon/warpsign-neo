//! Decides which portal capabilities to enable for a component and which of
//! its entitlements to drop, based on the capabilities the account offers.
//!
//! Port of Python warpsign's `EntitlementsProcessor`, pinned by
//! `fixtures/entitlements_processor.json`.

use std::collections::{BTreeSet, HashMap};

use plist::Dictionary;
use serde_json::Value as Json;

use crate::ProfileType;

/// Never removed (they're remapped instead), and never tied to a capability
const CORE_IDENTIFIERS: [&str; 3] = [
    "application-identifier",
    "com.apple.developer.team-identifier",
    "keychain-access-groups",
];

/// Always removed, even when the account has a matching capability
const BANNED_ENTITLEMENTS: [&str; 1] = ["com.apple.developer.in-app-payments"];

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EntitlementsPlan {
    /// Portal capability IDs to enable on the App ID
    pub capabilities_to_enable: BTreeSet<String>,
    /// Entitlement keys to strip before signing
    pub entitlements_to_remove: BTreeSet<String>,
}

pub struct EntitlementsProcessor {
    /// Entitlement key -> capability ID
    capabilities: HashMap<String, String>,
}

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

impl EntitlementsProcessor {
    /// `capabilities_response` is the body of `/services-account/v1/capabilities`
    pub fn new(capabilities_response: &Json, profile_type: ProfileType) -> Self {
        let wanted_distribution = match profile_type {
            ProfileType::Development => "Development",
            ProfileType::AdHoc => "Ad hoc",
        };
        let mut capabilities = HashMap::new();

        for capability in array(capabilities_response, "data") {
            let Some(id) = capability.get("id").and_then(Json::as_str) else {
                continue;
            };
            let attributes = capability.get("attributes").unwrap_or(&Json::Null);

            let supports_ios = array(attributes, "supportedSDKs")
                .iter()
                .any(|sdk| sdk.get("displayValue").and_then(Json::as_str) == Some("iOS"));
            let supports_profile_type = array(attributes, "distributionTypes").iter().any(|dist| {
                dist.get("displayValue").and_then(Json::as_str) == Some(wanted_distribution)
            });
            if !supports_ios || !supports_profile_type {
                continue;
            }

            let direct_keys = array(attributes, "entitlements")
                .iter()
                .filter_map(profile_key);
            let setting_keys = array(attributes, "settings")
                .iter()
                .flat_map(|setting| array(setting, "options"))
                .flat_map(|option| array(option, "entitlements"))
                .filter_map(profile_key);

            // A key offered by several capabilities belongs to the last one listed
            for key in direct_keys.chain(setting_keys) {
                capabilities.insert(key.to_owned(), id.to_owned());
            }
        }

        Self { capabilities }
    }

    pub fn process(&self, entitlements: &Dictionary) -> EntitlementsPlan {
        let mut plan = EntitlementsPlan::default();

        for key in entitlements.keys() {
            if CORE_IDENTIFIERS.contains(&key.as_str()) {
                continue;
            }
            let banned = BANNED_ENTITLEMENTS.contains(&key.as_str());
            if banned {
                plan.entitlements_to_remove.insert(key.clone());
            }
            // Python warpsign still enables the capability for a banned entitlement
            // (only the entitlement itself is stripped), so this does too.
            if let Some(capability) = self.capabilities.get(key) {
                plan.capabilities_to_enable.insert(capability.clone());
            } else if !banned {
                plan.entitlements_to_remove.insert(key.clone());
            }
        }

        plan
    }
}
