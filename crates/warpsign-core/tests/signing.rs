//! Signs the synthetic test app (built by tools/build_test_bundle.sh) and
//! reads every signature back.

use std::path::{Path, PathBuf};

use plist::{Dictionary, Value};
use warpsign_core::signature::{read_entitlements, read_signature};
use warpsign_core::signing::{BundleEntitlements, SigningError, SigningIdentity, sign_app_bundle};

const TEAM_ID: &str = "ZZZZZ99999";

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data")
}

fn identity() -> SigningIdentity {
    let p12 = std::fs::read(data_dir().join("test-cert.p12")).unwrap();
    SigningIdentity::from_p12(&p12, "secret").unwrap()
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

/// A fresh copy of the test app, with exec bits dropped like zip extraction does
fn test_app() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("Test.app");
    copy_dir(&data_dir().join("Test.app"), &app);
    #[cfg(unix)]
    for executable in ["Test", "PlugIns/Widget.appex/Widget"] {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(app.join(executable), std::fs::Permissions::from_mode(0o644))
            .unwrap();
    }
    (dir, app)
}

fn entitlements(pairs: &[(&str, Value)]) -> Dictionary {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

fn main_entitlements() -> Dictionary {
    entitlements(&[
        (
            "application-identifier",
            format!("{TEAM_ID}.com.example.test").into(),
        ),
        ("com.apple.developer.team-identifier", TEAM_ID.into()),
        ("get-task-allow", true.into()),
        (
            "keychain-access-groups",
            Value::Array(vec![format!("{TEAM_ID}.com.example.test").into()]),
        ),
        (
            "com.apple.security.application-groups",
            Value::Array(vec!["group.com.example.test".into()]),
        ),
        ("aps-environment", "development".into()),
    ])
}

fn widget_entitlements() -> Dictionary {
    entitlements(&[
        (
            "application-identifier",
            format!("{TEAM_ID}.com.example.test.widget").into(),
        ),
        ("com.apple.developer.team-identifier", TEAM_ID.into()),
        ("get-task-allow", true.into()),
        (
            "com.apple.security.application-groups",
            Value::Array(vec!["group.com.example.test".into()]),
        ),
    ])
}

fn sign(app: &Path, main: Dictionary, widget: Dictionary) {
    sign_app_bundle(
        app,
        &identity(),
        TEAM_ID,
        &[
            BundleEntitlements {
                bundle_path: String::new(),
                entitlements: main,
            },
            BundleEntitlements {
                bundle_path: "PlugIns/Widget.appex".into(),
                entitlements: widget,
            },
        ],
    )
    .unwrap();
}

#[test]
fn identity_reads_team_id_from_certificate() {
    assert_eq!(identity().team_id().as_deref(), Some(TEAM_ID));
}

#[test]
fn wrong_p12_password_is_an_error() {
    let p12 = std::fs::read(data_dir().join("test-cert.p12")).unwrap();
    assert!(matches!(
        SigningIdentity::from_p12(&p12, "wrong"),
        Err(SigningError::Certificate(_))
    ));
}

#[test]
fn unsigned_binaries_have_no_signature() {
    let app = data_dir().join("Test.app");
    assert_eq!(read_signature(&app.join("Test")).unwrap(), None);
    assert!(read_entitlements(&app.join("Test")).unwrap().is_empty());
}

#[test]
fn each_component_gets_its_own_signature() {
    let (_dir, app) = test_app();
    sign(&app, main_entitlements(), widget_entitlements());

    let main = read_signature(&app.join("Test")).unwrap().unwrap();
    assert_eq!(main.identifier, "com.example.test");
    assert_eq!(main.team_id.as_deref(), Some(TEAM_ID));
    assert_eq!(main.entitlements, main_entitlements());

    let widget = read_signature(&app.join("PlugIns/Widget.appex/Widget"))
        .unwrap()
        .unwrap();
    assert_eq!(widget.identifier, "com.example.test.widget");
    assert_eq!(widget.team_id.as_deref(), Some(TEAM_ID));
    assert_eq!(widget.entitlements, widget_entitlements());

    // Frameworks are signed (with the team ID dyld requires) but get no entitlements
    let framework = read_signature(&app.join("Frameworks/Foo.framework/Foo"))
        .unwrap()
        .unwrap();
    assert_eq!(framework.identifier, "com.example.foo");
    assert_eq!(framework.team_id.as_deref(), Some(TEAM_ID));
    assert!(framework.entitlements.is_empty());
}

#[cfg(unix)]
#[test]
fn executables_are_marked_executable() {
    use std::os::unix::fs::PermissionsExt;

    let (_dir, app) = test_app();
    sign(&app, main_entitlements(), widget_entitlements());

    for executable in [
        "Test",
        "PlugIns/Widget.appex/Widget",
        "Frameworks/Foo.framework/Foo",
    ] {
        let mode = std::fs::metadata(app.join(executable))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755, "{executable}");
    }
}

#[test]
fn resources_and_nested_bundles_are_sealed() {
    let (_dir, app) = test_app();
    sign(&app, main_entitlements(), widget_entitlements());

    let code_resources = Value::from_file(app.join("_CodeSignature/CodeResources")).unwrap();
    let files = code_resources
        .as_dictionary()
        .and_then(|d| d.get("files2"))
        .and_then(Value::as_dictionary)
        .expect("CodeResources has files2");
    for sealed in [
        "embedded.mobileprovision",
        "en.lproj/Localizable.strings",
        "Frameworks/Foo.framework/Foo",
        "Frameworks/Foo.framework/_CodeSignature/CodeResources",
        "PlugIns/Widget.appex/Widget",
        "PlugIns/Widget.appex/embedded.mobileprovision",
        "PlugIns/Widget.appex/_CodeSignature/CodeResources",
    ] {
        assert!(files.contains_key(sealed), "{sealed} is not sealed");
    }
    // Excluded by Apple's iOS rules, as codesign does
    assert!(!files.contains_key("Info.plist"));
}

#[test]
fn resigning_replaces_the_previous_signature() {
    let (_dir, app) = test_app();
    sign(&app, main_entitlements(), widget_entitlements());

    let mut release = main_entitlements();
    release.insert("get-task-allow".into(), false.into());
    sign(&app, release.clone(), widget_entitlements());

    assert_eq!(read_entitlements(&app.join("Test")).unwrap(), release);
}

#[test]
fn inspection_reads_back_what_was_signed() {
    use warpsign_core::inspect::{inspect_app, slash_path, team_ids};

    let (_dir, app) = test_app();
    sign(&app, main_entitlements(), widget_entitlements());

    let components = inspect_app(&app).unwrap();
    let summary: Vec<_> = components
        .iter()
        .map(|c| (slash_path(&c.path), c.bundle_id.as_str(), c.is_primary))
        .collect();
    assert_eq!(
        summary,
        [
            ("".to_string(), "com.example.test", true),
            ("Frameworks/Foo.framework".to_string(), "Foo", false),
            (
                "PlugIns/Widget.appex".to_string(),
                "com.example.test.widget",
                true
            ),
        ]
    );
    assert_eq!(components[0].entitlements, main_entitlements());
    assert!(components[1].entitlements.is_empty());
    assert_eq!(components[2].entitlements, widget_entitlements());
    assert_eq!(
        components[2].entitlements_bundle_id.as_deref(),
        Some("com.example.test.widget")
    );
    assert_eq!(team_ids(&components), [TEAM_ID]);
}
