//! The Swift-facing API of warpsign-core, exported with uniffi.
//!
//! Keep this layer thin: plain records in, plain records out, and the logic
//! in warpsign-core. Functions that do I/O block, so call them off the main
//! thread in Swift.

use std::path::Path;

use plist::Value;
use warpsign_core::inspect::{inspect_app, slash_path, team_ids};
use warpsign_core::ipa::extract_ipa;
use warpsign_core::profile::ProvisioningProfile;

uniffi::setup_scaffolding!();

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum WarpsignError {
    #[error("{message}")]
    Ipa { message: String },
    #[error("{message}")]
    Inspect { message: String },
    #[error("{message}")]
    Profile { message: String },
}

/// A signable part of an app
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Component {
    /// Path inside the app ("" for the app itself), always "/"-separated
    pub path: String,
    pub bundle_id: String,
    /// Apps and extensions get their own App ID and profile
    pub is_primary: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AppInfo {
    /// Where the app was extracted to
    pub app_path: String,
    pub bundle_id: String,
    pub display_name: Option<String>,
    pub version: Option<String>,
    pub components: Vec<Component>,
    /// Team IDs the app is currently signed for
    pub team_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ProfileInfo {
    pub name: Option<String>,
    pub uuid: Option<String>,
    pub team_ids: Vec<String>,
    pub device_count: u32,
    /// Apps signed with this profile check in with Apple on first launch
    pub ppq_check: bool,
}

/// warpsign-core's version
#[uniffi::export]
pub fn core_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Extract an IPA into `work_dir` (which must be empty or not exist) and
/// describe the app inside it
#[uniffi::export]
pub fn inspect_ipa(ipa_path: String, work_dir: String) -> Result<AppInfo, WarpsignError> {
    let app = extract_ipa(Path::new(&ipa_path), Path::new(&work_dir)).map_err(|e| {
        WarpsignError::Ipa {
            message: e.to_string(),
        }
    })?;
    let components = inspect_app(&app).map_err(|e| WarpsignError::Inspect {
        message: e.to_string(),
    })?;
    let main =
        components
            .iter()
            .find(|c| c.is_main_app())
            .ok_or_else(|| WarpsignError::Inspect {
                message: "the app has no main executable".into(),
            })?;
    let info_string = |key: &str| {
        main.info_plist
            .get(key)
            .and_then(Value::as_string)
            .map(String::from)
    };

    Ok(AppInfo {
        app_path: app.display().to_string(),
        bundle_id: main.bundle_id.clone(),
        display_name: info_string("CFBundleDisplayName").or_else(|| info_string("CFBundleName")),
        version: info_string("CFBundleShortVersionString"),
        team_ids: team_ids(&components),
        components: components
            .iter()
            .map(|c| Component {
                path: slash_path(&c.path),
                bundle_id: c.bundle_id.clone(),
                is_primary: c.is_primary,
            })
            .collect(),
    })
}

/// Read a `.mobileprovision` file
#[uniffi::export]
pub fn read_profile(path: String) -> Result<ProfileInfo, WarpsignError> {
    let data = std::fs::read(&path).map_err(|e| WarpsignError::Profile {
        message: format!("{path}: {e}"),
    })?;
    let profile = ProvisioningProfile::parse(&data).map_err(|e| WarpsignError::Profile {
        message: e.to_string(),
    })?;
    Ok(ProfileInfo {
        name: profile.name,
        uuid: profile.uuid,
        team_ids: profile.team_ids,
        device_count: profile.provisioned_devices.len() as u32,
        ppq_check: profile.ppq_check,
    })
}
