//! Installing apps on an iPhone or iPad, the way Xcode and SideStore do:
//! copy the IPA into the device's `PublicStaging` folder over AFC, then ask
//! `installation_proxy` to install it. No web server or itms-services link.
//!
//! The device is reached either
//! - from an app on the device itself, through a loopback VPN (LocalDevVPN)
//!   that routes [`LOOPBACK_ADDRESS`] back to the device, trusted with a
//!   pairing file made once on a computer. Since iOS 26.6, lockdownd resets
//!   connections that come through the VPN, so the device is reached with
//!   RemotePairing (iOS 17.4+) instead: pair-verify on its remote pairing
//!   port, then an encrypted tunnel to the same services, as SideStore now
//!   does. Lockdown pairing files still work on older iOS.
//! - from a computer, over USB through usbmuxd.

use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::time::Duration;

use idevice::afc::AfcClient;
use idevice::afc::opcode::AfcFopenMode;
use idevice::heartbeat::HeartbeatClient;
use idevice::installation_proxy::InstallationProxyClient;
use idevice::lockdown::LockdownClient;
use idevice::pairing_file::PairingFile;
use idevice::provider::{IdeviceProvider, TcpProvider};
use idevice::remote_pairing::{
    RemotePairingClient, RemotePairingLockdownService, RpPairingFile, RpPairingSocket,
    connect_tls_psk_tunnel_native,
};
use idevice::rsd::RsdHandshake;
use idevice::tcp::handle::AdapterHandle;
use idevice::usbmuxd::{Connection, UsbmuxdAddr, UsbmuxdConnection};
use idevice::{IdeviceError, IdeviceService, RsdService};
use tokio::io::AsyncWriteExt;

use crate::metadata::read_ipa_metadata;
use crate::progress::{ProgressObserver, Step, run_step_async};

/// Where LocalDevVPN (and StosVPN before it) loops connections back to the
/// device itself
pub const LOOPBACK_ADDRESS: Ipv4Addr = Ipv4Addr::new(10, 7, 0, 1);

/// lockdownd's port
const LOCKDOWN_PORT: u16 = 62078;

/// Where the device usually listens for RemotePairing (it advertises the
/// port as `_remotepairing._tcp`; some devices use another)
pub const REMOTE_PAIRING_PORT: u16 = 49152;

/// The name WarpSign gives itself in RemotePairing
const RP_HOST: &str = "WarpSign";

/// Steps `install_ipa` runs, in order
pub const INSTALL_STEPS: [Step; 3] = [Step::ConnectDevice, Step::UploadApp, Step::InstallApp];

const STAGING: &str = "PublicStaging";

#[derive(Debug, thiserror::Error)]
pub enum DeviceError {
    #[error("that isn't a pairing file ({0})")]
    InvalidPairingFile(String),
    #[error(
        "couldn't reach the device at {0}: check the loopback VPN (WarpSign's own, or LocalDevVPN) is connected, then try again"
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
    #[error(
        "the device closed the connection ({0}). Since iOS 26.6 it does this to lockdown pairing files over a VPN: make a new pairing file with `warpsign pair`, which makes a RemotePairing one."
    )]
    LockdownClosed(String),
    #[error("pairing was refused on the device: tap Trust when it asks, then try again")]
    PairingDenied,
    #[error("unlock the device and try again")]
    Locked,
    #[error("{0}")]
    InstallFailed(String),
    #[error("{context}: {}", detail(source))]
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

/// idevice's message, with the I/O error behind "device socket io failed"
fn detail(error: &IdeviceError) -> String {
    match error {
        IdeviceError::Socket(io) => format!("{error}: {io}"),
        error => error.to_string(),
    }
}

fn failed(context: &'static str) -> impl FnOnce(IdeviceError) -> DeviceError {
    move |source| match source {
        IdeviceError::PasswordProtected => DeviceError::Locked,
        IdeviceError::UserDeniedPairing => DeviceError::PairingDenied,
        source => DeviceError::Device { context, source },
    }
}

/// Which protocol a pairing file is for
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairingKind {
    /// lockdownd's (jitterbug's format): refused over the VPN since iOS 26.6
    Lockdown,
    /// RemotePairing's (iOS 17.4+), as made by `warpsign pair` and idevice_pair
    RemotePairing,
}

/// What a pairing file says about itself
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingInfo {
    pub kind: PairingKind,
    /// The device it pairs with. Other tools may leave it out.
    pub udid: Option<String>,
    /// The host's ID (lockdown) or identifier (RemotePairing)
    pub host_id: String,
    /// Empty for RemotePairing files
    pub wifi_mac_address: String,
}

/// Check a pairing file (`.mobiledevicepairing` or `.plist`) can be used
pub fn read_pairing_file(data: &[u8]) -> Result<PairingInfo, DeviceError> {
    Ok(match parse_pairing(data)? {
        Pairing::Lockdown(pairing) => PairingInfo {
            kind: PairingKind::Lockdown,
            udid: pairing.udid,
            host_id: pairing.host_id,
            wifi_mac_address: pairing.wifi_mac_address,
        },
        Pairing::Remote { file, udid } => PairingInfo {
            kind: PairingKind::RemotePairing,
            udid,
            host_id: file.identifier().to_string(),
            wifi_mac_address: String::new(),
        },
    })
}

enum Pairing {
    Lockdown(Box<PairingFile>),
    Remote {
        file: Box<RpPairingFile>,
        udid: Option<String>,
    },
}

/// The key `warpsign pair` adds to RemotePairing files, which other tools
/// ignore
const RP_UDID_KEY: &str = "UDID";

fn parse_pairing(data: &[u8]) -> Result<Pairing, DeviceError> {
    let invalid = |e: IdeviceError| DeviceError::InvalidPairingFile(e.to_string());
    let dict: Option<plist::Dictionary> = plist::from_bytes(data).ok();
    let remote = dict
        .as_ref()
        .is_some_and(|d| d.contains_key("private_key") && d.contains_key("identifier"));
    if remote {
        let file = RpPairingFile::from_bytes(data).map_err(invalid)?;
        let udid = dict
            .as_ref()
            .and_then(|d| d.get(RP_UDID_KEY))
            .and_then(|v| v.as_string())
            .map(String::from);
        return Ok(Pairing::Remote {
            file: Box::new(file),
            udid,
        });
    }
    PairingFile::from_bytes(data)
        .map(|file| Pairing::Lockdown(Box::new(file)))
        .map_err(invalid)
}

/// How to reach the device
#[derive(Clone)]
pub enum Target {
    /// The device this runs on, through the loopback VPN. RemotePairing uses
    /// `remote_pairing_port`, or [`REMOTE_PAIRING_PORT`].
    Loopback {
        pairing_file: Vec<u8>,
        remote_pairing_port: Option<u16>,
    },
    /// A device on this computer's USB, or the only one if `udid` is None
    Usb { udid: Option<String> },
    /// A device at an address, e.g. in tests or a custom VPN
    Network {
        address: IpAddr,
        pairing_file: Vec<u8>,
        remote_pairing_port: Option<u16>,
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

/// Open a TCP connection, failing fast and clearly when the VPN isn't up
async fn reach(address: SocketAddr) -> Result<tokio::net::TcpStream, DeviceError> {
    match tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::TcpStream::connect(address),
    )
    .await
    {
        Ok(Ok(stream)) => Ok(stream),
        Ok(Err(e)) => Err(DeviceError::Unreachable(format!("{address} ({e})"))),
        Err(_) => Err(DeviceError::Unreachable(format!("{address} (timed out)"))),
    }
}

/// A connection to the device's services, through lockdownd or a
/// RemotePairing tunnel
enum Services {
    Lockdown {
        provider: Box<dyn IdeviceProvider>,
        lockdown: LockdownClient,
        beating: Option<tokio::task::JoinHandle<()>>,
    },
    Tunnel {
        adapter: Box<AdapterHandle>,
        handshake: Box<RsdHandshake>,
        // The pairing connection the tunnel was opened on, kept open with it
        _control: Box<RemotePairingClient<RpPairingSocket<tokio::net::TcpStream>>>,
    },
}

impl Drop for Services {
    fn drop(&mut self) {
        if let Services::Lockdown {
            beating: Some(beating),
            ..
        } = self
        {
            beating.abort();
        }
    }
}

impl Services {
    async fn connect(target: &Target) -> Result<Self, DeviceError> {
        let (address, pairing_file, port) = match target {
            Target::Loopback {
                pairing_file,
                remote_pairing_port,
            } => (
                IpAddr::V4(LOOPBACK_ADDRESS),
                pairing_file,
                *remote_pairing_port,
            ),
            Target::Network {
                address,
                pairing_file,
                remote_pairing_port,
            } => (*address, pairing_file, *remote_pairing_port),
            Target::Usb { udid } => {
                let provider = usb_provider(udid.as_deref()).await?;
                let lockdown = session(&*provider).await?;
                // USB connections don't need a heartbeat
                return Ok(Services::Lockdown {
                    provider,
                    lockdown,
                    beating: None,
                });
            }
        };
        match parse_pairing(pairing_file)? {
            Pairing::Lockdown(pairing_file) => {
                reach(SocketAddr::new(address, LOCKDOWN_PORT)).await?;
                let provider: Box<dyn IdeviceProvider> = Box::new(TcpProvider {
                    addr: address,
                    scope_id: None,
                    pairing_file: *pairing_file,
                    label: "warpsign".into(),
                });
                let lockdown = session(&*provider).await.map_err(|e| match e {
                    // What iOS 26.6+ does to lockdown over the VPN
                    DeviceError::PairingRejected(message)
                        if message.contains("socket io failed") =>
                    {
                        DeviceError::LockdownClosed(message)
                    }
                    e => e,
                })?;
                // Network connections are dropped without a heartbeat
                let beating = HeartbeatClient::connect(&*provider)
                    .await
                    .ok()
                    .map(|client| tokio::spawn(heartbeat(client)));
                Ok(Services::Lockdown {
                    provider,
                    lockdown,
                    beating,
                })
            }
            Pairing::Remote { mut file, .. } => {
                tunnel(address, port.unwrap_or(REMOTE_PAIRING_PORT), &mut file).await
            }
        }
    }

    async fn value(&mut self, key: &str) -> String {
        let value = match self {
            Services::Lockdown { lockdown, .. } => lockdown.get_value(Some(key), None).await,
            Services::Tunnel {
                adapter, handshake, ..
            } => match LockdownClient::connect_rsd(&mut **adapter, handshake).await {
                Ok(mut lockdown) => lockdown.get_value(Some(key), None).await,
                Err(e) => Err(e),
            },
        };
        value
            .ok()
            .and_then(|v| v.as_string().map(String::from))
            .unwrap_or_default()
    }

    async fn describe(&mut self) -> DeviceInfo {
        DeviceInfo {
            udid: self.value("UniqueDeviceID").await,
            name: self.value("DeviceName").await,
            product_type: self.value("ProductType").await,
            ios_version: self.value("ProductVersion").await,
        }
    }

    async fn afc(&mut self) -> Result<AfcClient, IdeviceError> {
        match self {
            Services::Lockdown { provider, .. } => AfcClient::connect(&**provider).await,
            Services::Tunnel {
                adapter, handshake, ..
            } => AfcClient::connect_rsd(&mut **adapter, handshake).await,
        }
    }

    async fn installer(&mut self) -> Result<InstallationProxyClient, IdeviceError> {
        match self {
            Services::Lockdown { provider, .. } => {
                InstallationProxyClient::connect(&**provider).await
            }
            Services::Tunnel {
                adapter, handshake, ..
            } => InstallationProxyClient::connect_rsd(&mut **adapter, handshake).await,
        }
    }
}

/// The USB device with `udid`, or the only one, and usbmuxd
async fn usb_device(
    udid: Option<&str>,
) -> Result<(UsbmuxdConnection, idevice::usbmuxd::UsbmuxdDevice), DeviceError> {
    let mut usbmuxd = UsbmuxdConnection::default()
        .await
        .map_err(|e| DeviceError::Usbmuxd(detail(&e)))?;
    let devices = usbmuxd
        .get_devices()
        .await
        .map_err(|e| DeviceError::Usbmuxd(detail(&e)))?;
    let device = devices
        .into_iter()
        .filter(|d| d.connection_type == Connection::Usb)
        .find(|d| udid.is_none_or(|udid| d.udid == udid))
        .ok_or_else(|| match udid {
            Some(udid) => DeviceError::UsbDeviceNotFound(udid.to_string()),
            None => DeviceError::NoUsbDevice,
        })?;
    Ok((usbmuxd, device))
}

async fn usb_provider(udid: Option<&str>) -> Result<Box<dyn IdeviceProvider>, DeviceError> {
    let (_, device) = usb_device(udid).await?;
    Ok(Box::new(
        device.to_provider(UsbmuxdAddr::default(), "warpsign"),
    ))
}

/// Pair-verify over RemotePairing, then open its encrypted tunnel and find
/// the device's services in it, as SideStore and idevice's tools do
async fn tunnel(
    address: IpAddr,
    port: u16,
    file: &mut RpPairingFile,
) -> Result<Services, DeviceError> {
    let stream = reach(SocketAddr::new(address, port)).await?;
    let mut control = RemotePairingClient::new(RpPairingSocket::new(stream), RP_HOST);
    control
        .attempt_pair_verify()
        .await
        .map_err(failed("starting RemotePairing"))?;
    // Only ever verify: pairing from scratch needs a computer
    control
        .validate_pairing(file)
        .await
        .map_err(|e| DeviceError::PairingRejected(detail(&e)))?;

    let tunnel_port = control
        .create_tcp_listener()
        .await
        .map_err(failed("opening the tunnel"))?;
    let stream = reach(SocketAddr::new(address, tunnel_port)).await?;
    let tunnel = connect_tls_psk_tunnel_native(stream, control.encryption_key())
        .await
        .map_err(failed("opening the tunnel"))?;
    let bad_address = |e: std::net::AddrParseError| DeviceError::Device {
        context: "opening the tunnel",
        source: IdeviceError::UnexpectedResponse(format!("bad tunnel address: {e}")),
    };
    let client_ip: IpAddr = tunnel.info.client_address.parse().map_err(bad_address)?;
    let server_ip: IpAddr = tunnel.info.server_address.parse().map_err(bad_address)?;
    let mtu = tunnel.info.mtu as usize;
    let rsd_port = tunnel.info.server_rsd_port;

    let mut adapter =
        idevice::tcp::adapter::Adapter::new(Box::new(tunnel.into_inner()), client_ip, server_ip);
    adapter.set_mss(mtu.saturating_sub(60));
    let mut adapter = adapter.to_async_handle();
    let rsd = adapter
        .connect(rsd_port)
        .await
        .map_err(|e| failed("finding the device's services")(IdeviceError::Socket(e)))?;
    let handshake = RsdHandshake::new(rsd)
        .await
        .map_err(failed("finding the device's services"))?;
    Ok(Services::Tunnel {
        adapter: Box::new(adapter),
        handshake: Box::new(handshake),
        _control: Box::new(control),
    })
}

/// Start a lockdown session, which proves the pairing works
async fn session(provider: &dyn IdeviceProvider) -> Result<LockdownClient, DeviceError> {
    let mut lockdown = LockdownClient::connect(provider)
        .await
        .map_err(failed("connecting to lockdownd"))?;
    let pairing = provider
        .get_pairing_file()
        .await
        .map_err(|e| DeviceError::PairingRejected(detail(&e)))?;
    lockdown
        .start_session(&pairing)
        .await
        .map_err(|e| match e {
            IdeviceError::PasswordProtected => DeviceError::Locked,
            e => DeviceError::PairingRejected(detail(&e)),
        })?;
    Ok(lockdown)
}

/// Connect and describe the device: checks the VPN and pairing work
pub fn device_info(target: &Target) -> Result<DeviceInfo, DeviceError> {
    runtime()?.block_on(async {
        let mut services = Services::connect(target).await?;
        Ok(services.describe().await)
    })
}

/// An app installed on the device, as installation_proxy describes it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledApp {
    pub bundle_id: String,
    pub name: Option<String>,
    /// CFBundleShortVersionString
    pub version: Option<String>,
    /// CFBundleVersion
    pub build: Option<String>,
}

/// The attributes `installed_apps` asks for, so the device doesn't send
/// every app's whole Info.plist
const INSTALLED_ATTRIBUTES: &[&str] = &[
    "CFBundleIdentifier",
    "CFBundleDisplayName",
    "CFBundleName",
    "CFBundleShortVersionString",
    "CFBundleVersion",
];

/// The apps installed on the device that aren't Apple's: from the App
/// Store, installed over the web, with a pairing file, or by any other
/// installer
pub fn installed_apps(target: &Target) -> Result<Vec<InstalledApp>, DeviceError> {
    runtime()?.block_on(async {
        let mut services = Services::connect(target).await?;
        let mut proxy = services
            .installer()
            .await
            .map_err(failed("connecting to the installer"))?;
        let mut options = plist::Dictionary::new();
        options.insert("ApplicationType".into(), "User".into());
        options.insert(
            "ReturnAttributes".into(),
            plist::Value::Array(INSTALLED_ATTRIBUTES.iter().map(|&key| key.into()).collect()),
        );
        let apps = proxy
            .browse(Some(plist::Value::Dictionary(options)))
            .await
            .map_err(failed("listing the installed apps"))?;
        Ok(apps.iter().filter_map(installed_app).collect())
    })
}

/// One app from installation_proxy's Browse
fn installed_app(value: &plist::Value) -> Option<InstalledApp> {
    let info = value.as_dictionary()?;
    let text = |key: &str| {
        info.get(key)
            .and_then(plist::Value::as_string)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
    };
    Some(InstalledApp {
        bundle_id: text("CFBundleIdentifier")?,
        name: text("CFBundleDisplayName").or_else(|| text("CFBundleName")),
        version: text("CFBundleShortVersionString"),
        build: text("CFBundleVersion"),
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

    // Everything runs inside one block_on: a tunnel's TCP stack is a task
    // on this runtime, and must keep running between steps
    runtime.block_on(async {
        let mut services = run_step_async(observer, Step::ConnectDevice, async |log| {
            let mut services = Services::connect(target).await?;
            let info = services.describe().await;
            log.log(format!("{} (iOS {})", info.name, info.ios_version));
            Ok::<_, DeviceError>(services)
        })
        .await?;

        let remote = run_step_async(observer, Step::UploadApp, async |log| {
            let afc = services
                .afc()
                .await
                .map_err(failed("connecting to the device's file service"))?;
            upload(afc, ipa, log).await
        })
        .await?;

        run_step_async(observer, Step::InstallApp, async |log| {
            let mut proxy = services
                .installer()
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
                .map_err(|e| DeviceError::InstallFailed(explain_install_error(&detail(&e))))?;
            log.log(format!("Installed {bundle_id}"));
            // installd removes it, but don't leave it behind if not
            if let Ok(mut afc) = services.afc().await {
                let _ = afc.remove(remote.as_str()).await;
            }
            Ok::<_, DeviceError>(())
        })
        .await
    })
}

/// Stream the IPA into PublicStaging, returning its path there
async fn upload(
    mut afc: AfcClient,
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

/// Pair with a USB-connected device and return its UDID and a pairing file
/// for WarpSign on the device.
///
/// RemotePairing (iOS 17.4+, needed over the VPN since iOS 26.6) pairs
/// without a prompt, over the trust USB already has: idevice_pair's format,
/// plus the device's UDID, which other tools ignore. Lockdown files are
/// jitterbug's format, as SideStore used, with network connections allowed;
/// the user taps Trust.
pub fn create_pairing_file(
    udid: Option<&str>,
    host_name: &str,
    kind: PairingKind,
) -> Result<(String, Vec<u8>), DeviceError> {
    runtime()?.block_on(async {
        let (mut usbmuxd, device) = usb_device(udid).await?;
        let provider = device.to_provider(UsbmuxdAddr::default(), "warpsign");
        let data = match kind {
            PairingKind::RemotePairing => {
                let service = RemotePairingLockdownService::connect(&provider)
                    .await
                    .map_err(failed("starting RemotePairing (needs iOS 17.4 or later)"))?;
                let mut client = service
                    .into_client(RP_HOST)
                    .map_err(failed("starting RemotePairing"))?;
                let mut file = RpPairingFile::generate(RP_HOST);
                // No prompt over USB, so no PIN: this is never asked for
                client
                    .connect(&mut file, async || "000000".to_string())
                    .await
                    .map_err(failed("pairing"))?;
                let mut dict: plist::Dictionary = plist::from_bytes(&file.to_bytes())
                    .map_err(|e| DeviceError::InvalidPairingFile(e.to_string()))?;
                dict.insert(RP_UDID_KEY.into(), device.udid.clone().into());
                let mut data = Vec::new();
                plist::Value::Dictionary(dict)
                    .to_writer_xml(&mut data)
                    .map_err(|e| DeviceError::InvalidPairingFile(e.to_string()))?;
                data
            }
            PairingKind::Lockdown => {
                let buid = usbmuxd
                    .get_buid()
                    .await
                    .map_err(|e| DeviceError::Usbmuxd(detail(&e)))?;
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
                    .map_err(|e| DeviceError::PairingRejected(detail(&e)))?;
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
                pairing
                    .serialize()
                    .map_err(|e| DeviceError::InvalidPairingFile(e.to_string()))?
            }
        };
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
    fn reads_installed_apps() {
        let mut info = plist::Dictionary::new();
        info.insert("CFBundleIdentifier".into(), "com.example.app".into());
        info.insert("CFBundleName".into(), "Example".into());
        info.insert("CFBundleShortVersionString".into(), "3.9.0".into());
        info.insert("CFBundleVersion".into(), "301".into());
        assert_eq!(
            installed_app(&plist::Value::Dictionary(info.clone())),
            Some(InstalledApp {
                bundle_id: "com.example.app".into(),
                name: Some("Example".into()),
                version: Some("3.9.0".into()),
                build: Some("301".into()),
            })
        );
        // The display name wins over the bundle name
        info.insert("CFBundleDisplayName".into(), "Shown".into());
        assert_eq!(
            installed_app(&plist::Value::Dictionary(info.clone())).and_then(|app| app.name),
            Some("Shown".into())
        );
        // No bundle ID: not an app it can describe
        info.remove("CFBundleIdentifier");
        assert_eq!(installed_app(&plist::Value::Dictionary(info)), None);
        assert_eq!(installed_app(&plist::Value::String("x".into())), None);
    }

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
