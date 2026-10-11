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
use warpsign_core::metadata::{self, MetadataError};
use warpsign_core::pipeline::{self, INSPECT_STEPS, PipelineError};
use warpsign_core::profile::ProvisioningProfile;
use warpsign_core::progress::{self, ProgressObserver};
use warpsign_core::source;

uniffi::setup_scaffolding!();

mod account;
mod install;
pub use account::*;
pub use install::*;

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum WarpsignError {
    #[error("{message}")]
    Ipa { message: String },
    #[error("{message}")]
    Inspect { message: String },
    #[error("{message}")]
    Profile { message: String },
    #[error("{message}")]
    Source { message: String },
    /// Signing in failed
    #[error("{message}")]
    Auth { message: String },
    /// The two-factor code was wrong: ask again
    #[error("{message}")]
    IncorrectCode { message: String },
    /// Sign in again
    #[error("{message}")]
    SessionExpired { message: String },
    #[error("{message}")]
    Portal { message: String },
    #[error("{message}")]
    Identity { message: String },
    #[error("{message}")]
    Signing { message: String },
    /// The device can't be reached: usually the VPN isn't connected
    #[error("{message}")]
    DeviceUnreachable { message: String },
    #[error("{message}")]
    Device { message: String },
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
    PlanIdentifiers,
    RegisterIdentifiers,
    PatchInfoPlists,
    CreateProfiles,
    PatchBinaries,
    Sign,
    Package,
    ConnectDevice,
    UploadApp,
    InstallApp,
}

impl From<progress::Step> for Step {
    fn from(step: progress::Step) -> Self {
        match step {
            progress::Step::Extract => Step::Extract,
            progress::Step::InspectComponents => Step::InspectComponents,
            progress::Step::ReadTeamIds => Step::ReadTeamIds,
            progress::Step::PlanIdentifiers => Step::PlanIdentifiers,
            progress::Step::RegisterIdentifiers => Step::RegisterIdentifiers,
            progress::Step::PatchInfoPlists => Step::PatchInfoPlists,
            progress::Step::CreateProfiles => Step::CreateProfiles,
            progress::Step::PatchBinaries => Step::PatchBinaries,
            progress::Step::Sign => Step::Sign,
            progress::Step::Package => Step::Package,
            progress::Step::ConnectDevice => Step::ConnectDevice,
            progress::Step::UploadApp => Step::UploadApp,
            progress::Step::InstallApp => Step::InstallApp,
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
    /// How far through a long step is (0 to 1), for steps that know
    fn step_progress(&self, step: Step, fraction: f64);
}

pub(crate) struct ObserverBridge(pub(crate) Arc<dyn StepObserver>);

impl ProgressObserver for ObserverBridge {
    fn step_started(&self, step: progress::Step) {
        self.0.step_started(step.into());
    }
    fn step_log(&self, step: progress::Step, message: &str) {
        self.0.step_log(step.into(), message.to_string());
    }
    fn step_progress(&self, step: progress::Step, fraction: f64) {
        self.0.step_progress(step.into(), fraction);
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

/// Whether an app's main executable is still FairPlay-encrypted
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Encryption {
    Decrypted,
    /// Installs once re-signed, but crashes on launch
    Encrypted,
    Unknown,
}

/// An IPA's app, read without extracting it
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct IpaMetadata {
    pub display_name: String,
    pub bundle_id: String,
    pub version: Option<String>,
    pub build: Option<String>,
    pub minimum_os: Option<String>,
    pub file_size: u64,
    pub installed_size: u64,
    pub extension_count: u32,
    pub encryption: Encryption,
    /// PNG, usually Apple's CgBI variant (UIImage decodes it)
    pub icon: Option<Vec<u8>>,
}

/// Read an IPA's name, version and icon, without extracting it
#[uniffi::export]
pub fn read_ipa_metadata(ipa_path: String) -> Result<IpaMetadata, WarpsignError> {
    let metadata = metadata::read_ipa_metadata(Path::new(&ipa_path)).map_err(|e| match e {
        MetadataError::Ipa(_) | MetadataError::Zip { .. } => WarpsignError::Ipa {
            message: e.to_string(),
        },
        _ => WarpsignError::Inspect {
            message: e.to_string(),
        },
    })?;
    Ok(IpaMetadata {
        display_name: metadata.display_name,
        bundle_id: metadata.bundle_id,
        version: metadata.version,
        build: metadata.build,
        minimum_os: metadata.minimum_os,
        file_size: metadata.file_size,
        installed_size: metadata.installed_size,
        extension_count: metadata.extension_count,
        encryption: match metadata.encryption {
            metadata::Encryption::Decrypted => Encryption::Decrypted,
            metadata::Encryption::Encrypted => Encryption::Encrypted,
            metadata::Encryption::Unknown => Encryption::Unknown,
        },
        icon: metadata.icon,
    })
}

/// An AltStore-format app source
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Source {
    pub name: String,
    pub identifier: Option<String>,
    pub subtitle: Option<String>,
    pub description: Option<String>,
    pub icon_url: Option<String>,
    pub header_url: Option<String>,
    pub website: Option<String>,
    pub tint_color: Option<String>,
    pub featured_apps: Vec<String>,
    pub apps: Vec<SourceApp>,
    pub news: Vec<NewsItem>,
    /// Apps that were skipped, and why
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SourceApp {
    pub name: String,
    pub bundle_id: String,
    pub developer_name: Option<String>,
    pub subtitle: Option<String>,
    pub description: Option<String>,
    pub icon_url: Option<String>,
    pub tint_color: Option<String>,
    pub category: Option<String>,
    pub screenshot_urls: Vec<String>,
    /// Newest first, never empty
    pub versions: Vec<AppVersion>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AppVersion {
    pub version: String,
    pub build: Option<String>,
    pub date: Option<String>,
    pub notes: Option<String>,
    pub download_url: String,
    pub size: Option<u64>,
    pub min_os: Option<String>,
    pub max_os: Option<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct NewsItem {
    pub identifier: Option<String>,
    pub title: String,
    pub caption: Option<String>,
    pub date: Option<String>,
    pub image_url: Option<String>,
    pub url: Option<String>,
    pub app_id: Option<String>,
}

/// Parse a downloaded AltStore-format source
#[uniffi::export]
pub fn parse_source(data: Vec<u8>) -> Result<Source, WarpsignError> {
    let parsed = source::parse_source(&data).map_err(|e| WarpsignError::Source {
        message: e.to_string(),
    })?;
    Ok(Source {
        name: parsed.name,
        identifier: parsed.identifier,
        subtitle: parsed.subtitle,
        description: parsed.description,
        icon_url: parsed.icon_url,
        header_url: parsed.header_url,
        website: parsed.website,
        tint_color: parsed.tint_color,
        featured_apps: parsed.featured_apps,
        apps: parsed
            .apps
            .into_iter()
            .map(|app| SourceApp {
                name: app.name,
                bundle_id: app.bundle_id,
                developer_name: app.developer_name,
                subtitle: app.subtitle,
                description: app.description,
                icon_url: app.icon_url,
                tint_color: app.tint_color,
                category: app.category,
                screenshot_urls: app.screenshot_urls,
                versions: app
                    .versions
                    .into_iter()
                    .map(|v| AppVersion {
                        version: v.version,
                        build: v.build,
                        date: v.date,
                        notes: v.notes,
                        download_url: v.download_url,
                        size: v.size,
                        min_os: v.min_os,
                        max_os: v.max_os,
                    })
                    .collect(),
            })
            .collect(),
        news: parsed
            .news
            .into_iter()
            .map(|n| NewsItem {
                identifier: n.identifier,
                title: n.title,
                caption: n.caption,
                date: n.date,
                image_url: n.image_url,
                url: n.url,
                app_id: n.app_id,
            })
            .collect(),
        warnings: parsed.warnings,
    })
}
