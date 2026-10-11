//! The Apple Developer Portal: signing in and managing certificates, App
//! IDs, app groups, iCloud containers, devices and provisioning profiles.
//!
//! A port of Python warpsign's `apple_account_login.py` and
//! `developer_portal_api.py`, using the same web endpoints and request
//! shapes, with the fixes agreed in docs/design/portal.md. Nothing here has
//! been tried against Apple yet: the tests use recorded-style responses.

pub mod api;
pub mod auth;
pub mod http;

pub use api::Portal;
pub use auth::{AuthError, Authenticator, SignInOutcome, TrustedPhone, TwoFactorChallenge};
pub use http::{HttpClient, Request, Response, SessionState, Transport, TransportError};

use serde_json::Value;

/// A failed portal request, with Apple's explanation when it gave one
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{operation}: {message}")]
pub struct PortalError {
    /// What was being done, e.g. "registering App ID com.example.app"
    pub operation: String,
    /// HTTP status, if a response came back
    pub status: Option<u16>,
    pub message: String,
}

impl PortalError {
    pub(crate) fn new(operation: &str, status: Option<u16>, message: impl Into<String>) -> Self {
        Self {
            operation: operation.to_string(),
            status,
            message: message.into(),
        }
    }

    pub(crate) fn transport(operation: &str, error: TransportError) -> Self {
        Self::new(operation, None, error.0)
    }

    /// From an unsuccessful response
    pub(crate) fn from_response(operation: &str, response: &Response) -> Self {
        let message = apple_error_message(&response.body).unwrap_or_else(|| {
            let text = response.text();
            let text = text.trim();
            if text.is_empty() || text.starts_with('<') {
                format!("HTTP {}", response.status)
            } else {
                format!("HTTP {}: {}", response.status, truncate(text, 300))
            }
        });
        Self::new(operation, Some(response.status), message)
    }
}

fn truncate(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((index, _)) => format!("{}…", &text[..index]),
        None => text.to_string(),
    }
}

/// Apple's human-readable error from a response body, in any of the
/// formats its services use
pub fn apple_error_message(body: &[u8]) -> Option<String> {
    let json: Value = serde_json::from_slice(body).ok()?;
    let mut messages = Vec::new();

    // idmsa: {"serviceErrors": [{"code": "-20209", "message": "..."}]}
    for key in ["serviceErrors", "validationErrors"] {
        for error in json
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(message) = error.get("message").and_then(Value::as_str) {
                match error.get("code").and_then(Value::as_str) {
                    Some(code) if !code.is_empty() => messages.push(format!("{message} ({code})")),
                    _ => messages.push(message.to_string()),
                }
            }
        }
    }
    // JSON:API: {"errors": [{"title": "...", "detail": "..."}]}
    for error in json
        .get("errors")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let text = |key| {
            error
                .get(key)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
        };
        if let Some(message) = text("detail").or_else(|| text("title")) {
            messages.push(message.to_string());
        }
    }
    // Older portal actions: {"resultCode": 35, "userString": "...", "resultString": "..."}
    if messages.is_empty()
        && json
            .get("resultCode")
            .and_then(Value::as_i64)
            .is_some_and(|c| c != 0)
    {
        let text = |key| {
            json.get(key)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
        };
        if let Some(message) = text("userString").or_else(|| text("resultString")) {
            messages.push(message.to_string());
        }
    }

    messages.dedup();
    (!messages.is_empty()).then(|| messages.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_apple_error_formats() {
        assert_eq!(
            apple_error_message(
                br#"{"serviceErrors":[{"code":"-20209","message":"This Apple ID has been locked for security reasons.","suppressDismissal":false}]}"#
            )
            .as_deref(),
            Some("This Apple ID has been locked for security reasons. (-20209)")
        );
        assert_eq!(
            apple_error_message(
                br#"{"errors":[{"status":"409","code":"ENTITY_ERROR","title":"There is a problem with the request entity","detail":"An App ID with Identifier 'com.example' is not available."}]}"#
            )
            .as_deref(),
            Some("An App ID with Identifier 'com.example' is not available.")
        );
        assert_eq!(
            apple_error_message(
                br#"{"resultCode":35,"resultString":"Multiple profiles found","userString":"Multiple profiles found with the name 'x'."}"#
            )
            .as_deref(),
            Some("Multiple profiles found with the name 'x'.")
        );
        assert_eq!(apple_error_message(br#"{"resultCode":0}"#), None);
        assert_eq!(apple_error_message(b"<html>"), None);
    }

    #[test]
    fn falls_back_to_status_and_body() {
        let response = |status, body: &str| Response {
            status,
            headers: Vec::new(),
            body: body.as_bytes().to_vec(),
        };
        assert_eq!(
            PortalError::from_response("x", &response(502, "<html>bad gateway</html>")).message,
            "HTTP 502"
        );
        assert_eq!(
            PortalError::from_response("x", &response(500, "oops")).to_string(),
            "x: HTTP 500: oops"
        );
    }
}
