//! Installing apps on an iPhone or iPad, the way Xcode and SideStore do:
//! copy the IPA into the device's `PublicStaging` folder over AFC, then ask
//! `installation_proxy` to install it. No web server or itms-services link.
//!
//! The device is reached either
//! - from an app on the device itself, through a loopback VPN (LocalDevVPN)
//!   that routes [`LOOPBACK_ADDRESS`] back to the device's own lockdownd,
//!   trusted with a pairing file made once on a computer, or
//! - from a computer, over USB through usbmuxd.

use std::io::Read;
use std::net::{IpAddr, Ipv4Addr};
use std::path::Path;
use std::time::Duration;

use idevice::afc::AfcClient;
use idevice::afc::opcode::AfcFopenMode;
use idevice::heartbeat::HeartbeatClient;
use idevice::installation_proxy::InstallationProxyClient;
use idevice::lockdown::LockdownClient;
use idevice::pairing_file::PairingFile;
use idevice::provider::{IdeviceProvider, TcpProvider};
use idevice::usbmuxd::{Connection, UsbmuxdAddr, UsbmuxdConnection};
use idevice::{IdeviceError, IdeviceService};
use tokio::io::AsyncWriteExt;

use crate::metadata::read_ipa_metadata;
use crate::progress::{ProgressObserver, Step, run_step};

/// Where LocalDevVPN (and StosVPN before it) loops connections back to the
/// device itself
pub const LOOPBACK_ADDRESS: Ipv4Addr = Ipv4Addr::new(10, 7, 0, 1);

/// lockdownd's port
const LOCKDOWN_PORT: u16 = 62078;

/// Steps `install_ipa` runs, in order
pub const INSTALL_STEPS: [Step; 3] = [Step::ConnectDevice, Step::UploadApp, Step::InstallApp];

const STAGING: &str = "PublicStaging";

#[derive(Debug, thiserror::Error)]
pub enum DeviceError {
    #[error("that isn't a pairing file ({0})")]
    InvalidPairingFile(String),
    #[error(
        "couldn't reach the device at {0}: check LocalDevVPN is installed and connected, then try again"
    )]
    Unreachable(String),
    #[error("no iPhone or iPad is connected by USB")]
    NoUsbDevice,
    #[error("no device with UDID {0} is connected by USB")]
    UsbDeviceNotFound(String),
    #[error(
        "couldn't talk to usbmuxd, which finds USB devices ({0}). On Windows, install Apple Devices or iTunes; on Linux, usbmuxd."
    )]
    Usbmuxd(String),
    #[error(
        "the device didn't accept the pairing file ({0}). It may be for another device, or the device was reset or stopped trusting it: make a new one with `warpsign pair`."
    )]
    PairingRejected(String),
    #[error("pairing was refused on the device: tap Trust when it asks, then try again")]
    PairingDenied,
    #[error("unlock the device and try again")]
    Locked,
    #[error("{0}")]
    InstallFailed(String),
    #[error("{context}: {source}")]
    Device {
        context: &'static str,
        source: IdeviceError,
    },
    #[error("{path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("{0}")]
    Package(String),
}

fn failed(context: &'static str) -> impl FnOnce(IdeviceError) -> DeviceError {
    move |source| match source {
        IdeviceError::PasswordProtected => DeviceError::Locked,
        IdeviceError::UserDeniedPairing => DeviceError::PairingDenied,
        source => DeviceError::Device { context, source },
    }
}

/// What a pairing file says about itself
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingInfo {
    /// The device it pairs with. Older tools leave it out.
    pub udid: Option<String>,
    pub host_id: String,
    pub wifi_mac_address: String,
}

/// Check a pairing file (`.mobiledevicepairing` or `.plist`) can be used
pub fn read_pairing_file(data: &[u8]) -> Result<PairingInfo, DeviceError> {
    let pairing = parse_pairing(data)?;
    Ok(PairingInfo {
        udid: pairing.udid,
        host_id: pairing.host_id,
        wifi_mac_address: pairing.wifi_mac_address,
    })
}

fn parse_pairing(data: &[u8]) -> Result<PairingFile, DeviceError> {
    PairingFile::from_bytes(data).map_err(|e| DeviceError::InvalidPairingFile(e.to_string()))
}

/// How to reach the device
#[derive(Clone)]
pub enum Target {
    /// The device this runs on, through the loopback VPN
    Loopback { pairing_file: Vec<u8> },
    /// A device on this computer's USB, or the only one if `udid` is None
    Usb { udid: Option<String> },
    /// A device at an address, e.g. in tests or a custom VPN
    Network {
        address: IpAddr,
        pairing_file: Vec<u8>,
    },
}

impl std::fmt::Debug for Target {
    // Never print the pairing file: it holds private keys
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Target::Loopback { .. } => write!(f, "Loopback"),
            Target::Usb { udid } => write!(f, "Usb({udid:?})"),
            Target::Network { address, .. } => write!(f, "Network({address})"),
        }
    }
}

/// A connected device
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub udid: String,
    pub name: String,
    /// e.g. "iPhone16,1"
    pub product_type: String,
    pub ios_version: String,
}

fn runtime() -> Result<tokio::runtime::Runtime, DeviceError> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| DeviceError::Io {
            path: "async runtime".into(),
            source,
        })
}

/// A provider for the target, checked to be reachable
async fn connect(target: &Target) -> Result<Box<dyn IdeviceProvider>, DeviceError> {
    let (address, pairing_file) = match target {
        Target::Loopback { pairing_file } => (IpAddr::V4(LOOPBACK_ADDRESS), pairing_file),
        Target::Network {
            address,
            pairing_file,
        } => (*address, pairing_file),
        Target::Usb { udid } => {
            let mut usbmuxd = UsbmuxdConnection::default()
                .await
                .map_err(|e| DeviceError::Usbmuxd(e.to_string()))?;
            let devices = usbmuxd
                .get_devices()
                .await
                .map_err(|e| DeviceError::Usbmuxd(e.to_string()))?;
            let device = match udid {
                Some(udid) => devices
                    .into_iter()
                    .find(|d| &d.udid == udid)
                    .ok_or_else(|| DeviceError::UsbDeviceNotFound(udid.clone()))?,
                None => devices
                    .into_iter()
                    .find(|d| d.connection_type == Connection::Usb)
                    .ok_or(DeviceError::NoUsbDevice)?,
            };
            return Ok(Box::new(
                device.to_provider(UsbmuxdAddr::default(), "warpsign"),
            ));
        }
    };

    let pairing_file = parse_pairing(pairing_file)?;
    // Fail fast and clearly when the VPN isn't up
    let reachable = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::TcpStream::connect((address, LOCKDOWN_PORT)),
    )
    .await;
    match reachable {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => return Err(DeviceError::Unreachable(format!("{address} ({e})"))),
        Err(_) => return Err(DeviceError::Unreachable(format!("{address} (timed out)"))),
    }
    Ok(Box::new(TcpProvider {
        addr: address,
        scope_id: None,
        pairing_file,
        label: "warpsign".into(),
    }))
}

/// Start a lockdown session, which proves the pairing works
async fn session(provider: &dyn IdeviceProvider) -> Result<LockdownClient, DeviceError> {
    let mut lockdown = LockdownClient::connect(provider)
        .await
        .map_err(failed("connecting to lockdownd"))?;
    let pairing = provider
        .get_pairing_file()
        .await
        .map_err(|e| DeviceError::PairingRejected(e.to_string()))?;
    lockdown
        .start_session(&pairing)
        .await
        .map_err(|e| match e {
            IdeviceError::PasswordProtected => DeviceError::Locked,
            e => DeviceError::PairingRejected(e.to_string()),
        })?;
    Ok(lockdown)
}

async fn value(lockdown: &mut LockdownClient, key: &str) -> String {
    match lockdown.get_value(Some(key), None).await {
        Ok(value) => value.as_string().unwrap_or_default().to_string(),
        Err(_) => String::new(),
    }
}

/// Connect and describe the device: checks the VPN and pairing work
pub fn device_info(target: &Target) -> Result<DeviceInfo, DeviceError> {
    runtime()?.block_on(async {
        let provider = connect(target).await?;
        let mut lockdown = session(&*provider).await?;
        Ok(DeviceInfo {
            udid: value(&mut lockdown, "UniqueDeviceID").await,
            name: value(&mut lockdown, "DeviceName").await,
            product_type: value(&mut lockdown, "ProductType").await,
            ios_version: value(&mut lockdown, "ProductVersion").await,
        })
    })
}

/// Answer the device's heartbeat, which keeps network connections alive
async fn heartbeat(mut client: HeartbeatClient) {
    let mut interval = 15;
    while let Ok(next) = client.get_marco(interval).await {
        interval = next + 5;
        if client.send_polo().await.is_err() {
            break;
        }
    }
}

/// Install a signed IPA on the device, reporting each step
pub fn install_ipa(
    target: &Target,
    ipa: &Path,
    observer: &dyn ProgressObserver,
) -> Result<(), DeviceError> {
    let runtime = runtime()?;
    let bundle_id = read_ipa_metadata(ipa)
        .map_err(|e| DeviceError::Package(e.to_string()))?
        .bundle_id;

    let (provider, beating) = run_step(observer, Step::ConnectDevice, |log| {
        runtime.block_on(async {
            let provider = connect(target).await?;
            let mut lockdown = session(&*provider).await?;
            log.log(format!(
                "{} (iOS {})",
                value(&mut lockdown, "DeviceName").await,
                value(&mut lockdown, "ProductVersion").await
            ));
            // USB connections don't need a heartbeat; network ones are
            // dropped without one
            let beating = match target {
                Target::Usb { .. } => None,
                _ => match HeartbeatClient::connect(&*provider).await {
                    Ok(client) => Some(tokio::spawn(heartbeat(client))),
                    Err(e) => {
                        log.log(format!("No heartbeat ({e}): continuing"));
                        None
                    }
                },
            };
            Ok::<_, DeviceError>((provider, beating))
        })
    })?;

    let result = (|| {
        let remote = run_step(observer, Step::UploadApp, |log| {
            runtime.block_on(upload(&*provider, ipa, log))
        })?;
        run_step(observer, Step::InstallApp, |log| {
            runtime.block_on(async {
                let mut proxy = InstallationProxyClient::connect(&*provider)
                    .await
                    .map_err(failed("connecting to the installer"))?;
                let mut options = plist::Dictionary::new();
                options.insert("CFBundleIdentifier".into(), bundle_id.clone().into());
                let options = plist::Value::Dictionary(options);
                proxy
                    .install_with_callback(
                        remote.clone(),
                        Some(options),
                        |(percent, ())| {
                            log.progress(percent as f64 / 100.0);
                            async {}
                        },
                        (),
                    )
                    .await
                    .map_err(|e| {
                        DeviceError::InstallFailed(explain_install_error(&e.to_string()))
                    })?;
                log.log(format!("Installed {bundle_id}"));
                // installd removes it, but don't leave it behind if not
                if let Ok(mut afc) = AfcClient::connect(&*provider).await {
                    let _ = afc.remove(remote.as_str()).await;
                }
                Ok::<_, DeviceError>(())
            })
        })
    })();

    if let Some(beating) = beating {
        beating.abort();
    }
    result
}

/// Stream the IPA into PublicStaging, returning its path there
async fn upload(
    provider: &dyn IdeviceProvider,
    ipa: &Path,
    log: &crate::progress::StepLog<'_>,
) -> Result<String, DeviceError> {
    let io_error = |source| DeviceError::Io {
        path: ipa.display().to_string(),
        source,
    };
    let mut file = std::fs::File::open(ipa).map_err(io_error)?;
    let total = file.metadata().map_err(io_error)?.len().max(1);
    log.log(format!("{:.1} MB", total as f64 / 1_000_000.0));

    let mut afc = AfcClient::connect(provider)
        .await
        .map_err(failed("connecting to the device's file service"))?;
    if afc.get_file_info(STAGING).await.is_err() {
        afc.mk_dir(STAGING)
            .await
            .map_err(failed("creating the staging folder"))?;
    }
    let remote = format!("{STAGING}/warpsign.ipa");
    let mut destination = afc
        .open(remote.as_str(), AfcFopenMode::WrOnly)
        .await
        .map_err(failed("opening the staging file"))?;

    let mut buffer = vec![0u8; 1024 * 1024];
    let mut sent = 0u64;
    loop {
        let read = file.read(&mut buffer).map_err(io_error)?;
        if read == 0 {
            break;
        }
        destination
            .write_all(&buffer[..read])
            .await
            .map_err(|e| failed("copying to the device")(IdeviceError::Socket(e)))?;
        sent += read as u64;
        log.progress(sent as f64 / total as f64);
    }
    destination
        .close()
        .await
        .map_err(failed("finishing the copy"))?;
    Ok(remote)
}

/// installd's error, with what usually causes it
fn explain_install_error(message: &str) -> String {
    let hint = if message.contains("ApplicationVerificationFailed")
        || message.contains("could not be verified")
        || message.contains("valid provisioning profile")
    {
        Some(
            "The device didn't accept the signature or profile: check this device is registered on the team (Settings > Registered Devices) and the certificate is still valid, then sign again.",
        )
    } else if message.contains("MismatchedApplicationIdentifierEntitlement") {
        Some(
            "A version of this app signed by another team is installed: delete it first (its data goes with it).",
        )
    } else if message.contains("DeviceOSVersionTooLow") || message.contains("MinimumOSVersion") {
        Some("The app needs a newer iOS than this device has.")
    } else if message.contains("NoSpace") || message.contains("space") {
        Some("The device is out of space.")
    } else {
        None
    };
    match hint {
        Some(hint) => format!("{hint} ({message})"),
        None => format!("The device refused to install the app: {message}"),
    }
}

/// UDIDs of the iPhones and iPads connected to this computer by USB
pub fn usb_devices() -> Result<Vec<String>, DeviceError> {
    runtime()?.block_on(async {
        let mut usbmuxd = UsbmuxdConnection::default()
            .await
            .map_err(|e| DeviceError::Usbmuxd(e.to_string()))?;
        Ok(usbmuxd
            .get_devices()
            .await
            .map_err(|e| DeviceError::Usbmuxd(e.to_string()))?
            .into_iter()
            .filter(|d| d.connection_type == Connection::Usb)
            .map(|d| d.udid)
            .collect())
    })
}

/// Pair with a USB-connected device (the user taps Trust on it) and return
/// its UDID and a pairing file for WarpSign on the device: jitterbug's
/// format, as SideStore uses, with network connections allowed
pub fn create_pairing_file(
    udid: Option<&str>,
    host_name: &str,
) -> Result<(String, Vec<u8>), DeviceError> {
    runtime()?.block_on(async {
        let mut usbmuxd = UsbmuxdConnection::default()
            .await
            .map_err(|e| DeviceError::Usbmuxd(e.to_string()))?;
        let devices = usbmuxd
            .get_devices()
            .await
            .map_err(|e| DeviceError::Usbmuxd(e.to_string()))?;
        let device = devices
            .into_iter()
            .filter(|d| d.connection_type == Connection::Usb)
            .find(|d| udid.is_none_or(|udid| d.udid == udid))
            .ok_or_else(|| match udid {
                Some(udid) => DeviceError::UsbDeviceNotFound(udid.to_string()),
                None => DeviceError::NoUsbDevice,
            })?;
        let buid = usbmuxd
            .get_buid()
            .await
            .map_err(|e| DeviceError::Usbmuxd(e.to_string()))?;
        let provider = device.to_provider(UsbmuxdAddr::default(), "warpsign");

        let mut lockdown = LockdownClient::connect(&provider)
            .await
            .map_err(failed("connecting to lockdownd"))?;
        let mut pairing = lockdown
            .pair(new_host_id(), buid, Some(host_name))
            .await
            .map_err(failed("pairing"))?;
        // Prove it works before handing it out
        lockdown
            .start_session(&pairing)
            .await
            .map_err(|e| DeviceError::PairingRejected(e.to_string()))?;
        // Let lockdownd accept this pairing over the network (the VPN)
        lockdown
            .set_value(
                "EnableWifiDebugging",
                true.into(),
                Some("com.apple.mobile.wireless_lockdown"),
            )
            .await
            .map_err(failed("allowing network connections"))?;

        pairing.udid = Some(device.udid.clone());
        let data = pairing
            .serialize()
            .map_err(|e| DeviceError::InvalidPairingFile(e.to_string()))?;
        Ok((device.udid, data))
    })
}

/// A random UUID, uppercase, as hosts identify themselves
fn new_host_id() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("system random number generator");
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02X}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_ids_are_uuids() {
        let id = new_host_id();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
        assert_ne!(id, new_host_id());
    }

    #[test]
    fn explains_common_install_errors() {
        assert!(
            explain_install_error("ApplicationVerificationFailed: Failed to verify code signature")
                .contains("registered on the team")
        );
        assert!(
            explain_install_error("MismatchedApplicationIdentifierEntitlement")
                .contains("delete it first")
        );
        assert_eq!(
            explain_install_error("Something new"),
            "The device refused to install the app: Something new"
        );
    }
}
