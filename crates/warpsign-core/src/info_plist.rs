//! Info.plist patching: identifier remapping, optional tweaks to the main
//! app (name, device support, file sharing, display options), and byte-level
//! replacement of mapped IDs anywhere in the file.
//!
//! Port of Python warpsign's `AppPatcher.patch_info_plist`, pinned by
//! `fixtures/info_plist_patching.json`. Changing the app icon is not ported
//! yet.

use std::path::Path;

use plist::{Dictionary, Value};

use crate::bundle_mapping::{BundleMapping, IdType};

#[derive(Debug, thiserror::Error)]
pub enum InfoPlistError {
    #[error("{path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("{path} is not a valid Info.plist: {message}")]
    Invalid { path: String, message: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StatusBarStyle {
    /// Leave as is
    #[default]
    Default,
    Hidden,
    /// Light content, for dark backgrounds
    Light,
    /// Dark content, for light backgrounds
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UiStyle {
    /// Leave as is
    #[default]
    Automatic,
    Light,
    Dark,
}

/// Options that affect Info.plist patching
#[derive(Debug, Clone, PartialEq)]
pub struct PatchingOptions {
    /// Replace mapped IDs anywhere in the file, byte for byte
    pub patch_ids: bool,
    /// Keep every component's original CFBundleIdentifier
    pub force_original_id: bool,
    /// New display name for the main app
    pub bundle_name: Option<String>,
    /// Allow iPhone and iPad, drop device restrictions, lower MinimumOSVersion
    pub patch_all_devices: bool,
    /// Enable Files app and iTunes file sharing
    pub patch_file_sharing: bool,
    /// Enable ProMotion (120Hz)
    pub patch_promotion: bool,
    /// Require fullscreen on iPad (also hides the status bar)
    pub patch_fullscreen: bool,
    /// Allow every orientation
    pub patch_orientation: bool,
    /// Enable Game Mode
    pub patch_game_mode: bool,
    pub patch_status_bar: StatusBarStyle,
    pub patch_user_interface_style: UiStyle,
    /// Remove CFBundleURLTypes
    pub remove_url_schemes: bool,
}

impl Default for PatchingOptions {
    fn default() -> Self {
        Self {
            patch_ids: true,
            force_original_id: false,
            bundle_name: None,
            patch_all_devices: false,
            patch_file_sharing: false,
            patch_promotion: false,
            patch_fullscreen: false,
            patch_orientation: false,
            patch_game_mode: false,
            patch_status_bar: StatusBarStyle::Default,
            patch_user_interface_style: UiStyle::Automatic,
            remove_url_schemes: false,
        }
    }
}

/// Info.plist keys whose values are identifiers to remap
const IDENTIFIER_KEYS: [(&str, IdType); 3] = [
    ("com.apple.security.application-groups", IdType::AppGroup),
    ("keychain-access-groups", IdType::Keychain),
    (
        "com.apple.developer.icloud-container-identifiers",
        IdType::ICloud,
    ),
];

const ALL_ORIENTATIONS: [&str; 4] = [
    "UIInterfaceOrientationPortrait",
    "UIInterfaceOrientationLandscapeLeft",
    "UIInterfaceOrientationLandscapeRight",
    "UIInterfaceOrientationPortraitUpsideDown",
];

/// Patches the Info.plists of one app. Patch the main app first: with
/// `force_original_id`, which replacements apply depends on its bundle ID.
pub struct InfoPlistPatcher {
    options: PatchingOptions,
    main_bundle_id: Option<String>,
}

impl InfoPlistPatcher {
    pub fn new(options: PatchingOptions) -> Self {
        Self {
            options,
            main_bundle_id: None,
        }
    }

    /// Patch the Info.plist at `path` in place (written as XML) and return
    /// its final contents
    pub fn patch_file(
        &mut self,
        path: &Path,
        mapping: &mut BundleMapping,
        is_main_app: bool,
    ) -> Result<Dictionary, InfoPlistError> {
        let io_error = |source| InfoPlistError::Io {
            path: path.display().to_string(),
            source,
        };
        let invalid = |message: String| InfoPlistError::Invalid {
            path: path.display().to_string(),
            message,
        };

        let mut info = match Value::from_file(path) {
            Ok(Value::Dictionary(info)) => info,
            Ok(_) => return Err(invalid("not a dictionary".into())),
            Err(e) => return Err(invalid(e.to_string())),
        };

        self.patch_identifiers(&mut info, mapping, is_main_app);
        if is_main_app {
            self.patch_main_app(&mut info);
        }

        let mut bytes = Vec::new();
        Value::Dictionary(info)
            .to_writer_xml(&mut bytes)
            .map_err(|e| invalid(e.to_string()))?;

        if self.options.patch_ids {
            let mut replacements: Vec<(String, String)> =
                self.filter_replacements(mapping, mapping.binary_patches().into_iter().collect());
            // Longest first, so a shorter ID can't clobber part of a longer one
            replacements.sort_by_key(|(old, _)| std::cmp::Reverse(old.chars().count()));
            for (old, new) in &replacements {
                bytes = replace_all(&bytes, old.as_bytes(), new.as_bytes());
            }
        }

        std::fs::write(path, &bytes).map_err(io_error)?;
        match Value::from_reader_xml(bytes.as_slice()) {
            Ok(Value::Dictionary(info)) => Ok(info),
            Ok(_) => Err(invalid("not a dictionary after patching".into())),
            Err(e) => Err(invalid(format!("invalid after patching: {e}"))),
        }
    }

    fn patch_identifiers(
        &mut self,
        info: &mut Dictionary,
        mapping: &mut BundleMapping,
        is_main_app: bool,
    ) {
        if let Some(old_bundle_id) = info
            .get("CFBundleIdentifier")
            .and_then(Value::as_string)
            .map(String::from)
        {
            let bundle_id = if self.options.force_original_id {
                old_bundle_id
            } else {
                let new_bundle_id = mapping.map_bundle_id(&old_bundle_id);
                info.insert(
                    "CFBundleIdentifier".into(),
                    Value::String(new_bundle_id.clone()),
                );
                new_bundle_id
            };
            if is_main_app {
                self.main_bundle_id = Some(bundle_id);
            }
        }

        for (key, id_type) in IDENTIFIER_KEYS {
            let Some(value) = info.get(key) else {
                continue;
            };
            let values = match value {
                Value::Array(values) => values.clone(),
                other => vec![other.clone()],
            };
            let mapped = values
                .into_iter()
                .map(|value| match value {
                    Value::String(id) => Value::String(mapping.map_id(&id, id_type)),
                    other => other,
                })
                .collect();
            info.insert(key.into(), Value::Array(mapped));
        }
    }

    fn patch_main_app(&self, info: &mut Dictionary) {
        let options = &self.options;
        let orientations = || {
            Value::Array(
                ALL_ORIENTATIONS
                    .iter()
                    .map(|o| Value::String((*o).into()))
                    .collect(),
            )
        };

        if let Some(name) = &options.bundle_name {
            set(info, "CFBundleDisplayName", name.as_str().into());
            set(info, "CFBundleName", name.as_str().into());
        }
        if options.patch_all_devices {
            info.remove("UISupportedDevices");
            set(
                info,
                "UIDeviceFamily",
                Value::Array(vec![1.into(), 2.into()]),
            );
            set(info, "MinimumOSVersion", "10.0".into());
        }
        if options.patch_file_sharing {
            set(info, "UIFileSharingEnabled", true.into());
            set(info, "UISupportsDocumentBrowser", true.into());
            set(info, "LSSupportsOpeningDocumentsInPlace", true.into());
        }
        if options.patch_promotion {
            set(info, "CADisableMinimumFrameDurationOnPhone", true.into());
            set(info, "CAHighFrameRateDisplay", true.into());
        }
        if options.patch_fullscreen {
            set(info, "UIRequiresFullScreen", true.into());
            set(info, "UIStatusBarHidden", true.into());
            set(
                info,
                "UIViewControllerBasedStatusBarAppearance",
                false.into(),
            );
        }
        if options.patch_orientation {
            set(info, "UISupportedInterfaceOrientations", orientations());
            set(
                info,
                "UISupportedInterfaceOrientations~ipad",
                orientations(),
            );
        }
        if options.patch_game_mode {
            set(info, "GCSupportsGameMode", true.into());
        }
        match options.patch_status_bar {
            StatusBarStyle::Default => {}
            StatusBarStyle::Hidden => {
                set(
                    info,
                    "UIViewControllerBasedStatusBarAppearance",
                    false.into(),
                );
                set(info, "UIStatusBarHidden", true.into());
            }
            StatusBarStyle::Light => {
                set(
                    info,
                    "UIViewControllerBasedStatusBarAppearance",
                    false.into(),
                );
                set(
                    info,
                    "UIStatusBarStyle",
                    "UIStatusBarStyleLightContent".into(),
                );
            }
            StatusBarStyle::Dark => {
                set(
                    info,
                    "UIViewControllerBasedStatusBarAppearance",
                    false.into(),
                );
                set(
                    info,
                    "UIStatusBarStyle",
                    "UIStatusBarStyleDarkContent".into(),
                );
            }
        }
        match options.patch_user_interface_style {
            UiStyle::Automatic => {}
            UiStyle::Light => set(info, "UIUserInterfaceStyle", "Light".into()),
            UiStyle::Dark => set(info, "UIUserInterfaceStyle", "Dark".into()),
        }
        if options.remove_url_schemes {
            info.remove("CFBundleURLTypes");
        }
    }

    /// With `force_original_id`, drop replacements of the original bundle ID
    /// (and IDs starting with it) so they stay in the plist. Port of Python
    /// warpsign's `_filter_replacements`.
    fn filter_replacements(
        &self,
        mapping: &BundleMapping,
        replacements: Vec<(String, String)>,
    ) -> Vec<(String, String)> {
        if !self.options.force_original_id {
            return replacements;
        }
        let main_bundle_id = self.main_bundle_id.as_deref().unwrap_or_default();
        replacements
            .into_iter()
            .filter(|(old, _)| {
                old.starts_with("iCloud.")
                    || old.starts_with("group.")
                    || looks_like_team_id(old)
                    || mapping
                        .original_team_ids()
                        .iter()
                        .any(|team_id| old.contains(team_id.as_str()))
                    || (!main_bundle_id.is_empty() && !old.starts_with(main_bundle_id))
            })
            .collect()
    }
}

fn set(info: &mut Dictionary, key: &str, value: Value) {
    info.insert(key.into(), value);
}

/// Python's `s.isupper() and s.isalnum() and len(s) == 10`
fn looks_like_team_id(s: &str) -> bool {
    s.chars().count() == 10
        && s.chars().all(char::is_alphanumeric)
        && s.chars().any(char::is_alphabetic)
        && s.chars()
            .filter(|c| c.is_alphabetic())
            .all(char::is_uppercase)
}

/// Replace every non-overlapping occurrence, scanning left to right
/// (like Python's `bytes.replace`)
fn replace_all(haystack: &[u8], old: &[u8], new: &[u8]) -> Vec<u8> {
    if old.is_empty() {
        return haystack.to_vec();
    }
    let mut result = Vec::with_capacity(haystack.len());
    let mut i = 0;
    while i < haystack.len() {
        if haystack[i..].starts_with(old) {
            result.extend_from_slice(new);
            i += old.len();
        } else {
            result.push(haystack[i]);
            i += 1;
        }
    }
    result
}
