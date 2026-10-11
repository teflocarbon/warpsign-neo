//! Reading app metadata and icons from IPAs without extracting them.

use std::io::Write;
use std::path::{Path, PathBuf};

use plist::{Dictionary, Value};
use warpsign_core::ipa::{IpaError, package_ipa};
use warpsign_core::metadata::{Encryption, MetadataError, read_ipa_metadata};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data")
}

fn write_zip(path: &Path, entries: &[(&str, Vec<u8>)]) {
    let mut zip = ZipWriter::new(std::fs::File::create(path).unwrap());
    for (name, contents) in entries {
        zip.start_file(*name, SimpleFileOptions::default()).unwrap();
        zip.write_all(contents).unwrap();
    }
    zip.finish().unwrap();
}

fn plist_bytes(dict: Dictionary) -> Vec<u8> {
    let mut data = Vec::new();
    plist::to_writer_binary(&mut data, &Value::Dictionary(dict)).unwrap();
    data
}

/// A PNG header claiming `width` pixels, CgBI-style like Xcode writes
fn png(width: u32) -> Vec<u8> {
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend(4u32.to_be_bytes());
    png.extend(b"CgBI");
    png.extend([0u8; 8]);
    png.extend(13u32.to_be_bytes());
    png.extend(b"IHDR");
    png.extend(width.to_be_bytes());
    png.extend(width.to_be_bytes());
    png.extend([8, 6, 0, 0, 0, 0, 0, 0, 0]);
    png
}

fn info(pairs: &[(&str, Value)]) -> Dictionary {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

#[test]
fn reads_the_test_app() {
    let dir = tempfile::tempdir().unwrap();
    let ipa = dir.path().join("Test.ipa");
    package_ipa(&data_dir().join("Test.app"), &ipa).unwrap();

    let metadata = read_ipa_metadata(&ipa).unwrap();
    assert_eq!(metadata.app_name, "Test.app");
    assert_eq!(metadata.bundle_id, "com.example.test");
    assert_eq!(metadata.extension_count, 1);
    // A plain arm64 binary with no encryption info
    assert_eq!(metadata.encryption, Encryption::Decrypted);
    assert_eq!(metadata.file_size, std::fs::metadata(&ipa).unwrap().len());
    assert!(metadata.installed_size > metadata.file_size / 2);
}

#[test]
fn picks_the_largest_declared_icon() {
    let dir = tempfile::tempdir().unwrap();
    let ipa = dir.path().join("App.ipa");
    let icons = Value::Dictionary(info(&[(
        "CFBundlePrimaryIcon",
        Value::Dictionary(info(&[(
            "CFBundleIconFiles",
            Value::Array(vec!["AppIcon20x20".into(), "AppIcon60x60".into()]),
        )])),
    )]));
    let plist = info(&[
        ("CFBundleIdentifier", "com.example.app".into()),
        ("CFBundleName", "Short".into()),
        ("CFBundleDisplayName", "Example App".into()),
        ("CFBundleShortVersionString", "2.1".into()),
        ("CFBundleVersion", "210".into()),
        ("MinimumOSVersion", "16.0".into()),
        ("CFBundleIcons", icons),
    ]);
    write_zip(
        &ipa,
        &[
            ("Payload/Example.app/Info.plist", plist_bytes(plist)),
            ("Payload/Example.app/AppIcon20x20@3x.png", png(60)),
            ("Payload/Example.app/AppIcon60x60@2x.png", png(120)),
            ("Payload/Example.app/AppIcon60x60@3x.png", png(180)),
            // Not declared, and in a subfolder: ignored
            ("Payload/Example.app/Big.png", png(1024)),
            (
                "Payload/Example.app/AppIcon60x60.bundle/AppIcon60x60@4x.png",
                png(240),
            ),
            (
                "Payload/Example.app/PlugIns/Widget.appex/Info.plist",
                plist_bytes(info(&[])),
            ),
        ],
    );

    let metadata = read_ipa_metadata(&ipa).unwrap();
    assert_eq!(metadata.display_name, "Example App");
    assert_eq!(metadata.version.as_deref(), Some("2.1"));
    assert_eq!(metadata.build.as_deref(), Some("210"));
    assert_eq!(metadata.minimum_os.as_deref(), Some("16.0"));
    assert_eq!(metadata.extension_count, 1);
    assert_eq!(metadata.icon, Some(png(180)));
    // No CFBundleExecutable
    assert_eq!(metadata.encryption, Encryption::Unknown);
}

#[test]
fn falls_back_to_conventional_icon_names() {
    let dir = tempfile::tempdir().unwrap();
    let ipa = dir.path().join("App.ipa");
    write_zip(
        &ipa,
        &[
            (
                "Payload/Fallback.app/Info.plist",
                plist_bytes(info(&[(
                    "CFBundleIdentifier",
                    "com.example.fallback".into(),
                )])),
            ),
            ("Payload/Fallback.app/AppIcon60x60@2x.png", png(120)),
            ("Payload/Fallback.app/Splash.png", png(2000)),
        ],
    );
    let metadata = read_ipa_metadata(&ipa).unwrap();
    assert_eq!(metadata.display_name, "Fallback");
    assert_eq!(metadata.icon, Some(png(120)));
}

#[test]
fn rejects_ipas_without_exactly_one_app() {
    let dir = tempfile::tempdir().unwrap();
    let plist = || plist_bytes(info(&[("CFBundleIdentifier", "com.example.a".into())]));

    let none = dir.path().join("none.ipa");
    write_zip(&none, &[("Payload/readme.txt", b"hi".to_vec())]);
    assert!(matches!(
        read_ipa_metadata(&none),
        Err(MetadataError::Ipa(IpaError::NoApp(_)))
    ));

    let two = dir.path().join("two.ipa");
    write_zip(
        &two,
        &[
            ("Payload/A.app/Info.plist", plist()),
            ("Payload/B.app/Info.plist", plist()),
        ],
    );
    assert!(matches!(
        read_ipa_metadata(&two),
        Err(MetadataError::Ipa(IpaError::MultipleApps(_)))
    ));

    let no_id = dir.path().join("no-id.ipa");
    write_zip(
        &no_id,
        &[("Payload/A.app/Info.plist", plist_bytes(info(&[])))],
    );
    assert!(matches!(
        read_ipa_metadata(&no_id),
        Err(MetadataError::NoBundleId(_))
    ));
}
