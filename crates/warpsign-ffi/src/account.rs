//! Apple ID sign-in, signing identities and signing, for Swift.

use std::path::Path;
use std::sync::Arc;

use warpsign_core::identity::{self, CertificateKind, Identity, IdentityError};
use warpsign_core::info_plist::{PatchingOptions, StatusBarStyle, UiStyle};
use warpsign_core::pipeline::{self, SIGN_STEPS, SignOptions};
use warpsign_core::portal::api::{PortalCertificate, Team};
use warpsign_core::portal::http;
use warpsign_core::portal::{AuthError, Authenticator, Portal, PortalError, SessionState};

use crate::{ObserverBridge, StepInfo, StepObserver, WarpsignError};

// HTTP, done by URLSession in the app

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct HttpHeader {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct HttpRequest {
    /// "GET", "POST", "PUT", "PATCH" or "DELETE"
    pub method: String,
    pub url: String,
    pub headers: Vec<HttpHeader>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct HttpResponse {
    pub status: u16,
    /// Every header line; `Set-Cookie` may repeat or be comma-joined
    pub headers: Vec<HttpHeader>,
    pub body: Vec<u8>,
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum TransportFailure {
    #[error("{message}")]
    Failed { message: String },
}

impl From<uniffi::UnexpectedUniFFICallbackError> for TransportFailure {
    fn from(error: uniffi::UnexpectedUniFFICallbackError) -> Self {
        TransportFailure::Failed {
            message: error.reason,
        }
    }
}

/// Sends one HTTP request, blocking until the response arrives. Must not
/// follow redirects, and must not store or send cookies: warpsign-core
/// handles both. Called from a background thread.
#[uniffi::export(with_foreign)]
pub trait HttpTransport: Send + Sync {
    fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportFailure>;
}

struct TransportBridge(Arc<dyn HttpTransport>);

impl http::Transport for TransportBridge {
    fn send(&self, request: &http::Request) -> Result<http::Response, http::TransportError> {
        let response = self
            .0
            .send(HttpRequest {
                method: request.method.as_str().to_string(),
                url: request.url.clone(),
                headers: request
                    .headers
                    .iter()
                    .map(|(name, value)| HttpHeader {
                        name: name.clone(),
                        value: value.clone(),
                    })
                    .collect(),
                body: request.body.clone(),
            })
            .map_err(|e| http::TransportError(e.to_string()))?;
        Ok(http::Response {
            status: response.status,
            headers: response
                .headers
                .into_iter()
                .map(|header| (header.name, header.value))
                .collect(),
            body: response.body,
        })
    }
}

// Records

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct TeamInfo {
    pub team_id: String,
    pub name: String,
    /// e.g. "INDIVIDUAL", "COMPANY"
    pub kind: String,
    pub roles: Vec<String>,
}

impl From<Team> for TeamInfo {
    fn from(team: Team) -> Self {
        Self {
            team_id: team.team_id,
            name: team.name,
            kind: team.kind,
            roles: team.roles,
        }
    }
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct TrustedPhoneInfo {
    pub id: i64,
    pub number: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct TwoFactorInfo {
    pub code_length: u32,
    pub trusted_devices: bool,
    pub phone_numbers: Vec<TrustedPhoneInfo>,
    /// Without trusted devices, Apple texts this number straight away
    pub sms_sent_to: Option<TrustedPhoneInfo>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum SignInResult {
    SignedIn { teams: Vec<TeamInfo> },
    NeedsCode { challenge: TwoFactorInfo },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CertificateType {
    Development,
    Distribution,
}

impl From<CertificateType> for CertificateKind {
    fn from(kind: CertificateType) -> Self {
        match kind {
            CertificateType::Development => CertificateKind::Development,
            CertificateType::Distribution => CertificateKind::Distribution,
        }
    }
}

impl From<CertificateKind> for CertificateType {
    fn from(kind: CertificateKind) -> Self {
        match kind {
            CertificateKind::Development => CertificateType::Development,
            CertificateKind::Distribution => CertificateType::Distribution,
        }
    }
}

/// A certificate and its private key, as DER. The key is secret: keep it
/// in the Keychain.
#[derive(Clone, PartialEq, uniffi::Record)]
pub struct IdentityData {
    pub certificate: Vec<u8>,
    /// PKCS#8
    pub private_key: Vec<u8>,
}

impl std::fmt::Debug for IdentityData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IdentityData").finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct IdentityInfo {
    pub common_name: Option<String>,
    pub kind: Option<CertificateType>,
    pub team_id: Option<String>,
    /// Uppercase hex, as the portal lists it
    pub serial_number: String,
    /// Seconds since 1970
    pub expires_at: i64,
}

/// A certificate as listed on the portal
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PortalCertificateInfo {
    pub id: String,
    pub serial_number: String,
    pub kind: Option<CertificateType>,
    pub name: String,
    pub display_name: Option<String>,
    pub expiration_date: Option<String>,
}

impl From<PortalCertificate> for PortalCertificateInfo {
    fn from(certificate: PortalCertificate) -> Self {
        Self {
            kind: certificate.kind().map(Into::into),
            id: certificate.id,
            serial_number: certificate.serial_number,
            name: certificate.name,
            display_name: certificate.display_name,
            expiration_date: certificate.expiration_date,
        }
    }
}

/// A device registered on the portal
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub udid: String,
    /// "IPHONE" or "IPAD"
    pub device_class: String,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum StatusBar {
    /// Leave as the app has it
    Unchanged,
    Hidden,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum InterfaceStyle {
    Automatic,
    Light,
    Dark,
}

/// Everything about how an app is signed, with Python warpsign's names
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SignSettings {
    /// New display name
    pub bundle_name: Option<String>,
    /// Give identifiers a new stable form (default on; off only for apps
    /// you own)
    pub encode_ids: bool,
    /// Replace old IDs inside binaries and plists (default on)
    pub patch_ids: bool,
    /// Keep the original CFBundleIdentifier. Refused with PPQ profiles.
    pub force_original_id: bool,
    /// Allow debuggers (get-task-allow)
    pub patch_debug: bool,
    pub patch_all_devices: bool,
    pub patch_file_sharing: bool,
    pub patch_promotion: bool,
    pub patch_fullscreen: bool,
    pub patch_orientation: bool,
    pub patch_game_mode: bool,
    pub status_bar: StatusBar,
    pub interface_style: InterfaceStyle,
    pub remove_url_schemes: bool,
}

/// Python warpsign's defaults
#[uniffi::export]
pub fn default_sign_settings() -> SignSettings {
    let options = SignOptions::default();
    SignSettings {
        bundle_name: None,
        encode_ids: options.encode_ids,
        patch_ids: options.patching.patch_ids,
        force_original_id: options.patching.force_original_id,
        patch_debug: options.patch_debug,
        patch_all_devices: false,
        patch_file_sharing: false,
        patch_promotion: false,
        patch_fullscreen: false,
        patch_orientation: false,
        patch_game_mode: false,
        status_bar: StatusBar::Unchanged,
        interface_style: InterfaceStyle::Automatic,
        remove_url_schemes: false,
    }
}

impl From<SignSettings> for SignOptions {
    fn from(settings: SignSettings) -> Self {
        SignOptions {
            patching: PatchingOptions {
                patch_ids: settings.patch_ids,
                force_original_id: settings.force_original_id,
                bundle_name: settings.bundle_name.filter(|name| !name.trim().is_empty()),
                patch_all_devices: settings.patch_all_devices,
                patch_file_sharing: settings.patch_file_sharing,
                patch_promotion: settings.patch_promotion,
                patch_fullscreen: settings.patch_fullscreen,
                patch_orientation: settings.patch_orientation,
                patch_game_mode: settings.patch_game_mode,
                patch_status_bar: match settings.status_bar {
                    StatusBar::Unchanged => StatusBarStyle::Default,
                    StatusBar::Hidden => StatusBarStyle::Hidden,
                    StatusBar::Light => StatusBarStyle::Light,
                    StatusBar::Dark => StatusBarStyle::Dark,
                },
                patch_user_interface_style: match settings.interface_style {
                    InterfaceStyle::Automatic => UiStyle::Automatic,
                    InterfaceStyle::Light => UiStyle::Light,
                    InterfaceStyle::Dark => UiStyle::Dark,
                },
                remove_url_schemes: settings.remove_url_schemes,
            },
            encode_ids: settings.encode_ids,
            patch_debug: settings.patch_debug,
        }
    }
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SignedProfileInfo {
    pub bundle_path: String,
    pub bundle_id: String,
    pub name: String,
    pub ppq_check: bool,
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SignResult {
    pub output_path: String,
    pub bundle_id: String,
    pub profiles: Vec<SignedProfileInfo>,
    pub warnings: Vec<String>,
}

// Errors

fn auth_error(error: AuthError) -> WarpsignError {
    let message = error.to_string();
    match error {
        AuthError::IncorrectCode(_) => WarpsignError::IncorrectCode { message },
        AuthError::SessionExpired => WarpsignError::SessionExpired { message },
        _ => WarpsignError::Auth { message },
    }
}

fn portal_error(error: PortalError) -> WarpsignError {
    match error.status {
        Some(401) => WarpsignError::SessionExpired {
            message: error.to_string(),
        },
        _ => WarpsignError::Portal {
            message: error.to_string(),
        },
    }
}

fn identity_error(error: IdentityError) -> WarpsignError {
    WarpsignError::Identity {
        message: error.to_string(),
    }
}

fn unix_time(time: std::time::SystemTime) -> i64 {
    match time.duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => i64::try_from(duration.as_secs()).unwrap_or(i64::MAX),
        Err(error) => -i64::try_from(error.duration().as_secs()).unwrap_or(i64::MAX),
    }
}

fn to_identity(data: &IdentityData) -> Result<Identity, WarpsignError> {
    Identity::new(&data.certificate, &data.private_key).map_err(identity_error)
}

fn identity_info_of(identity: &Identity) -> IdentityInfo {
    IdentityInfo {
        common_name: identity.common_name(),
        kind: identity.kind().map(Into::into),
        team_id: identity.team_id(),
        serial_number: identity.serial_number(),
        expires_at: unix_time(identity.expires()),
    }
}

// Identities

/// Describe a certificate and key, checking they belong together
#[uniffi::export]
pub fn identity_info(identity: IdentityData) -> Result<IdentityInfo, WarpsignError> {
    Ok(identity_info_of(&to_identity(&identity)?))
}

/// One password-protected `.p12` with all the identities (legacy 3DES, so
/// Keychain Access and other tools can read it)
#[uniffi::export]
pub fn export_identities(
    identities: Vec<IdentityData>,
    password: String,
) -> Result<Vec<u8>, WarpsignError> {
    let identities = identities
        .iter()
        .map(to_identity)
        .collect::<Result<Vec<_>, _>>()?;
    identity::export_p12(&identities, &password).map_err(identity_error)
}

/// Every identity in a `.p12` (legacy or AES)
#[uniffi::export]
pub fn import_identities(
    p12: Vec<u8>,
    password: String,
) -> Result<Vec<IdentityData>, WarpsignError> {
    Ok(identity::import_p12(&p12, &password)
        .map_err(identity_error)?
        .into_iter()
        .map(|identity| IdentityData {
            certificate: identity.certificate_der().to_vec(),
            private_key: identity.private_key_der().to_vec(),
        })
        .collect())
}

/// A strong passphrase for the backup `.p12`
#[uniffi::export]
pub fn generate_passphrase() -> String {
    identity::generate_passphrase()
}

/// The steps `AppleAccount.sign_ipa` runs, in order
#[uniffi::export]
pub fn sign_steps() -> Vec<StepInfo> {
    SIGN_STEPS
        .iter()
        .map(|&step| StepInfo {
            step: step.into(),
            title: step.title().to_string(),
        })
        .collect()
}

// The account

/// A signed-in (or signing-in) Apple ID. Every method does network I/O and
/// blocks: call from a background thread.
#[derive(uniffi::Object)]
pub struct AppleAccount {
    auth: Authenticator,
}

#[uniffi::export]
impl AppleAccount {
    /// Start signing in
    #[uniffi::constructor]
    pub fn new(transport: Arc<dyn HttpTransport>) -> Arc<Self> {
        Arc::new(Self {
            auth: Authenticator::new(Arc::new(TransportBridge(transport))),
        })
    }

    /// Resume from `session_json()`. Check it with `check_session`.
    #[uniffi::constructor]
    pub fn restore(
        transport: Arc<dyn HttpTransport>,
        session_json: String,
    ) -> Result<Arc<Self>, WarpsignError> {
        let session: SessionState =
            serde_json::from_str(&session_json).map_err(|e| WarpsignError::SessionExpired {
                message: format!("the saved session is unreadable ({e}): sign in again"),
            })?;
        Ok(Arc::new(Self {
            auth: Authenticator::with_session(Arc::new(TransportBridge(transport)), session),
        }))
    }

    pub fn sign_in(
        &self,
        account: String,
        password: String,
    ) -> Result<SignInResult, WarpsignError> {
        use warpsign_core::portal::SignInOutcome;
        Ok(
            match self.auth.sign_in(&account, &password).map_err(auth_error)? {
                SignInOutcome::SignedIn { teams } => SignInResult::SignedIn {
                    teams: teams.into_iter().map(Into::into).collect(),
                },
                SignInOutcome::NeedsCode(challenge) => {
                    let phone = |phone: warpsign_core::portal::TrustedPhone| TrustedPhoneInfo {
                        id: phone.id,
                        number: phone.number,
                    };
                    SignInResult::NeedsCode {
                        challenge: TwoFactorInfo {
                            code_length: challenge.code_length,
                            trusted_devices: challenge.trusted_devices,
                            phone_numbers: challenge.phone_numbers.into_iter().map(phone).collect(),
                            sms_sent_to: challenge.sms_sent_to.map(phone),
                        },
                    }
                }
            },
        )
    }

    pub fn request_sms(&self, phone_id: i64) -> Result<(), WarpsignError> {
        self.auth.request_sms(phone_id).map_err(auth_error)
    }

    pub fn verify_device_code(&self, code: String) -> Result<Vec<TeamInfo>, WarpsignError> {
        Ok(self
            .auth
            .verify_device_code(&code)
            .map_err(auth_error)?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    pub fn verify_sms_code(
        &self,
        phone_id: i64,
        code: String,
    ) -> Result<Vec<TeamInfo>, WarpsignError> {
        Ok(self
            .auth
            .verify_sms_code(phone_id, &code)
            .map_err(auth_error)?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// Check a restored session still works
    pub fn check_session(&self) -> Result<Vec<TeamInfo>, WarpsignError> {
        Ok(self
            .auth
            .restore()
            .map_err(auth_error)?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// The session to keep in the Keychain. Secret.
    pub fn session_json(&self) -> String {
        serde_json::to_string(&self.auth.session()).expect("sessions serialize")
    }

    pub fn certificates(
        &self,
        team_id: String,
    ) -> Result<Vec<PortalCertificateInfo>, WarpsignError> {
        Ok(self
            .portal(&team_id)
            .certificates()
            .map_err(portal_error)?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// The team's enabled iPhones and iPads: the devices signed apps install on
    pub fn devices(&self, team_id: String) -> Result<Vec<DeviceInfo>, WarpsignError> {
        Ok(self
            .portal(&team_id)
            .devices()
            .map_err(portal_error)?
            .into_iter()
            .map(|device| DeviceInfo {
                id: device.id,
                name: device.name,
                udid: device.udid,
                device_class: device.device_class,
                model: device.model,
            })
            .collect())
    }

    /// Make a new signing identity: a key generated here, and a certificate
    /// for it from the portal. Uses one of the team's certificate slots.
    pub fn create_identity(
        &self,
        team_id: String,
        kind: CertificateType,
    ) -> Result<IdentityData, WarpsignError> {
        let key = identity::generate_private_key().map_err(identity_error)?;
        let csr =
            identity::certificate_signing_request(&key, "WarpSign").map_err(identity_error)?;
        let (_, certificate) = self
            .portal(&team_id)
            .create_certificate(kind.into(), &csr)
            .map_err(portal_error)?;
        // Check what came back really is for this key
        let identity = Identity::new(&certificate, &key).map_err(identity_error)?;
        Ok(IdentityData {
            certificate: identity.certificate_der().to_vec(),
            private_key: key,
        })
    }

    /// Which of `identities` still have their certificate on the portal (not
    /// revoked or expired), as their serial numbers
    pub fn identities_on_portal(
        &self,
        team_id: String,
        identities: Vec<IdentityData>,
    ) -> Result<Vec<String>, WarpsignError> {
        let certificates = self.portal(&team_id).certificates().map_err(portal_error)?;
        let mut found = Vec::new();
        for data in &identities {
            let identity = to_identity(data)?;
            if certificates
                .iter()
                .any(|certificate| identity.has_serial(&certificate.serial_number))
            {
                found.push(identity.serial_number());
            }
        }
        Ok(found)
    }

    /// Re-sign an IPA for `team_id` with `identity`, registering whatever it
    /// needs on the portal, reporting each step to `observer`
    #[allow(clippy::too_many_arguments)]
    pub fn sign_ipa(
        &self,
        team_id: String,
        identity: IdentityData,
        ipa_path: String,
        output_path: String,
        work_dir: String,
        settings: SignSettings,
        observer: Arc<dyn StepObserver>,
    ) -> Result<SignResult, WarpsignError> {
        let identity = to_identity(&identity)?;
        let portal = self.portal(&team_id);
        let signed = pipeline::sign_ipa(
            &portal,
            &identity,
            Path::new(&ipa_path),
            Path::new(&output_path),
            Path::new(&work_dir),
            &settings.into(),
            &ObserverBridge(observer),
        )
        .map_err(|error| match error {
            pipeline::PipelineError::Portal(error) => portal_error(error),
            pipeline::PipelineError::Ipa(_) => WarpsignError::Ipa {
                message: error.to_string(),
            },
            _ => WarpsignError::Signing {
                message: error.to_string(),
            },
        })?;
        Ok(SignResult {
            output_path: signed.output.display().to_string(),
            bundle_id: signed.bundle_id,
            profiles: signed
                .profiles
                .into_iter()
                .map(|profile| SignedProfileInfo {
                    bundle_path: profile.bundle_path,
                    bundle_id: profile.bundle_id,
                    name: profile.name,
                    ppq_check: profile.ppq_check,
                    expires_at: profile.expiration_date.map(unix_time),
                })
                .collect(),
            warnings: signed.warnings,
        })
    }
}

impl AppleAccount {
    fn portal(&self, team_id: &str) -> Portal {
        Portal::new(self.auth.http(), team_id)
    }
}
