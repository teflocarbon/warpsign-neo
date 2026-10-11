//! The exported functions, called from Rust with warpsign-core's test data

use std::path::PathBuf;

use warpsign_ffi::{Component, WarpsignError, core_version, inspect_ipa, read_profile};

fn core_data() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../warpsign-core/tests/data")
}

#[test]
fn reports_version() {
    assert!(!core_version().is_empty());
}

#[test]
fn inspects_an_ipa() {
    let dir = tempfile::tempdir().unwrap();
    let ipa = dir.path().join("Test.ipa");
    warpsign_core::ipa::package_ipa(&core_data().join("Test.app"), &ipa).unwrap();

    let work = dir.path().join("work");
    let info = inspect_ipa(ipa.to_str().unwrap().into(), work.to_str().unwrap().into()).unwrap();

    assert_eq!(info.bundle_id, "com.example.test");
    assert_eq!(info.display_name.as_deref(), Some("Test"));
    assert_eq!(info.version.as_deref(), Some("1.0"));
    assert!(info.app_path.ends_with("Test.app"));
    assert_eq!(
        info.components,
        [
            Component {
                path: "".into(),
                bundle_id: "com.example.test".into(),
                is_primary: true
            },
            Component {
                path: "Frameworks/Foo.framework".into(),
                bundle_id: "Foo".into(),
                is_primary: false
            },
            Component {
                path: "PlugIns/Widget.appex".into(),
                bundle_id: "com.example.test.widget".into(),
                is_primary: true
            },
        ]
    );
    // The test app is unsigned
    assert!(info.team_ids.is_empty());
}

#[test]
fn bad_ipa_is_an_ipa_error() {
    let dir = tempfile::tempdir().unwrap();
    let bogus = dir.path().join("bogus.ipa");
    std::fs::write(&bogus, b"not a zip").unwrap();

    let result = inspect_ipa(
        bogus.to_str().unwrap().into(),
        dir.path().join("work").to_str().unwrap().into(),
    );
    assert!(matches!(result, Err(WarpsignError::Ipa { .. })));
}

#[test]
fn reads_profiles() {
    let ppq = read_profile(
        core_data()
            .join("profiles/ppq.mobileprovision")
            .to_str()
            .unwrap()
            .into(),
    )
    .unwrap();
    assert_eq!(ppq.name.as_deref(), Some("WS ppq Development"));
    assert_eq!(ppq.team_ids, ["ZZZZZ99999"]);
    assert_eq!(ppq.device_count, 2);
    assert!(ppq.ppq_check);

    let missing = read_profile("/nonexistent.mobileprovision".into());
    assert!(matches!(missing, Err(WarpsignError::Profile { .. })));
}
