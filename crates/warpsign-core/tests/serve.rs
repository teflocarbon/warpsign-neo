//! The loopback server iOS's installer fetches apps from.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use warpsign_core::progress::{ProgressObserver, Step};
use warpsign_core::serve::{InstallServer, SERVER_PORT, ServeError, manifest};

#[derive(Default, Clone)]
struct Events(Arc<Mutex<Vec<String>>>);

impl ProgressObserver for Events {
    fn step_started(&self, step: Step) {
        self.0.lock().unwrap().push(format!("start {step:?}"));
    }
    fn step_log(&self, _step: Step, _message: &str) {}
    fn step_finished(&self, step: Step, _elapsed: Duration, error: Option<&str>) {
        self.0
            .lock()
            .unwrap()
            .push(format!("finish {step:?} {error:?}"));
    }
    fn step_progress(&self, step: Step, fraction: f64) {
        let mut events = self.0.lock().unwrap();
        let event = format!("progress {step:?}");
        if events.last() != Some(&event) {
            events.push(event);
        }
        let _ = fraction;
    }
}

/// A fake IPA big enough to need several chunks
fn ipa() -> (tempfile::TempDir, std::path::PathBuf, Vec<u8>) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("App.ipa");
    let data: Vec<u8> = (0..700_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(&path, &data).unwrap();
    (dir, path, data)
}

/// Send a raw HTTP request; return (status line, headers, body)
fn request(port: u16, request: &str) -> (String, String, Vec<u8>) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    let split = response.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8(response[..split].to_vec()).unwrap();
    let body = response[split + 4..].to_vec();
    let (status, headers) = head.split_once("\r\n").unwrap_or((&head, ""));
    (status.to_string(), headers.to_string(), body)
}

#[test]
fn serves_the_app_to_ios() {
    let (_dir, path, data) = ipa();
    let events = Events::default();
    let server = InstallServer::start_on(0, &path, None, 7, Box::new(events.clone())).unwrap();
    let port = server.port();
    assert_eq!(
        events.0.lock().unwrap().as_slice(),
        ["start WaitForInstall"]
    );

    // The icon for iOS's progress display, and HEAD, don't count as installing
    let (status, headers, icon) = request(port, "GET /app7.png HTTP/1.1\r\nHost: x\r\n\r\n");
    assert_eq!(status, "HTTP/1.1 200 OK");
    assert!(headers.contains("Content-Type: image/png"));
    assert!(icon.starts_with(b"\x89PNG"));
    let (status, headers, body) = request(port, "HEAD /app7.ipa HTTP/1.1\r\n\r\n");
    assert_eq!(status, "HTTP/1.1 200 OK");
    assert!(headers.contains(&format!("Content-Length: {}", data.len())));
    assert!(body.is_empty());
    assert_eq!(events.0.lock().unwrap().len(), 1);

    let (status, headers, body) = request(port, "GET /app7.ipa HTTP/1.1\r\n\r\n");
    assert_eq!(status, "HTTP/1.1 200 OK");
    assert!(headers.contains("Accept-Ranges: bytes"));
    assert_eq!(body, data);
    assert_eq!(
        events.0.lock().unwrap().as_slice(),
        [
            "start WaitForInstall",
            "finish WaitForInstall None",
            "start SendToIos",
            "progress SendToIos",
            "finish SendToIos None",
        ]
    );
}

#[test]
fn resumes_with_ranges() {
    let (_dir, path, data) = ipa();
    let server = InstallServer::start_on(0, &path, None, 3, Box::new(Events::default())).unwrap();
    let (status, headers, body) = request(
        server.port(),
        "GET /app3.ipa HTTP/1.1\r\nRange: bytes=1000-1999\r\n\r\n",
    );
    assert_eq!(status, "HTTP/1.1 206 Partial Content");
    assert!(headers.contains(&format!("Content-Range: bytes 1000-1999/{}", data.len())));
    assert_eq!(body, &data[1000..2000]);

    let (_, _, body) = request(
        server.port(),
        "GET /app3.ipa HTTP/1.1\r\nRange: bytes=699990-\r\n\r\n",
    );
    assert_eq!(body, &data[699_990..]);

    let (status, _, _) = request(
        server.port(),
        "GET /app3.ipa HTTP/1.1\r\nRange: bytes=9999999-\r\n\r\n",
    );
    assert_eq!(status, "HTTP/1.1 416 Range Not Satisfiable");
}

#[test]
fn serves_only_its_own_slot() {
    let (_dir, path, _) = ipa();
    let server = InstallServer::start_on(0, &path, None, 3, Box::new(Events::default())).unwrap();
    for path in ["/app4.ipa", "/", "/../App.ipa", "/app3.plist"] {
        let (status, _, _) = request(server.port(), &format!("GET {path} HTTP/1.1\r\n\r\n"));
        assert_eq!(status, "HTTP/1.1 404 Not Found", "{path}");
    }
    let (status, _, _) = request(server.port(), "POST /app3.ipa HTTP/1.1\r\n\r\n");
    assert_eq!(status, "HTTP/1.1 405 Method Not Allowed");
}

#[test]
fn stops_and_frees_the_port() {
    let (_dir, path, _) = ipa();
    let mut first =
        InstallServer::start_on(0, &path, None, 1, Box::new(Events::default())).unwrap();
    let port = first.port();
    // Taken while running: explained, not a panic
    let error = InstallServer::start_on(port, &path, None, 1, Box::new(Events::default()))
        .err()
        .unwrap();
    assert!(matches!(error, ServeError::Port(_)), "{error}");
    assert!(error.to_string().contains("Another installer"));

    first.stop();
    let _again =
        InstallServer::start_on(port, &path, None, 1, Box::new(Events::default())).unwrap();
}

#[test]
fn manifests_point_at_the_server() {
    let plist = plist::Value::from_reader_xml(manifest(42).as_slice()).unwrap();
    let item = &plist.as_dictionary().unwrap()["items"].as_array().unwrap()[0];
    let assets = item.as_dictionary().unwrap()["assets"].as_array().unwrap();
    let package = assets[0].as_dictionary().unwrap();
    assert_eq!(package["kind"].as_string(), Some("software-package"));
    assert_eq!(
        package["url"].as_string(),
        Some(format!("http://127.0.0.1:{SERVER_PORT}/app42.ipa").as_str())
    );
    let metadata = item.as_dictionary().unwrap()["metadata"]
        .as_dictionary()
        .unwrap();
    assert_eq!(
        metadata["bundle-identifier"].as_string(),
        Some("dev.warpsign.install.42")
    );
    assert_ne!(manifest(1), manifest(2));
}
