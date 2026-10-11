//! End-to-end operations on IPAs, run as reported steps (see `progress`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use plist::{Dictionary, Value};

use crate::ProfileType;
use crate::bundle_mapping::BundleMapping;
use crate::entitlements::EntitlementsProcessor;
use crate::identity::{CertificateKind, Identity, IdentityError};
use crate::info_plist::{InfoPlistError, InfoPlistPatcher, PatchingOptions};
use crate::inspect::{AppComponent, InspectError, inspect_app_with, slash_path, team_ids};
use crate::ipa::{IpaError, extract_ipa, package_ipa};
use crate::portal::{Portal, PortalError};
use crate::profile::{PpqRisk, ProfileError, ProvisioningProfile, check_ppq_risk};
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
}

impl Default for SignOptions {
    fn default() -> Self {
        Self {
            patching: PatchingOptions::default(),
            encode_ids: true,
            patch_debug: false,
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
    let (mut mapping, plans, app_groups, icloud_containers) =
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
                if !plan.entitlements_to_remove.is_empty() {
                    log.log(format!(
                        "  not supported by this account, so removed: {}",
                        join(&plan.entitlements_to_remove)
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
            Ok::<_, PipelineError>((mapping, plans, app_groups, icloud_containers))
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

        let mut group_ids = Vec::new();
        for group in &app_groups {
            let name = format!("WS App Group {}", group.replace('.', " "));
            let registered_group = portal.register_app_group(group, &name)?;
            log.log(format!("App group {group}: {}", registered_group.id));
            group_ids.push(registered_group.id);
            record(&mut registered, &mapping, group);
        }

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
            let name = format!("WS iCloud Container {}", container.replace('.', " "));
            let registered_container = portal.register_cloud_container(&container, &name)?;
            log.log(format!(
                "iCloud container {container}: {}",
                registered_container.id
            ));
            container_ids.push(registered_container.id);
            record(&mut registered, &mapping, &container);
        }

        let mut bundles = Vec::new();
        for plan in &plans {
            let component = &app.components[plan.component];
            let kind = if component.is_main_app() {
                "App"
            } else {
                "App Extension"
            };
            // Python warpsign's names
            let name = format!(
                "WS {kind} {}",
                plan.new_bundle_id.rsplit('.').next().unwrap_or_default()
            );
            let bundle = portal.register_bundle_id(&plan.new_bundle_id, &name)?;
            log.log(format!("App ID {}: {}", bundle.identifier, bundle.id));
            record(&mut registered, &mapping, &plan.new_bundle_id);
            for original_team_id in mapping.original_team_ids() {
                if component.bundle_id.contains(original_team_id.as_str()) {
                    registered.insert(original_team_id.clone());
                    registered.insert(team_id.clone());
                }
            }

            if !plan.capabilities.is_empty() {
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
        }
        mapping.registered_identifiers = registered;
        Ok::<_, PipelineError>(bundles)
    })?;

    // _update_info_plists. The main app first: which IDs get replaced with
    // force_original_id depends on its bundle ID.
    let mut patcher = InfoPlistPatcher::new(options.patching.clone());
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
                main_bundle_id = bundle_id;
            }
        }
        Ok::<_, PipelineError>(main_bundle_id)
    })?;

    // _create_provisioning_profiles
    let profiles = run_step(observer, Step::CreateProfiles, |log| {
        let devices = portal.devices()?;
        if devices.is_empty() {
            return Err(PipelineError::Setup(
                "no iPhones or iPads are registered on this team: add this device at developer.apple.com first".into(),
            ));
        }
        log.log(format!("{} devices", devices.len()));
        let device_ids: Vec<String> = devices.into_iter().map(|device| device.id).collect();

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

        // Reuse profiles by name rather than making a new one every time
        let existing = portal.profiles()?;
        let type_name = match profile_type {
            ProfileType::Development => "Development",
            ProfileType::AdHoc => "Ad Hoc",
        };
        let mut profiles = Vec::new();
        let mut parsed = Vec::new();
        for (plan, bundle) in plans.iter().zip(&bundles) {
            let component = &app.components[plan.component];
            let name = format!("TS {} {type_name}", plan.new_bundle_id);
            let existing_id = existing
                .iter()
                .find(|profile| profile.name == name)
                .map(|profile| profile.id.as_str());
            let content = portal.create_or_regenerate_profile(
                existing_id,
                bundle,
                &name,
                std::slice::from_ref(&certificate.id),
                &device_ids,
                profile_type,
            )?;
            let profile = ProvisioningProfile::parse(&content)?;
            log.log(format!(
                "{}: {name}{}{}",
                describe(component),
                if existing_id.is_some() {
                    " (regenerated)"
                } else {
                    ""
                },
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

    run_step(observer, Step::Sign, |log| {
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
        Ok::<_, PipelineError>(())
    })?;

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
    })
}

/// The entitlements to sign a primary component with, as in Python
/// warpsign's `_sign_components` and `patch_app_binary`
fn signing_entitlements(
    component: &AppComponent,
    plan: &Plan,
    mapping: &mut BundleMapping,
    team_id: &str,
    options: &SignOptions,
    log: &StepLog,
) -> Dictionary {
    let bundle_id = mapping.map_bundle_id(&component.bundle_id);
    let mut entitlements = if component.entitlements.is_empty() {
        log.log(format!(
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
        log.log(format!(
            "Warning: added missing {} for {}",
            missing.join(", "),
            describe(component)
        ));
    }

    if options.patch_debug {
        entitlements.insert("get-task-allow".into(), Value::Boolean(true));
    } else {
        entitlements.remove("get-task-allow");
    }
    entitlements
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
