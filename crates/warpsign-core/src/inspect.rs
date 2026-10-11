//! Finds an app's signable components and the team IDs it was signed with.
//!
//! Port of Python warpsign's `IPAInspector`, pinned by
//! `fixtures/app_inspection.json`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use plist::{Dictionary, Value};

use crate::signature;

#[derive(Debug, thiserror::Error)]
pub enum InspectError {
    #[error("{path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("{path} is not a valid Info.plist: {message}")]
    InfoPlist { path: String, message: String },
    #[error("{path} has no CFBundleIdentifier")]
    MissingBundleId { path: String },
}

/// Something in the app bundle that gets signed
#[derive(Debug, Clone, PartialEq)]
pub struct AppComponent {
    /// Path relative to the app; empty for the app itself
    pub path: PathBuf,
    /// CFBundleIdentifier for apps and extensions; the file stem for
    /// frameworks, dylibs and plug-in bundles
    pub bundle_id: String,
    /// Main executable, relative to the app
    pub executable: PathBuf,
    /// Entitlements the executable is currently signed with
    pub entitlements: Dictionary,
    /// Info.plist contents (only read for apps and extensions)
    pub info_plist: Dictionary,
    /// Apps and extensions get their own App ID, profile and entitlements.
    /// Frameworks, dylibs and plug-in bundles are only re-signed.
    pub is_primary: bool,
    /// Bundle ID part of the `application-identifier` entitlement
    pub entitlements_bundle_id: Option<String>,
}

impl AppComponent {
    pub fn is_main_app(&self) -> bool {
        self.path.as_os_str().is_empty()
    }
}

fn io_error(path: &Path) -> impl FnOnce(std::io::Error) -> InspectError + '_ {
    move |source| InspectError::Io {
        path: path.display().to_string(),
        source,
    }
}

fn read_info_plist(path: &Path) -> Result<Dictionary, InspectError> {
    match Value::from_file(path) {
        Ok(Value::Dictionary(dict)) => Ok(dict),
        Ok(_) => Err(InspectError::InfoPlist {
            path: path.display().to_string(),
            message: "not a dictionary".into(),
        }),
        Err(e) => Err(InspectError::InfoPlist {
            path: path.display().to_string(),
            message: e.to_string(),
        }),
    }
}

fn string_value<'a>(dict: &'a Dictionary, key: &str) -> Option<&'a str> {
    dict.get(key)
        .and_then(Value::as_string)
        .filter(|s| !s.is_empty())
}

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn extension(path: &Path) -> Option<&str> {
    path.extension().and_then(|e| e.to_str())
}

/// Paths (relative to `app_dir`) matching the patterns Python warpsign globs
/// for: `**/*.framework`, `**/*.dylib`, `**/PlugIns/*.bundle`, `**/*.appex`
/// and `*.app`
fn find_candidates(app_dir: &Path) -> Result<Vec<PathBuf>, InspectError> {
    fn walk(app_dir: &Path, relative: &Path, found: &mut Vec<PathBuf>) -> Result<(), InspectError> {
        let dir = app_dir.join(relative);
        let mut entries = std::fs::read_dir(&dir)
            .map_err(io_error(&dir))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(io_error(&dir))?;
        entries.sort_by_key(|entry| entry.file_name());

        for entry in entries {
            let path = relative.join(entry.file_name());
            let is_match = match extension(&path) {
                Some("framework" | "dylib" | "appex") => true,
                Some("bundle") => relative
                    .file_name()
                    .is_some_and(|parent| parent == "PlugIns"),
                Some("app") => relative.as_os_str().is_empty(),
                _ => false,
            };
            if is_match {
                found.push(path.clone());
            }
            let file_type = entry.file_type().map_err(io_error(&entry.path()))?;
            if file_type.is_dir() {
                walk(app_dir, &path, found)?;
            }
        }
        Ok(())
    }

    let mut found = Vec::new();
    walk(app_dir, Path::new(""), &mut found)?;
    // Deepest first, so nested code is signed before what contains it. Ties
    // are ordered by path string (as in the fixtures), not component-wise.
    found.sort_by(|a, b| {
        let depth = |p: &Path| p.components().count();
        depth(b)
            .cmp(&depth(a))
            .then_with(|| a.to_string_lossy().cmp(&b.to_string_lossy()))
    });
    Ok(found)
}

fn entitlements_bundle_id(entitlements: &Dictionary) -> Option<String> {
    string_value(entitlements, "application-identifier")
        .and_then(|app_id| app_id.split_once('.'))
        .map(|(_, bundle_id)| bundle_id.to_owned())
}

/// Find the signable components of an extracted .app, reading entitlements
/// from each executable's signature
pub fn inspect_app(app_dir: &Path) -> Result<Vec<AppComponent>, InspectError> {
    inspect_app_with(app_dir, |executable| {
        // Unreadable signatures count as no entitlements, as with ldid in Python
        signature::read_entitlements(executable).unwrap_or_default()
    })
}

/// [`inspect_app`] with a custom way to read an executable's entitlements
pub fn inspect_app_with(
    app_dir: &Path,
    read_entitlements: impl Fn(&Path) -> Dictionary,
) -> Result<Vec<AppComponent>, InspectError> {
    let mut components = Vec::new();

    let main_info_path = app_dir.join("Info.plist");
    let main_info = read_info_plist(&main_info_path)?;
    if let Some(executable) = string_value(&main_info, "CFBundleExecutable")
        && app_dir.join(executable).exists()
    {
        let bundle_id = string_value(&main_info, "CFBundleIdentifier").ok_or_else(|| {
            InspectError::MissingBundleId {
                path: main_info_path.display().to_string(),
            }
        })?;
        let entitlements = read_entitlements(&app_dir.join(executable));
        components.push(AppComponent {
            path: PathBuf::new(),
            bundle_id: bundle_id.to_owned(),
            executable: PathBuf::from(executable),
            entitlements_bundle_id: entitlements_bundle_id(&entitlements),
            entitlements,
            info_plist: main_info.clone(),
            is_primary: true,
        });
    }

    for path in find_candidates(app_dir)? {
        let bundle = app_dir.join(&path);
        let is_primary = matches!(extension(&path), Some("app" | "appex"));

        if !is_primary {
            let executable = if extension(&path) == Some("dylib") {
                path.clone()
            } else {
                // The declared executable, falling back to Foo.framework/Foo.
                // An unreadable Info.plist also falls back.
                let declared = read_info_plist(&bundle.join("Info.plist"))
                    .ok()
                    .and_then(|info| string_value(&info, "CFBundleExecutable").map(String::from));
                path.join(declared.unwrap_or_else(|| file_stem(&path)))
            };
            if !app_dir.join(&executable).exists() {
                continue;
            }
            let entitlements = read_entitlements(&app_dir.join(&executable));
            components.push(AppComponent {
                bundle_id: file_stem(&path),
                entitlements_bundle_id: entitlements_bundle_id(&entitlements),
                entitlements,
                path,
                executable,
                info_plist: Dictionary::new(),
                is_primary: false,
            });
            continue;
        }

        let info_path = bundle.join("Info.plist");
        if !info_path.exists() {
            continue;
        }
        let info = read_info_plist(&info_path)?;
        let Some(executable) = string_value(&info, "CFBundleExecutable") else {
            continue;
        };
        let executable = path.join(executable);
        if !app_dir.join(&executable).exists() {
            continue;
        }
        let bundle_id = string_value(&info, "CFBundleIdentifier")
            .ok_or_else(|| InspectError::MissingBundleId {
                path: info_path.display().to_string(),
            })?
            .to_owned();
        let entitlements = read_entitlements(&app_dir.join(&executable));
        components.push(AppComponent {
            path,
            bundle_id,
            executable,
            entitlements_bundle_id: entitlements_bundle_id(&entitlements),
            entitlements,
            info_plist: info,
            is_primary: true,
        });
    }

    Ok(components)
}

/// An Apple team ID: exactly 10 alphanumeric characters
pub fn is_valid_team_id(team_id: &str) -> bool {
    team_id.chars().count() == 10 && team_id.chars().all(char::is_alphanumeric)
}

/// Team IDs found in a component's entitlements and Info.plist. An app can
/// carry several (e.g. shared keychain groups from another team).
pub fn extract_team_ids(entitlements: &Dictionary, info_plist: &Dictionary) -> BTreeSet<String> {
    let mut team_ids = BTreeSet::new();
    let mut add = |candidate: &str| {
        if is_valid_team_id(candidate) {
            team_ids.insert(candidate.to_owned());
        }
    };

    if let Some(team_id) = string_value(entitlements, "com.apple.developer.team-identifier") {
        add(team_id);
    }
    if let Some(prefix) = string_value(info_plist, "AppIdentifierPrefix") {
        add(prefix.trim_end_matches('.'));
    }
    if let Some(app_id) = string_value(entitlements, "application-identifier") {
        add(app_id.split('.').next().unwrap_or_default());
    }
    if let Some(Value::Array(groups)) = entitlements.get("keychain-access-groups") {
        for group in groups.iter().filter_map(Value::as_string) {
            if let Some((team_id, _)) = group.split_once('.') {
                add(team_id);
            }
        }
    }
    team_ids
}

/// All team IDs found across an app's components, sorted
pub fn team_ids(components: &[AppComponent]) -> Vec<String> {
    components
        .iter()
        .flat_map(|c| extract_team_ids(&c.entitlements, &c.info_plist))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
