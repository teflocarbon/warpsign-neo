//! Step-by-step progress reporting, so front-ends can show what's happening
//! (and where it went wrong) while an IPA is processed.

use std::fmt::Display;
use std::time::{Duration, Instant};

/// A stage of processing an IPA. Front-ends show these as a checklist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Step {
    /// Unpack the IPA
    Extract,
    /// Find the app, its extensions, frameworks and their entitlements
    InspectComponents,
    /// Work out which team(s) the app is currently signed for
    ReadTeamIds,
    /// Map identifiers and decide which capabilities each part needs
    PlanIdentifiers,
    /// Register App IDs, app groups and iCloud containers, and set
    /// capabilities, on the Developer Portal
    RegisterIdentifiers,
    /// Rewrite Info.plists with the new identifiers and chosen tweaks
    PatchInfoPlists,
    /// Create (or regenerate) and download provisioning profiles
    CreateProfiles,
    /// Replace old identifiers inside executables
    PatchBinaries,
    /// Code sign every bundle and executable
    Sign,
    /// Zip the signed app into an IPA
    Package,
}

impl Step {
    pub fn title(self) -> &'static str {
        match self {
            Step::Extract => "Extract IPA",
            Step::InspectComponents => "Inspect components",
            Step::ReadTeamIds => "Read team IDs",
            Step::PlanIdentifiers => "Plan identifiers",
            Step::RegisterIdentifiers => "Register identifiers",
            Step::PatchInfoPlists => "Patch Info.plists",
            Step::CreateProfiles => "Create provisioning profiles",
            Step::PatchBinaries => "Patch binaries",
            Step::Sign => "Sign",
            Step::Package => "Package IPA",
        }
    }
}

/// Receives progress events. Implementations must be cheap and must not
/// block: they're called from the thread doing the work.
pub trait ProgressObserver: Send + Sync {
    fn step_started(&self, step: Step);
    fn step_log(&self, step: Step, message: &str);
    /// `error` is set if the step failed; no further steps start after that
    fn step_finished(&self, step: Step, elapsed: Duration, error: Option<&str>);
}

/// Ignores all progress
pub struct NoProgress;

impl ProgressObserver for NoProgress {
    fn step_started(&self, _step: Step) {}
    fn step_log(&self, _step: Step, _message: &str) {}
    fn step_finished(&self, _step: Step, _elapsed: Duration, _error: Option<&str>) {}
}

/// Logging handle for the step currently running
pub struct StepLog<'a> {
    observer: &'a dyn ProgressObserver,
    step: Step,
}

impl StepLog<'_> {
    pub fn log(&self, message: impl AsRef<str>) {
        self.observer.step_log(self.step, message.as_ref());
    }
}

/// Run `work` as `step`, reporting its start, logs, duration and outcome
pub fn run_step<T, E: Display>(
    observer: &dyn ProgressObserver,
    step: Step,
    work: impl FnOnce(&StepLog) -> Result<T, E>,
) -> Result<T, E> {
    observer.step_started(step);
    let started = Instant::now();
    let result = work(&StepLog { observer, step });
    let error = result.as_ref().err().map(ToString::to_string);
    observer.step_finished(step, started.elapsed(), error.as_deref());
    result
}
