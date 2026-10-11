//! Signing in to the Developer Portal with an Apple ID: SRP sign-in at
//! idmsa.apple.com, two-factor authentication, then the portal session.
//!
//! Python warpsign's flow, plus (docs/design/portal.md):
//! - the `X-APPLE-HC` hashcash header, which Apple has required since 2023
//! - a 403 is final and is never retried (repeated refusals lock accounts)
//! - SMS codes as well as trusted-device codes
//! - fetching the anti-CSRF tokens after two-factor authentication too
//! - the password derivation Apple asks for (`s2k` or `s2k_fo`)
//! - rejecting Apple IDs without a developer team

use std::sync::{Arc, OnceLock};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::{Value, json};

use super::api::{Team, fetch_teams};
use super::http::{HttpClient, Method, Request, Response, SessionState, Transport};
use super::{PortalError, apple_error_message};
use crate::auth::hashcash;
use crate::auth::srp::{Protocol, SrpClient, SrpError, derive_password};

const AUTH: &str = "https://idmsa.apple.com/appleauth/auth";
const WIDGET_KEY_URL: &str =
    "https://appstoreconnect.apple.com/olympus/v1/app/config?hostname=itunesconnect.apple.com";
const ACCOUNT_URL: &str = "https://developer.apple.com/account/resources";

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum AuthError {
    #[error("couldn't reach Apple: {0}")]
    Transport(String),
    #[error("{0}")]
    InvalidCredentials(String),
    /// Apple refused the sign-in (HTTP 403). Retrying can lock the Apple ID,
    /// so don't.
    #[error("Apple refused the sign-in: {0}")]
    Refused(String),
    /// Something needs doing on the web first, e.g. accepting new terms
    #[error(
        "Apple needs you to review something for this Apple ID first: sign in at https://account.apple.com, then try again ({0})"
    )]
    ActionRequired(String),
    #[error("{0}")]
    IncorrectCode(String),
    #[error(
        "this Apple ID uses the older two-step verification, which isn't supported: turn on two-factor authentication at https://account.apple.com"
    )]
    TwoStepVerification,
    #[error(
        "this Apple ID isn't in a paid Apple Developer Program team (free Apple accounts aren't supported)"
    )]
    NoTeams,
    #[error("the saved session has expired: sign in again")]
    SessionExpired,
    #[error(transparent)]
    Srp(#[from] SrpError),
    #[error(transparent)]
    Portal(#[from] PortalError),
    #[error("unexpected response from Apple: {0}")]
    Unexpected(String),
}

/// A phone number codes can be sent to
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedPhone {
    pub id: i64,
    /// Obfuscated, e.g. "+44 ••••• ••••12"
    pub number: String,
}

/// Two-factor authentication is needed to finish signing in
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwoFactorChallenge {
    pub code_length: u32,
    /// Whether a code was pushed to the account's trusted devices
    pub trusted_devices: bool,
    pub phone_numbers: Vec<TrustedPhone>,
    /// Without trusted devices, Apple texts this number straight away
    pub sms_sent_to: Option<TrustedPhone>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SignInOutcome {
    SignedIn { teams: Vec<Team> },
    NeedsCode(TwoFactorChallenge),
}

/// Signs in, and keeps the session's cookies and tokens
pub struct Authenticator {
    http: Arc<HttpClient>,
    widget_key: OnceLock<String>,
    srp_secret: Option<Vec<u8>>,
}

impl Authenticator {
    pub fn new(transport: Arc<dyn Transport>) -> Self {
        Self::with_session(transport, SessionState::default())
    }

    /// Resume a saved session (check it with [`Self::restore`])
    pub fn with_session(transport: Arc<dyn Transport>, session: SessionState) -> Self {
        Self {
            http: Arc::new(HttpClient::with_state(transport, session)),
            widget_key: OnceLock::new(),
            srp_secret: None,
        }
    }

    /// Use a fixed SRP secret, so tests can check the exchange against
    /// known values. Never use outside tests.
    #[doc(hidden)]
    pub fn with_srp_secret(mut self, secret: &[u8]) -> Self {
        self.srp_secret = Some(secret.to_vec());
        self
    }

    /// For the portal API once signed in
    pub fn http(&self) -> Arc<HttpClient> {
        self.http.clone()
    }

    /// The session to save. Secret.
    pub fn session(&self) -> SessionState {
        self.http.state()
    }

    /// Check a saved session still works, without a password
    pub fn restore(&self) -> Result<Vec<Team>, AuthError> {
        self.finish()
    }

    pub fn sign_in(&self, account: &str, password: &str) -> Result<SignInOutcome, AuthError> {
        let widget_key = self.widget_key()?;
        let hashcash = self.hashcash(&widget_key)?;

        let srp = match &self.srp_secret {
            Some(secret) => SrpClient::with_secret(account, secret),
            None => SrpClient::new(account),
        };
        let init = self.idmsa(Request::post(format!("{AUTH}/signin/init")).json(&json!({
            "a": BASE64.encode(srp.public_key()),
            "accountName": account,
            "protocols": ["s2k", "s2k_fo"],
        })))?;
        if !init.is_success() {
            return Err(sign_in_error(&init));
        }
        let body = init
            .json()
            .ok_or_else(|| AuthError::Unexpected("sign-in challenge isn't JSON".into()))?;
        let field = |key: &str| {
            body.get(key)
                .ok_or_else(|| AuthError::Unexpected(format!("sign-in challenge has no {key}")))
        };
        let decode = |key: &str| -> Result<Vec<u8>, AuthError> {
            let text = field(key)?.as_str().unwrap_or_default();
            BASE64
                .decode(text)
                .map_err(|_| AuthError::Unexpected(format!("sign-in challenge {key} isn't base64")))
        };
        let salt = decode("salt")?;
        let server_key = decode("b")?;
        let iterations = field("iteration")?
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| AuthError::Unexpected("sign-in challenge iteration".into()))?;
        let protocol = match body.get("protocol").and_then(Value::as_str) {
            // Python warpsign always used s2k
            None => Protocol::S2k,
            Some(name) => Protocol::from_name(name).ok_or_else(|| {
                AuthError::Unexpected(format!("unknown password protocol {name}"))
            })?,
        };
        let derived = derive_password(password, &salt, iterations, protocol);
        let proof = srp.process_challenge(&salt, &server_key, &derived)?;

        let mut complete = Request::post(format!("{AUTH}/signin/complete"))
            .query(&[("isRememberMeEnabled", "true")])
            .json(&json!({
                "accountName": account,
                "c": field("c")?,
                "m1": BASE64.encode(&proof.client_proof),
                "m2": BASE64.encode(&proof.expected_server_proof),
                "rememberMe": true,
            }));
        if let Some(hashcash) = hashcash {
            complete = complete.header("X-Apple-HC", hashcash);
        }
        let response = self.idmsa(complete)?;
        match response.status {
            200 | 204 => Ok(SignInOutcome::SignedIn {
                teams: self.finish()?,
            }),
            409 => Ok(SignInOutcome::NeedsCode(self.two_factor_options()?)),
            _ => Err(sign_in_error(&response)),
        }
    }

    /// Ask for a code by text message
    pub fn request_sms(&self, phone_id: i64) -> Result<(), AuthError> {
        let response = self.idmsa(
            Request::new(Method::Put, format!("{AUTH}/verify/phone"))
                .json(&json!({"phoneNumber": {"id": phone_id}, "mode": "sms"})),
        )?;
        if response.is_success() {
            Ok(())
        } else {
            Err(code_error(&response))
        }
    }

    /// Finish signing in with a code shown on a trusted device
    pub fn verify_device_code(&self, code: &str) -> Result<Vec<Team>, AuthError> {
        let response = self.idmsa(
            Request::post(format!("{AUTH}/verify/trusteddevice/securitycode"))
                .json(&json!({"securityCode": {"code": code.trim()}})),
        )?;
        self.after_code(response)
    }

    /// Finish signing in with a code sent by text message
    pub fn verify_sms_code(&self, phone_id: i64, code: &str) -> Result<Vec<Team>, AuthError> {
        let response = self.idmsa(
            Request::post(format!("{AUTH}/verify/phone/securitycode")).json(&json!({
                "securityCode": {"code": code.trim()},
                "phoneNumber": {"id": phone_id},
                "mode": "sms",
            })),
        )?;
        self.after_code(response)
    }

    fn after_code(&self, response: Response) -> Result<Vec<Team>, AuthError> {
        if !response.is_success() {
            return Err(code_error(&response));
        }
        // Trust this session, so the next sign-in doesn't need a code
        let trust = self.idmsa(Request::get(format!("{AUTH}/2sv/trust")))?;
        if !trust.is_success() {
            return Err(sign_in_error(&trust));
        }
        self.finish()
    }

    /// Open the portal: pick up its anti-CSRF tokens and the teams
    fn finish(&self) -> Result<Vec<Team>, AuthError> {
        let page = self
            .http
            .send(Request::get(ACCOUNT_URL))
            .map_err(|e| AuthError::Transport(e.0))?;
        if page.is_success() {
            let state = self.http.state();
            let csrf = state
                .cookie("csrf")
                .map(String::from)
                .or_else(|| page.header("csrf").map(String::from))
                .or_else(|| find_js_string(&page.text(), "csrf"));
            let csrf_ts = state
                .cookie("csrf_ts")
                .map(String::from)
                .or_else(|| page.header("csrf_ts").map(String::from))
                .or_else(|| find_js_string(&page.text(), "csrf_ts"));
            self.http.update_state(|state| {
                if csrf.is_some() && csrf_ts.is_some() {
                    state.csrf = csrf;
                    state.csrf_ts = csrf_ts;
                }
            });
        }

        let teams = fetch_teams(&self.http).map_err(|error| match error.status {
            Some(401 | 403) => AuthError::SessionExpired,
            _ => AuthError::Portal(error),
        })?;
        if teams.is_empty() {
            return Err(AuthError::NoTeams);
        }
        Ok(teams)
    }

    fn two_factor_options(&self) -> Result<TwoFactorChallenge, AuthError> {
        // Asking for the options is also what pushes a code to devices
        let response = self.idmsa(Request::get(AUTH))?;
        let Some(body) = response.json().filter(|_| response.is_success()) else {
            // Not fatal: a trusted-device code may still arrive
            return Ok(TwoFactorChallenge {
                code_length: 6,
                trusted_devices: true,
                phone_numbers: Vec::new(),
                sms_sent_to: None,
            });
        };
        if body.get("authenticationType").and_then(Value::as_str) == Some("hsa") {
            return Err(AuthError::TwoStepVerification);
        }
        let phone = |value: &Value| {
            Some(TrustedPhone {
                id: value.get("id")?.as_i64()?,
                number: value
                    .get("numberWithDialCode")
                    .or_else(|| value.get("obfuscatedNumber"))
                    .and_then(Value::as_str)?
                    .to_string(),
            })
        };
        let trusted_devices = !body
            .get("noTrustedDevices")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let phone_numbers: Vec<TrustedPhone> = body
            .get("trustedPhoneNumbers")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(phone)
            .collect();
        let sms_sent_to = if trusted_devices {
            None
        } else {
            body.get("trustedPhoneNumber")
                .and_then(phone)
                .or_else(|| phone_numbers.first().cloned())
        };
        Ok(TwoFactorChallenge {
            code_length: body
                .pointer("/securityCode/length")
                .and_then(Value::as_u64)
                .map_or(6, |n| n as u32),
            trusted_devices,
            phone_numbers,
            sms_sent_to,
        })
    }

    fn widget_key(&self) -> Result<String, AuthError> {
        if let Some(key) = self.widget_key.get() {
            return Ok(key.clone());
        }
        let response = self
            .http
            .send(Request::get(WIDGET_KEY_URL))
            .map_err(|e| AuthError::Transport(e.0))?;
        let key = response
            .json()
            .and_then(|body| body.get("authServiceKey")?.as_str().map(String::from))
            .filter(|key| !key.is_empty())
            .ok_or_else(|| {
                AuthError::Unexpected(format!(
                    "no sign-in service key from App Store Connect (HTTP {})",
                    response.status
                ))
            })?;
        Ok(self.widget_key.get_or_init(|| key).clone())
    }

    /// The `X-APPLE-HC` header for this sign-in, if Apple asks for one
    fn hashcash(&self, widget_key: &str) -> Result<Option<String>, AuthError> {
        let response =
            self.idmsa(Request::get(format!("{AUTH}/signin")).query(&[("widgetKey", widget_key)]))?;
        let bits = response
            .header("X-Apple-HC-Bits")
            .and_then(|bits| bits.trim().parse::<u32>().ok());
        let challenge = response.header("X-Apple-HC-Challenge");
        Ok(match (bits, challenge) {
            // More than 32 bits would take forever: not a real challenge
            (Some(bits), Some(challenge)) if bits <= 32 => Some(hashcash::make(bits, challenge)),
            _ => None,
        })
    }

    /// A request to idmsa.apple.com with the headers its web client sends.
    /// Keeps the session ID and `scnt` it hands out.
    fn idmsa(&self, request: Request) -> Result<Response, AuthError> {
        let widget_key = self.widget_key.get().cloned().unwrap_or_default();
        let state = self.http.state();
        let mut request = request.headers(&[
            ("Accept", "application/json, text/javascript"),
            ("Content-Type", "application/json"),
            ("X-Requested-With", "XMLHttpRequest"),
        ]);
        if !widget_key.is_empty() {
            request = request.header("X-Apple-Widget-Key", widget_key);
        }
        if let (Some(session_id), Some(scnt)) = (state.apple_session_id, state.scnt) {
            request = request
                .header("X-Apple-ID-Session-Id", session_id)
                .header("scnt", scnt);
        }
        let response = self
            .http
            .send(request)
            .map_err(|e| AuthError::Transport(e.0))?;
        let session_id = response.header("X-Apple-ID-Session-Id").map(String::from);
        let scnt = response.header("scnt").map(String::from);
        self.http.update_state(|state| {
            if session_id.is_some() {
                state.apple_session_id = session_id;
            }
            if scnt.is_some() {
                state.scnt = scnt;
            }
        });
        Ok(response)
    }
}

fn sign_in_error(response: &Response) -> AuthError {
    let message = apple_error_message(&response.body);
    match response.status {
        401 => AuthError::InvalidCredentials(
            message.unwrap_or_else(|| "incorrect Apple ID or password".into()),
        ),
        403 => AuthError::Refused(message.unwrap_or_else(|| "access denied (HTTP 403)".into())),
        412 => AuthError::ActionRequired(message.unwrap_or_else(|| "HTTP 412".into())),
        status => AuthError::Unexpected(message.unwrap_or_else(|| format!("HTTP {status}"))),
    }
}

fn code_error(response: &Response) -> AuthError {
    let message = apple_error_message(&response.body);
    match response.status {
        400 | 401 => AuthError::IncorrectCode(
            message.unwrap_or_else(|| "incorrect verification code".into()),
        ),
        _ => sign_in_error(response),
    }
}

/// The quoted value after `key` in a page's script: `csrf: "..."`,
/// `'csrf' : '...'`. The same as Python warpsign's regex
/// `key["']\s*:\s*["']([^"']+)["']`.
fn find_js_string(text: &str, key: &str) -> Option<String> {
    for (index, _) in text.match_indices(key) {
        let rest = &text[index + key.len()..];
        let Some(rest) = rest.strip_prefix(['"', '\'']) else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix(':') else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix(['"', '\'']) else {
            continue;
        };
        let end = rest.find(['"', '\''])?;
        if end > 0 {
            return Some(rest[..end].to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_csrf_in_page_scripts() {
        let page =
            r#"<script>window.config = {"csrf_ts": "1700000000", "csrf" : 'abc123'};</script>"#;
        assert_eq!(find_js_string(page, "csrf").as_deref(), Some("abc123"));
        assert_eq!(
            find_js_string(page, "csrf_ts").as_deref(),
            Some("1700000000")
        );
        assert_eq!(find_js_string("csrf = 1", "csrf"), None);
    }
}
