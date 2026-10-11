//! The exported functions, called from Rust with warpsign-core's test data

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use warpsign_ffi::{
    Component, Step, StepObserver, WarpsignError, core_version, inspect_ipa, inspect_steps,
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
