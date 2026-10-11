//! On-device installs, as far as they can go without a device.

use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;

use warpsign_core::device::{
    DeviceError, INSTALL_STEPS, Target, device_info, install_ipa, read_pairing_file,
};
use warpsign_core::progress::{NoProgress, Step};

fn pairing_file() -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/test.mobiledevicepairing"),
    )
    .unwrap()
}

#[test]
fn reads_pairing_files() {
    let info = read_pairing_file(&pairing_file()).unwrap();
    assert_eq!(info.udid.as_deref(), Some("00008110-000A0B0C0D0E0F10"));
    assert_eq!(info.host_id, "11111111-2222-4333-8444-555555555555");

    let error = read_pairing_file(b"<plist><dict/></plist>").unwrap_err();
    assert!(
        matches!(error, DeviceError::InvalidPairingFile(_)),
        "{error}"
    );
    assert!(read_pairing_file(b"nonsense").is_err());
}

#[test]
fn explains_an_unreachable_device() {
    // Nothing listens on lockdownd's port here: like the VPN being off
    let target = Target::Network {
        address: IpAddr::V4(Ipv4Addr::LOCALHOST),
        pairing_file: pairing_file(),
    };
    let error = device_info(&target).unwrap_err();
    assert!(matches!(error, DeviceError::Unreachable(_)), "{error}");
    assert!(error.to_string().contains("LocalDevVPN"), "{error}");
}

#[test]
fn a_bad_pairing_file_fails_before_connecting() {
    let target = Target::Loopback {
        pairing_file: b"nonsense".to_vec(),
    };
    assert!(matches!(
        device_info(&target),
        Err(DeviceError::InvalidPairingFile(_))
    ));
    assert!(
        format!("{target:?}") == "Loopback",
        "pairing files stay out of logs"
    );
}

#[test]
fn installs_need_a_signed_ipa() {
    let target = Target::Loopback {
        pairing_file: pairing_file(),
    };
    let error = install_ipa(&target, &PathBuf::from("missing.ipa"), &NoProgress).unwrap_err();
    assert!(matches!(error, DeviceError::Package(_)), "{error}");
}

#[test]
fn install_steps() {
    assert_eq!(
        INSTALL_STEPS,
        [Step::ConnectDevice, Step::UploadApp, Step::InstallApp]
    );
}
