//! Anisette: the device headers Apple's GrandSlam sign-in (`gsa.apple.com`)
//! and Xcode's developer services expect on every request
//! (`X-Apple-I-MD`, `X-Apple-I-MD-M` and friends).
//!
//! They come from Apple's ADI. Here it runs in-process: Apple's ADI
//! libraries for Android (from Apple Music, fetched from Apple by
//! [`apk::download`]) are loaded into an AArch64 emulator (Unicorn, loaded
//! at run time) with a small emulated libc. No JIT is needed with Unicorn's
//! interpreter build, so this works on a stock iPhone as well as on a
//! computer. See docs/spikes/anisette-on-device.md.
//!
//! The identity this creates is a machine Apple trusts once provisioned:
//! keep [`AnisetteState`] secret, and never log the headers.

pub mod adi;
pub mod apk;
pub mod elf;
pub mod machine;
pub mod provisioning;
pub mod unicorn;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};

use crate::portal::http::Transport;
use adi::{Adi, AdiError, PROVISIONING_PATH};
use apk::Libraries;
use machine::Files;
use unicorn::Unicorn;

/// How the device describes itself. GrandSlam has refused `com.apple.dt.Xcode`
/// as the client since September 2026 and `macOS;` as the system since about
/// 20 November 2026 (SideStore#1745), so this follows what AltStore and
/// AnisetteKit send now.
pub const DEFAULT_CLIENT_INFO: &str =
    "<MacBookPro18,3> <Mac OS X;26.6;25F84> <com.apple.AuthKit/1 (com.apple.akd/1.0)>";

/// What the server says when it doesn't give one at provisioning
const DEFAULT_ROUTING_INFO: &str = "17106176";

#[derive(Debug, thiserror::Error)]
pub enum AnisetteError {
    #[error(transparent)]
    Adi(#[from] AdiError),
    #[error("couldn't reach Apple: {0}")]
    Http(String),
    #[error("{0}")]
    Apple(String),
    #[error("{0}")]
    Apk(String),
    #[error("saved anisette state is unusable: {0}")]
    State(String),
}

/// The device identity, and ADI's saved files once provisioned. Secret:
/// whoever has it can make valid headers for this identity.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnisetteState {
    /// ADI's identifier: 8 random bytes, as lowercase hex
    pub identifier: String,
    /// `X-Mme-Device-Id`: a random UUID, uppercase
    pub device_id: String,
    /// `X-Apple-I-MD-LU`: 32 random bytes, as uppercase hex
    pub local_user: String,
    /// `X-MMe-Client-Info`
    pub client_info: String,
    /// `X-Apple-I-MD-RINFO`, from provisioning
    pub routing_info: String,
    /// ADI's files under its provisioning path, base64
    pub files: BTreeMap<String, String>,
}

impl std::fmt::Debug for AnisetteState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnisetteState")
            .field("files", &self.files.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

fn random_hex(bytes: usize, upper: bool) -> String {
    let mut data = vec![0u8; bytes];
    getrandom::fill(&mut data).expect("the system has randomness");
    data.iter()
        .map(|b| {
            if upper {
                format!("{b:02X}")
            } else {
                format!("{b:02x}")
            }
        })
        .collect()
}

impl AnisetteState {
    /// A new, unprovisioned identity
    pub fn generate() -> Self {
        let uuid = random_hex(16, true);
        let device_id = format!(
            "{}-{}-{}-{}-{}",
            &uuid[0..8],
            &uuid[8..12],
            &uuid[12..16],
            &uuid[16..20],
            &uuid[20..32]
        );
        Self {
            identifier: random_hex(8, false),
            device_id,
            local_user: random_hex(32, true),
            client_info: DEFAULT_CLIENT_INFO.to_string(),
            routing_info: DEFAULT_ROUTING_INFO.to_string(),
            files: BTreeMap::new(),
        }
    }

    fn to_files(&self) -> Result<Files, AnisetteError> {
        let mut files = Files::default();
        files.create_directories(PROVISIONING_PATH);
        for (name, data) in &self.files {
            if name.contains('/') {
                return Err(AnisetteError::State(format!("bad file name {name}")));
            }
            let data = STANDARD
                .decode(data)
                .map_err(|_| AnisetteError::State(format!("{name} isn't base64")))?;
            files
                .files
                .insert(format!("{PROVISIONING_PATH}/{name}"), data);
        }
        Ok(files)
    }

    fn take_files(&mut self, files: &Files) {
        let prefix = format!("{PROVISIONING_PATH}/");
        self.files = files
            .files
            .iter()
            .filter_map(|(path, data)| {
                let name = path.strip_prefix(&prefix)?;
                (!name.contains('/')).then(|| (name.to_string(), STANDARD.encode(data)))
            })
            .collect();
    }
}

/// `X-Apple-I-Client-Time`: now, UTC, to the second
pub fn client_time() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (year, month, day) = crate::auth::hashcash::civil_from_days(now.div_euclid(86_400));
    let seconds = now.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        seconds / 3600,
        seconds % 3600 / 60,
        seconds % 60
    )
}

/// Anisette made on this machine
pub struct Anisette {
    adi: Adi,
    state: AnisetteState,
}

impl Anisette {
    /// Load ADI for `state` (from [`AnisetteState::generate`] the first time)
    pub fn new(
        unicorn: Arc<Unicorn>,
        libraries: &Libraries,
        state: AnisetteState,
    ) -> Result<Self, AnisetteError> {
        let adi = Adi::new(
            unicorn,
            &libraries.store_services_core,
            &libraries.core_adi,
            &state.identifier,
            state.to_files()?,
        )?;
        Ok(Self { adi, state })
    }

    pub fn is_provisioned(&mut self) -> Result<bool, AnisetteError> {
        Ok(self.adi.is_provisioned()?)
    }

    /// Provision with Apple, if not already
    pub fn provision(&mut self, transport: &dyn Transport) -> Result<(), AnisetteError> {
        if self.adi.is_provisioned()? {
            return Ok(());
        }
        provisioning::provision(transport, &mut self.adi, &mut self.state)?;
        self.state.take_files(self.adi.files());
        Ok(())
    }

    /// The headers for one request. Each call makes a new one-time password.
    pub fn headers(&mut self) -> Result<Vec<(String, String)>, AnisetteError> {
        let otp = self.adi.request_otp()?;
        // ADI may update its files when it makes a password
        self.state.take_files(self.adi.files());
        let state = &self.state;
        Ok(vec![
            ("X-Apple-I-MD".into(), STANDARD.encode(&otp.otp)),
            ("X-Apple-I-MD-M".into(), STANDARD.encode(&otp.machine_id)),
            ("X-Apple-I-MD-RINFO".into(), state.routing_info.clone()),
            ("X-Apple-I-MD-LU".into(), state.local_user.clone()),
            ("X-Mme-Device-Id".into(), state.device_id.clone()),
            ("X-MMe-Client-Info".into(), state.client_info.clone()),
            ("X-Apple-I-Client-Time".into(), client_time()),
            ("X-Apple-I-TimeZone".into(), "UTC".into()),
            ("X-Apple-Locale".into(), "en_US".into()),
            ("X-Apple-I-SRL-NO".into(), "0".into()),
        ])
    }

    /// The identity and ADI's files, to save (secretly) for next time
    pub fn state(&self) -> &AnisetteState {
        &self.state
    }

    /// Library imports the emulated libc lacks; only a problem if called
    pub fn missing_imports(&self) -> Vec<String> {
        self.adi.missing_imports()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_identities_look_like_apples() {
        let state = AnisetteState::generate();
        assert_eq!(state.identifier.len(), 16);
        assert!(
            state
                .identifier
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
        assert_eq!(state.device_id.len(), 36);
        assert_eq!(state.local_user.len(), 64);
        assert_ne!(AnisetteState::generate().identifier, state.identifier);
    }

    #[test]
    fn files_round_trip_through_state() {
        let mut state = AnisetteState::generate();
        let mut files = Files::default();
        files.create_directories(PROVISIONING_PATH);
        files.files.insert("/adi/adi.pb".into(), vec![1, 2, 3]);
        files.files.insert("/lib/other".into(), vec![9]);
        state.take_files(&files);
        assert_eq!(state.files.keys().collect::<Vec<_>>(), ["adi.pb"]);
        let back = state.to_files().unwrap();
        assert_eq!(back.files.get("/adi/adi.pb"), Some(&vec![1, 2, 3]));
    }

    #[test]
    fn debug_hides_the_identity() {
        let state = AnisetteState::generate();
        let text = format!("{state:?}");
        assert!(!text.contains(&state.identifier));
        assert!(!text.contains(&state.device_id));
    }

    #[test]
    fn client_time_is_iso_utc() {
        let time = client_time();
        assert_eq!(time.len(), 20);
        assert!(time.ends_with('Z') && time.as_bytes()[10] == b'T');
    }
}
