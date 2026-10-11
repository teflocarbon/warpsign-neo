//! Signs app bundles with apple-codesign (the library behind rcodesign), so
//! no Mac or Apple tooling is needed. See docs/spikes/rcodesign.md.
//!
//! Compared to running rcodesign with default settings, this:
//! - marks every bundle's main executable as executable (755), since zip
//!   extraction can drop the bit and the signer keeps whatever it finds
//! - always records the team ID, which apple-codesign only takes from
//!   certificates that chain to Apple's root (dyld refuses frameworks without it)
//! - never contacts a time-stamp server (codesign doesn't for iOS signing)

use std::path::Path;

use apple_codesign::cryptography::{InMemoryPrivateKey, parse_pfx_data};
use apple_codesign::{AppleCertificate, SettingsScope, SigningSettings, UnifiedSigner};
use plist::{Dictionary, Value};
use x509_certificate::CapturedX509Certificate;

#[derive(Debug, thiserror::Error)]
pub enum SigningError {
    #[error("loading signing certificate: {0}")]
    Certificate(String),
    #[error("{path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("invalid entitlements for {bundle}: {message}")]
    Entitlements { bundle: String, message: String },
    #[error("signing {path}: {message}")]
    Sign { path: String, message: String },
}

/// A signing certificate and its private key
pub struct SigningIdentity {
    certificate: CapturedX509Certificate,
    key: InMemoryPrivateKey,
}

impl SigningIdentity {
    /// Load from a .p12 file's contents.
    ///
    /// apple-codesign 0.29 only reads legacy (3DES) encrypted .p12 files, as
    /// exported by Keychain Access. OpenSSL 3's default AES encryption fails
    /// with an "incorrect password" error even when the password is right.
    pub fn from_p12(data: &[u8], password: &str) -> Result<Self, SigningError> {
        let (certificate, key) =
            parse_pfx_data(data, password).map_err(|e| SigningError::Certificate(e.to_string()))?;
        Ok(Self { certificate, key })
    }

    /// Team ID from the certificate's organizational unit
    pub fn team_id(&self) -> Option<String> {
        self.certificate.apple_team_id()
    }
}

/// Entitlements to sign one bundle with
#[derive(Debug, Clone)]
pub struct BundleEntitlements {
    /// Bundle path relative to the app, e.g. `PlugIns/Widget.appex`.
    /// An empty string means the app itself.
    pub bundle_path: String,
    pub entitlements: Dictionary,
}

/// Sign an .app bundle in place, including everything nested inside it.
///
/// Each bundle in `entitlements` is signed with exactly those entitlements;
/// anything not listed (frameworks, dylibs) is signed without entitlements.
/// Existing signatures are replaced.
pub fn sign_app_bundle(
    app_dir: &Path,
    identity: &SigningIdentity,
    team_id: &str,
    entitlements: &[BundleEntitlements],
) -> Result<(), SigningError> {
    mark_executables(app_dir)?;

    let mut settings = SigningSettings::default();
    settings.set_signing_key(&identity.key, identity.certificate.clone());
    settings.chain_apple_certificates();
    settings.set_team_id(team_id);

    for bundle in entitlements {
        let scope = if bundle.bundle_path.is_empty() {
            SettingsScope::Main
        } else {
            SettingsScope::Path(bundle.bundle_path.clone())
        };
        let mut xml = Vec::new();
        Value::Dictionary(bundle.entitlements.clone())
            .to_writer_xml(&mut xml)
            .map_err(|e| SigningError::Entitlements {
                bundle: bundle.bundle_path.clone(),
                message: e.to_string(),
            })?;
        let xml = String::from_utf8(xml).map_err(|e| SigningError::Entitlements {
            bundle: bundle.bundle_path.clone(),
            message: e.to_string(),
        })?;
        settings
            .set_entitlements_xml(scope, xml)
            .map_err(|e| SigningError::Entitlements {
                bundle: bundle.bundle_path.clone(),
                message: e.to_string(),
            })?;
    }

    UnifiedSigner::new(settings)
        .sign_path_in_place(app_dir)
        .map_err(|e| SigningError::Sign {
            path: app_dir.display().to_string(),
            message: e.to_string(),
        })
}

/// Set 755 on the main executable of `dir` and of every bundle inside it
fn mark_executables(dir: &Path) -> Result<(), SigningError> {
    let io_error = |path: &Path| {
        let path = path.display().to_string();
        move |source| SigningError::Io { path, source }
    };

    if let Ok(Value::Dictionary(info)) = Value::from_file(dir.join("Info.plist"))
        && let Some(executable) = info.get("CFBundleExecutable").and_then(Value::as_string)
    {
        let executable = dir.join(executable);
        if executable.is_file() {
            set_executable(&executable).map_err(io_error(&executable))?;
        }
    }

    for entry in std::fs::read_dir(dir).map_err(io_error(dir))? {
        let path = entry.map_err(io_error(dir))?.path();
        if path.is_dir() && !path.is_symlink() {
            mark_executables(&path)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn set_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> std::io::Result<()> {
    // No exec bit to set; it's recorded when the IPA is zipped instead
    Ok(())
}
