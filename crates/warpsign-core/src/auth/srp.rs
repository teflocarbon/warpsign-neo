//! Apple's SRP-6a sign-in (the `signin/init` and `signin/complete` steps).
//!
//! Matches the `srp` Python library as configured by warpsign
//! (`rfc5054_enable()`, `no_username_in_x()`, SHA-256, 2048-bit group), with
//! Apple's password derivation. Pinned by `fixtures/srp.json`.
//!
//! The details that matter: `k` and `u` hash their inputs padded to the
//! group size, `M` and `H_AMK` use them unpadded, and `x` leaves the username
//! out while `M` includes it.

use num_bigint::BigUint;
use sha2::{Digest, Sha256};

/// RFC 5054 2048-bit group prime (as used by Apple sign-in), from the srp library
const N_HEX: &str = concat!(
    "AC6BDB41324A9A9BF166DE5E1389582FAF72B6651987EE07FC3192943DB56050",
    "A37329CBB4A099ED8193E0757767A13DD52312AB4B03310DCD7F48A9DA04FD50",
    "E8083969EDB767B0CF6095179A163AB3661A05FBD5FAAAE82918A9962F0B93B8",
    "55F97993EC975EEAA80D740ADBF4FF747359D041D5C33EA71D281E446B14773B",
    "CA97B43A23FB801676BD207A436C6481F1D2B9078717461A5B9D32E688F87748",
    "544523B524B0D57D5EA77A2775D2ECFA032CFBDBF52FB3786160279004E57AE6",
    "AF874E7303CE53299CCC041C7BC308D82A5698F3A8D0C38271AE35F8E9DBFBB6",
    "94B5C803D89F7AE435DE236D525F54759B65E372FCD68EF20FA7111F9E4AFF73",
);

const G: u32 = 2;
/// Size of the group in bytes; values are padded to this where the RFC says
const WIDTH: usize = 256;

/// Password derivation protocol named by Apple in the `signin/init` response
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// PBKDF2 over the raw SHA-256 of the password
    S2k,
    /// PBKDF2 over the hex SHA-256 of the password
    S2kFo,
}

impl Protocol {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "s2k" => Some(Self::S2k),
            "s2k_fo" => Some(Self::S2kFo),
            _ => None,
        }
    }
}

/// The SRP "password": PBKDF2-SHA256 (32 bytes) of the hashed real password
pub fn derive_password(
    password: &str,
    salt: &[u8],
    iterations: u32,
    protocol: Protocol,
) -> Vec<u8> {
    let hash = Sha256::digest(password.as_bytes());
    let input = match protocol {
        Protocol::S2k => hash.to_vec(),
        Protocol::S2kFo => hex(&hash).into_bytes(),
    };
    let mut derived = vec![0u8; 32];
    pbkdf2::pbkdf2_hmac::<Sha256>(&input, salt, iterations, &mut derived);
    derived
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn n() -> BigUint {
    BigUint::parse_bytes(N_HEX.as_bytes(), 16).expect("valid group prime")
}

/// Big-endian bytes without leading zeros (Python srp's `long_to_bytes`)
fn to_bytes(n: &BigUint) -> Vec<u8> {
    if *n == BigUint::ZERO {
        Vec::new()
    } else {
        n.to_bytes_be()
    }
}

fn padded(n: &BigUint) -> Vec<u8> {
    let bytes = to_bytes(n);
    let mut out = vec![0u8; WIDTH.saturating_sub(bytes.len())];
    out.extend_from_slice(&bytes);
    out
}

fn sha256(parts: &[&[u8]]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part);
    }
    hasher.finalize().to_vec()
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SrpError {
    #[error("the server's SRP challenge is invalid")]
    InvalidChallenge,
    #[error("the server's SRP proof doesn't match: it may not be Apple")]
    ServerProofMismatch,
}

/// Client side of one SRP sign-in
pub struct SrpClient {
    username: String,
    a: BigUint,
    big_a: BigUint,
}

/// What `signin/complete` needs, plus what to check the server against
pub struct SrpProof {
    /// `m1`
    pub client_proof: Vec<u8>,
    /// `m2`: Apple's response should match this (`H_AMK`)
    pub expected_server_proof: Vec<u8>,
}

impl SrpClient {
    /// Start a sign-in with a random secret
    pub fn new(username: &str) -> Self {
        let mut secret = [0u8; 32];
        getrandom::fill(&mut secret).expect("system random number generator");
        Self::with_secret(username, &secret)
    }

    /// Start a sign-in with a given secret `a` (for tests)
    pub fn with_secret(username: &str, secret: &[u8]) -> Self {
        let a = BigUint::from_bytes_be(secret);
        let big_a = BigUint::from(G).modpow(&a, &n());
        Self {
            username: username.to_string(),
            a,
            big_a,
        }
    }

    /// `A`, sent (base64) as `a` in `signin/init`
    pub fn public_key(&self) -> Vec<u8> {
        to_bytes(&self.big_a)
    }

    /// Compute the proof from the server's salt and `B`, given the derived
    /// password (see [`derive_password`])
    pub fn process_challenge(
        &self,
        salt: &[u8],
        server_public_key: &[u8],
        derived_password: &[u8],
    ) -> Result<SrpProof, SrpError> {
        let n = n();
        let g = BigUint::from(G);
        let big_b = BigUint::from_bytes_be(server_public_key);
        if &big_b % &n == BigUint::ZERO {
            return Err(SrpError::InvalidChallenge);
        }

        let k = BigUint::from_bytes_be(&sha256(&[&padded(&n), &padded(&g)]));
        let u = BigUint::from_bytes_be(&sha256(&[&padded(&self.big_a), &padded(&big_b)]));
        if u == BigUint::ZERO {
            return Err(SrpError::InvalidChallenge);
        }
        // No username in x (no_username_in_x)
        let inner = sha256(&[b":", derived_password]);
        let x = BigUint::from_bytes_be(&sha256(&[salt, &inner]));
        let v = g.modpow(&x, &n);

        // S = (B - k*v) ^ (a + u*x) mod N, keeping the base non-negative
        let kv = (&k * &v) % &n;
        let base = ((&big_b % &n) + &n - kv) % &n;
        let s = base.modpow(&(&self.a + &u * &x), &n);
        let key = sha256(&[&to_bytes(&s)]);

        // H(N) xor H(padded g)
        let h_n = sha256(&[&to_bytes(&n)]);
        let h_g = sha256(&[&padded(&g)]);
        let n_xor_g: Vec<u8> = h_n.iter().zip(&h_g).map(|(a, b)| a ^ b).collect();
        let h_user = sha256(&[self.username.as_bytes()]);

        let a_bytes = to_bytes(&self.big_a);
        let m1 = sha256(&[&n_xor_g, &h_user, salt, &a_bytes, &to_bytes(&big_b), &key]);
        let m2 = sha256(&[&a_bytes, &m1, &key]);
        Ok(SrpProof {
            client_proof: m1,
            expected_server_proof: m2,
        })
    }
}

impl SrpProof {
    /// Check Apple's `m2`
    pub fn verify_server(&self, server_proof: &[u8]) -> Result<(), SrpError> {
        if server_proof == self.expected_server_proof.as_slice() {
            Ok(())
        } else {
            Err(SrpError::ServerProofMismatch)
        }
    }
}
