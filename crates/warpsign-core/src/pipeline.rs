//! End-to-end operations on IPAs, run as reported steps (see `progress`).

use std::path::{Path, PathBuf};

use crate::inspect::{AppComponent, InspectError, inspect_app_with, slash_path, team_ids};
use crate::ipa::{IpaError, extract_ipa};
use crate::progress::{ProgressObserver, Step, run_step};
use crate::signature;

#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error(transparent)]
    Ipa(#[from] IpaError),
    #[error(transparent)]
    Inspect(#[from] InspectError),
    #[error("the app has no main executable")]
    NoMainApp,
}

/// An extracted and inspected app
#[derive(Debug, Clone)]
pub struct InspectedApp {
    /// The extracted .app
    pub app_dir: PathBuf,
    /// The app first, then nested components deepest first
    pub components: Vec<AppComponent>,
    pub team_ids: Vec<String>,
}

impl InspectedApp {
    pub fn main(&self) -> &AppComponent {
        // inspect_ipa only returns apps that have one
        self.components
            .iter()
            .find(|c| c.is_main_app())
            .expect("inspected app has a main component")
    }
}

/// Steps `inspect_ipa` runs, in order
pub const INSPECT_STEPS: [Step; 3] = [Step::Extract, Step::InspectComponents, Step::ReadTeamIds];

/// Extract an IPA into `work_dir` and inspect it, reporting each step
pub fn inspect_ipa(
    ipa: &Path,
    work_dir: &Path,
    observer: &dyn ProgressObserver,
) -> Result<InspectedApp, PipelineError> {
    let app_dir = run_step(observer, Step::Extract, |log| {
        let size = std::fs::metadata(ipa).map(|m| m.len()).unwrap_or_default();
        log.log(format!(
            "{} ({:.1} MB)",
            ipa.file_name().unwrap_or_default().to_string_lossy(),
            size as f64 / 1_000_000.0
        ));
        let app_dir = extract_ipa(ipa, work_dir)?;
        log.log(format!(
            "Found {}",
            app_dir.file_name().unwrap_or_default().to_string_lossy()
        ));
        Ok::<_, PipelineError>(app_dir)
    })?;

    let components = run_step(observer, Step::InspectComponents, |log| {
        let components = inspect_app_with(&app_dir, |executable| {
            match signature::read_entitlements(executable) {
                Ok(entitlements) => entitlements,
                Err(e) => {
                    // Treated as unsigned, as with ldid in Python warpsign
                    log.log(format!("Couldn't read signature: {e}"));
                    Default::default()
                }
            }
        })?;
        if !components.iter().any(AppComponent::is_main_app) {
            return Err(PipelineError::NoMainApp);
        }
        for component in &components {
            let kind = if component.is_main_app() {
                "App"
            } else if component.is_primary {
                "Extension"
            } else {
                "Framework"
            };
            let path = if component.is_main_app() {
                String::new()
            } else {
                format!(" {}", slash_path(&component.path))
            };
            log.log(format!(
                "{kind}{path}: {} ({} entitlements)",
                component.bundle_id,
                component.entitlements.len()
            ));
        }
        Ok(components)
    })?;

    let team_ids = run_step(observer, Step::ReadTeamIds, |log| {
        let team_ids = team_ids(&components);
        if team_ids.is_empty() {
            log.log("No team IDs found: the app looks unsigned");
        } else {
            log.log(team_ids.join(", "));
        }
        Ok::<_, PipelineError>(team_ids)
    })?;

    Ok(InspectedApp {
        app_dir,
        components,
        team_ids,
    })
}
