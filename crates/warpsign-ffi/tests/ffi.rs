//! The exported functions, called from Rust with warpsign-core's test data

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use plist::{Dictionary, Value};
use warpsign_ffi::{
    Encryption, EntitlementChangeKind, SignedComponentInfo, Step, StepObserver, WarpsignError,
    core_version, entitlement_names, inspect_ipa, inspect_steps, parse_source, read_ipa_metadata,
    read_profile,
};

fn core_data() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../warpsign-core/tests/data")
}

/// Records events the way the Swift app receives them
#[derive(Default)]
struct Recorder(Mutex<Vec<String>>);

impl StepObserver for Recorder {
    fn step_started(&self, step: Step) {
        self.0.lock().unwrap().push(format!("start {step:?}"));
    }
    fn step_log(&self, step: Step, message: String) {
        self.0
            .lock()
            .unwrap()
            .push(format!("log {step:?}: {message}"));
    }
    fn step_finished(&self, step: Step, _elapsed_ms: u64, error: Option<String>) {
        let outcome = match error {
            Some(error) => format!("failed: {error}"),
            None => "ok".into(),
        };
        self.0
            .lock()
            .unwrap()
            .push(format!("finish {step:?} {outcome}"));
    }
    fn step_progress(&self, _step: Step, _fraction: f64) {}
}

impl Recorder {
    fn events(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

#[test]
fn reports_version() {
    assert!(!core_version().is_empty());
}

#[test]
fn lists_inspect_steps() {
    let steps: Vec<_> = inspect_steps()
        .into_iter()
        .map(|s| (s.step, s.title))
        .collect();
    assert_eq!(
        steps,
        [
            (Step::Extract, "Extract IPA".to_string()),
            (Step::InspectComponents, "Inspect components".to_string()),
            (Step::ReadTeamIds, "Read team IDs".to_string()),
        ]
    );
}

#[test]
fn inspects_an_ipa_step_by_step() {
    let dir = tempfile::tempdir().unwrap();
    let ipa = dir.path().join("Test.ipa");
    warpsign_core::ipa::package_ipa(&core_data().join("Test.app"), &ipa).unwrap();
    let recorder = Arc::new(Recorder::default());

    let info = inspect_ipa(
        ipa.to_str().unwrap().into(),
        dir.path().join("work").to_str().unwrap().into(),
        recorder.clone(),
    )
    .unwrap();

    assert_eq!(info.bundle_id, "com.example.test");
    assert_eq!(info.display_name.as_deref(), Some("Test"));
    assert_eq!(info.version.as_deref(), Some("1.0"));
    assert!(info.app_path.ends_with("Test.app"));
    let outline: Vec<_> = info
        .components
        .iter()
        .map(|c| (c.path.as_str(), c.bundle_id.as_str(), c.is_primary))
        .collect();
    assert_eq!(
        outline,
        [
            ("", "com.example.test", true),
            ("Frameworks/Foo.framework", "Foo", false),
            ("PlugIns/Widget.appex", "com.example.test.widget", true),
        ]
    );
    // Unsigned: no entitlements, but still a readable plist
    for component in &info.components {
        assert_eq!(entitlements(&component.entitlements_xml), Dictionary::new());
    }
    // The test app is unsigned
    assert!(info.team_ids.is_empty());

    let events = recorder.events();
    let outline: Vec<_> = events.iter().filter(|e| !e.starts_with("log")).collect();
    assert_eq!(
        outline,
        [
            "start Extract",
            "finish Extract ok",
            "start InspectComponents",
            "finish InspectComponents ok",
            "start ReadTeamIds",
            "finish ReadTeamIds ok",
        ]
    );
    assert!(events.contains(&"log Extract: Found Test.app".to_string()));
}

/// An XML plist from the FFI, decoded
fn entitlements(xml: &str) -> Dictionary {
    Value::from_reader_xml(xml.as_bytes())
        .unwrap()
        .into_dictionary()
        .unwrap()
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
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

#[test]
fn inspection_shows_each_components_entitlements() {
    use warpsign_core::signing::{BundleEntitlements, sign_app_bundle};

    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("Payload/Test.app");
    copy_dir(&core_data().join("Test.app"), &app);
    let main: Dictionary = [
        (
            "application-identifier",
            Value::from("ABCDE12345.com.example.test"),
        ),
        ("aps-environment", "production".into()),
        (
            "keychain-access-groups",
            Value::Array(vec!["ABCDE12345.com.example.test".into()]),
        ),
        ("com.example.custom", true.into()),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_string(), value))
    .collect();
    let identity = warpsign_core::identity::import_p12(
        &std::fs::read(core_data().join("test-cert.p12")).unwrap(),
        "secret",
    )
    .unwrap()
    .remove(0);
    sign_app_bundle(
        &app,
        &identity.signing_identity().unwrap(),
        "ABCDE12345",
        &[BundleEntitlements {
            bundle_path: String::new(),
            entitlements: main.clone(),
        }],
    )
    .unwrap();
    let ipa = dir.path().join("Test.ipa");
    warpsign_core::ipa::package_ipa(&app, &ipa).unwrap();

    let info = inspect_ipa(
        ipa.to_str().unwrap().into(),
        dir.path().join("work").to_str().unwrap().into(),
        Arc::new(Recorder::default()),
    )
    .unwrap();
    let app = &info.components[0];
    assert_eq!(entitlements(&app.entitlements_xml), main);
}

#[test]
fn names_entitlements_from_a_capabilities_response() {
    let fixture = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/capabilities_response.json"),
    )
    .unwrap();
    let names = entitlement_names(Some(fixture));
    let name = |key: &str| {
        names
            .iter()
            .find(|name| name.key == key)
            .map(|name| (name.name.as_str(), name.capability.as_deref()))
    };
    assert_eq!(
        name("aps-environment"),
        Some(("Push Notifications", Some("PUSH_NOTIFICATIONS")))
    );
    // Only under iCloud's settings options
    assert_eq!(
        name("com.apple.developer.ubiquity-kvstore-identifier"),
        Some(("iCloud", Some("ICLOUD")))
    );
    assert_eq!(name("get-task-allow"), Some(("Debugging", None)));

    // Without a response (or with a broken one), only keys that are never
    // capabilities have names
    for offline in [entitlement_names(None), entitlement_names(Some("{".into()))] {
        assert!(
            offline
                .iter()
                .any(|name| name.key == "application-identifier")
        );
        assert!(!offline.iter().any(|name| name.key == "aps-environment"));
    }
}

#[test]
fn signed_components_carry_plists_and_reasons() {
    use warpsign_core::capability_names::EntitlementNames;
    use warpsign_core::pipeline::{ChangeKind, ChangeReason, EntitlementChange, SignedComponent};

    let original: Dictionary = [("get-task-allow".to_string(), Value::from(true))]
        .into_iter()
        .collect();
    let signed: Dictionary = [("application-identifier".to_string(), Value::from("T.x"))]
        .into_iter()
        .collect();
    let info = SignedComponentInfo::from(SignedComponent {
        path: String::new(),
        is_primary: true,
        original_bundle_id: "com.example.test".into(),
        bundle_id: "x".into(),
        original_entitlements: original.clone(),
        entitlements: signed.clone(),
        capabilities: vec!["APP_GROUPS".into()],
        changes: vec![EntitlementChange {
            key: "get-task-allow".into(),
            kind: ChangeKind::Removed,
            reason: Some(ChangeReason::DebuggingOff),
        }],
        names: [(
            "get-task-allow".to_string(),
            EntitlementNames::without_portal()
                .get("get-task-allow")
                .unwrap(),
        )]
        .into_iter()
        .collect(),
    });
    assert_eq!(entitlements(&info.original_entitlements_xml), original);
    assert_eq!(entitlements(&info.entitlements_xml), signed);
    assert_eq!(info.changes[0].kind, EntitlementChangeKind::Removed);
    assert_eq!(
        info.changes[0].reason.as_deref(),
        Some("Debugging is off in the signing options")
    );
    assert_eq!(info.entitlement_names[0].key, "get-task-allow");
    assert_eq!(info.entitlement_names[0].name, "Debugging");
}

#[test]
fn bad_ipa_fails_at_the_extract_step() {
    let dir = tempfile::tempdir().unwrap();
    let bogus = dir.path().join("bogus.ipa");
    std::fs::write(&bogus, b"not a zip").unwrap();
    let recorder = Arc::new(Recorder::default());

    let result = inspect_ipa(
        bogus.to_str().unwrap().into(),
        dir.path().join("work").to_str().unwrap().into(),
        recorder.clone(),
    );

    assert!(matches!(result, Err(WarpsignError::Ipa { .. })));
    let events = recorder.events();
    assert!(
        events.last().unwrap().starts_with("finish Extract failed:"),
        "{events:?}"
    );
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
    assert!(!ppq.is_wildcard);
    assert_eq!(ppq.expires_at, Some(1_893_456_000));

    let missing = read_profile("/nonexistent.mobileprovision".into());
    assert!(matches!(missing, Err(WarpsignError::Profile { .. })));
}

#[test]
fn reads_ipa_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let ipa = dir.path().join("Test.ipa");
    warpsign_core::ipa::package_ipa(&core_data().join("Test.app"), &ipa).unwrap();
    let metadata = read_ipa_metadata(ipa.display().to_string()).unwrap();
    assert_eq!(metadata.bundle_id, "com.example.test");
    assert_eq!(metadata.encryption, Encryption::Decrypted);

    let error = read_ipa_metadata(dir.path().join("missing.ipa").display().to_string());
    assert!(matches!(error, Err(WarpsignError::Ipa { .. })));
}

#[test]
fn parses_sources() {
    let data = std::fs::read(core_data().join("sources/altstore.json")).unwrap();
    let source = parse_source(data).unwrap();
    assert_eq!(source.apps.len(), 2);
    assert_eq!(source.apps[0].versions[0].version, "2.0");
    assert!(matches!(
        parse_source(b"nope".to_vec()),
        Err(WarpsignError::Source { .. })
    ));
}

mod account {
    use std::sync::{Arc, Mutex};

    use warpsign_ffi::{
        AppleAccount, HttpHeader, HttpRequest, HttpResponse, HttpTransport, IdentityData, Step,
        TransportFailure, WarpsignError, default_sign_settings, export_identities,
        generate_passphrase, identity_info, import_identities, sign_steps,
    };

    use super::core_data;

    /// Plays Apple the way the Swift transport would see it: comma-joined
    /// Set-Cookie headers included
    #[derive(Default)]
    struct FakeTransport {
        requests: Mutex<Vec<HttpRequest>>,
    }

    fn json(status: u16, headers: &[(&str, &str)], body: &str) -> HttpResponse {
        HttpResponse {
            status,
            headers: headers
                .iter()
                .map(|(name, value)| HttpHeader {
                    name: name.to_string(),
                    value: value.to_string(),
                })
                .collect(),
            body: body.as_bytes().to_vec(),
        }
    }

    impl HttpTransport for FakeTransport {
        fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportFailure> {
            self.requests.lock().unwrap().push(request.clone());
            let url = request.url.as_str();
            Ok(if url.ends_with("appstoreconnect.apple.com/logout") {
                json(
                    302,
                    &[(
                        "Location",
                        "https://appstoreconnect.apple.com/login?widgetKey=key",
                    )],
                    "",
                )
            } else if url.contains("/appleauth/auth/signin?") {
                json(
                    200,
                    &[(
                        "Set-Cookie",
                        "aasp=1; Path=/; Expires=Wed, 21 Oct 2037 07:28:00 GMT, dslang=GB-EN; Path=/",
                    )],
                    "",
                )
            } else if url.ends_with("/signin/init") {
                json(
                    401,
                    &[],
                    r#"{"serviceErrors": [{"code": "-20101", "message": "Your Apple ID or password was incorrect."}]}"#,
                )
            } else {
                return Err(TransportFailure::Failed {
                    message: format!("offline: {url}"),
                });
            })
        }
    }

    #[test]
    fn sign_in_failures_carry_apples_message() {
        let transport = Arc::new(FakeTransport::default());
        let account = AppleAccount::new(transport.clone());
        let error = account
            .sign_in("someone@example.com".into(), "wrong".into())
            .unwrap_err();
        match error {
            WarpsignError::WrongPassword { message } => {
                assert_eq!(message, "Your Apple ID or password was incorrect. (-20101)")
            }
            other => panic!("{other:?}"),
        }
        // Both cookies from the joined header went back to Apple
        let requests = transport.requests.lock().unwrap();
        let init = requests
            .iter()
            .find(|r| r.url.ends_with("/signin/init"))
            .unwrap();
        let cookie = init.headers.iter().find(|h| h.name == "Cookie").unwrap();
        assert_eq!(cookie.value, "aasp=1; dslang=GB-EN");
        assert_eq!(init.method, "POST");
        // Saved sessions keep them
        assert!(account.session_json().contains("dslang"));
    }

    #[test]
    fn transport_failures_are_reported() {
        let account =
            AppleAccount::restore(Arc::new(FakeTransport::default()), "{}".into()).unwrap();
        match account.check_session().unwrap_err() {
            WarpsignError::Offline { message } => assert!(message.contains("offline"), "{message}"),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            AppleAccount::restore(Arc::new(FakeTransport::default()), "nope".into()),
            Err(WarpsignError::SessionExpired { .. })
        ));
    }

    #[test]
    fn identities_round_trip_through_a_backup() {
        let legacy = import_identities(
            std::fs::read(core_data().join("test-cert.p12")).unwrap(),
            "secret".into(),
        )
        .unwrap();
        let aes = import_identities(
            std::fs::read(core_data().join("test-distribution-aes.p12")).unwrap(),
            "secret".into(),
        )
        .unwrap();
        let both: Vec<IdentityData> = legacy.into_iter().chain(aes).collect();
        let passphrase = generate_passphrase();
        let backup = export_identities(both.clone(), passphrase.clone()).unwrap();
        let restored = import_identities(backup, passphrase).unwrap();
        assert_eq!(restored.len(), 2);

        let info = identity_info(both[0].clone()).unwrap();
        assert_eq!(info.team_id.as_deref(), Some("ZZZZZ99999"));
        assert_eq!(info.kind, Some(warpsign_ffi::CertificateType::Development));
        assert!(info.expires_at > 1_900_000_000);

        let mismatched = IdentityData {
            certificate: both[0].certificate.clone(),
            private_key: both[1].private_key.clone(),
        };
        assert!(matches!(
            identity_info(mismatched),
            Err(WarpsignError::Identity { .. })
        ));
        assert!(matches!(
            import_identities(b"nope".to_vec(), "x".into()),
            Err(WarpsignError::Identity { .. })
        ));
    }

    #[test]
    fn describes_signing() {
        let steps: Vec<Step> = sign_steps().into_iter().map(|s| s.step).collect();
        assert_eq!(steps.first(), Some(&Step::Extract));
        assert_eq!(steps.last(), Some(&Step::Package));
        assert_eq!(steps.len(), 10);
        let settings = default_sign_settings();
        assert!(settings.encode_ids && settings.patch_ids);
        assert!(!settings.force_original_id && !settings.patch_debug);
    }
}

mod install {
    use warpsign_ffi::{Step, WarpsignError, connected_device, install_steps, pairing_file_info};

    use super::core_data;

    #[test]
    fn checks_pairing_files() {
        let data = std::fs::read(core_data().join("test.mobiledevicepairing")).unwrap();
        let info = pairing_file_info(data).unwrap();
        assert_eq!(info.udid.as_deref(), Some("00008110-000A0B0C0D0E0F10"));
        assert!(matches!(
            pairing_file_info(b"nope".to_vec()),
            Err(WarpsignError::Device { .. })
        ));
        // Not on the device, so the loopback address goes nowhere
        assert!(connected_device(b"nope".to_vec(), None).is_err());
    }

    #[test]
    fn lists_install_steps() {
        let steps: Vec<Step> = install_steps().into_iter().map(|s| s.step).collect();
        assert_eq!(
            steps,
            [Step::ConnectDevice, Step::UploadApp, Step::InstallApp]
        );
    }
}
