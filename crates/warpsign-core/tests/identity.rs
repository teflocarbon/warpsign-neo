//! Signing identities: key generation, CSRs and `.p12` backups.

use std::path::PathBuf;

use bcder::encode::Values;
use x509_certificate::CapturedX509Certificate;
use x509_certificate::rfc2986::CertificationRequest;

use warpsign_core::identity::{
    CertificateKind, Identity, IdentityError, certificate_signing_request, export_p12,
    generate_passphrase, generate_private_key, import_p12,
};

fn data(name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data")
            .join(name),
    )
    .unwrap()
}

/// The legacy (3DES) development identity
fn development() -> Identity {
    let mut identities = import_p12(&data("test-cert.p12"), "secret").unwrap();
    assert_eq!(identities.len(), 1);
    identities.remove(0)
}

/// The modern (AES) distribution identity
fn distribution() -> Identity {
    let mut identities = import_p12(&data("test-distribution-aes.p12"), "secret").unwrap();
    assert_eq!(identities.len(), 1);
    identities.remove(0)
}

#[test]
fn imports_legacy_p12() {
    let identity = development();
    assert_eq!(
        identity.common_name().as_deref(),
        Some("Apple Development: Test User (ABCDE12345)")
    );
    assert_eq!(identity.kind(), Some(CertificateKind::Development));
    assert_eq!(identity.team_id().as_deref(), Some("ZZZZZ99999"));
}

#[test]
fn imports_aes_p12() {
    let identity = distribution();
    assert_eq!(identity.kind(), Some(CertificateKind::Distribution));
    assert_eq!(identity.team_id().as_deref(), Some("ZZZZZ99999"));
    // apple-codesign alone can't read these: its parse_pfx_data panics on
    // the SHA-256 MAC, so it's not called here
}

#[test]
fn wrong_password_is_an_error() {
    let error = import_p12(&data("test-cert.p12"), "wrong").unwrap_err();
    assert!(matches!(error, IdentityError::P12Read(_)), "{error}");
    assert!(import_p12(b"not a p12", "secret").is_err());
}

#[test]
fn exports_both_identities_in_one_legacy_p12() {
    let (dev, dist) = (development(), distribution());
    let passphrase = generate_passphrase();
    let p12 = export_p12(&[dev.clone(), dist.clone()], &passphrase).unwrap();

    let mut restored = import_p12(&p12, &passphrase).unwrap();
    restored.sort_by_key(|identity| identity.common_name());
    assert_eq!(restored.len(), 2);
    for (original, restored) in [&dev, &dist].into_iter().zip(&restored) {
        assert_eq!(original.certificate_der(), restored.certificate_der());
        assert_eq!(original.private_key_der(), restored.private_key_der());
    }

    // Legacy encryption: readable by apple-codesign (and so by Keychain
    // Access and Python warpsign), which rejects AES
    let (certificate, _) = apple_codesign::cryptography::parse_pfx_data(&p12, &passphrase).unwrap();
    let readable = certificate.constructed_data();
    assert!(readable == dev.certificate_der() || readable == dist.certificate_der());

    assert!(import_p12(&p12, "wrong").is_err());
}

#[test]
fn restored_identity_can_sign() {
    let p12 = export_p12(&[distribution()], "secret").unwrap();
    let identity = import_p12(&p12, "secret").unwrap().remove(0);
    let signer = identity.signing_identity().unwrap();
    assert_eq!(signer.team_id().as_deref(), Some("ZZZZZ99999"));
    // ...and SigningIdentity reads AES files directly too
    let signer = warpsign_core::signing::SigningIdentity::from_p12(
        &data("test-distribution-aes.p12"),
        "secret",
    )
    .unwrap();
    assert_eq!(signer.team_id().as_deref(), Some("ZZZZZ99999"));
}

#[test]
fn rejects_mismatched_key() {
    let (dev, dist) = (development(), distribution());
    let error = Identity::new(dev.certificate_der(), dist.private_key_der()).unwrap_err();
    assert!(matches!(error, IdentityError::KeyMismatch), "{error}");
}

#[test]
fn serial_numbers_compare_like_the_portal() {
    let identity = development();
    let serial = identity.serial_number();
    assert!(!serial.starts_with('0'));
    assert_eq!(serial, serial.to_ascii_uppercase());
    assert!(identity.has_serial(&serial));
    assert!(identity.has_serial(&serial.to_ascii_lowercase()));
    assert!(identity.has_serial(&format!("00{serial}")));
    assert!(!identity.has_serial("1234"));
}

#[test]
fn certificate_signing_request_is_signed_by_the_key() {
    // Use the test identity's key, so its certificate can check the signature
    let identity = development();
    let pem = certificate_signing_request(identity.private_key_der(), "WarpSign").unwrap();
    let der = pem::parse(&pem).unwrap();
    assert_eq!(der.tag(), "CERTIFICATE REQUEST");
    let request = bcder::decode::Constructed::decode(der.contents(), bcder::Mode::Der, |cons| {
        CertificationRequest::take_from(cons)
    })
    .unwrap();

    let info = &request.certificate_request_info;
    let public_key = info
        .subject_public_key_info
        .subject_public_key
        .octet_bytes();
    let certificate = CapturedX509Certificate::from_der(identity.certificate_der()).unwrap();
    assert_eq!(public_key, certificate.public_key_data());
    assert_eq!(
        info.subject
            .iter_common_name()
            .next()
            .map(|cn| cn.to_string().unwrap()),
        Some("WarpSign".to_string())
    );

    let mut signed = Vec::new();
    info.encode_ref()
        .write_encoded(bcder::Mode::Der, &mut signed)
        .unwrap();
    certificate
        .verify_signed_data(&signed, request.signature.octet_bytes())
        .unwrap();
}

#[test]
fn generated_key_makes_a_valid_request() {
    let key = generate_private_key().unwrap();
    let pem = certificate_signing_request(&key, "WarpSign").unwrap();
    assert!(pem.starts_with("-----BEGIN CERTIFICATE REQUEST-----"));
    assert_ne!(key, generate_private_key().unwrap());
}

#[test]
fn passphrases_are_long_and_unambiguous() {
    let passphrase = generate_passphrase();
    assert_eq!(passphrase.len(), 29);
    let groups: Vec<_> = passphrase.split('-').collect();
    assert_eq!(groups.len(), 6);
    assert!(groups.iter().all(|group| group.len() == 4));
    assert!(!passphrase.contains(['0', 'O', '1', 'I']));
    assert_ne!(passphrase, generate_passphrase());
}
