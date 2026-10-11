//! The exported functions, called from Rust with warpsign-core's test data

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use warpsign_ffi::{
    Component, Encryption, Step, StepObserver, WarpsignError, core_version, inspect_ipa,
    inspect_steps, parse_source, read_ipa_metadata, read_profile,
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
            Ok(if url.contains("olympus/v1/app/config") {
                json(200, &[], r#"{"authServiceKey": "key"}"#)
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
            WarpsignError::Auth { message } => {
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
            WarpsignError::Auth { message } => assert!(message.contains("offline"), "{message}"),
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
