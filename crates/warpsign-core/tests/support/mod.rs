//! A scripted stand-in for Apple's servers

// Each test file uses a different part of this
#![allow(dead_code)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use warpsign_core::portal::http::{Request, Response, Transport, TransportError};

type Handler = Box<dyn Fn(&Request) -> Response + Send>;

/// Answers each request with the next scripted response, checking it was
/// the request expected, and keeps every request for inspection
#[derive(Default)]
pub struct MockApple {
    script: Mutex<VecDeque<(String, Handler)>>,
    sent: Mutex<Vec<Request>>,
}

impl MockApple {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Expect a request whose URL starts with `url`, and answer it
    pub fn expect(&self, url: &str, handler: impl Fn(&Request) -> Response + Send + 'static) {
        self.script
            .lock()
            .unwrap()
            .push_back((url.to_string(), Box::new(handler)));
    }

    pub fn respond(&self, url: &str, response: Response) {
        self.expect(url, move |_| response.clone());
    }

    pub fn sent(&self) -> Vec<Request> {
        self.sent.lock().unwrap().clone()
    }

    pub fn assert_done(&self) {
        let script = self.script.lock().unwrap();
        let left: Vec<&String> = script.iter().map(|(url, _)| url).collect();
        assert!(left.is_empty(), "requests never made: {left:?}");
    }
}

impl Transport for MockApple {
    fn send(&self, request: &Request) -> Result<Response, TransportError> {
        self.sent.lock().unwrap().push(request.clone());
        let (url, handler) = self.script.lock().unwrap().pop_front().unwrap_or_else(|| {
            panic!(
                "unexpected request: {} {}",
                request.method.as_str(),
                request.url
            )
        });
        assert!(
            request.url.starts_with(&url),
            "expected a request to {url}, got {} {}",
            request.method.as_str(),
            request.url
        );
        Ok(handler(request))
    }
}

pub fn response(status: u16, headers: &[(&str, &str)], body: impl Into<Vec<u8>>) -> Response {
    Response {
        status,
        headers: headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        body: body.into(),
    }
}

pub fn json_response(status: u16, body: serde_json::Value) -> Response {
    response(status, &[], serde_json::to_vec(&body).unwrap())
}
