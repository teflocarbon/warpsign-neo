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
//!
//! Only what decrypting needs is embedded: the encrypted binaries, every
//! `Info.plist` and every `SC_Info` (the bundle layout `fairplayd` finds
//! licences by), not the app's images, assets and other resources. The
//! helper hands back that layout decrypted, and `merge_decrypted_ipa` puts
//! its binaries into the original IPA.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use plist::Value;

use crate::fairplay;
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
    #[error("these didn't come back decrypted: {0}")]
    NotDecrypted(String),
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
    keep_only_layout(&embedded)?;
    Ok(())
}

/// Delete everything decrypting doesn't need from the embedded app: keep
/// encrypted binaries, `Info.plist`s and `SC_Info` folders, in place.
fn keep_only_layout(dir: &Path) -> Result<(), StageError> {
    for entry in fs::read_dir(dir).map_err(io(dir))? {
        let entry = entry.map_err(io(dir))?;
        let path = entry.path();
        let name = entry.file_name();
        if entry.file_type().map_err(io(&path))?.is_dir() {
            if name != "SC_Info" {
                keep_only_layout(&path)?;
            }
        } else if name != "Info.plist" && encryption(&path)? == Some(false) {
            fs::remove_file(&path).map_err(io(&path))?;
        }
    }
    Ok(())
}

/// Whether `path` is a Mach-O with a FairPlay-encrypted arm64 slice: nil
/// for one whose load commands don't parse (staging keeps it, and the
/// helper says what it makes of it; merging leaves it as it was)
fn encryption(path: &Path) -> Result<Option<bool>, StageError> {
    if !looks_like_macho(path)? {
        return Ok(Some(false));
    }
    let data = fs::read(path).map_err(io(path))?;
    Ok(fairplay::arm64_encryption_infos(&data)
        .ok()
        .map(|infos| !infos.is_empty()))
}

/// Whether `path` starts with a Mach-O or FAT magic number (read without
/// loading the file: most of an app is images and other resources)
fn looks_like_macho(path: &Path) -> Result<bool, StageError> {
    let mut magic = [0u8; 4];
    let mut file = fs::File::open(path).map_err(io(path))?;
    let mut read = 0;
    while read < 4 {
        match file.read(&mut magic[read..]).map_err(io(path))? {
            0 => return Ok(false),
            n => read += n,
        }
    }
    Ok(matches!(
        magic,
        [0xcf, 0xfa, 0xed, 0xfe]
            | [0xce, 0xfa, 0xed, 0xfe]
            | [0xca, 0xfe, 0xba, 0xbe]
            | [0xca, 0xfe, 0xba, 0xbf]
    ))
}

/// Put the binaries the helper decrypted (`decrypted_ipa`: the embedded
/// layout, decrypted) into `original_ipa`, and package the result as
/// `output_ipa`. Fails, writing nothing, if any encrypted binary in the
/// original didn't come back decrypted. Returns how many were replaced.
/// `work_dir` is scratch space.
pub fn merge_decrypted_ipa(
    original_ipa: &Path,
    decrypted_ipa: &Path,
    output_ipa: &Path,
    work_dir: &Path,
) -> Result<usize, StageError> {
    let original = ipa::extract_ipa(original_ipa, &work_dir.join("original"))?;
    let decrypted = ipa::extract_ipa(decrypted_ipa, &work_dir.join("decrypted"))?;

    let mut encrypted = Vec::new();
    collect_encrypted(&original, &original, &mut encrypted)?;
    let mut missing = Vec::new();
    for rel in &encrypted {
        let from = decrypted.join(rel);
        let decrypted_binary = from.is_file() && encryption(&from)? == Some(false);
        if decrypted_binary {
            let to = original.join(rel);
            fs::copy(&from, &to).map_err(io(&to))?;
        } else {
            // "/" on every platform: the paths are inside the app
            let parts: Vec<_> = rel.iter().map(|part| part.to_string_lossy()).collect();
            missing.push(parts.join("/"));
        }
    }
    if !missing.is_empty() {
        return Err(StageError::NotDecrypted(missing.join(", ")));
    }
    ipa::package_ipa(&original, output_ipa)?;
    Ok(encrypted.len())
}

/// The encrypted binaries under `dir`, relative to `root`
fn collect_encrypted(root: &Path, dir: &Path, found: &mut Vec<PathBuf>) -> Result<(), StageError> {
    for entry in fs::read_dir(dir).map_err(io(dir))? {
        let entry = entry.map_err(io(dir))?;
        let path = entry.path();
        if entry.file_type().map_err(io(&path))?.is_dir() {
            if entry.file_name() != "SC_Info" {
                collect_encrypted(root, &path, found)?;
            }
        } else if encryption(&path)? == Some(true) {
            found.push(path.strip_prefix(root).unwrap_or(&path).to_path_buf());
        }
    }
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
        write(&app.join("Demo"), &encrypted_macho());
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

    /// An encrypted arm64 Mach-O (one page of cryptid 1), and the same
    /// decrypted (cryptid 0)
    fn encrypted_macho() -> Vec<u8> {
        fairplay::tests::build_macho(0x4000, 0x4000, 0).0
    }

    fn decrypted_macho() -> Vec<u8> {
        let (mut data, cryptid) = fairplay::tests::build_macho(0x4000, 0x4000, 0);
        data[cryptid..cryptid + 4].copy_from_slice(&0u32.to_le_bytes());
        data
    }

    /// Demo.app: an encrypted main binary and framework, their Info.plists,
    /// the licence, and resources decrypting doesn't need
    fn demo_app(dir: &Path) -> PathBuf {
        let app = dir.join("build/Payload/Demo.app");
        write(&app.join("Demo"), &encrypted_macho());
        write(&app.join("Info.plist"), b"plist");
        write(&app.join("Assets.car"), b"assets");
        write(&app.join("Frameworks/X.framework/X"), &encrypted_macho());
        write(&app.join("Frameworks/X.framework/Info.plist"), b"plist");
        write(&app.join("Frameworks/X.framework/image.png"), b"png");
        write(&app.join("SC_Info/Demo.sinf"), b"SINF");
        let mut m = plist::Dictionary::new();
        m.insert(
            "SinfReplicationPaths".into(),
            Value::Array(vec![Value::String(
                "Frameworks/X.framework/SC_Info/X.sinf".into(),
            )]),
        );
        plist::to_file_binary(app.join("SC_Info/Manifest.plist"), &Value::Dictionary(m)).unwrap();
        app
    }

    fn demo_ipa(dir: &Path) -> PathBuf {
        let app = demo_app(dir);
        let ipa = dir.join("demo.ipa");
        ipa::package_ipa(&app, &ipa).unwrap();
        ipa
    }

    #[test]
    fn build_helper_ipa_embeds_only_the_layout() {
        let dir = tempfile::tempdir().unwrap();
        let ipa = demo_ipa(dir.path());
        let template = dir.path().join("Template.app");
        fs::create_dir_all(&template).unwrap();
        fs::write(template.join("Template"), b"helper").unwrap();

        let out = dir.path().join("helper.ipa");
        build_helper_ipa(&template, &ipa, &out, &dir.path().join("work")).unwrap();

        let helper = ipa::extract_ipa(&out, &dir.path().join("check")).unwrap();
        let embedded = helper.join("EmbeddedPayload/Demo");
        for kept in [
            "Demo",
            "Info.plist",
            "SC_Info/Demo.sinf",
            "SC_Info/Manifest.plist",
            "Frameworks/X.framework/X",
            "Frameworks/X.framework/Info.plist",
            "Frameworks/X.framework/SC_Info/X.sinf",
        ] {
            assert!(embedded.join(kept).is_file(), "{kept} kept");
        }
        for dropped in ["Assets.car", "Frameworks/X.framework/image.png"] {
            assert!(!embedded.join(dropped).exists(), "{dropped} dropped");
        }
    }

    /// What the helper hands back: the layout, binaries decrypted
    fn handed_back(dir: &Path, with_framework: bool) -> PathBuf {
        let app = dir.join("back/Payload/Demo.app");
        write(&app.join("Demo"), &decrypted_macho());
        write(&app.join("Info.plist"), b"plist");
        if with_framework {
            write(&app.join("Frameworks/X.framework/X"), &decrypted_macho());
        }
        let ipa = dir.join("back.ipa");
        ipa::package_ipa(&app, &ipa).unwrap();
        ipa
    }

    #[test]
    fn merge_puts_decrypted_binaries_into_the_original() {
        let dir = tempfile::tempdir().unwrap();
        let original = demo_ipa(dir.path());
        let back = handed_back(dir.path(), true);
        let out = dir.path().join("out.ipa");

        let replaced =
            merge_decrypted_ipa(&original, &back, &out, &dir.path().join("work")).unwrap();
        assert_eq!(replaced, 2);

        let app = ipa::extract_ipa(&out, &dir.path().join("check")).unwrap();
        assert_eq!(fs::read(app.join("Demo")).unwrap(), decrypted_macho());
        assert_eq!(
            fs::read(app.join("Frameworks/X.framework/X")).unwrap(),
            decrypted_macho()
        );
        // Everything else as it was
        assert_eq!(fs::read(app.join("Assets.car")).unwrap(), b"assets");
        assert!(app.join("SC_Info/Demo.sinf").is_file());
    }

    #[test]
    fn merge_refuses_a_binary_left_encrypted() {
        let dir = tempfile::tempdir().unwrap();
        let original = demo_ipa(dir.path());
        let back = handed_back(dir.path(), false);
        let out = dir.path().join("out.ipa");

        let error =
            merge_decrypted_ipa(&original, &back, &out, &dir.path().join("work")).unwrap_err();
        assert!(
            error.to_string().contains("Frameworks/X.framework/X"),
            "{error}"
        );
        assert!(!out.exists());
    }
}
