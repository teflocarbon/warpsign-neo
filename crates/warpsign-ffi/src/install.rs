//! Installing on this device through the loopback VPN, for Swift.

use std::path::Path;
use std::sync::{Arc, Mutex};

use warpsign_core::device::{self, DeviceError, INSTALL_STEPS, Target};
use warpsign_core::serve::{self, InstallServer, SERVER_INSTALL_STEPS, SLOTS};

use crate::{ObserverBridge, StepInfo, StepObserver, WarpsignError};

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PairingFileInfo {
    /// The device it's for, if the file says
    pub udid: Option<String>,
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
    Ok(PairingFileInfo { udid: info.udid })
}

/// Connect to this device through the VPN: checks the VPN and pairing file
/// work. Blocks: call from a background thread.
#[uniffi::export]
pub fn connected_device(pairing_file: Vec<u8>) -> Result<ConnectedDevice, WarpsignError> {
    let info = device::device_info(&Target::Loopback { pairing_file }).map_err(device_error)?;
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
    ipa_path: String,
    observer: Arc<dyn StepObserver>,
) -> Result<(), WarpsignError> {
    device::install_ipa(
        &Target::Loopback { pairing_file },
        Path::new(&ipa_path),
        &ObserverBridge(observer),
    )
    .map_err(device_error)
}

/// Installing through iOS's own installer, ESign-style: serves the IPA on
/// loopback while iOS fetches it via a published manifest. No setup.
#[derive(uniffi::Object)]
pub struct WebInstall {
    server: Mutex<Option<InstallServer>>,
    link: String,
}

#[uniffi::export]
impl WebInstall {
    /// Start serving `ipa_path` for manifest `slot` (rotate through
    /// `web_install_slots()`), reporting the steps to `observer`. Open
    /// `install_link()` next.
    #[uniffi::constructor]
    pub fn start(
        ipa_path: String,
        icon_png: Option<Vec<u8>>,
        slot: u32,
        observer: Arc<dyn StepObserver>,
    ) -> Result<Arc<Self>, WarpsignError> {
        let slot = slot % SLOTS;
        let server = InstallServer::start(
            Path::new(&ipa_path),
            icon_png,
            slot,
            Box::new(ObserverBridge(observer)),
        )
        .map_err(|e| WarpsignError::Device {
            message: e.to_string(),
        })?;
        Ok(Arc::new(Self {
            server: Mutex::new(Some(server)),
            link: serve::install_link(slot),
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
