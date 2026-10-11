//! HTTP for the portal client: one request in, the response as-is out. No
//! redirects followed and no cookies kept: warpsign-core does both.

use std::io::Read;
use std::time::Duration;

use warpsign_core::portal::http::{Request, Response, Transport, TransportError};

pub struct UreqTransport {
    agent: ureq::Agent,
}

impl UreqTransport {
    pub fn new() -> Self {
        Self {
            agent: ureq::AgentBuilder::new()
                .redirects(0)
                .timeout(Duration::from_secs(120))
                .user_agent(concat!("warpsign/", env!("CARGO_PKG_VERSION")))
                .build(),
        }
    }
}

impl Transport for UreqTransport {
    fn send(&self, request: &Request) -> Result<Response, TransportError> {
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
