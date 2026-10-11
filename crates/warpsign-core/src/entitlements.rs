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

/// Whether `key` is always removed before signing, whatever the account
/// offers
pub fn is_banned(key: &str) -> bool {
    BANNED_ENTITLEMENTS.contains(&key)
}

pub struct EntitlementsProcessor {
    /// Entitlement key -> capability ID
    capabilities: HashMap<String, String>,
    /// Entitlement key -> whether its capability must be requested from
    /// Apple (`canRequestFromPortal`), and if so, its details
    request_only: HashMap<String, Option<RequestOnly>>,
}

/// A capability a team has to ask Apple for before it can use it, e.g.
/// Family Controls (Distribution)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestOnly {
    pub capability: String,
    pub name: String,
    /// Where to ask for it
    pub request_url: Option<String>,
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
        let mut request_only_by_key: HashMap<String, Option<RequestOnly>> = HashMap::new();

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

            // Some capabilities are only granted on request from Apple, e.g.
            // Family Controls (Distribution), which offers the same key as
            // Family Controls. Prefer one that needs no request; among
            // equals the last listed wins (as Python warpsign, dbed168).
            let request_only = (attributes.get("canRequestFromPortal") == Some(&Json::Bool(true)))
                .then(|| RequestOnly {
                    capability: id.to_owned(),
                    name: attributes
                        .get("name")
                        .and_then(Json::as_str)
                        .unwrap_or(id)
                        .to_owned(),
                    request_url: attributes
                        .get("requestFormUrl")
                        .and_then(Json::as_str)
                        .map(String::from),
                });
            for key in direct_keys.chain(setting_keys) {
                if request_only.is_some() && matches!(request_only_by_key.get(key), Some(None)) {
                    continue;
                }
                request_only_by_key.insert(key.to_owned(), request_only.clone());
                capabilities.insert(key.to_owned(), id.to_owned());
            }
        }

        Self {
            capabilities,
            request_only: request_only_by_key,
        }
    }

    /// The capability `key` maps to, if any
    pub fn capability(&self, key: &str) -> Option<&str> {
        self.capabilities.get(key).map(String::as_str)
    }

    /// The capability `key` maps to, if the team must request it from Apple
    pub fn request_only(&self, key: &str) -> Option<&RequestOnly> {
        self.request_only.get(key).and_then(Option::as_ref)
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
