//! Maps an app's identifiers (bundle IDs, app groups, iCloud containers,
//! keychain groups, team IDs) onto the signing team.
//!
//! Port of Python warpsign's `BundleMapping`. Output must match it exactly:
//! encoded IDs are what gets registered on the developer portal, so any
//! difference would give re-signed apps a new bundle ID and lose their data.
//! Behaviour is pinned by `fixtures/bundle_mapping.json` and
//! `fixtures/random_ids.json`.

use std::collections::{BTreeSet, HashMap, HashSet};

use indexmap::IndexMap;
use plist::{Dictionary, Value};

use crate::ProfileType;
use crate::pyrandom::PyRandom;

const ICLOUD_CONTAINER_KEYS: [&str; 3] = [
    "com.apple.developer.icloud-container-identifiers",
    "com.apple.developer.ubiquity-container-identifiers",
    "com.apple.developer.icloud-container-development-container-identifiers",
];

const RANDOM_ID_ALPHABET: [char; 36] = [
    'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's',
    't', 'u', 'v', 'w', 'x', 'y', 'z', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9',
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IdType {
    ICloud,
    Keychain,
    AppGroup,
    Bundle,
}

impl IdType {
    /// Name used by Python warpsign's `IDType` enum
    pub fn name(self) -> &'static str {
        match self {
            IdType::ICloud => "ICLOUD",
            IdType::Keychain => "KEYCHAIN",
            IdType::AppGroup => "APP_GROUP",
            IdType::Bundle => "BUNDLE",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdMapping {
    pub new_id: String,
    pub id_type: IdType,
}

#[derive(Clone)]
pub struct BundleMapping {
    team_id: String,
    original_team_ids: Vec<String>,
    original_main_bundle_id: String,
    main_bundle_id: String,
    encode_ids: bool,
    mappings: IndexMap<String, IdMapping>,
    random_id_cache: HashMap<String, String>,
    pub force_original_id: bool,
    pub profile_type: ProfileType,
    pub registered_identifiers: HashSet<String>,
}

fn char_len(s: &str) -> usize {
    s.chars().count()
}

impl BundleMapping {
    pub fn new(
        original_team_ids: Vec<String>,
        new_team_id: impl Into<String>,
        original_main_bundle_id: impl Into<String>,
        encode_ids: bool,
    ) -> Self {
        let original_main_bundle_id = original_main_bundle_id.into();
        let mut mapping = Self {
            team_id: new_team_id.into(),
            original_team_ids,
            main_bundle_id: original_main_bundle_id.clone(),
            original_main_bundle_id: original_main_bundle_id.clone(),
            encode_ids,
            mappings: IndexMap::new(),
            random_id_cache: HashMap::new(),
            force_original_id: false,
            profile_type: ProfileType::Development,
            registered_identifiers: HashSet::new(),
        };
        mapping.main_bundle_id = mapping.gen_random_id(&original_main_bundle_id);
        let main_bundle_id = mapping.main_bundle_id.clone();
        mapping.store_mapping(&original_main_bundle_id, main_bundle_id, IdType::Bundle);
        mapping
    }

    pub fn team_id(&self) -> &str {
        &self.team_id
    }

    pub fn original_team_ids(&self) -> &[String] {
        &self.original_team_ids
    }

    pub fn main_bundle_id(&self) -> &str {
        &self.main_bundle_id
    }

    pub fn mappings(&self) -> &IndexMap<String, IdMapping> {
        &self.mappings
    }

    fn store_mapping(&mut self, original_id: &str, new_id: String, id_type: IdType) {
        self.mappings
            .insert(original_id.to_owned(), IdMapping { new_id, id_type });
    }

    /// Returns the matching original team ID and the rest of `id`
    fn team_id_match<'a>(&self, id: &'a str) -> Option<(&str, &'a str)> {
        self.original_team_ids
            .iter()
            .find(|team_id| id.starts_with(team_id.as_str()))
            .map(|team_id| (team_id.as_str(), &id[team_id.len()..]))
    }

    /// Deterministically encode each dot-separated part, keeping its length.
    /// Seeded by the part and the new team ID, so the same app always gets
    /// the same IDs on the same team.
    pub fn gen_random_id(&mut self, original_id: &str) -> String {
        if !self.encode_ids {
            return original_id.to_owned();
        }

        let cache_key = format!("{original_id}:{}", self.team_id);
        if let Some(cached) = self.random_id_cache.get(&cache_key) {
            return cached.clone();
        }

        let result = original_id
            .split('.')
            .map(|part| {
                PyRandom::from_str_seed(&format!("{part}{}", self.team_id))
                    .choices(&RANDOM_ID_ALPHABET, char_len(part))
            })
            .collect::<Vec<_>>()
            .join(".");

        self.random_id_cache.insert(cache_key, result.clone());
        result
    }

    fn handle_bundle_id(&mut self, original_id: &str) -> String {
        let mut new_id = if let Some((_, remaining)) = self.team_id_match(original_id) {
            format!("{}{remaining}", self.team_id)
        } else if let Some(suffix) = original_id.strip_prefix(&self.original_main_bundle_id) {
            format!("{}{suffix}", self.main_bundle_id)
        } else {
            self.gen_random_id(original_id)
        };

        // Binary patching needs equal lengths: pad with "x" or truncate
        let original_len = char_len(original_id);
        let new_len = char_len(&new_id);
        if new_len < original_len {
            new_id.extend(std::iter::repeat_n('x', original_len - new_len));
        } else if new_len > original_len {
            new_id = new_id.chars().take(original_len).collect();
        }
        new_id
    }

    pub fn map_id(&mut self, original_id: &str, id_type: IdType) -> String {
        if original_id == "com.apple.token" || original_id.is_empty() || !self.encode_ids {
            return original_id.to_owned();
        }
        if let Some(mapping) = self.mappings.get(original_id) {
            return mapping.new_id.clone();
        }

        let new_id = match id_type {
            IdType::ICloud => {
                let base_id = if original_id.starts_with("iCloud.") {
                    // Python's str.replace swaps every occurrence, not just the prefix
                    original_id.replace("iCloud.", "")
                } else if let Some((_, remaining)) = self.team_id_match(original_id) {
                    // Drop the "." after the team ID
                    remaining.chars().skip(1).collect()
                } else {
                    original_id.to_owned()
                };
                format!("iCloud.{}", self.gen_random_id(&base_id))
            }
            IdType::Keychain => match self.team_id_match(original_id) {
                Some((_, remaining)) => format!("{}{remaining}", self.team_id),
                None => format!("{}.{original_id}", self.team_id),
            },
            IdType::AppGroup => {
                let base_id = original_id.replace("group.", "");
                match self.team_id_match(&base_id) {
                    Some((_, remaining)) => format!("group.{}{remaining}", self.team_id),
                    None => format!("group.{}", self.gen_random_id(&base_id)),
                }
            }
            IdType::Bundle => self.handle_bundle_id(original_id),
        };

        self.store_mapping(original_id, new_id.clone(), id_type);
        new_id
    }

    /// Map a bundle identifier (CFBundleIdentifier of the app or an extension)
    pub fn map_bundle_id(&mut self, original_id: &str) -> String {
        self.map_id(original_id, IdType::Bundle)
    }

    fn map_list_entitlement(&mut self, entitlements: &mut Dictionary, key: &str, id_type: IdType) {
        let Some(value) = entitlements.get(key) else {
            return;
        };
        let values = match value {
            Value::Array(values) => values.clone(),
            other => vec![other.clone()],
        };
        let mapped = values
            .into_iter()
            .map(|value| match value {
                Value::String(id) => Value::String(self.map_id(&id, id_type)),
                other => other,
            })
            .collect();
        entitlements.insert(key.to_owned(), Value::Array(mapped));
    }

    /// Remap entitlements onto the new team and IDs.
    /// `override_bundle_id` is the bundle ID written to the component's Info.plist.
    pub fn map_entitlements(
        &mut self,
        entitlements: &Dictionary,
        override_bundle_id: Option<&str>,
    ) -> Dictionary {
        let mut result = entitlements.clone();

        if let Some(app_id) = result.get("application-identifier").cloned() {
            let bundle_id = match override_bundle_id {
                Some(bundle_id) => Some(bundle_id.to_owned()),
                None => app_id
                    .as_string()
                    .and_then(|app_id| app_id.split_once('.'))
                    .map(|(_, bundle_id)| self.map_id(bundle_id, IdType::Bundle)),
            };
            if let Some(bundle_id) = bundle_id {
                result.insert(
                    "application-identifier".to_owned(),
                    Value::String(format!("{}.{bundle_id}", self.team_id)),
                );
            }
        }

        result.insert(
            "com.apple.developer.team-identifier".to_owned(),
            Value::String(self.team_id.clone()),
        );

        if result.contains_key("aps-environment") {
            let environment = match self.profile_type {
                ProfileType::Development => "development",
                ProfileType::AdHoc => "production",
            };
            result.insert(
                "aps-environment".to_owned(),
                Value::String(environment.to_owned()),
            );
        }

        self.map_list_entitlement(&mut result, "keychain-access-groups", IdType::Keychain);
        self.map_list_entitlement(
            &mut result,
            "com.apple.security.application-groups",
            IdType::AppGroup,
        );
        self.map_list_entitlement(&mut result, "application-groups", IdType::AppGroup);
        for key in ICLOUD_CONTAINER_KEYS {
            self.map_list_entitlement(&mut result, key, IdType::ICloud);
        }

        let kvstore_key = "com.apple.developer.ubiquity-kvstore-identifier";
        if let Some(Value::String(kvstore_id)) = result.get(kvstore_key).cloned() {
            let parts: Vec<&str> = kvstore_id.split('.').collect();
            let new_kvstore_id = if parts.len() == 2 && char_len(parts[1]) == 1 {
                // Short "TEAMID.x" form: only the team ID changes
                Some(format!("{}.{}", self.team_id, parts[1]))
            } else {
                kvstore_id.split_once('.').map(|(_, bundle_id)| {
                    let new_bundle_id = self.map_id(bundle_id, IdType::Bundle);
                    format!("{}.{new_bundle_id}", self.team_id)
                })
            };
            if let Some(new_kvstore_id) = new_kvstore_id {
                result.insert(kvstore_key.to_owned(), Value::String(new_kvstore_id));
            }
        }

        result
    }

    /// Same-length replacements to apply to binaries and Info.plists
    pub fn binary_patches(&self) -> IndexMap<String, String> {
        let mut patches = IndexMap::new();
        let mut seen_values = HashSet::new();

        let mut team_ids: Vec<&String> = self.original_team_ids.iter().collect();
        team_ids.sort_by_key(|team_id| std::cmp::Reverse(char_len(team_id)));
        for team_id in team_ids {
            if char_len(team_id) == char_len(&self.team_id) {
                patches.insert(team_id.clone(), self.team_id.clone());
            }
        }

        for (original_id, mapping) in &self.mappings {
            if !self.registered_identifiers.contains(original_id)
                || char_len(original_id) != char_len(&mapping.new_id)
            {
                continue;
            }
            if self.force_original_id && *original_id == self.original_main_bundle_id {
                continue;
            }
            if seen_values.insert(mapping.new_id.clone()) {
                patches.insert(original_id.clone(), mapping.new_id.clone());
            }
        }

        if self.force_original_id {
            patches.shift_remove(&self.original_main_bundle_id);
        }
        patches
    }

    /// App groups and iCloud containers to register, already mapped
    pub fn extract_resources_from_entitlements(
        &mut self,
        entitlements: &Dictionary,
    ) -> (BTreeSet<String>, BTreeSet<String>) {
        let mut app_groups = BTreeSet::new();
        let mut icloud_containers = BTreeSet::new();

        if let Some(Value::Array(groups)) =
            entitlements.get("com.apple.security.application-groups")
        {
            for group in groups.iter().filter_map(Value::as_string) {
                app_groups.insert(self.map_id(group, IdType::AppGroup));
            }
        }

        for key in ICLOUD_CONTAINER_KEYS {
            if let Some(Value::Array(containers)) = entitlements.get(key) {
                for container in containers.iter().filter_map(Value::as_string) {
                    icloud_containers.insert(self.map_id(container, IdType::ICloud));
                }
            }
        }

        (app_groups, icloud_containers)
    }
}
