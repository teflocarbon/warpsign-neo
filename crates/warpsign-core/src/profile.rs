//! Provisioning profiles (`.mobileprovision`): a CMS-signed plist.

use cryptographic_message_syntax::SignedData;
use plist::{Dictionary, Value};

#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("not a provisioning profile (CMS): {0}")]
    Cms(String),
    #[error("provisioning profile has no embedded content")]
    NoContent,
    #[error("provisioning profile content is not a valid plist: {0}")]
    Plist(String),
}

/// The parts of a provisioning profile warpsign uses
#[derive(Debug, Clone, PartialEq)]
pub struct ProvisioningProfile {
    pub name: Option<String>,
    pub uuid: Option<String>,
    pub team_ids: Vec<String>,
    pub entitlements: Dictionary,
    pub provisioned_devices: Vec<String>,
    /// Whether apps signed with this profile check in with Apple (PPQ) on
    /// first launch. Teams created after 6 June 2021 get this; older teams
    /// don't, and their profiles omit the key.
    pub ppq_check: bool,
    /// The whole plist, for anything not exposed above
    pub plist: Dictionary,
}

impl ProvisioningProfile {
    /// Parse the contents of a `.mobileprovision` file.
    ///
    /// The CMS signature isn't verified: profiles come straight from Apple's
    /// portal, and the device checks the signature at install time anyway.
    pub fn parse(data: &[u8]) -> Result<Self, ProfileError> {
        let signed = SignedData::parse_ber(data).map_err(|e| ProfileError::Cms(e.to_string()))?;
        let content = signed.signed_content().ok_or(ProfileError::NoContent)?;
        let plist = match Value::from_reader(std::io::Cursor::new(content)) {
            Ok(Value::Dictionary(dict)) => dict,
            Ok(_) => return Err(ProfileError::Plist("not a dictionary".into())),
            Err(e) => return Err(ProfileError::Plist(e.to_string())),
        };

        let string = |key: &str| plist.get(key).and_then(Value::as_string).map(String::from);
        let strings = |key: &str| {
            plist
                .get(key)
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_string)
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default()
        };

        Ok(Self {
            name: string("Name"),
            uuid: string("UUID"),
            team_ids: strings("TeamIdentifier"),
            entitlements: plist
                .get("Entitlements")
                .and_then(Value::as_dictionary)
                .cloned()
                .unwrap_or_default(),
            provisioned_devices: strings("ProvisionedDevices"),
            ppq_check: plist
                .get("PPQCheck")
                .and_then(Value::as_boolean)
                .unwrap_or(false),
            plist,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "this account's provisioning profiles have PPQ checking on ({profile}), so apps check in \
     with Apple on first launch. Keeping the original bundle ID (force original ID) exposes \
     App Store bundle IDs to that check and can get the account banned. Sign with encoded \
     IDs instead."
)]
pub struct PpqRisk {
    /// Name (or UUID) of the profile that has PPQ checking on
    pub profile: String,
}

/// Refuse to keep original bundle IDs when any of the profiles has PPQ
/// checking on. Call with the profiles created for the app before signing.
pub fn check_ppq_risk(
    profiles: &[ProvisioningProfile],
    force_original_id: bool,
) -> Result<(), PpqRisk> {
    if !force_original_id {
        return Ok(());
    }
    match profiles.iter().find(|profile| profile.ppq_check) {
        Some(profile) => Err(PpqRisk {
            profile: profile
                .name
                .clone()
                .or_else(|| profile.uuid.clone())
                .unwrap_or_else(|| "unnamed profile".into()),
        }),
        None => Ok(()),
    }
}
