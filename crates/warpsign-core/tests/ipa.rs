use std::io::Write;
use std::path::{Path, PathBuf};

use plist::{Dictionary, Value};
use warpsign_core::inspect::slash_path;
use warpsign_core::ipa::{IpaError, extract_ipa, package_ipa};
use warpsign_core::signature::read_signature;
use warpsign_core::signing::{BundleEntitlements, SigningIdentity, sign_app_bundle};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data")
}

/// Write a zip with the given (name, contents, unix mode) entries
fn write_zip(path: &Path, entries: &[(&str, &[u8], Option<u32>)]) {
    let mut zip = ZipWriter::new(std::fs::File::create(path).unwrap());
    for (name, contents, mode) in entries {
        let mut options = SimpleFileOptions::default();
        if let Some(mode) = mode {
            options = options.unix_permissions(*mode);
        }
        if name.ends_with('/') {
            zip.add_directory(*name, options).unwrap();
        } else {
            zip.start_file(*name, options).unwrap();
            zip.write_all(contents).unwrap();
        }
    }
    zip.finish().unwrap();
}

fn zip_entry_names(path: &Path) -> Vec<String> {
    let archive = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    archive.file_names().map(String::from).collect()
}

#[cfg(unix)]
fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::symlink_metadata(path)
        .unwrap()
        .permissions()
        .mode()
        & 0o777
}

#[test]
fn extract_finds_the_app() {
    let dir = tempfile::tempdir().unwrap();
    let ipa = dir.path().join("in.ipa");
    write_zip(
        &ipa,
        &[
            ("Payload/Test.app/Info.plist", b"plist", None),
            ("Payload/Test.app/Test", b"binary", None),
            ("iTunesMetadata.plist", b"metadata", None),
        ],
    );

    let app = extract_ipa(&ipa, &dir.path().join("out")).unwrap();

    assert_eq!(app, dir.path().join("out/Payload/Test.app"));
    assert_eq!(std::fs::read(app.join("Test")).unwrap(), b"binary");
}

#[test]
fn extract_requires_exactly_one_app() {
    let dir = tempfile::tempdir().unwrap();

    let none = dir.path().join("none.ipa");
    write_zip(&none, &[("Payload/readme.txt", b"no app here", None)]);
    assert!(matches!(
        extract_ipa(&none, &dir.path().join("a")),
        Err(IpaError::NoApp(_))
    ));

    let two = dir.path().join("two.ipa");
    write_zip(
        &two,
        &[
            ("Payload/One.app/Info.plist", b"", None),
            ("Payload/Two.app/Info.plist", b"", None),
        ],
    );
    assert!(matches!(
        extract_ipa(&two, &dir.path().join("b")),
        Err(IpaError::MultipleApps(_))
    ));
}

#[test]
fn extract_never_writes_outside_the_destination() {
    let dir = tempfile::tempdir().unwrap();
    let ipa = dir.path().join("evil.ipa");
    write_zip(
        &ipa,
        &[
            ("Payload/Test.app/Info.plist", b"plist", None),
            ("../escaped.txt", b"gotcha", None),
        ],
    );

    let dest = dir.path().join("nested/out");
    let _ = extract_ipa(&ipa, &dest);

    assert!(!dir.path().join("nested/escaped.txt").exists());
    assert!(!dir.path().join("escaped.txt").exists());
}

#[cfg(unix)]
#[test]
fn extract_restores_modes_and_makes_files_writable() {
    let dir = tempfile::tempdir().unwrap();
    let ipa = dir.path().join("in.ipa");
    write_zip(
        &ipa,
        &[
            ("Payload/Test.app/Test", b"binary", Some(0o755)),
            ("Payload/Test.app/ReadOnly.txt", b"locked", Some(0o444)),
        ],
    );

    let app = extract_ipa(&ipa, &dir.path().join("out")).unwrap();

    assert_eq!(mode(&app.join("Test")), 0o755);
    assert_eq!(mode(&app.join("ReadOnly.txt")), 0o644);
}

#[cfg(unix)]
#[test]
fn package_round_trips_modes_and_symlinks() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("Test.app");
    std::fs::create_dir_all(app.join("Frameworks/Foo.framework/Versions/A")).unwrap();
    std::fs::write(app.join("Test"), b"binary").unwrap();
    std::fs::set_permissions(app.join("Test"), std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(app.join("Info.plist"), b"plist").unwrap();
    std::fs::write(app.join("Frameworks/Foo.framework/Versions/A/Foo"), b"lib").unwrap();
    symlink("Versions/A/Foo", app.join("Frameworks/Foo.framework/Foo")).unwrap();

    let ipa = dir.path().join("out.ipa");
    package_ipa(&app, &ipa).unwrap();

    let names = zip_entry_names(&ipa);
    assert!(names.iter().all(|n| n.starts_with("Payload/")), "{names:?}");
    assert!(names.contains(&"Payload/Test.app/Test".to_string()));
    assert!(!dir.path().join("out.ipa.partial").exists());

    let extracted = extract_ipa(&ipa, &dir.path().join("extracted")).unwrap();
    assert_eq!(mode(&extracted.join("Test")), 0o755);
    assert_eq!(mode(&extracted.join("Info.plist")) & 0o111, 0);
    let link = extracted.join("Frameworks/Foo.framework/Foo");
    assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
    assert_eq!(std::fs::read(&link).unwrap(), b"lib");
}

#[test]
fn package_rejects_non_app_directories() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        package_ipa(dir.path(), &dir.path().join("out.ipa")),
        Err(IpaError::NotAnApp(_))
    ));
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// An IPA zipped without unix modes (like Python warpsign's output) comes
/// out of extract → sign → package with working executables
#[test]
fn ipa_in_signed_ipa_out() {
    let dir = tempfile::tempdir().unwrap();

    // Build an input IPA with no permission information at all
    let source = dir.path().join("source/Test.app");
    copy_dir(&data_dir().join("Test.app"), &source);
    let mut entries = Vec::new();
    fn collect(root: &Path, path: &Path, entries: &mut Vec<(String, Vec<u8>)>) {
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                collect(root, &path, entries);
            } else {
                let name = slash_path(path.strip_prefix(root).unwrap());
                entries.push((
                    format!("Payload/Test.app/{name}"),
                    std::fs::read(&path).unwrap(),
                ));
            }
        }
    }
    collect(&source, &source, &mut entries);
    let input = dir.path().join("input.ipa");
    let refs: Vec<(&str, &[u8], Option<u32>)> = entries
        .iter()
        .map(|(n, c)| (n.as_str(), c.as_slice(), None))
        .collect();
    write_zip(&input, &refs);

    let app = extract_ipa(&input, &dir.path().join("work")).unwrap();
    let p12 = std::fs::read(data_dir().join("test-cert.p12")).unwrap();
    let identity = SigningIdentity::from_p12(&p12, "secret").unwrap();
    let main: Dictionary = [(
        "application-identifier".to_string(),
        Value::String("ZZZZZ99999.com.example.test".into()),
    )]
    .into_iter()
    .collect();
    sign_app_bundle(
        &app,
        &identity,
        "ZZZZZ99999",
        &[BundleEntitlements {
            bundle_path: String::new(),
            entitlements: main.clone(),
        }],
    )
    .unwrap();
    let output = dir.path().join("Test-signed.ipa");
    package_ipa(&app, &output).unwrap();

    let result = extract_ipa(&output, &dir.path().join("result")).unwrap();
    let signature = read_signature(&result.join("Test")).unwrap().unwrap();
    assert_eq!(signature.entitlements, main);
    assert_eq!(signature.team_id.as_deref(), Some("ZZZZZ99999"));
    assert!(result.join("_CodeSignature/CodeResources").exists());
    #[cfg(unix)]
    for executable in [
        "Test",
        "PlugIns/Widget.appex/Widget",
        "Frameworks/Foo.framework/Foo",
    ] {
        assert_eq!(mode(&result.join(executable)), 0o755, "{executable}");
    }
}
