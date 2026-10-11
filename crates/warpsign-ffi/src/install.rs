//! Installing on this device through the loopback VPN, for Swift.

use std::path::Path;
use std::sync::{Arc, Mutex};

use warpsign_core::device::{self, DeviceError, INSTALL_STEPS, Target};
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

/// The manifest server WarpSign uses unless Settings says otherwise
/// (empty until one is deployed)
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
