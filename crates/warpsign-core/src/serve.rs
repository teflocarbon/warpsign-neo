//! Installing through iOS's own over-the-air installer, as ESign does: no
//! computer, no VPN.
//!
//! iOS installs from an `itms-services://?action=download-manifest&url=...`
//! link, and insists the manifest comes over trusted HTTPS, but not the IPA
//! it points to. So the manifests are static files on GitHub (the same for
//! everyone: slot N's says "the app is at http://127.0.0.1:PORT/appN.ipa"),
//! and the app serves the IPA itself from loopback with [`InstallServer`].
//! Slots rotate, so iOS never sees the same manifest twice in a row.
//!
//! Python warpsign never had this; the trick is from ESign 5.0.2, which
//! uses its own manifests the same way (see docs/design/installing.md).

use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::metadata::read_ipa_metadata;
use crate::portal::http::form_encode;
use crate::progress::{ProgressObserver, Step};

/// The manifest server: a Cloudflare Worker (worker/ in this repo) that
/// answers each install with a manifest naming the app. iOS 27 refuses an
/// app whose bundle ID doesn't match its manifest, so the static
/// placeholder manifests ESign pioneered no longer work.
pub const DEFAULT_MANIFEST_SERVER: &str = "https://warpsign-manifests.teflo.dev";

/// The port the server listens on, and the manifests point at
pub const SERVER_PORT: u16 = 14310;

/// Installs rotate through this many slots, so each has its own URLs and
/// iOS never mixes one install up with the last
pub const SLOTS: u32 = 1000;

/// Steps of an install through iOS's installer, in order
pub const SERVER_INSTALL_STEPS: [Step; 2] = [Step::WaitForInstall, Step::SendToIos];

/// What a manifest says about the app. iOS checks the bundle ID against
/// the IPA, and shows the title while installing.
#[derive(Debug, Clone, PartialEq)]
pub struct ManifestApp {
    pub bundle_id: String,
    pub title: String,
    pub version: String,
}

impl ManifestApp {
    /// As the IPA's Info.plist says, so the manifest always matches
    pub fn from_ipa(ipa: &Path) -> Result<Self, ServeError> {
        let metadata = read_ipa_metadata(ipa).map_err(|e| ServeError::Metadata(e.to_string()))?;
        // The manifest server accepts at most 100 characters of title
        let title: String = metadata.display_name.chars().take(100).collect();
        // ... and a plain version, which bundle-version doesn't need to match
        let version = metadata
            .version
            .filter(|v| {
                !v.is_empty()
                    && v.len() <= 64
                    && v.chars()
                        .all(|c| c.is_ascii_alphanumeric() || ".+-".contains(c))
            })
            .unwrap_or_else(|| "1.0".into());
        Ok(Self {
            bundle_id: metadata.bundle_id,
            title: if title.trim().is_empty() {
                "App".into()
            } else {
                title
            },
            version,
        })
    }
}

/// The manifest for an install, byte for byte what the manifest server
/// sends (both are checked against worker/test/manifest.plist)
pub fn manifest(slot: u32, port: u16, app: &ManifestApp) -> Vec<u8> {
    let base = format!("http://127.0.0.1:{port}/app{slot}");
    let asset = |kind: &str, url: String| {
        let mut asset = plist::Dictionary::new();
        asset.insert("kind".into(), kind.into());
        if kind != "software-package" {
            asset.insert("needs-shine".into(), true.into());
        }
        asset.insert("url".into(), url.into());
        plist::Value::Dictionary(asset)
    };
    let mut metadata = plist::Dictionary::new();
    metadata.insert("bundle-identifier".into(), app.bundle_id.clone().into());
    metadata.insert("bundle-version".into(), app.version.clone().into());
    metadata.insert("kind".into(), "software".into());
    metadata.insert("title".into(), app.title.clone().into());

    let mut item = plist::Dictionary::new();
    item.insert(
        "assets".into(),
        plist::Value::Array(vec![
            asset("software-package", format!("{base}.ipa")),
            asset("display-image", format!("{base}.png")),
            asset("full-size-image", format!("{base}.png")),
        ]),
    );
    item.insert("metadata".into(), plist::Value::Dictionary(metadata));
    let mut root = plist::Dictionary::new();
    root.insert(
        "items".into(),
        plist::Value::Array(vec![plist::Value::Dictionary(item)]),
    );

    let mut data = Vec::new();
    plist::Value::Dictionary(root)
        .to_writer_xml(&mut data)
        .expect("plists serialize");
    data
}

/// The link that makes iOS ask to install `app` from `slot`, with its
/// manifest from `server` (e.g. https://warpsign.example.workers.dev)
pub fn install_link(server: &str, slot: u32, app: &ManifestApp) -> String {
    let port = SERVER_PORT.to_string();
    let slot = slot.to_string();
    let manifest_url = format!(
        "{}/manifest?{}",
        server.trim_end_matches('/'),
        form_encode(&[
            ("slot", &slot),
            ("port", &port),
            ("id", &app.bundle_id),
            ("title", &app.title),
            ("version", &app.version),
        ])
    );
    format!(
        "itms-services://?{}",
        form_encode(&[("action", "download-manifest"), ("url", &manifest_url)])
    )
}

#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    #[error(
        "couldn't start the install server on port {SERVER_PORT} ({0}). Another installer (or a previous install) may be using it: close other signing apps and try again."
    )]
    Port(std::io::Error),
    #[error("couldn't read the app's details: {0}")]
    Metadata(String),
    #[error("{path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
}

/// Serves one signed IPA (and an icon) to iOS's installer on loopback until
/// dropped or stopped
pub struct InstallServer {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    port: u16,
}

struct Shared {
    ipa: PathBuf,
    size: u64,
    icon: Vec<u8>,
    slot: u32,
    observer: Box<dyn ProgressObserver>,
    state: Mutex<Progress>,
}

#[derive(Default)]
struct Progress {
    waiting_since: Option<Instant>,
    sending_since: Option<Instant>,
    sent: u64,
    done: bool,
}

impl InstallServer {
    /// Start serving `ipa` for `slot` on [`SERVER_PORT`]
    pub fn start(
        ipa: &Path,
        icon: Option<Vec<u8>>,
        slot: u32,
        observer: Box<dyn ProgressObserver>,
    ) -> Result<Self, ServeError> {
        Self::start_on(SERVER_PORT, ipa, icon, slot, observer)
    }

    /// As [`Self::start`] on another port (0 for any): for tests
    pub fn start_on(
        port: u16,
        ipa: &Path,
        icon: Option<Vec<u8>>,
        slot: u32,
        observer: Box<dyn ProgressObserver>,
    ) -> Result<Self, ServeError> {
        let size = std::fs::metadata(ipa)
            .map_err(|source| ServeError::Io {
                path: ipa.display().to_string(),
                source,
            })?
            .len();
        // Loopback only: nothing else on the network can fetch the app
        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))
            .map_err(ServeError::Port)?;
        let port = listener.local_addr().map_err(ServeError::Port)?.port();
        listener.set_nonblocking(true).map_err(ServeError::Port)?;

        observer.step_started(Step::WaitForInstall);
        observer.step_log(
            Step::WaitForInstall,
            "Tap Install when iOS asks. Keep WarpSign open until the app is sent.",
        );
        let shared = Arc::new(Shared {
            ipa: ipa.to_path_buf(),
            size,
            icon: icon.unwrap_or_else(placeholder_icon),
            slot,
            observer,
            state: Mutex::new(Progress {
                waiting_since: Some(Instant::now()),
                ..Default::default()
            }),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let stop = stop.clone();
            std::thread::Builder::new()
                .name("warpsign-install-server".into())
                .spawn(move || accept_loop(listener, shared, stop))
                .map_err(ServeError::Port)?
        };
        Ok(Self {
            stop,
            thread: Some(thread),
            port,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for InstallServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn accept_loop(listener: TcpListener, shared: Arc<Shared>, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                let shared = shared.clone();
                let stop = stop.clone();
                // iOS may open several connections (e.g. icon and package)
                let _ = std::thread::Builder::new()
                    .name("warpsign-install-request".into())
                    .spawn(move || handle_logged(stream, &shared, &stop));
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

struct HttpRequest {
    method: String,
    path: String,
    range: Option<(u64, Option<u64>)>,
    /// Which part of iOS is asking, e.g. `com.apple.appstored/1.0 iOS/27.0`
    agent: Option<String>,
}

fn read_request(stream: &TcpStream) -> std::io::Result<Option<HttpRequest>> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let mut parts = line.split_whitespace();
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
        return Ok(None);
    };
    let path = target.split('?').next().unwrap_or(target).to_string();
    let mut range = None;
    let mut agent = None;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            let name = name.trim();
            if name.eq_ignore_ascii_case("range") {
                range = parse_range(value.trim());
            } else if name.eq_ignore_ascii_case("user-agent") {
                agent = Some(value.trim().to_string());
            }
        }
    }
    Ok(Some(HttpRequest {
        method: method.to_ascii_uppercase(),
        path,
        range,
        agent,
    }))
}

/// `bytes=START-` or `bytes=START-END` (a single range)
fn parse_range(value: &str) -> Option<(u64, Option<u64>)> {
    let spec = value.strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None;
    }
    let (start, end) = spec.split_once('-')?;
    let start: u64 = start.trim().parse().ok()?;
    let end = match end.trim() {
        "" => None,
        end => Some(end.parse().ok()?),
    };
    Some((start, end))
}

impl Shared {
    /// Log to whichever step is current, so a failed install shows what
    /// iOS asked for
    fn log(&self, message: String) {
        let waiting = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .waiting_since
            .is_some();
        let step = if waiting {
            Step::WaitForInstall
        } else {
            Step::SendToIos
        };
        self.observer.step_log(step, &message);
    }
}

fn handle_logged(stream: TcpStream, shared: &Shared, stop: &AtomicBool) {
    if let Err(error) = handle(stream, shared, stop) {
        shared.log(format!("Connection ended early: {error}"));
    }
}

fn handle(mut stream: TcpStream, shared: &Shared, stop: &AtomicBool) -> std::io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    let Some(request) = read_request(&stream)? else {
        return Ok(());
    };
    let range = match request.range {
        Some((start, Some(end))) => format!(" bytes {start}-{end}"),
        Some((start, None)) => format!(" bytes {start}-"),
        None => String::new(),
    };
    shared.log(format!(
        "iOS asked for {} {}{range} ({})",
        request.method,
        request.path,
        request.agent.as_deref().unwrap_or("no user agent")
    ));
    let head = request.method == "HEAD";
    if request.method != "GET" && !head {
        return respond(
            &mut stream,
            "405 Method Not Allowed",
            "text/plain",
            b"",
            head,
        );
    }

    let prefix = format!("/app{}", shared.slot);
    if request.path == format!("{prefix}.png") {
        respond(&mut stream, "200 OK", "image/png", &shared.icon, head)
    } else if request.path == format!("{prefix}.ipa") {
        send_ipa(&mut stream, shared, request.range, head, stop)
    } else {
        respond(
            &mut stream,
            "404 Not Found",
            "text/plain",
            b"not found",
            head,
        )
    }
}

fn respond(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
    head: bool,
) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    if !head {
        stream.write_all(body)?;
    }
    stream.flush()
}

fn send_ipa(
    stream: &mut TcpStream,
    shared: &Shared,
    range: Option<(u64, Option<u64>)>,
    head: bool,
    stop: &AtomicBool,
) -> std::io::Result<()> {
    let size = shared.size;
    let (start, end) = match range {
        Some((start, end)) if start < size => (start, end.unwrap_or(size - 1).min(size - 1)),
        Some(_) => {
            write!(
                stream,
                "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{size}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )?;
            return stream.flush();
        }
        None => (0, size.saturating_sub(1)),
    };
    let length = if size == 0 { 0 } else { end - start + 1 };
    let status = if range.is_some() {
        format!("206 Partial Content\r\nContent-Range: bytes {start}-{end}/{size}")
    } else {
        "200 OK".to_string()
    };
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: application/octet-stream\r\nContent-Length: {length}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n"
    )?;
    if head {
        return stream.flush();
    }

    // iOS asked for the app: the user tapped Install
    {
        let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(since) = state.waiting_since.take() {
            shared
                .observer
                .step_finished(Step::WaitForInstall, since.elapsed(), None);
            shared.observer.step_started(Step::SendToIos);
            state.sending_since = Some(Instant::now());
        }
        state.sent = state.sent.max(start);
    }

    let mut file = std::fs::File::open(&shared.ipa)?;
    file.seek(SeekFrom::Start(start))?;
    let mut remaining = length;
    let mut buffer = vec![0u8; 256 * 1024];
    while remaining > 0 && !stop.load(Ordering::Relaxed) {
        let want = remaining.min(buffer.len() as u64) as usize;
        let read = file.read(&mut buffer[..want])?;
        if read == 0 {
            break;
        }
        stream.write_all(&buffer[..read])?;
        remaining -= read as u64;

        let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        state.sent = state.sent.max(end + 1 - remaining);
        shared
            .observer
            .step_progress(Step::SendToIos, state.sent as f64 / size.max(1) as f64);
        if state.sent >= size && !state.done {
            state.done = true;
            let elapsed = state.sending_since.map(|s| s.elapsed()).unwrap_or_default();
            shared.observer.step_log(
                Step::SendToIos,
                "iOS has the app and is installing it: watch the Home Screen",
            );
            shared
                .observer
                .step_finished(Step::SendToIos, elapsed, None);
        }
    }
    stream.flush()
}

/// A plain square for iOS's install progress when the app has no icon
fn placeholder_icon() -> Vec<u8> {
    let mut image = image::RgbImage::new(57, 57);
    for pixel in image.pixels_mut() {
        *pixel = image::Rgb([92, 64, 230]);
    }
    let mut data = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut data, image::ImageFormat::Png)
        .expect("PNGs encode");
    data.into_inner()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ranges() {
        assert_eq!(parse_range("bytes=0-"), Some((0, None)));
        assert_eq!(parse_range("bytes=100-199"), Some((100, Some(199))));
        assert_eq!(parse_range("bytes=0-1,5-6"), None);
        assert_eq!(parse_range("items=0-1"), None);
    }

    fn example() -> ManifestApp {
        ManifestApp {
            bundle_id: "com.example.app".into(),
            title: "Tom's <Fish> & \"Chips\" 🐟".into(),
            version: "2.1+beta".into(),
        }
    }

    /// The manifest server must send exactly this too: see worker/test
    #[test]
    fn manifests_match_the_shared_fixture() {
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../worker/test/manifest.plist");
        let made = manifest(42, 14310, &example());
        if std::env::var_os("WARPSIGN_WRITE_FIXTURE").is_some() {
            std::fs::write(&fixture, &made).unwrap();
        }
        assert_eq!(
            String::from_utf8(made).unwrap(),
            std::fs::read_to_string(fixture).unwrap()
        );
    }

    #[test]
    fn links_carry_the_app_to_the_manifest_server() {
        assert_eq!(
            install_link("https://warpsign.example.workers.dev/", 7, &example()),
            "itms-services://?action=download-manifest&url=https%3A%2F%2Fwarpsign.example.workers.dev%2Fmanifest%3Fslot%3D7%26port%3D14310%26id%3Dcom.example.app%26title%3DTom%2527s%2B%253CFish%253E%2B%2526%2B%2522Chips%2522%2B%25F0%259F%2590%259F%26version%3D2.1%252Bbeta"
        );
    }
}
