//! HTTP for the portal client: one request in, the response as-is out. No
//! redirects followed and no cookies kept: warpsign-core does both.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use warpsign_core::portal::http::{Request, Response, Transport, TransportError};

pub struct UreqTransport {
    agent: ureq::Agent,
    log: Option<Mutex<std::fs::File>>,
}

impl UreqTransport {
    pub fn new() -> Self {
        Self {
            log: None,
            agent: ureq::AgentBuilder::new()
                .redirects(0)
                .timeout(Duration::from_secs(120))
                .user_agent(concat!("warpsign/", env!("CARGO_PKG_VERSION")))
                .build(),
        }
    }

    /// Also append every exchange to `path`, with secrets redacted
    pub fn with_log(mut self, path: &Path) -> std::io::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        self.log = Some(Mutex::new(file));
        Ok(self)
    }

    fn record(&self, request: &Request, outcome: &Result<Response, TransportError>) {
        let Some(log) = &self.log else { return };
        let mut text = format!("> {} {}\n", request.method.as_str(), request.url);
        for (name, value) in &request.headers {
            text += &format!(">   {name}: {}\n", redact_header(name, value));
        }
        if !request.body.is_empty() {
            text += &format!(">   {}\n", redact_body(&request.body));
        }
        match outcome {
            Ok(response) => {
                text += &format!("< {}\n", response.status);
                for (name, value) in &response.headers {
                    text += &format!("<   {name}: {}\n", redact_header(name, value));
                }
                if !response.body.is_empty() {
                    text += &format!("<   {}\n", redact_body(&response.body));
                }
            }
            Err(error) => text += &format!("< failed: {error}\n"),
        }
        text += "\n";
        if let Ok(mut file) = log.lock() {
            let _ = file.write_all(text.as_bytes());
        }
    }
}

/// Headers that carry the session
const SECRET_HEADERS: [&str; 7] = [
    "cookie",
    "set-cookie",
    "scnt",
    "x-apple-id-session-id",
    "x-apple-session-token",
    "csrf",
    "csrf_ts",
];

fn redact_header(name: &str, value: &str) -> String {
    let lower = name.to_ascii_lowercase();
    if !SECRET_HEADERS.contains(&lower.as_str()) {
        return value.to_string();
    }
    if lower.contains("cookie") {
        // Keep cookie names, which help debugging, but not values
        return value
            .split([';', ','])
            .filter_map(|part| part.split_once('=').map(|(name, _)| name.trim()))
            .filter(|name| !name.is_empty())
            .map(|name| format!("{name}=…"))
            .collect::<Vec<_>>()
            .join("; ");
    }
    "…".into()
}

/// JSON with SRP and code values blanked; anything else summarised
fn redact_body(body: &[u8]) -> String {
    const SECRET_KEYS: [&str; 8] = ["a", "b", "c", "m1", "m2", "salt", "code", "csrContent"];
    fn scrub(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, value) in map.iter_mut() {
                    if SECRET_KEYS.contains(&key.as_str()) && !value.is_object() {
                        *value = serde_json::Value::String("…".into());
                    } else {
                        scrub(value);
                    }
                }
            }
            serde_json::Value::Array(items) => items.iter_mut().for_each(scrub),
            _ => {}
        }
    }
    match serde_json::from_slice::<serde_json::Value>(body) {
        Ok(mut json) => {
            scrub(&mut json);
            let text = json.to_string();
            match text.char_indices().nth(4000) {
                Some((end, _)) => format!("{}… ({} bytes)", &text[..end], text.len()),
                None => text,
            }
        }
        Err(_) if body.starts_with(b"<") => format!("(HTML, {} bytes)", body.len()),
        Err(_) => format!("({} bytes)", body.len()),
    }
}

impl Transport for UreqTransport {
    fn send(&self, request: &Request) -> Result<Response, TransportError> {
        let outcome = self.exchange(request);
        self.record(request, &outcome);
        outcome
    }
}

impl UreqTransport {
    fn exchange(&self, request: &Request) -> Result<Response, TransportError> {
        let mut outgoing = self.agent.request(request.method.as_str(), &request.url);
        for (name, value) in &request.headers {
            outgoing = outgoing.set(name, value);
        }
        let result = if request.body.is_empty() {
            outgoing.call()
        } else {
            outgoing.send_bytes(&request.body)
        };
        let response = match result {
            Ok(response) => response,
            // Error statuses are answers too
            Err(ureq::Error::Status(_, response)) => response,
            Err(ureq::Error::Transport(error)) => return Err(TransportError(error.to_string())),
        };

        let status = response.status();
        let mut headers = Vec::new();
        for name in response.headers_names() {
            for value in response.all(&name) {
                headers.push((name.clone(), value.to_string()));
            }
        }
        let mut body = Vec::new();
        response
            .into_reader()
            .take(64 * 1024 * 1024)
            .read_to_end(&mut body)
            .map_err(|e| TransportError(e.to_string()))?;
        Ok(Response {
            status,
            headers,
            body,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_secrets() {
        assert_eq!(
            redact_header("Set-Cookie", "myacinfo=SECRET; Domain=.apple.com; Path=/"),
            "myacinfo=…; Domain=…; Path=…"
        );
        assert_eq!(redact_header("scnt", "SECRET"), "…");
        assert_eq!(
            redact_header("Content-Type", "application/json"),
            "application/json"
        );
        let body = redact_body(
            br#"{"a":"SECRET","accountName":"me@example.com","securityCode":{"code":"123456"}}"#,
        );
        assert!(
            !body.contains("SECRET") && !body.contains("123456"),
            "{body}"
        );
        assert!(body.contains("me@example.com"));
        assert_eq!(redact_body(b"<html>"), "(HTML, 6 bytes)");
    }
}
