//! Signing identities: a certificate plus its private key.
//!
//! warpsign-neo creates its own (see docs/design/certificates.md): it
//! generates a key, sends a certificate signing request (CSR) through the
//! Developer Portal, and pairs the returned certificate with the key. Both
//! identities are backed up as one `.p12` the user keeps.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use apple_codesign::AppleCertificate;
use apple_codesign::cryptography::InMemoryPrivateKey;
use p12_keystore::{
    Certificate, EncryptionAlgorithm, KeyStore, KeyStoreEntry, MacAlgorithm, Pkcs12ImportPolicy,
    PrivateKey, PrivateKeyChain,
};
use rsa::pkcs8::EncodePrivateKey;
use sha2::{Digest, Sha256};
use x509_certificate::{CapturedX509Certificate, Sign, X509CertificateBuilder};

use crate::signing::SigningIdentity;

#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    #[error("generating a private key failed: {0}")]
    KeyGeneration(String),
    #[error("invalid private key: {0}")]
    Key(String),
    #[error("invalid certificate: {0}")]
    Certificate(String),
    #[error("the private key doesn't belong to the certificate")]
    KeyMismatch,
    #[error("creating the certificate signing request failed: {0}")]
    Csr(String),
    #[error("wrong password, or not a .p12 file ({0})")]
    P12Read(String),
    #[error("writing the .p12 failed: {0}")]
    P12Write(String),
    #[error("the .p12 has no certificate with its private key")]
    NoIdentities,
}

/// Which kind of signing certificate
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CertificateKind {
    Development,
    Distribution,
}

/// Generate a new RSA 2048-bit private key (PKCS#8 DER), the kind Apple's
/// portal issues signing certificates for
pub fn generate_private_key() -> Result<Vec<u8>, IdentityError> {
    let key = rsa::RsaPrivateKey::new(&mut rand_core::OsRng, 2048)
        .map_err(|e| IdentityError::KeyGeneration(e.to_string()))?;
    let der = key
        .to_pkcs8_der()
        .map_err(|e| IdentityError::KeyGeneration(e.to_string()))?;
    Ok(der.as_bytes().to_vec())
}

/// A PEM certificate signing request for `private_key` (PKCS#8 DER), to
/// submit to the portal. Apple ignores the subject and issues its own.
pub fn certificate_signing_request(
    private_key: &[u8],
    common_name: &str,
) -> Result<String, IdentityError> {
    let key = InMemoryPrivateKey::from_pkcs8_der(private_key)
        .map_err(|e| IdentityError::Key(e.to_string()))?;
    let mut builder = X509CertificateBuilder::default();
    builder
        .subject()
        .append_common_name_utf8_string(common_name)
        .map_err(|e| IdentityError::Csr(format!("{e:?}")))?;
    builder
        .create_certificate_signing_request(&key)
        .map_err(|e| IdentityError::Csr(e.to_string()))?
        .encode_pem()
        .map_err(|e| IdentityError::Csr(e.to_string()))
}

/// A certificate and its private key
#[derive(Clone)]
pub struct Identity {
    certificate: CapturedX509Certificate,
    private_key: Vec<u8>,
}

impl std::fmt::Debug for Identity {
    // Never print the private key
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity")
            .field("common_name", &self.common_name())
            .field("serial_number", &self.serial_number())
            .finish_non_exhaustive()
    }
}

impl Identity {
    /// Pair a certificate (DER) with its private key (PKCS#8 DER), checking
    /// that they belong together
    pub fn new(certificate: &[u8], private_key: &[u8]) -> Result<Self, IdentityError> {
        let certificate = CapturedX509Certificate::from_der(certificate.to_vec())
            .map_err(|e| IdentityError::Certificate(e.to_string()))?;
        let key = InMemoryPrivateKey::from_pkcs8_der(private_key)
            .map_err(|e| IdentityError::Key(e.to_string()))?;
        if key.public_key_data().as_ref() != certificate.public_key_data().as_ref() {
            return Err(IdentityError::KeyMismatch);
        }
        Ok(Self {
            certificate,
            private_key: private_key.to_vec(),
        })
    }

    pub fn certificate_der(&self) -> &[u8] {
        self.certificate.constructed_data()
    }

    /// PKCS#8 DER. Keep it secret.
    pub fn private_key_der(&self) -> &[u8] {
        &self.private_key
    }

    /// e.g. "Apple Development: Jane Appleseed (ABCDE12345)"
    pub fn common_name(&self) -> Option<String> {
        self.certificate.subject_common_name()
    }

    pub fn kind(&self) -> Option<CertificateKind> {
        let name = self.common_name()?;
        if name.starts_with("Apple Development") || name.starts_with("iPhone Developer") {
            Some(CertificateKind::Development)
        } else if name.starts_with("Apple Distribution") || name.starts_with("iPhone Distribution")
        {
            Some(CertificateKind::Distribution)
        } else {
            None
        }
    }

    /// Team ID from the certificate's organizational unit
    pub fn team_id(&self) -> Option<String> {
        self.certificate.apple_team_id()
    }

    /// Serial number as uppercase hex without leading zeros, as the portal
    /// lists it
    pub fn serial_number(&self) -> String {
        let hex: String = self
            .certificate
            .serial_number_asn1()
            .as_slice()
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect();
        let trimmed = hex.trim_start_matches('0');
        if trimmed.is_empty() {
            "0".into()
        } else {
            trimmed.into()
        }
    }

    /// Whether `serial` (as given by the portal) is this certificate's
    pub fn has_serial(&self, serial: &str) -> bool {
        let normalised = serial.trim_start_matches('0').to_ascii_uppercase();
        normalised == self.serial_number()
    }

    pub fn expires(&self) -> SystemTime {
        let seconds = self.certificate.validity_not_after().timestamp();
        UNIX_EPOCH + Duration::from_secs(seconds.max(0) as u64)
    }

    /// For signing apps with this identity
    pub fn signing_identity(&self) -> Result<SigningIdentity, IdentityError> {
        SigningIdentity::from_der(self.certificate_der(), &self.private_key)
            .map_err(|e| IdentityError::Key(e.to_string()))
    }
}

/// Write identities into one password-protected `.p12`.
///
/// Uses legacy 3DES encryption with an HMAC-SHA1 MAC, which Keychain Access,
/// Python warpsign and apple-codesign can all read.
pub fn export_p12(identities: &[Identity], password: &str) -> Result<Vec<u8>, IdentityError> {
    let mut store = KeyStore::new();
    for (index, identity) in identities.iter().enumerate() {
        let key = PrivateKey::from_der(identity.private_key_der())
            .map_err(|e| IdentityError::P12Write(e.to_string()))?;
        let certificate = Certificate::from_der(identity.certificate_der())
            .map_err(|e| IdentityError::P12Write(e.to_string()))?;
        let local_key_id = Sha256::digest(identity.certificate_der())[..20].to_vec();
        let alias = identity
            .common_name()
            .unwrap_or_else(|| format!("WarpSign identity {}", index + 1));
        store.add_entry(
            &alias,
            KeyStoreEntry::PrivateKeyChain(PrivateKeyChain::new(local_key_id, key, [certificate])),
        );
    }
    store
        .writer(password)
        .encryption_algorithm(EncryptionAlgorithm::PbeWithShaAnd3KeyTripleDesCbc)
        .mac_algorithm(MacAlgorithm::HmacSha1)
        .write()
        .map_err(|e| IdentityError::P12Write(e.to_string()))
}

/// Read every identity (certificate with its private key) from a `.p12`,
/// whether it uses legacy (3DES) or modern (AES) encryption
pub fn import_p12(data: &[u8], password: &str) -> Result<Vec<Identity>, IdentityError> {
    let store = KeyStore::from_pkcs12(data, password, Pkcs12ImportPolicy::Relaxed)
        .map_err(|e| IdentityError::P12Read(e.to_string()))?;
    let mut identities = Vec::new();
    for (_, entry) in store.entries() {
        if let KeyStoreEntry::PrivateKeyChain(chain) = entry
            && let Some(certificate) = chain.certs().first()
        {
            identities.push(Identity::new(certificate.as_der(), chain.key().as_der())?);
        }
    }
    if identities.is_empty() {
        return Err(IdentityError::NoIdentities);
    }
    Ok(identities)
}

/// A random passphrase for the backup `.p12`: 6 groups of 4 characters
/// from an alphabet without look-alikes, about 120 bits
pub fn generate_passphrase() -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut random = [0u8; 24];
    getrandom::fill(&mut random).expect("system random number generator");
    random
        .chunks(4)
        .map(|group| {
            group
                .iter()
                .map(|b| ALPHABET[(*b % 32) as usize] as char)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("-")
}
