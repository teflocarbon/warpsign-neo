//! Making a RemotePairing file on the device itself, without a computer
//! (iOS 27 and later).
//!
//! iOS 27 lets a device start pairing with a computer: Settings → Privacy &
//! Security → Developer Mode lists the computers that advertise
//! `_remotepairing-pairable-host._tcp` on the local network, and "Pair with
//! …" connects to the one tapped. The computer shows a six-digit code, the
//! user types it into Settings, and the two run RemotePairing's pair-setup
//! (SRP, then Ed25519 identities) with the roles swapped: the device drives
//! it, the computer answers.
//!
//! An app can be that computer. WarpSign listens here, advertises itself
//! (from Swift, through mDNSResponder: an app can't send multicast itself
//! without a restricted entitlement), and the device it runs on connects to
//! it over Wi-Fi, from itself to itself. What comes out is the same kind of
//! RemotePairing file `warpsign pair` makes over USB, and it's used the same
//! way: pair-verify through the loopback VPN, then the tunnel.
//!
//! idevice's [`PairableHost`] does the protocol; StikDebug showed the way.
//! See docs/design/on-device-pairing.md.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use idevice::remote_pairing::{
    PairableHost as Responder, PairableHostInfo, RpPairingFile, RpPairingSocket,
};
use tokio::sync::Notify;

use crate::device::{DeviceError, failed, runtime};

/// The DNS-SD type to advertise, in the form `NetService` takes (idevice's
/// `PAIRABLE_HOST_SERVICE_TYPE` has the `local.` domain on the end)
pub const SERVICE_TYPE: &str = "_remotepairing-pairable-host._tcp.";

/// How WarpSign shows itself to the device: as a Mac, because iOS only
/// lists computers. iOS shows the name; the model picks the icon.
const MODEL: &str = "Mac17,7";

/// The device that paired, and the pairing file for it
#[derive(Debug, Clone)]
pub struct PairedDevice {
    /// A RemotePairing file, as [`crate::device::read_pairing_file`] reads
    pub pairing_file: Vec<u8>,
    /// What the device calls itself, e.g. "iPhone"
    pub name: String,
    /// e.g. "iPhone17,1"
    pub model: String,
}

/// A pairable host, listening for this device. Advertise
/// [`Self::service_name`] as [`SERVICE_TYPE`] on [`Self::port`] with
/// [`Self::txt_records`], then wait in [`Self::accept`].
pub struct PairableHost {
    listener: Mutex<Option<std::net::TcpListener>>,
    service_name: String,
    txt_records: Vec<(String, String)>,
    port: u16,
    /// Taken by the one `accept`
    pairing: Mutex<Option<(RpPairingFile, PairableHostInfo)>>,
    cancelled: AtomicBool,
    cancel: Notify,
}

impl PairableHost {
    /// Listen on every IPv4 interface, on a port the system picks. `name` is
    /// what Settings shows ("Pair with <name>").
    pub fn new(name: &str) -> Result<Self, DeviceError> {
        let io = |source| DeviceError::Io {
            path: "pairing listener".into(),
            source,
        };
        // IPv4 only, as StikDebug: the device reaches itself at its Wi-Fi
        // address, and comparing that with ours is simplest in IPv4
        let listener = std::net::TcpListener::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)))
            .map_err(io)?;
        listener.set_nonblocking(true).map_err(io)?;
        let port = listener.local_addr().map_err(io)?.port();

        let file = RpPairingFile::generate(name);
        let info = PairableHostInfo::generate(name, MODEL);
        // The instance name is the host's identifier, which the device is
        // also told during pairing
        let service_name = file.identifier().to_string();
        let txt_records = info.mdns_txt_records(&service_name);
        Ok(Self {
            listener: Mutex::new(Some(listener)),
            service_name,
            txt_records,
            port,
            pairing: Mutex::new(Some((file, info))),
            cancelled: AtomicBool::new(false),
            cancel: Notify::new(),
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// The DNS-SD instance name to advertise
    pub fn service_name(&self) -> &str {
        &self.service_name
    }

    /// The TXT record to advertise, as (key, value) pairs
    pub fn txt_records(&self) -> &[(String, String)] {
        &self.txt_records
    }

    /// Wait for this device to connect and pair. `show_pin` is called once
    /// with the code to type into Settings. Connections from other devices
    /// on the network are closed unanswered: this only pairs the device it
    /// runs on. Blocks until paired, failed or [`Self::cancel`]led; it can
    /// only be called once.
    pub fn accept(&self, show_pin: impl FnOnce(String)) -> Result<PairedDevice, DeviceError> {
        let listener = self.listener.lock().unwrap().take();
        let pairing = self.pairing.lock().unwrap().take();
        let (Some(listener), Some((mut file, info))) = (listener, pairing) else {
            return Err(DeviceError::PairingCancelled);
        };
        runtime()?.block_on(async {
            let listener =
                tokio::net::TcpListener::from_std(listener).map_err(|source| DeviceError::Io {
                    path: "pairing listener".into(),
                    source,
                })?;
            let pair = async {
                let stream = accept_local(&listener).await?;
                let mut responder = Responder::new(RpPairingSocket::new_device(stream), info);
                let peer = responder
                    .accept(&mut file, |pin| async move { show_pin(pin) })
                    .await
                    .map_err(failed("pairing"))?;
                Ok(PairedDevice {
                    pairing_file: file.to_bytes(),
                    name: peer.name,
                    model: peer.model,
                })
            };
            tokio::select! {
                biased;
                _ = self.cancelled() => Err(DeviceError::PairingCancelled),
                result = pair => result,
            }
        })
    }

    /// Stop waiting: [`Self::accept`] returns
    /// [`DeviceError::PairingCancelled`]. Safe from any thread, before or
    /// during it.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.cancel.notify_one();
    }

    async fn cancelled(&self) {
        let notified = self.cancel.notified();
        if self.cancelled.load(Ordering::SeqCst) {
            return;
        }
        notified.await;
    }
}

/// The first connection from this device itself: one whose address is ours
async fn accept_local(
    listener: &tokio::net::TcpListener,
) -> Result<tokio::net::TcpStream, DeviceError> {
    loop {
        let (stream, peer) = listener.accept().await.map_err(|source| DeviceError::Io {
            path: "pairing listener".into(),
            source,
        })?;
        let local = stream.local_addr().map(|a| a.ip()).ok();
        if local == Some(peer.ip()) {
            return Ok(stream);
        }
        // Another device on the network: never pair it from here
        drop(stream);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn advertises_itself() {
        let host = PairableHost::new("WarpSign").unwrap();
        assert_ne!(host.port(), 0);
        assert_eq!(
            idevice::remote_pairing::PAIRABLE_HOST_SERVICE_TYPE,
            format!("{SERVICE_TYPE}local.")
        );
        let txt: std::collections::HashMap<_, _> = host.txt_records().iter().cloned().collect();
        assert_eq!(txt["name"], "WarpSign");
        assert_eq!(txt["model"], MODEL);
        assert_eq!(txt["identifier"], host.service_name());
        assert!(!txt["authTag"].is_empty());
    }

    #[test]
    fn cancels_before_and_during_accept() {
        let host = PairableHost::new("WarpSign").unwrap();
        host.cancel();
        assert!(matches!(
            host.accept(|_| {}),
            Err(DeviceError::PairingCancelled)
        ));
        // Only once
        assert!(matches!(
            host.accept(|_| {}),
            Err(DeviceError::PairingCancelled)
        ));

        let host = Arc::new(PairableHost::new("WarpSign").unwrap());
        let waiting = {
            let host = host.clone();
            std::thread::spawn(move || host.accept(|_| {}))
        };
        std::thread::sleep(Duration::from_millis(100));
        host.cancel();
        assert!(matches!(
            waiting.join().unwrap(),
            Err(DeviceError::PairingCancelled)
        ));
    }

    #[test]
    fn answers_a_connection_from_itself() {
        // A connection from this machine is "this device": it gets the
        // handshake, which fails on garbage rather than being dropped
        let host = Arc::new(PairableHost::new("WarpSign").unwrap());
        let port = host.port();
        let waiting = {
            let host = host.clone();
            std::thread::spawn(move || host.accept(|_| {}))
        };
        let mut stream = std::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        std::io::Write::write_all(&mut stream, b"not RemotePairing\n").unwrap();
        drop(stream);
        let result = waiting.join().unwrap();
        assert!(
            matches!(
                result,
                Err(DeviceError::Device {
                    context: "pairing",
                    ..
                })
            ),
            "{:?}",
            result.err().map(|e| e.to_string())
        );
    }
}
