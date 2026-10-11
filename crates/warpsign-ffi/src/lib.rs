//! The Swift-facing API of warpsign-core, exported with uniffi.
//!
//! Keep this layer thin: plain records in, plain records out, and the logic
//! in warpsign-core. Functions that do I/O block, so call them off the main
//! thread in Swift.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use plist::Value;
use warpsign_core::inspect::slash_path;
use warpsign_core::pipeline::{self, INSPECT_STEPS, PipelineError};
use warpsign_core::profile::ProvisioningProfile;
use warpsign_core::progress::{self, ProgressObserver};

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
    /// Wildcard profiles aren't supported
    pub is_wildcard: bool,
    /// Seconds since 1970
    pub expires_at: Option<i64>,
}

/// A stage of processing an IPA (mirrors warpsign_core::progress::Step)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum Step {
    Extract,
    InspectComponents,
    ReadTeamIds,
}

impl From<progress::Step> for Step {
    fn from(step: progress::Step) -> Self {
        match step {
            progress::Step::Extract => Step::Extract,
            progress::Step::InspectComponents => Step::InspectComponents,
            progress::Step::ReadTeamIds => Step::ReadTeamIds,
        }
    }
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct StepInfo {
    pub step: Step,
    pub title: String,
}

/// Implemented in Swift to follow progress. Called from a background
/// thread, in order, while the operation runs.
#[uniffi::export(with_foreign)]
pub trait StepObserver: Send + Sync {
    fn step_started(&self, step: Step);
    fn step_log(&self, step: Step, message: String);
    /// `error` is set if the step failed; nothing runs after a failed step
    fn step_finished(&self, step: Step, elapsed_ms: u64, error: Option<String>);
}

struct ObserverBridge(Arc<dyn StepObserver>);

impl ProgressObserver for ObserverBridge {
    fn step_started(&self, step: progress::Step) {
        self.0.step_started(step.into());
    }
    fn step_log(&self, step: progress::Step, message: &str) {
        self.0.step_log(step.into(), message.to_string());
    }
    fn step_finished(&self, step: progress::Step, elapsed: Duration, error: Option<&str>) {
        self.0.step_finished(
            step.into(),
            elapsed.as_millis().try_into().unwrap_or(u64::MAX),
            error.map(String::from),
        );
    }
}

/// The steps `inspect_ipa` runs, in order, so they can be shown up front
#[uniffi::export]
pub fn inspect_steps() -> Vec<StepInfo> {
    INSPECT_STEPS
        .iter()
        .map(|&step| StepInfo {
            step: step.into(),
            title: step.title().to_string(),
        })
        .collect()
}

/// warpsign-core's version
#[uniffi::export]
pub fn core_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Extract an IPA into `work_dir` (which must be empty or not exist) and
/// describe the app inside it, reporting each step to `observer`
#[uniffi::export]
pub fn inspect_ipa(
    ipa_path: String,
    work_dir: String,
    observer: Arc<dyn StepObserver>,
) -> Result<AppInfo, WarpsignError> {
    let app = pipeline::inspect_ipa(
        Path::new(&ipa_path),
        Path::new(&work_dir),
        &ObserverBridge(observer),
    )
    .map_err(|e| match e {
        PipelineError::Ipa(_) => WarpsignError::Ipa {
            message: e.to_string(),
        },
        _ => WarpsignError::Inspect {
            message: e.to_string(),
        },
    })?;

    let main = app.main();
    let info_string = |key: &str| {
        main.info_plist
            .get(key)
            .and_then(Value::as_string)
            .map(String::from)
    };
    Ok(AppInfo {
        app_path: app.app_dir.display().to_string(),
        bundle_id: main.bundle_id.clone(),
        display_name: info_string("CFBundleDisplayName").or_else(|| info_string("CFBundleName")),
        version: info_string("CFBundleShortVersionString"),
        components: app
            .components
            .iter()
            .map(|c| Component {
                path: slash_path(&c.path),
                bundle_id: c.bundle_id.clone(),
                is_primary: c.is_primary,
            })
            .collect(),
        team_ids: app.team_ids.clone(),
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
    let expires_at = profile.expiration_date.and_then(|date| {
        date.duration_since(std::time::UNIX_EPOCH)
            .ok()
            .and_then(|d| i64::try_from(d.as_secs()).ok())
    });
    Ok(ProfileInfo {
        is_wildcard: profile.is_wildcard(),
        expires_at,
        name: profile.name,
        uuid: profile.uuid,
        team_ids: profile.team_ids,
        device_count: profile.provisioned_devices.len() as u32,
        ppq_check: profile.ppq_check,
    })
}
