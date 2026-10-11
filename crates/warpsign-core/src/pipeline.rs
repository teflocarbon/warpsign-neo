//! End-to-end operations on IPAs, run as reported steps (see `progress`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use plist::{Dictionary, Value};

use crate::ProfileType;
use crate::bundle_mapping::BundleMapping;
use crate::capability_names::{EntitlementName, EntitlementNames};
use crate::entitlements::EntitlementsProcessor;
use crate::identity::{CertificateKind, Identity, IdentityError};
use crate::info_plist::{InfoPlistError, InfoPlistPatcher, PatchingOptions};
use crate::inspect::{AppComponent, InspectError, inspect_app_with, slash_path, team_ids};
use crate::ipa::{IpaError, extract_ipa, package_ipa};
use crate::portal::api::PortalProfile;
use crate::portal::{Portal, PortalError};
use crate::profile::{PpqRisk, ProfileError, ProvisioningProfile, check_ppq_risk};
use crate::profile_reuse::{NotReusable, ProfileCache, check, portal_date};
use crate::progress::{ProgressObserver, Step, StepLog, run_step};
use crate::signature;
use crate::signing::{BundleEntitlements, SigningError, sign_app_bundle};

#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error(transparent)]
    Ipa(#[from] IpaError),
    #[error(transparent)]
    Inspect(#[from] InspectError),
    #[error("the app has no main executable")]
    NoMainApp,
    #[error(transparent)]
    Identity(#[from] IdentityError),
    #[error(transparent)]
    Portal(#[from] PortalError),
    #[error(transparent)]
    InfoPlist(#[from] InfoPlistError),
    #[error("the portal sent an invalid provisioning profile: {0}")]
    Profile(#[from] ProfileError),
    #[error(transparent)]
    PpqRisk(#[from] PpqRisk),
    #[error(transparent)]
    Signing(#[from] SigningError),
    #[error("{path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    /// Something about the account or certificate that needs fixing first
    #[error("{0}")]
    Setup(String),
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
        for removed in remove_unsupported_apps(&app_dir)? {
            log.log(format!("Removed {removed}"));
        }
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

/// Steps `sign_ipa` runs, in order
pub const SIGN_STEPS: [Step; 10] = [
    Step::Extract,
    Step::InspectComponents,
    Step::ReadTeamIds,
    Step::PlanIdentifiers,
    Step::RegisterIdentifiers,
    Step::PatchInfoPlists,
    Step::CreateProfiles,
    Step::PatchBinaries,
    Step::Sign,
    Step::Package,
];

/// How to sign
#[derive(Debug, Clone, PartialEq)]
pub struct SignOptions {
    pub patching: PatchingOptions,
    /// Give every identifier a new random-looking (but stable) form, so
    /// the signed app doesn't clash with the original. Python warpsign's
    /// default; only turn off for apps you own.
    pub encode_ids: bool,
    /// Allow debuggers to attach (`get-task-allow`)
    pub patch_debug: bool,
    /// Where to keep provisioning profiles between signs (see
    /// [`crate::profile_reuse`]); None downloads them each time
    pub profile_cache: Option<PathBuf>,
}

impl Default for SignOptions {
    fn default() -> Self {
        Self {
            patching: PatchingOptions::default(),
            encode_ids: true,
            patch_debug: false,
            profile_cache: None,
        }
    }
}

/// A provisioning profile made for a signed app
#[derive(Debug, Clone, PartialEq)]
pub struct SignedProfile {
    /// Bundle path inside the app ("" for the app itself)
    pub bundle_path: String,
    pub bundle_id: String,
    pub name: String,
    pub ppq_check: bool,
    pub expiration_date: Option<std::time::SystemTime>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SignedApp {
    pub output: PathBuf,
    /// The main app's CFBundleIdentifier after signing
    pub bundle_id: String,
    pub profiles: Vec<SignedProfile>,
    /// Things that worked out, but not quite as asked
    pub warnings: Vec<String>,
    /// Every component, in the order of [`InspectedApp::components`], with
    /// what it was signed with
    pub components: Vec<SignedComponent>,
    /// The portal's capabilities response fetched while signing, for
    /// front-ends to cache (it names entitlements: see `capability_names`)
    pub capabilities_response: serde_json::Value,
}

/// What one component of a signed app was signed with, and how that
/// differs from what it had before
#[derive(Debug, Clone, PartialEq)]
pub struct SignedComponent {
    /// Bundle path inside the app ("" for the app itself), "/"-separated
    pub path: String,
    /// Apps and extensions; frameworks, dylibs and plug-in bundles aren't
    pub is_primary: bool,
    /// CFBundleIdentifier before signing (the file stem for frameworks)
    pub original_bundle_id: String,
    /// CFBundleIdentifier after signing. Frameworks keep theirs.
    pub bundle_id: String,
    /// Entitlements the component's executable had before
    pub original_entitlements: Dictionary,
    /// Entitlements it was signed with. Always empty for frameworks,
    /// which are signed without any.
    pub entitlements: Dictionary,
    /// Portal capabilities enabled on its App ID (primary components only)
    pub capabilities: Vec<String>,
    /// Differences between the two entitlement dictionaries, by key
    pub changes: Vec<EntitlementChange>,
    /// Friendly names for the keys in either dictionary that have one,
    /// from the portal's capabilities or a built-in table
    pub names: BTreeMap<String, EntitlementName>,
}

/// One entitlement that differs between the original and signed component
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntitlementChange {
    pub key: String,
    pub kind: ChangeKind,
    /// Why, when the pipeline knows
    pub reason: Option<ChangeReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Removed,
    /// Present before and after, with a different value
    Changed,
}

/// Why an entitlement was added, removed or changed
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeReason {
    /// The team has no portal capability that grants it
    NoCapability,
    /// Always removed (see `entitlements::is_banned`)
    Banned,
    /// get-task-allow, with debugging off in the signing options
    DebuggingOff,
    /// get-task-allow, with debugging on in the signing options
    DebuggingOn,
    /// Needed to sign (application-identifier, team-identifier), and missing
    Required,
    /// Points at the new team, App ID, app groups or containers
    Remapped,
    /// aps-environment, set for the profile type
    PushEnvironment,
    /// Frameworks and other non-primary components are signed without
    /// entitlements
    NotPrimary,
}

impl ChangeReason {
    /// A short explanation for people
    pub fn description(self) -> &'static str {
        match self {
            ChangeReason::NoCapability => "No matching capability on the Developer Portal",
            ChangeReason::Banned => "Always removed when re-signing",
            ChangeReason::DebuggingOff => "Debugging is off in the signing options",
            ChangeReason::DebuggingOn => "Debugging is on in the signing options",
            ChangeReason::Required => "Required for signing",
            ChangeReason::Remapped => "Remapped to the new team and identifiers",
            ChangeReason::PushEnvironment => "Set to match the provisioning profile type",
            ChangeReason::NotPrimary => "Frameworks and libraries are signed without entitlements",
        }
    }
}

impl std::fmt::Display for ChangeReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.description())
    }
}

/// How `signed` differs from `component`'s original entitlements, sorted
/// by key. `removals` is the processor's removal list for a primary
/// component, `None` for one signed without entitlements.
fn entitlement_changes(
    component: &AppComponent,
    signed: &Dictionary,
    removals: Option<&BTreeSet<String>>,
    patch_debug: bool,
) -> Vec<EntitlementChange> {
    let original = &component.entitlements;
    let mut changes = Vec::new();
    for (key, value) in original {
        let (kind, reason) = match signed.get(key) {
            None => {
                let reason = match removals {
                    None => Some(ChangeReason::NotPrimary),
                    Some(_) if key == "get-task-allow" && !patch_debug => {
                        Some(ChangeReason::DebuggingOff)
                    }
                    Some(removals) if removals.contains(key) => {
                        if crate::entitlements::is_banned(key) {
                            Some(ChangeReason::Banned)
                        } else {
                            Some(ChangeReason::NoCapability)
                        }
                    }
                    Some(_) => None,
                };
                (ChangeKind::Removed, reason)
            }
            Some(new) if new != value => {
                let reason = match key.as_str() {
                    "get-task-allow" if patch_debug => ChangeReason::DebuggingOn,
                    "aps-environment" => ChangeReason::PushEnvironment,
                    _ => ChangeReason::Remapped,
                };
                (ChangeKind::Changed, Some(reason))
            }
            Some(_) => continue,
        };
        changes.push(EntitlementChange {
            key: key.clone(),
            kind,
            reason,
        });
    }
    for key in signed.keys() {
        if original.contains_key(key) {
            continue;
        }
        let reason = match key.as_str() {
            "get-task-allow" if patch_debug => Some(ChangeReason::DebuggingOn),
            "application-identifier" | "com.apple.developer.team-identifier" => {
                Some(ChangeReason::Required)
            }
            _ => None,
        };
        changes.push(EntitlementChange {
            key: key.clone(),
            kind: ChangeKind::Added,
            reason,
        });
    }
    changes.sort_by(|a, b| a.key.cmp(&b.key));
    changes
}

/// A primary component (app or extension) and what it needs
struct Plan {
    component: usize,
    new_bundle_id: String,
    capabilities: BTreeSet<String>,
    removals: BTreeSet<String>,
}

/// Re-sign an IPA for the signed-in team: a port of Python warpsign's
/// `SignOrchestrator.sign_ipa`. Registers what the app needs on the
/// Developer Portal, patches it, makes provisioning profiles, signs it
/// with `identity` and writes the IPA to `output`.
pub fn sign_ipa(
    portal: &Portal,
    identity: &Identity,
    ipa: &Path,
    output: &Path,
    work_dir: &Path,
    options: &SignOptions,
    observer: &dyn ProgressObserver,
) -> Result<SignedApp, PipelineError> {
    let team_id = portal.team_id().to_string();
    // Python warpsign chose the profile type from the certificate
    let profile_type = match identity.kind() {
        Some(CertificateKind::Development) => ProfileType::Development,
        Some(CertificateKind::Distribution) => ProfileType::AdHoc,
        None => {
            return Err(PipelineError::Setup(format!(
                "{} isn't an Apple Development or Apple Distribution certificate",
                identity
                    .common_name()
                    .unwrap_or_else(|| "the certificate".into())
            )));
        }
    };
    if identity.team_id().as_deref() != Some(team_id.as_str()) {
        return Err(PipelineError::Setup(format!(
            "the certificate belongs to team {}, but you're signed in to team {team_id}",
            identity.team_id().unwrap_or_else(|| "(none)".into())
        )));
    }
    let force_original_id = options.patching.force_original_id;
    let mut warnings = Vec::new();

    let app = inspect_ipa(ipa, work_dir, observer)?;
    let main = app.main();

    // _analyze_components
    let (mut mapping, plans, app_groups, icloud_containers, capabilities_response) =
        run_step(observer, Step::PlanIdentifiers, |log| {
            if app.team_ids.is_empty() {
                log.log("Warning: no team IDs found in the app. Check the IPA.");
            }
            let mut mapping = BundleMapping::new(
                app.team_ids.clone(),
                &team_id,
                &main.bundle_id,
                options.encode_ids,
            );
            mapping.force_original_id = force_original_id;
            mapping.profile_type = profile_type;

            let capabilities = portal.capabilities()?;
            let processor = EntitlementsProcessor::new(&capabilities, profile_type);
            let mut plans = Vec::new();
            let mut app_groups = BTreeSet::new();
            let mut icloud_containers = BTreeSet::new();
            for (index, component) in app.components.iter().enumerate() {
                if !component.is_primary {
                    continue;
                }
                let new_bundle_id = mapping.map_bundle_id(&component.bundle_id);
                let plan = processor.process(&component.entitlements);
                let (groups, containers) =
                    mapping.extract_resources_from_entitlements(&component.entitlements);
                app_groups.extend(groups);
                icloud_containers.extend(containers);
                log.log(format!(
                    "{}: {} → {new_bundle_id}",
                    describe(component),
                    component.bundle_id
                ));
                if !plan.capabilities_to_enable.is_empty() {
                    log.log(format!(
                        "  capabilities: {}",
                        join(&plan.capabilities_to_enable)
                    ));
                }
                // get-task-allow is in Python's removal list too, but the
                // debugging option decides it (see signing_entitlements),
                // so it isn't reported here
                let removed: BTreeSet<String> = plan
                    .entitlements_to_remove
                    .iter()
                    .filter(|key| *key != "get-task-allow")
                    .cloned()
                    .collect();
                if !removed.is_empty() {
                    log.log(format!(
                        "  no matching portal capability, so removed: {}",
                        join(&removed)
                    ));
                }
                plans.push(Plan {
                    component: index,
                    new_bundle_id,
                    capabilities: plan.capabilities_to_enable,
                    removals: plan.entitlements_to_remove,
                });
            }
            for group in &app_groups {
                log.log(format!("App group: {group}"));
            }
            for container in &icloud_containers {
                log.log(format!("iCloud container: {container}"));
            }
            Ok::<_, PipelineError>((mapping, plans, app_groups, icloud_containers, capabilities))
        })?;

    // _register_app_resources
    let bundles = run_step(observer, Step::RegisterIdentifiers, |log| {
        let mut registered = std::mem::take(&mut mapping.registered_identifiers);
        // Registered identifiers, and what they were mapped from, are the
        // ones patched into binaries
        let record = |registered: &mut std::collections::HashSet<String>,
                      mapping: &BundleMapping,
                      new_id: &str| {
            if let Some(original) = original_of(mapping, new_id) {
                registered.insert(original);
            }
            registered.insert(new_id.to_string());
        };

        // What the team already has, listed once rather than discovered by
        // trying to register each identifier again (not in Python warpsign:
        // see profile_reuse)
        let existing_groups = if app_groups.is_empty() {
            Vec::new()
        } else {
            portal.app_groups()?
        };
        let mut group_ids = Vec::new();
        for group in &app_groups {
            let registered_group = match existing_groups.iter().find(|g| &g.identifier == group) {
                Some(existing) => {
                    log.log(format!(
                        "App group {group}: {} (already registered)",
                        existing.id
                    ));
                    existing.clone()
                }
                None => {
                    let name = format!("WS App Group {}", group.replace('.', " "));
                    let registered_group = portal.register_app_group(group, &name)?;
                    log.log(format!("App group {group}: {}", registered_group.id));
                    registered_group
                }
            };
            group_ids.push(registered_group.id);
            record(&mut registered, &mapping, group);
        }

        let existing_containers = if icloud_containers.is_empty() {
            Vec::new()
        } else {
            portal.cloud_containers()?
        };
        let mut container_ids = Vec::new();
        for container in &icloud_containers {
            let container = if container.starts_with("iCloud.") {
                container.clone()
            } else {
                // Python warpsign's safety net: the mapping always adds the
                // prefix, so this shouldn't happen
                let fixed = format!(
                    "iCloud.{}",
                    container.rsplit('.').next().unwrap_or_default()
                );
                log.log(format!(
                    "Error: iCloud container without prefix: {container}, using {fixed}"
                ));
                fixed
            };
            let registered_container = match existing_containers
                .iter()
                .find(|c| c.identifier == container)
            {
                Some(existing) => {
                    log.log(format!(
                        "iCloud container {container}: {} (already registered)",
                        existing.id
                    ));
                    existing.clone()
                }
                None => {
                    let name = format!("WS iCloud Container {}", container.replace('.', " "));
                    let registered_container =
                        portal.register_cloud_container(&container, &name)?;
                    log.log(format!(
                        "iCloud container {container}: {}",
                        registered_container.id
                    ));
                    registered_container
                }
            };
            container_ids.push(registered_container.id);
            record(&mut registered, &mapping, &container);
        }

        // For deciding which profiles can be used as they are, before
        // changing any App ID
        let devices = portal.devices()?;
        let device_udids: Vec<String> = devices.iter().map(|device| device.udid.clone()).collect();
        let existing_profiles = portal.profiles()?;
        let existing_bundles = portal.bundle_ids()?;
        let cache = options
            .profile_cache
            .as_deref()
            .map(|root| ProfileCache::new(root, &team_id));
        // Worked out as the Sign step will, without touching the real mapping
        let mut preview_mapping = mapping.clone();
        let now = std::time::SystemTime::now();

        let mut bundles = Vec::new();
        let mut profile_plans = Vec::new();
        for plan in &plans {
            let component = &app.components[plan.component];
            let kind = if component.is_main_app() {
                "App"
            } else {
                "App Extension"
            };
            let bundle = match existing_bundles
                .iter()
                .find(|bundle| bundle.identifier == plan.new_bundle_id)
            {
                Some(existing) => {
                    log.log(format!(
                        "App ID {}: {} (already registered)",
                        existing.identifier, existing.id
                    ));
                    existing.clone()
                }
                None => {
                    // Python warpsign's names
                    let name = format!(
                        "WS {kind} {}",
                        plan.new_bundle_id.rsplit('.').next().unwrap_or_default()
                    );
                    let bundle = portal.register_bundle_id(&plan.new_bundle_id, &name)?;
                    log.log(format!("App ID {}: {}", bundle.identifier, bundle.id));
                    bundle
                }
            };

            let (wanted, _) = signing_entitlements_quiet(
                component,
                plan,
                &mut preview_mapping,
                &team_id,
                options,
            );
            let profile_plan = plan_profile(
                portal,
                cache.as_ref(),
                &existing_profiles,
                &profile_name(&plan.new_bundle_id, profile_type),
                &format!("{team_id}.{}", plan.new_bundle_id),
                identity.certificate_der(),
                &device_udids,
                &wanted,
                now,
            )?;
            record(&mut registered, &mapping, &plan.new_bundle_id);
            for original_team_id in mapping.original_team_ids() {
                if component.bundle_id.contains(original_team_id.as_str()) {
                    registered.insert(original_team_id.clone());
                    registered.insert(team_id.clone());
                }
            }

            let capabilities_current = match &profile_plan {
                ProfilePlan::Reuse { .. } => true,
                ProfilePlan::Make { reason, .. } => reason
                    .as_ref()
                    .is_some_and(|reason| !reason.needs_capabilities()),
            };
            if !plan.capabilities.is_empty() && capabilities_current {
                log.log("  capabilities unchanged: its profile already grants them");
            } else if !plan.capabilities.is_empty() {
                let mut relationships = BTreeMap::new();
                if plan.capabilities.contains("APP_GROUPS") && !group_ids.is_empty() {
                    relationships.insert("APP_GROUPS".to_string(), group_ids.clone());
                }
                if plan.capabilities.contains("ICLOUD") && !container_ids.is_empty() {
                    relationships.insert("ICLOUD".to_string(), container_ids.clone());
                }
                let capabilities: Vec<String> = plan.capabilities.iter().cloned().collect();
                let update = portal.update_capabilities(&bundle, &capabilities, &relationships)?;
                log.log(format!("  capabilities set: {}", capabilities.join(", ")));
                if !update.skipped.is_empty() {
                    let warning = format!(
                        "{}: left off {}, which need settings WarpSign doesn't know how to choose",
                        bundle.identifier,
                        update.skipped.join(", ")
                    );
                    log.log(format!("Warning: {warning}"));
                    warnings.push(warning);
                }
            }
            bundles.push(bundle);
            profile_plans.push(profile_plan);
        }
        mapping.registered_identifiers = registered;
        Ok::<_, PipelineError>((bundles, profile_plans, devices, existing_profiles, cache))
    })?;
    let (bundles, profile_plans, devices, existing_profiles, cache) = bundles;

    // _update_info_plists. The main app first: which IDs get replaced with
    // force_original_id depends on its bundle ID.
    let mut patcher = InfoPlistPatcher::new(options.patching.clone());
    // The CFBundleIdentifier each primary component ended up with, by index
    let mut patched_bundle_ids = BTreeMap::new();
    let main_bundle_id = run_step(observer, Step::PatchInfoPlists, |log| {
        let mut order: Vec<&Plan> = plans.iter().collect();
        order.sort_by_key(|plan| !app.components[plan.component].is_main_app());
        let mut main_bundle_id = main.bundle_id.clone();
        for plan in order {
            let component = &app.components[plan.component];
            let path = app.app_dir.join(&component.path).join("Info.plist");
            let patched = patcher.patch_file(&path, &mut mapping, component.is_main_app())?;
            let bundle_id = patched
                .get("CFBundleIdentifier")
                .and_then(Value::as_string)
                .unwrap_or_default()
                .to_string();
            log.log(format!(
                "{}: CFBundleIdentifier {bundle_id}",
                describe(component)
            ));
            if component.is_main_app() {
                main_bundle_id = bundle_id.clone();
            }
            patched_bundle_ids.insert(plan.component, bundle_id);
        }
        Ok::<_, PipelineError>(main_bundle_id)
    })?;

    // _create_provisioning_profiles
    let profiles = run_step(observer, Step::CreateProfiles, |log| {
        if devices.is_empty() {
            return Err(PipelineError::Setup(
                "no iPhones or iPads are registered on this team: add this device at developer.apple.com first".into(),
            ));
        }
        log.log(format!("{} devices", devices.len()));
        let device_ids: Vec<String> = devices.iter().map(|device| device.id.clone()).collect();

        let serial = identity.serial_number();
        let certificate = portal
            .certificates()?
            .into_iter()
            .find(|certificate| identity.has_serial(&certificate.serial_number))
            .ok_or_else(|| {
                PipelineError::Setup(format!(
                    "the signing certificate (serial {serial}) isn't on the portal: it may have been revoked"
                ))
            })?;
        // The portal's name lacks the person ID that's in the certificate's
        // own name, so log that instead, with the portal's certificate ID
        log.log(format!(
            "Certificate {} (portal ID {})",
            identity.common_name().unwrap_or(certificate.name),
            certificate.id
        ));

        let mut listed: std::collections::HashSet<String> = existing_profiles
            .iter()
            .map(|profile| profile.id.clone())
            .collect();
        let mut profiles = Vec::new();
        let mut parsed = Vec::new();
        for ((plan, bundle), profile_plan) in plans.iter().zip(&bundles).zip(profile_plans) {
            let component = &app.components[plan.component];
            let name = profile_name(&plan.new_bundle_id, profile_type);
            let (content, how) = match profile_plan {
                ProfilePlan::Reuse { content, .. } => {
                    (content, " (up to date, reused)".to_string())
                }
                ProfilePlan::Make {
                    existing_id,
                    reason,
                } => {
                    let (id, content) = portal.create_or_regenerate_profile(
                        existing_id.as_deref(),
                        bundle,
                        &name,
                        std::slice::from_ref(&certificate.id),
                        &device_ids,
                        profile_type,
                    )?;
                    if let Some(cache) = &cache {
                        cache.store(&id, &content);
                    }
                    listed.insert(id);
                    let how = match (existing_id, reason) {
                        (None, _) => String::new(),
                        (Some(_), Some(reason)) => format!(" (regenerated: {reason})"),
                        (Some(_), None) => " (regenerated)".to_string(),
                    };
                    (content, how)
                }
            };
            let profile = ProvisioningProfile::parse(&content)?;
            log.log(format!(
                "{}: {name}{how}{}",
                describe(component),
                if profile.ppq_check {
                    ", PPQCheck on"
                } else {
                    ""
                },
            ));
            let path = app
                .app_dir
                .join(&component.path)
                .join("embedded.mobileprovision");
            std::fs::write(&path, &content).map_err(|source| PipelineError::Io {
                path: path.display().to_string(),
                source,
            })?;
            profiles.push(SignedProfile {
                bundle_path: slash_path(&component.path),
                bundle_id: plan.new_bundle_id.clone(),
                name,
                ppq_check: profile.ppq_check,
                expiration_date: profile.expiration_date,
            });
            parsed.push(profile);
        }
        if let Some(cache) = &cache {
            cache.prune(&listed);
        }
        // Keeping the original bundle ID with a PPQ profile risks the
        // Apple ID: refuse before anything is signed
        check_ppq_risk(&parsed, force_original_id)?;
        Ok(profiles)
    })?;

    run_step(observer, Step::PatchBinaries, |log| {
        // App Store DRM metadata. Python warpsign removed it after signing,
        // which breaks the signature's seal, so it's removed before.
        let sc_info = app.app_dir.join("SC_Info");
        if sc_info.is_dir() {
            log.log("Removing SC_Info (App Store DRM metadata): the app may still be encrypted");
            std::fs::remove_dir_all(&sc_info).map_err(|source| PipelineError::Io {
                path: sc_info.display().to_string(),
                source,
            })?;
        }
        for component in &app.components {
            let executable = app.app_dir.join(&component.executable);
            if !executable.is_file() {
                continue;
            }
            let count = patcher.patch_binary(&executable, &mapping)?;
            if count > 0 {
                log.log(format!(
                    "{}: {count} replacements",
                    slash_path(&component.executable)
                ));
            }
        }
        Ok::<_, PipelineError>(())
    })?;

    let signed_entitlements = run_step(observer, Step::Sign, |log| {
        let mut bundle_entitlements = Vec::new();
        for plan in &plans {
            let component = &app.components[plan.component];
            let entitlements =
                signing_entitlements(component, plan, &mut mapping, &team_id, options, log);
            log.log(format!(
                "{}: {} entitlements",
                describe(component),
                entitlements.len()
            ));
            bundle_entitlements.push(BundleEntitlements {
                bundle_path: slash_path(&component.path),
                entitlements,
            });
        }
        let signer = identity.signing_identity()?;
        sign_app_bundle(&app.app_dir, &signer, &team_id, &bundle_entitlements)?;
        log.log(format!(
            "Signed with {}",
            identity.common_name().unwrap_or_default()
        ));
        // Anything iOS would refuse, named now rather than as installd's
        // "invalid provisioning profile" later
        let profiled: Vec<String> = plans
            .iter()
            .map(|plan| slash_path(&app.components[plan.component].path))
            .collect();
        for problem in unsigned_code(&app.app_dir, &team_id, &profiled) {
            log.log(format!("Warning: {problem}"));
            warnings.push(problem);
        }
        // In the order of `plans`
        Ok::<_, PipelineError>(
            bundle_entitlements
                .into_iter()
                .map(|bundle| bundle.entitlements)
                .collect::<Vec<_>>(),
        )
    })?;

    // What each component was signed with, for front-ends to show
    let names = EntitlementNames::from_capabilities(&capabilities_response);
    let components = app
        .components
        .iter()
        .enumerate()
        .map(|(index, component)| {
            let signed = plans
                .iter()
                .zip(&signed_entitlements)
                .find(|(plan, _)| plan.component == index);
            let (entitlements, capabilities, removals) = match signed {
                Some((plan, entitlements)) => (
                    entitlements.clone(),
                    plan.capabilities.iter().cloned().collect(),
                    Some(&plan.removals),
                ),
                None => (Dictionary::new(), Vec::new(), None),
            };
            SignedComponent {
                path: slash_path(&component.path),
                is_primary: component.is_primary,
                original_bundle_id: component.bundle_id.clone(),
                bundle_id: patched_bundle_ids
                    .get(&index)
                    .cloned()
                    .unwrap_or_else(|| component.bundle_id.clone()),
                changes: entitlement_changes(
                    component,
                    &entitlements,
                    removals,
                    options.patch_debug,
                ),
                names: component
                    .entitlements
                    .keys()
                    .chain(entitlements.keys())
                    .filter_map(|key| Some((key.clone(), names.get(key)?)))
                    .collect(),
                original_entitlements: component.entitlements.clone(),
                entitlements,
                capabilities,
            }
        })
        .collect();

    run_step(observer, Step::Package, |log| {
        package_ipa(&app.app_dir, output)?;
        let size = std::fs::metadata(output)
            .map(|m| m.len())
            .unwrap_or_default();
        log.log(format!(
            "{} ({:.1} MB)",
            output.file_name().unwrap_or_default().to_string_lossy(),
            size as f64 / 1_000_000.0
        ));
        Ok::<_, PipelineError>(())
    })?;

    Ok(SignedApp {
        output: output.to_path_buf(),
        bundle_id: main_bundle_id,
        profiles,
        warnings,
        components,
        capabilities_response,
    })
}

/// The entitlements to sign a primary component with, as in Python
/// warpsign's `_sign_components` and `patch_app_binary`
/// Folders of apps that can't come along with a re-signed app, and why
const UNSUPPORTED_APPS: [(&str, &str); 3] = [
    (
        "AppClips",
        "App Clips need an App ID and profile of their own, and iOS won't install them with a re-signed app",
    ),
    (
        "Watch",
        "watch apps can only be installed through a paired Apple Watch",
    ),
    (
        "com.apple.WatchPlaceholder",
        "a placeholder for a watch app",
    ),
];

/// Take App Clips and watch apps out of an extracted app, before anything
/// looks at it: left in, they keep their original signature and make iOS
/// refuse the whole app. Not in Python warpsign, which installs fail with.
/// Returns what was removed, and why.
fn remove_unsupported_apps(app_dir: &Path) -> Result<Vec<String>, PipelineError> {
    let mut removed = Vec::new();
    for (folder, why) in UNSUPPORTED_APPS {
        let path = app_dir.join(folder);
        if !path.is_dir() {
            continue;
        }
        let mut apps: Vec<String> = std::fs::read_dir(&path)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        apps.sort();
        std::fs::remove_dir_all(&path).map_err(|source| PipelineError::Io {
            path: path.display().to_string(),
            source,
        })?;
        let what = if apps.is_empty() {
            format!("{folder}/")
        } else {
            apps.iter()
                .map(|app| format!("{folder}/{app}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        removed.push(format!("{what}: {why}"));
    }
    Ok(removed)
}

/// Code iOS won't accept in a signed app: app and extension bundles at
/// any depth that didn't get a provisioning profile of their own (e.g. an
/// App Clip in AppClips/ or a watch app in Watch/), and Mach-O files not
/// signed by `team_id`. Not in Python warpsign, where these only show up
/// as an install failure.
fn unsigned_code(app_dir: &Path, team_id: &str, profiled: &[String]) -> Vec<String> {
    fn walk(
        app_dir: &Path,
        relative: &Path,
        team_id: &str,
        profiled: &[String],
        found: &mut Vec<String>,
    ) {
        let Ok(entries) = std::fs::read_dir(app_dir.join(relative)) else {
            return;
        };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = relative.join(entry.file_name());
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                let is_bundle = matches!(
                    path.extension().and_then(|e| e.to_str()),
                    Some("app" | "appex")
                );
                let shown = slash_path(&path);
                if is_bundle && !profiled.contains(&shown) {
                    found.push(format!(
                        "{shown} has no provisioning profile of its own, so iOS will refuse to install the app. App Clips and watch apps can't be installed this way: remove it from the IPA."
                    ));
                }
                walk(app_dir, &path, team_id, profiled, found);
            } else if file_type.is_file() && is_macho(&entry.path()) {
                let shown = slash_path(&path);
                match signature::read_signature(&entry.path()) {
                    Ok(Some(info)) if info.team_id.as_deref() == Some(team_id) => {}
                    Ok(Some(info)) => found.push(format!(
                        "{shown} is still signed by team {}: iOS may refuse to install the app or load it",
                        info.team_id.as_deref().unwrap_or("(none)")
                    )),
                    Ok(None) => found.push(format!(
                        "{shown} isn't signed: iOS may refuse to install the app or load it"
                    )),
                    // Not something this can read: leave it to iOS
                    Err(_) => {}
                }
            }
        }
    }
    let mut found = Vec::new();
    walk(app_dir, Path::new(""), team_id, profiled, &mut found);
    found
}

/// Whether a file starts like a Mach-O binary (thin or universal)
fn is_macho(path: &Path) -> bool {
    use std::io::Read;
    let mut magic = [0u8; 4];
    let read = std::fs::File::open(path).and_then(|mut file| file.read_exact(&mut magic));
    if read.is_err() {
        return false;
    }
    matches!(u32::from_le_bytes(magic), 0xfeed_face | 0xfeed_facf)
        || matches!(u32::from_be_bytes(magic), 0xcafe_babe | 0xcafe_babf)
}

/// Python warpsign's profile names, which reusing profiles goes by
fn profile_name(bundle_id: &str, profile_type: ProfileType) -> String {
    let type_name = match profile_type {
        ProfileType::Development => "Development",
        ProfileType::AdHoc => "Ad Hoc",
    };
    format!("TS {bundle_id} {type_name}")
}

/// What to do about a component's provisioning profile
enum ProfilePlan {
    /// The portal's current one already does: use it as it is
    Reuse { content: Vec<u8> },
    /// Make one, regenerating `existing_id` if there is one. `reason` says
    /// why the existing one won't do, when it was checked.
    Make {
        existing_id: Option<String>,
        reason: Option<NotReusable>,
    },
}

/// Whether the profile named `name` can be used as it is (see
/// [`crate::profile_reuse`]), from the cache or one download
#[allow(clippy::too_many_arguments)]
fn plan_profile(
    portal: &Portal,
    cache: Option<&ProfileCache>,
    existing: &[PortalProfile],
    name: &str,
    app_identifier: &str,
    certificate_der: &[u8],
    device_udids: &[String],
    entitlements: &Dictionary,
    now: std::time::SystemTime,
) -> Result<ProfilePlan, PipelineError> {
    let Some(listed) = existing.iter().find(|profile| profile.name == name) else {
        return Ok(ProfilePlan::Make {
            existing_id: None,
            reason: None,
        });
    };
    let make = |reason| {
        Ok(ProfilePlan::Make {
            existing_id: Some(listed.id.clone()),
            reason,
        })
    };
    if listed.state != "ACTIVE" {
        return make(Some(NotReusable::State(listed.state.clone())));
    }
    let expires = listed.expiration_date.as_deref().and_then(portal_date);
    let cached = match (cache, expires) {
        (Some(cache), Some(expires)) => cache.load(&listed.id, expires),
        _ => None,
    };
    let content = match cached {
        Some(content) => content,
        // Can't tell, so regenerate, as before
        None => match portal.download_profile(&listed.id) {
            Ok(content) => {
                if let Some(cache) = cache {
                    cache.store(&listed.id, &content);
                }
                content
            }
            Err(_) => return make(None),
        },
    };
    let Ok(profile) = ProvisioningProfile::parse(&content) else {
        return make(None);
    };
    match check(
        &profile,
        app_identifier,
        certificate_der,
        device_udids,
        entitlements,
        now,
    ) {
        Ok(()) => Ok(ProfilePlan::Reuse { content }),
        Err(reason) => make(Some(reason)),
    }
}

fn signing_entitlements(
    component: &AppComponent,
    plan: &Plan,
    mapping: &mut BundleMapping,
    team_id: &str,
    options: &SignOptions,
    log: &StepLog,
) -> Dictionary {
    let (entitlements, messages) =
        signing_entitlements_quiet(component, plan, mapping, team_id, options);
    for message in messages {
        log.log(message);
    }
    entitlements
}

/// What [`signing_entitlements`] makes, with its messages returned rather
/// than logged, so it can also be worked out ahead of the Sign step
fn signing_entitlements_quiet(
    component: &AppComponent,
    plan: &Plan,
    mapping: &mut BundleMapping,
    team_id: &str,
    options: &SignOptions,
) -> (Dictionary, Vec<String>) {
    let mut messages = Vec::new();
    let bundle_id = mapping.map_bundle_id(&component.bundle_id);
    let mut entitlements = if component.entitlements.is_empty() {
        messages.push(format!(
            "Warning: {} has no entitlements, using the minimum",
            describe(component)
        ));
        Dictionary::new()
    } else {
        let mut mapped = mapping.map_entitlements(&component.entitlements, Some(&bundle_id));
        for key in &plan.removals {
            mapped.remove(key);
        }
        mapped
    };

    // _ensure_critical_entitlements
    let mut missing = Vec::new();
    for (key, value) in [
        ("application-identifier", format!("{team_id}.{bundle_id}")),
        ("com.apple.developer.team-identifier", team_id.to_string()),
    ] {
        if !entitlements.contains_key(key) {
            entitlements.insert(key.into(), Value::String(value));
            missing.push(key);
        }
    }
    if !missing.is_empty() && !component.entitlements.is_empty() {
        messages.push(format!(
            "Warning: added missing {} for {}",
            missing.join(", "),
            describe(component)
        ));
    }

    if options.patch_debug {
        entitlements.insert("get-task-allow".into(), Value::Boolean(true));
    } else {
        entitlements.remove("get-task-allow");
        if component.entitlements.contains_key("get-task-allow") {
            messages.push(format!(
                "{}: get-task-allow removed, as debugging is off in the signing options",
                describe(component)
            ));
        }
    }
    (entitlements, messages)
}

/// The original identifier mapped to `new_id`, if any
fn original_of(mapping: &BundleMapping, new_id: &str) -> Option<String> {
    mapping
        .mappings()
        .iter()
        .find(|(_, mapped)| mapped.new_id == new_id)
        .map(|(original, _)| original.clone())
}

fn describe(component: &AppComponent) -> String {
    if component.is_main_app() {
        "App".into()
    } else {
        slash_path(&component.path)
    }
}

fn join(items: &BTreeSet<String>) -> String {
    items.iter().cloned().collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn component(entitlements: Dictionary, is_primary: bool) -> AppComponent {
        AppComponent {
            path: PathBuf::new(),
            bundle_id: "com.example.test".into(),
            executable: PathBuf::from("Test"),
            entitlements,
            info_plist: Dictionary::new(),
            is_primary,
            entitlements_bundle_id: None,
        }
    }

    fn dict(pairs: &[(&str, Value)]) -> Dictionary {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.clone()))
            .collect()
    }

    fn change(key: &str, kind: ChangeKind, reason: Option<ChangeReason>) -> EntitlementChange {
        EntitlementChange {
            key: key.into(),
            kind,
            reason,
        }
    }

    #[test]
    fn explains_removed_entitlements() {
        let original = dict(&[
            ("com.apple.developer.healthkit", true.into()),
            ("com.apple.developer.in-app-payments", Value::Array(vec![])),
            ("get-task-allow", true.into()),
            ("com.example.mystery", true.into()),
        ]);
        let signed = dict(&[
            ("application-identifier", "T.x".into()),
            ("com.apple.developer.team-identifier", "T".into()),
        ]);
        let removals: BTreeSet<String> = [
            "com.apple.developer.healthkit",
            "com.apple.developer.in-app-payments",
            "get-task-allow",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        assert_eq!(
            entitlement_changes(
                &component(original.clone(), true),
                &signed,
                Some(&removals),
                false
            ),
            [
                change(
                    "application-identifier",
                    ChangeKind::Added,
                    Some(ChangeReason::Required)
                ),
                change(
                    "com.apple.developer.healthkit",
                    ChangeKind::Removed,
                    Some(ChangeReason::NoCapability)
                ),
                change(
                    "com.apple.developer.in-app-payments",
                    ChangeKind::Removed,
                    Some(ChangeReason::Banned)
                ),
                change(
                    "com.apple.developer.team-identifier",
                    ChangeKind::Added,
                    Some(ChangeReason::Required)
                ),
                change("com.example.mystery", ChangeKind::Removed, None),
                change(
                    "get-task-allow",
                    ChangeKind::Removed,
                    Some(ChangeReason::DebuggingOff)
                ),
            ]
        );

        // Frameworks are signed without entitlements
        let changes =
            entitlement_changes(&component(original, false), &Dictionary::new(), None, false);
        assert_eq!(changes.len(), 4);
        assert!(
            changes
                .iter()
                .all(|change| change.kind == ChangeKind::Removed
                    && change.reason == Some(ChangeReason::NotPrimary))
        );
    }
}
