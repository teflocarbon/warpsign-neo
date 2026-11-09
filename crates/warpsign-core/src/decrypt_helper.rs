//! Staging for the on-device FairPlay decrypt helper (Pathway A).
//!
//! The helper decrypts an App Store app you own on the device itself
//! (`ios/DecryptHelper`, `mremap_encrypted(cryptid=2)`; see
//! `docs/spikes/fairplay-ondevice.md`). For that, the target app must be
//! embedded in the helper's installed bundle as a full app-bundle structure,
//! with a FairPlay licence (`.sinf`) next to every encrypted binary so
//! `fairplayd` can find it by path.
//!
//! This injects the encrypted app under `EmbeddedPayload/<App>/` (the `.app`
//! suffix dropped so signing treats it as opaque resources — see the
//! `EmbeddedPayload` handling in `inspect`/`signing`) and replicates the master
//! `.sinf` to every `SinfReplicationPaths` location from `SC_Info/Manifest.plist`.
//! The replication follows teflocarbon/ios-decrypt-py
//! (`teflodecrypt/ipa/codesign.py`); see NOTICE.

use std::fs;
use std::path::{Path, PathBuf};

use plist::Value;

use crate::ipa::{self, IpaError};

#[derive(Debug, thiserror::Error)]
pub enum StageError {
    #[error(transparent)]
    Ipa(#[from] IpaError),
    #[error("{path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("{0} has no SC_Info/Manifest.plist")]
    NoManifest(String),
    #[error("{0}: Manifest.plist has no SinfReplicationPaths")]
    NoReplicationPaths(String),
    #[error("{0} has no master .sinf in SC_Info")]
    NoMasterSinf(String),
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> StageError + '_ {
    move |source| StageError::Io {
        path: path.display().to_string(),
        source,
    }
}

/// Embed `encrypted_ipa`'s app into `helper_app` (modified in place) as
/// `EmbeddedPayload/<App>/`, with the master `.sinf` replicated per the app's
/// `Manifest.plist`. `work_dir` is scratch space for unpacking the IPA.
pub fn stage_decrypt_helper(
    helper_app: &Path,
    encrypted_ipa: &Path,
    work_dir: &Path,
) -> Result<(), StageError> {
    let app_dir = ipa::extract_ipa(encrypted_ipa, work_dir)?;
    let app_name = app_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = app_name.strip_suffix(".app").unwrap_or(&app_name);

    let embedded_root = helper_app.join("EmbeddedPayload");
    fs::create_dir_all(&embedded_root).map_err(io(&embedded_root))?;
    let embedded = embedded_root.join(stem);
    if embedded.exists() {
        fs::remove_dir_all(&embedded).map_err(io(&embedded))?;
    }
    move_dir(&app_dir, &embedded)?;

    replicate_sinfs(&embedded)?;
    Ok(())
}

/// Produce a signable decrypt-helper IPA: copy the helper template, stage the
/// owned encrypted app into it (`EmbeddedPayload/<App>/` + replicated sinfs),
/// and package it. The result is ready to sign (the signer treats
/// `EmbeddedPayload/` as opaque) and install. `work_dir` is scratch space.
pub fn build_helper_ipa(
    template_app: &Path,
    encrypted_ipa: &Path,
    output_ipa: &Path,
    work_dir: &Path,
) -> Result<(), StageError> {
    let helper = work_dir.join("DecryptHelper.app");
    if helper.exists() {
        fs::remove_dir_all(&helper).map_err(io(&helper))?;
    }
    copy_tree(template_app, &helper)?;
    stage_decrypt_helper(&helper, encrypted_ipa, &work_dir.join("extract"))?;
    ipa::package_ipa(&helper, output_ipa)?;
    Ok(())
}

/// Copy the master `.sinf` to every `SinfReplicationPaths` location so each
/// encrypted framework/appex binary has a licence next to it.
pub fn replicate_sinfs(app_dir: &Path) -> Result<usize, StageError> {
    let sc_info = app_dir.join("SC_Info");
    let manifest = sc_info.join("Manifest.plist");
    if !manifest.is_file() {
        return Err(StageError::NoManifest(app_dir.display().to_string()));
    }
    let value = Value::from_file(&manifest).map_err(|source| StageError::Io {
        path: manifest.display().to_string(),
        source: std::io::Error::other(source),
    })?;
    let paths = value
        .as_dictionary()
        .and_then(|d| d.get("SinfReplicationPaths"))
        .and_then(Value::as_array)
        .ok_or_else(|| StageError::NoReplicationPaths(app_dir.display().to_string()))?;

    let master = master_sinf(&sc_info)
        .ok_or_else(|| StageError::NoMasterSinf(app_dir.display().to_string()))?;

    let mut copied = 0;
    for rel in paths.iter().filter_map(Value::as_string) {
        let dst = app_dir.join(rel);
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(io(parent))?;
        }
        if !dst.exists() {
            fs::copy(&master, &dst).map_err(io(&dst))?;
            copied += 1;
        }
    }
    Ok(copied)
}

/// The main app's licence: the single `.sinf` in its top-level `SC_Info`.
fn master_sinf(sc_info: &Path) -> Option<PathBuf> {
    fs::read_dir(sc_info)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "sinf"))
}

/// Move `src` to `dst`, falling back to a recursive copy across filesystems.
fn move_dir(src: &Path, dst: &Path) -> Result<(), StageError> {
    if fs::rename(src, dst).is_ok() {
        return Ok(());
    }
    copy_tree(src, dst)?;
    fs::remove_dir_all(src).map_err(io(src))?;
    Ok(())
}

fn copy_tree(src: &Path, dst: &Path) -> Result<(), StageError> {
    fs::create_dir_all(dst).map_err(io(dst))?;
    for entry in fs::read_dir(src).map_err(io(src))? {
        let entry = entry.map_err(io(src))?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        let ft = entry.file_type().map_err(io(&from))?;
        if ft.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            fs::copy(&from, &to).map_err(io(&from))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn replicates_sinf_to_every_manifest_path() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("Test.app");
        // Main licence + a Manifest listing two framework targets.
        write(&app.join("SC_Info/Test.sinf"), b"MASTER-SINF");
        let mut manifest = plist::Dictionary::new();
        manifest.insert(
            "SinfReplicationPaths".into(),
            Value::Array(vec![
                Value::String("Frameworks/A.framework/SC_Info/A.sinf".into()),
                Value::String("Frameworks/B.framework/SC_Info/B.sinf".into()),
            ]),
        );
        plist::to_file_binary(
            app.join("SC_Info/Manifest.plist"),
            &Value::Dictionary(manifest),
        )
        .unwrap();

        let copied = replicate_sinfs(&app).unwrap();
        assert_eq!(copied, 2);
        for rel in [
            "Frameworks/A.framework/SC_Info/A.sinf",
            "Frameworks/B.framework/SC_Info/B.sinf",
        ] {
            assert_eq!(fs::read(app.join(rel)).unwrap(), b"MASTER-SINF");
        }
    }

    #[test]
    fn staging_injects_embedded_payload() {
        let dir = tempfile::tempdir().unwrap();
        // A minimal "encrypted" IPA: Payload/Demo.app with SC_Info + Manifest.
        let build = dir.path().join("build");
        let app = build.join("Payload/Demo.app");
        write(&app.join("Demo"), b"binary");
        write(&app.join("SC_Info/Demo.sinf"), b"SINF");
        let mut manifest = plist::Dictionary::new();
        manifest.insert(
            "SinfReplicationPaths".into(),
            Value::Array(vec![Value::String(
                "Frameworks/X.framework/SC_Info/X.sinf".into(),
            )]),
        );
        plist::to_file_binary(
            app.join("SC_Info/Manifest.plist"),
            &Value::Dictionary(manifest),
        )
        .unwrap();
        let ipa = dir.path().join("demo.ipa");
        ipa::package_ipa(&app, &ipa).unwrap();

        let helper = dir.path().join("Helper.app");
        fs::create_dir_all(&helper).unwrap();
        fs::write(helper.join("Helper"), b"helper").unwrap();

        stage_decrypt_helper(&helper, &ipa, &dir.path().join("work")).unwrap();

        let embedded = helper.join("EmbeddedPayload/Demo");
        assert!(embedded.join("Demo").is_file());
        assert_eq!(
            fs::read(embedded.join("Frameworks/X.framework/SC_Info/X.sinf")).unwrap(),
            b"SINF"
        );
    }

    fn demo_ipa(dir: &Path) -> PathBuf {
        let app = dir.join("build/Payload/Demo.app");
        write(&app.join("Demo"), b"binary");
        write(&app.join("SC_Info/Demo.sinf"), b"SINF");
        let mut m = plist::Dictionary::new();
        m.insert(
            "SinfReplicationPaths".into(),
            Value::Array(vec![Value::String(
                "Frameworks/X.framework/SC_Info/X.sinf".into(),
            )]),
        );
        plist::to_file_binary(app.join("SC_Info/Manifest.plist"), &Value::Dictionary(m)).unwrap();
        let ipa = dir.join("demo.ipa");
        ipa::package_ipa(&app, &ipa).unwrap();
        ipa
    }

    #[test]
    fn build_helper_ipa_packages_staged_app() {
        let dir = tempfile::tempdir().unwrap();
        let ipa = demo_ipa(dir.path());
        let template = dir.path().join("Template.app");
        fs::create_dir_all(&template).unwrap();
        fs::write(template.join("Template"), b"helper").unwrap();

        let out = dir.path().join("helper.ipa");
        build_helper_ipa(&template, &ipa, &out, &dir.path().join("work")).unwrap();

        let helper_app = ipa::extract_ipa(&out, &dir.path().join("check")).unwrap();
        assert!(helper_app.join("EmbeddedPayload/Demo/Demo").is_file());
        assert!(
            helper_app
                .join("EmbeddedPayload/Demo/Frameworks/X.framework/SC_Info/X.sinf")
                .is_file()
        );
    }
}
