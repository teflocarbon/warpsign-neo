//! Python warpsign's own fix dylibs: `--inject-warpsign-fix` (WarpsignFix
//! and WarpsignFixUI, which remap keychain groups, app groups and iCloud
//! containers to the new team at runtime, reading `remappings.json`) and
//! `--hide-home-indicator` (ForceHideHomeIndicator).
//!
//! A port of `SignOrchestrator._setup_dylibs` and the injection parts of
//! `AppPatcher.patch_app_binary` and `generate_remappings_json`, pinned by
//! `fixtures/warpsign_fix.json`, quirks included. The dylibs themselves are
//! built from source in `ios/WarpsignFix` rather than taken from Python
//! warpsign: WarpSign carries them in its bundle (`FixDylibs/`), and the
//! CLI takes them from the `WarpsignFix-dylibs.zip` release asset. Either
//! way they come in as a folder ([`stage`]).

use std::path::Path;

use crate::bundle_mapping::{BundleMapping, IdType};
use crate::macho_edit::{self, Dylib, DylibKind, MachoEditError};

pub const WARPSIGN_FIX: &str = "WarpsignFix.dylib";
pub const WARPSIGN_FIX_UI: &str = "WarpsignFixUI.dylib";
pub const HOME_INDICATOR: &str = "ForceHideHomeIndicator.dylib";

/// Python warpsign's own dylibs: signed without patching, wherever they are
/// and whether or not this sign injects them
pub const KNOWN_DYLIBS: [&str; 3] = [WARPSIGN_FIX, WARPSIGN_FIX_UI, HOME_INDICATOR];

/// Other sideloading fix-ups that conflict with WarpsignFix
/// (`warpsign/src/constants/conflicts.py`)
pub const CONFLICTING_DYLIBS: [&str; 10] = [
    "pluginsinject.dylib",
    "sideloadFixerLol.dylib",
    "sideloadKeychainfix.dylib",
    "ExtensionFix.dylib",
    "Widget Extension Fix.dylib",
    "Plugins Inject.dylib",
    "pluginsinjectnf.dylib",
    "zxPluginsInject.dylib",
    "Sideloadbypass1_1.0_@thisismanpreets_apptesters.org.dylib",
    "Sideloadbypass2_1.0_@thisismanpreets_apptesters.org.dylib",
];

#[derive(Debug, thiserror::Error)]
pub enum WarpsignFixError {
    #[error("{path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("{path}: {source}")]
    Macho {
        path: String,
        source: MachoEditError,
    },
    /// A dylib to inject isn't in the fix dylibs folder, or none was given
    #[error("{name} isn't in {dir}, so it can't be injected")]
    MissingDylib { name: &'static str, dir: String },
}

/// Which fix dylibs to inject
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FixOptions {
    /// `--inject-warpsign-fix`: WarpsignFix into every non-dylib
    /// executable, WarpsignFixUI into the app's, conflicting fix-ups removed
    pub inject_warpsign_fix: bool,
    /// `--hide-home-indicator`: ForceHideHomeIndicator into the app's
    /// executable
    pub hide_home_indicator: bool,
}

fn io_error(path: &Path) -> impl FnOnce(std::io::Error) -> WarpsignFixError + '_ {
    move |source| WarpsignFixError::Io {
        path: path.display().to_string(),
        source,
    }
}

/// Put the dylibs in the app's Frameworks/ before its components are
/// listed, so they're signed like the rest (`_setup_dylibs`), copied from
/// `dylibs` (a folder of `WarpsignFix.dylib` and so on). Logs what it does
/// through `log`.
pub fn stage(
    app_dir: &Path,
    options: FixOptions,
    dylibs: Option<&Path>,
    log: &mut dyn FnMut(String),
) -> Result<(), WarpsignFixError> {
    // Python warpsign creates Frameworks/ on every sign, injecting or not
    let frameworks = app_dir.join("Frameworks");
    std::fs::create_dir_all(&frameworks).map_err(io_error(&frameworks))?;

    if options.inject_warpsign_fix {
        // Only at the top of Frameworks/, by exact name
        for name in CONFLICTING_DYLIBS {
            let path = frameworks.join(name);
            if path.exists() {
                std::fs::remove_file(&path).map_err(io_error(&path))?;
                log(format!(
                    "Removed Frameworks/{name}: it conflicts with {WARPSIGN_FIX}"
                ));
            }
        }
    }

    let mut wanted = Vec::new();
    if options.inject_warpsign_fix {
        wanted.push(WARPSIGN_FIX);
        wanted.push(WARPSIGN_FIX_UI);
    }
    if options.hide_home_indicator {
        wanted.push(HOME_INDICATOR);
    }
    for name in wanted {
        let path = frameworks.join(name);
        // A copy already in the app (from an earlier sign) is kept as it is
        if path.exists() {
            log(format!("Frameworks/{name} is already there"));
            continue;
        }
        let source = dylibs
            .map(|dir| dir.join(name))
            .filter(|source| source.is_file())
            .ok_or_else(|| WarpsignFixError::MissingDylib {
                name,
                dir: dylibs.map_or("the fix dylibs folder (none was given)".into(), |dir| {
                    dir.display().to_string()
                }),
            })?;
        std::fs::copy(&source, &path).map_err(io_error(&source))?;
        log(format!("Copied {name} to Frameworks"));
    }
    Ok(())
}

/// Whether an executable is one of Python warpsign's own dylibs, which
/// are signed without bundle ID patching or injection
pub fn is_known_dylib(executable: &Path) -> bool {
    executable
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| KNOWN_DYLIBS.contains(&name))
}

/// The load path for a dylib in the app's Frameworks/, from an executable
/// (relative to the app): `@loader_path/` and a `../` per folder between
/// them (`get_dylib_path`). Executables in a nested app (Watch/W.app) go
/// to the outer app's Frameworks/, as in Python, which takes the first
/// `.app` in the path.
pub fn dylib_path(executable: &Path, name: &str) -> String {
    let steps = executable
        .parent()
        .map(|parent| parent.components().count())
        .unwrap_or(0);
    format!("@loader_path/{}Frameworks/{name}", "../".repeat(steps))
}

/// Remove conflicting fix-ups from a component's executable and inject the
/// fix dylibs into it, as `patch_app_binary` does after patching bundle
/// IDs. `executable` is relative to `app_dir`; `is_main` says whether it's
/// the app's own.
pub fn patch_binary(
    app_dir: &Path,
    executable: &Path,
    is_main: bool,
    options: FixOptions,
    log: &mut dyn FnMut(String),
) -> Result<(), WarpsignFixError> {
    let path = app_dir.join(executable);
    let shown = crate::inspect::slash_path(executable);
    let macho_error = |source| WarpsignFixError::Macho {
        path: shown.clone(),
        source,
    };

    if options.inject_warpsign_fix {
        // In each slice, the first dylib command (the install name too)
        // whose path contains a conflicting name, not every one
        let removed = macho_edit::edit_file(&path, |data| {
            macho_edit::remove_first_dylib_command_where(data, |dylib| {
                CONFLICTING_DYLIBS
                    .iter()
                    .any(|name| dylib.path.contains(name))
            })
        })
        .map_err(macho_error)?;
        if removed {
            log(format!("{shown}: removed a conflicting dylib"));
        }
    }

    let mut inject = Vec::new();
    if options.inject_warpsign_fix {
        if path.extension().is_some_and(|e| e == "dylib") {
            // Not into dylibs
        } else {
            inject.push(WARPSIGN_FIX);
        }
        if is_main {
            inject.push(WARPSIGN_FIX_UI);
        }
    }
    if is_main && options.hide_home_indicator {
        inject.push(HOME_INDICATOR);
    }
    for name in inject {
        let dylib = Dylib {
            path: dylib_path(executable, name),
            // LIEF's defaults for add_library
            timestamp: 2,
            current_version: 0,
            compatibility_version: 0,
        };
        // A strong load, appended even if the binary already loads it (an
        // app signed before gets a second): as LIEF's add_library does
        macho_edit::edit_file(&path, |data| {
            macho_edit::append_dylib(data, &dylib, DylibKind::Load).map(|()| true)
        })
        .map_err(macho_error)?;
        log(format!("{shown}: injected {}", dylib.path));
    }
    Ok(())
}

/// `remappings.json`, which WarpsignFix reads: the identifiers that changed,
/// by kind, byte for byte as Python's `json.dump(indent=4)` writes it. None
/// when nothing changed, and Python writes no file.
pub fn remappings_json(mapping: &BundleMapping) -> Option<String> {
    let mut keychain = Vec::new();
    let mut app_groups = Vec::new();
    let mut icloud = Vec::new();
    let mut team_ids = Vec::new();
    for original in mapping.original_team_ids() {
        if original != mapping.team_id() {
            team_ids.push((original.as_str(), mapping.team_id()));
        }
    }
    for (original, mapped) in mapping.mappings() {
        if *original == mapped.new_id {
            continue;
        }
        let entry = (original.as_str(), mapped.new_id.as_str());
        match mapped.id_type {
            IdType::Keychain => keychain.push(entry),
            IdType::AppGroup => app_groups.push(entry),
            IdType::ICloud => icloud.push(entry),
            IdType::Bundle => {}
        }
    }
    let sections = [
        ("keychainAccessGroups", keychain),
        ("appGroups", app_groups),
        ("iCloudContainers", icloud),
        ("teamIds", team_ids),
    ];
    if sections.iter().all(|(_, entries)| entries.is_empty()) {
        return None;
    }
    let mut out = String::from("{\n");
    for (n, (key, entries)) in sections.iter().enumerate() {
        out.push_str(&format!("    {}: ", python_json_string(key)));
        if entries.is_empty() {
            out.push_str("[]");
        } else {
            out.push_str("[\n");
            for (i, (original, remapped)) in entries.iter().enumerate() {
                out.push_str(&format!(
                    "        {{\n            \"original\": {},\n            \"remapped\": {}\n        }}",
                    python_json_string(original),
                    python_json_string(remapped)
                ));
                out.push_str(if i + 1 < entries.len() { ",\n" } else { "\n" });
            }
            out.push_str("    ]");
        }
        out.push_str(if n + 1 < sections.len() { ",\n" } else { "\n" });
    }
    out.push('}');
    Some(out)
}

/// A JSON string as Python's json module writes it (ensure_ascii)
fn python_json_string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            ' '..='~' => out.push(c),
            _ => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            }
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_strings_as_python_does() {
        assert_eq!(python_json_string("a\"b\\c"), r#""a\"b\\c""#);
        assert_eq!(python_json_string("é\u{7f}\n"), r#""\u00e9\u007f\n""#);
        assert_eq!(python_json_string("😀"), r#""\ud83d\ude00""#);
    }
}
