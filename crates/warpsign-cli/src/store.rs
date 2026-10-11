//! What the CLI keeps between runs, in the config directory
//! (`~/.config/warpsign` on Linux, `~/Library/Application Support/warpsign`
//! on macOS, `%APPDATA%\warpsign` on Windows). The session and identities
//! are secrets: files are written readable by the owner only.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::{Deserialize, Serialize};
use warpsign_core::identity::{CertificateKind, Identity};
use warpsign_core::portal::SessionState;

pub struct Store {
    dir: PathBuf,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Account {
    pub apple_id: Option<String>,
    pub team_id: Option<String>,
    pub team_name: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct StoredIdentity {
    certificate: String,
    private_key: String,
}

impl Store {
    pub fn open(dir: Option<PathBuf>) -> Result<Self> {
        let dir = match dir {
            Some(dir) => dir,
            None => dirs::config_dir()
                .context("no config directory on this system: use --config-dir")?
                .join("warpsign"),
        };
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        Ok(Self { dir })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn account(&self) -> Account {
        self.read_json("account.json").unwrap_or_default()
    }

    pub fn save_account(&self, account: &Account) -> Result<()> {
        self.write_json("account.json", account)
    }

    pub fn session(&self) -> Option<SessionState> {
        self.read_json("session.json")
    }

    pub fn save_session(&self, session: &SessionState) -> Result<()> {
        self.write_json("session.json", session)
    }

    pub fn forget_session(&self) -> Result<()> {
        remove(&self.dir.join("session.json"))
    }

    pub fn identity(&self, kind: CertificateKind) -> Result<Option<Identity>> {
        let Some(stored) = self.read_json::<StoredIdentity>(identity_file(kind)) else {
            return Ok(None);
        };
        let certificate = BASE64.decode(stored.certificate)?;
        let private_key = BASE64.decode(stored.private_key)?;
        Ok(Some(Identity::new(&certificate, &private_key)?))
    }

    pub fn identities(&self) -> Result<Vec<Identity>> {
        let mut identities = Vec::new();
        for kind in [CertificateKind::Development, CertificateKind::Distribution] {
            identities.extend(self.identity(kind)?);
        }
        Ok(identities)
    }

    pub fn save_identity(&self, kind: CertificateKind, identity: &Identity) -> Result<()> {
        self.write_json(
            identity_file(kind),
            &StoredIdentity {
                certificate: BASE64.encode(identity.certificate_der()),
                private_key: BASE64.encode(identity.private_key_der()),
            },
        )
    }

    pub fn remove_identity(&self, kind: CertificateKind) -> Result<()> {
        remove(&self.dir.join(identity_file(kind)))
    }

    fn read_json<T: for<'de> Deserialize<'de>>(&self, name: &str) -> Option<T> {
        let data = std::fs::read(self.dir.join(name)).ok()?;
        serde_json::from_slice(&data).ok()
    }

    fn write_json<T: Serialize>(&self, name: &str, value: &T) -> Result<()> {
        let path = self.dir.join(name);
        write_private(&path, &serde_json::to_vec_pretty(value)?)
            .with_context(|| format!("writing {}", path.display()))
    }
}

fn identity_file(kind: CertificateKind) -> &'static str {
    match kind {
        CertificateKind::Development => "identity-development.json",
        CertificateKind::Distribution => "identity-distribution.json",
    }
}

fn remove(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(e).with_context(|| format!("removing {}", path.display()))
        }
        _ => Ok(()),
    }
}

/// Write a file only its owner can read
pub fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let partial = path.with_extension("partial");
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&partial)?;
        file.write_all(data)?;
    }
    #[cfg(not(unix))]
    std::fs::write(&partial, data)?;
    std::fs::rename(&partial, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_identities_and_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(Some(dir.path().to_path_buf())).unwrap();
        assert!(store.session().is_none());
        assert!(store.identities().unwrap().is_empty());

        let p12 = std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../warpsign-core/tests/data/test-cert.p12"),
        )
        .unwrap();
        let identity = warpsign_core::identity::import_p12(&p12, "secret")
            .unwrap()
            .remove(0);
        store
            .save_identity(CertificateKind::Development, &identity)
            .unwrap();
        let loaded = store
            .identity(CertificateKind::Development)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.certificate_der(), identity.certificate_der());
        assert!(
            store
                .identity(CertificateKind::Distribution)
                .unwrap()
                .is_none()
        );

        let session = SessionState {
            csrf: Some("token".into()),
            ..Default::default()
        };
        store.save_session(&session).unwrap();
        assert_eq!(store.session(), Some(session));
        store.forget_session().unwrap();
        store.forget_session().unwrap();
        assert!(store.session().is_none());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("identity-development.json"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
