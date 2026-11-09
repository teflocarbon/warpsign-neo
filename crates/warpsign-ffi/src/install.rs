//! Installing on this device through the loopback VPN, for Swift.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use warpsign_core::device::{self, DeviceError, INSTALL_STEPS, Target};
use warpsign_core::pairable_host;
use warpsign_core::serve::{self, InstallServer, ManifestApp, SERVER_INSTALL_STEPS, SLOTS};

use crate::{ObserverBridge, StepInfo, StepObserver, WarpsignError};

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PairingFileInfo {
    /// The device it's for, if the file says
    pub udid: Option<String>,
    /// RemotePairing (works over the VPN on iOS 26.6+) rather than lockdown
    pub remote_pairing: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ConnectedDevice {
    pub udid: String,
    pub name: String,
    pub product_type: String,
    pub ios_version: String,
}

fn device_error(error: DeviceError) -> WarpsignError {
    let message = error.to_string();
    match error {
        DeviceError::Unreachable(_) => WarpsignError::DeviceUnreachable { message },
        DeviceError::PairingCancelled => WarpsignError::PairingCancelled { message },
        _ => WarpsignError::Device { message },
    }
}

/// Check a pairing file before keeping it
#[uniffi::export]
pub fn pairing_file_info(pairing_file: Vec<u8>) -> Result<PairingFileInfo, WarpsignError> {
    let info = device::read_pairing_file(&pairing_file).map_err(device_error)?;
    Ok(PairingFileInfo {
        udid: info.udid,
        remote_pairing: info.kind == device::PairingKind::RemotePairing,
    })
}

/// The port RemotePairing is tried on unless told otherwise
#[uniffi::export]
pub fn default_remote_pairing_port() -> u16 {
    device::REMOTE_PAIRING_PORT
}

/// Connect to this device through the VPN: checks the VPN and pairing file
/// work. `remote_pairing_port` overrides where RemotePairing files connect.
/// Blocks: call from a background thread.
#[uniffi::export]
pub fn connected_device(
    pairing_file: Vec<u8>,
    remote_pairing_port: Option<u16>,
) -> Result<ConnectedDevice, WarpsignError> {
    let info = device::device_info(&Target::Loopback {
        pairing_file,
        remote_pairing_port,
    })
    .map_err(device_error)?;
    Ok(ConnectedDevice {
        udid: info.udid,
        name: info.name,
        product_type: info.product_type,
        ios_version: info.ios_version,
    })
}

/// An app installed on this device
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct InstalledApp {
    pub bundle_id: String,
    pub name: Option<String>,
    pub version: Option<String>,
    pub build: Option<String>,
}

/// Every app installed on this device that isn't Apple's, however it was
/// installed (with a pairing file, over the web, from the App Store), by
/// asking installation_proxy through the VPN. Blocks: call from a
/// background thread.
#[uniffi::export]
pub fn installed_apps(
    pairing_file: Vec<u8>,
    remote_pairing_port: Option<u16>,
) -> Result<Vec<InstalledApp>, WarpsignError> {
    let apps = device::installed_apps(&Target::Loopback {
        pairing_file,
        remote_pairing_port,
    })
    .map_err(device_error)?;
    Ok(apps
        .into_iter()
        .map(|app| InstalledApp {
            bundle_id: app.bundle_id,
            name: app.name,
            version: app.version,
            build: app.build,
        })
        .collect())
}

/// The steps `install_ipa` runs, in order
#[uniffi::export]
pub fn install_steps() -> Vec<StepInfo> {
    INSTALL_STEPS
        .iter()
        .map(|&step| StepInfo {
            step: step.into(),
            title: step.title().to_string(),
        })
        .collect()
}

/// Install a signed IPA on this device through the VPN, reporting each step.
/// Blocks: call from a background thread.
#[uniffi::export]
pub fn install_ipa(
    pairing_file: Vec<u8>,
    remote_pairing_port: Option<u16>,
    ipa_path: String,
    observer: Arc<dyn StepObserver>,
) -> Result<(), WarpsignError> {
    device::install_ipa(
        &Target::Loopback {
            pairing_file,
            remote_pairing_port,
        },
        Path::new(&ipa_path),
        &ObserverBridge(observer),
    )
    .map_err(device_error)
}

/// Installing through iOS's own installer, ESign-style: serves the IPA on
/// loopback while iOS fetches it via a manifest from the manifest server.
/// No setup on the phone.
#[derive(uniffi::Object)]
pub struct WebInstall {
    server: Mutex<Option<InstallServer>>,
    link: String,
}

#[uniffi::export]
impl WebInstall {
    /// Start serving `ipa_path` for `slot` (rotate through
    /// `web_install_slots()`), reporting the steps to `observer`. Open
    /// `install_link()` next. `manifest_server` is the manifest Worker's
    /// URL, e.g. `https://warpsign-manifests.example.workers.dev`.
    #[uniffi::constructor]
    pub fn start(
        ipa_path: String,
        icon_png: Option<Vec<u8>>,
        slot: u32,
        manifest_server: String,
        observer: Arc<dyn StepObserver>,
    ) -> Result<Arc<Self>, WarpsignError> {
        let failed = |message: String| WarpsignError::Device { message };
        let manifest_server = manifest_server.trim();
        if !manifest_server.starts_with("https://") {
            return Err(failed(
                "Installing over the web needs a manifest server: set one in Settings → Installing, or use the pairing route."
                    .into(),
            ));
        }
        let ipa = Path::new(&ipa_path);
        // From the signed IPA itself, so the manifest always matches it
        let app = ManifestApp::from_ipa(ipa).map_err(|e| failed(e.to_string()))?;
        let slot = slot % SLOTS;
        let server = InstallServer::start(ipa, icon_png, slot, Box::new(ObserverBridge(observer)))
            .map_err(|e| failed(e.to_string()))?;
        Ok(Arc::new(Self {
            server: Mutex::new(Some(server)),
            link: serve::install_link(manifest_server, slot, &app),
        }))
    }

    /// The itms-services link that makes iOS ask to install the app
    pub fn install_link(&self) -> String {
        self.link.clone()
    }

    /// Stop serving (also happens when released)
    pub fn stop(&self) {
        let mut server = self.server.lock().unwrap_or_else(|e| e.into_inner());
        server.take();
    }
}

/// Serves a signed IPA on loopback for WarpSign Companion to fetch: how
/// WarpSign hands over its own updates, as it can't install over itself.
/// Nothing but this device can reach it.
#[derive(uniffi::Object)]
pub struct LoopbackShare {
    server: Mutex<Option<InstallServer>>,
    url: String,
}

#[uniffi::export]
impl LoopbackShare {
    /// Start serving `ipa_path` for `slot`, reporting to `observer` (the
    /// install server's steps: waiting, then sending)
    #[uniffi::constructor]
    pub fn start(
        ipa_path: String,
        slot: u32,
        observer: Arc<dyn StepObserver>,
    ) -> Result<Arc<Self>, WarpsignError> {
        let failed = |message: String| WarpsignError::Device { message };
        let slot = slot % SLOTS;
        let server = InstallServer::start(
            Path::new(&ipa_path),
            None,
            slot,
            Box::new(ObserverBridge(observer)),
        )
        .map_err(|e| failed(e.to_string()))?;
        let url = format!("http://127.0.0.1:{}/app{slot}.ipa", server.port());
        Ok(Arc::new(Self {
            server: Mutex::new(Some(server)),
            url,
        }))
    }

    /// Where Companion fetches it
    pub fn url(&self) -> String {
        self.url.clone()
    }

    /// Stop serving (also happens when released)
    pub fn stop(&self) {
        let mut server = self.server.lock().unwrap_or_else(|e| e.into_inner());
        server.take();
    }
}

/// The manifest server WarpSign uses unless Settings says otherwise
#[uniffi::export]
pub fn default_manifest_server() -> String {
    serve::DEFAULT_MANIFEST_SERVER.to_string()
}

/// How many manifest slots there are to rotate through
#[uniffi::export]
pub fn web_install_slots() -> u32 {
    SLOTS
}

/// The steps a web install reports, in order
#[uniffi::export]
pub fn web_install_steps() -> Vec<StepInfo> {
    SERVER_INSTALL_STEPS
        .iter()
        .map(|&step| StepInfo {
            step: step.into(),
            title: step.title().to_string(),
        })
        .collect()
}

/// Implemented in Swift: show the pairing code
#[uniffi::export(with_foreign)]
pub trait PairingCodeObserver: Send + Sync {
    /// The six-digit code to type into Settings. Called once, from a
    /// background thread, while `OnDevicePairing.accept` waits.
    fn show_code(&self, code: String);
}

/// This device, paired from WarpSign
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PairedDevice {
    /// A RemotePairing file for this device: keep it like an imported one
    pub pairing_file: Vec<u8>,
    pub name: String,
    pub model: String,
}

/// Pairing with this device without a computer (iOS 27+): advertise
/// `service_type()` as `service_name()` on `port()` with `txt_record()`
/// (NetService), then call `accept` on a background thread. The user taps
/// "Pair with …" in Settings → Privacy & Security → Developer Mode and types
/// the code.
#[derive(uniffi::Object)]
pub struct OnDevicePairing {
    host: pairable_host::PairableHost,
}

#[uniffi::export]
impl OnDevicePairing {
    /// Start listening. `name` is what Settings shows.
    #[uniffi::constructor]
    pub fn new(name: String) -> Result<Arc<Self>, WarpsignError> {
        Ok(Arc::new(Self {
            host: pairable_host::PairableHost::new(&name).map_err(device_error)?,
        }))
    }

    /// The Bonjour type, as NetService takes it
    pub fn service_type(&self) -> String {
        pairable_host::SERVICE_TYPE.to_string()
    }

    /// The Bonjour instance name
    pub fn service_name(&self) -> String {
        self.host.service_name().to_string()
    }

    pub fn port(&self) -> u16 {
        self.host.port()
    }

    /// The TXT record, for `NetService.data(fromTXTRecord:)`
    pub fn txt_record(&self) -> HashMap<String, String> {
        self.host.txt_records().iter().cloned().collect()
    }

    /// Wait for this device to pair. Blocks; `cancel` from another thread
    /// ends it with `WarpsignError.PairingCancelled`. Only once.
    pub fn accept(
        &self,
        observer: Arc<dyn PairingCodeObserver>,
    ) -> Result<PairedDevice, WarpsignError> {
        let paired = self
            .host
            .accept(|code| observer.show_code(code))
            .map_err(device_error)?;
        Ok(PairedDevice {
            pairing_file: paired.pairing_file,
            name: paired.name,
            model: paired.model,
        })
    }

    /// Stop waiting
    pub fn cancel(&self) {
        self.host.cancel();
    }
}
