//! Installing on this device through the loopback VPN, for Swift.

use std::path::Path;
use std::sync::Arc;

use warpsign_core::device::{self, DeviceError, INSTALL_STEPS, Target};

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
