use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use warpsign_core::ipa::package_ipa;
use warpsign_core::pipeline::{INSPECT_STEPS, inspect_ipa};
use warpsign_core::progress::{ProgressObserver, Step};

#[derive(Debug, Clone, PartialEq)]
enum Event {
    Started(Step),
    Log(Step, String),
    Finished(Step, Option<String>),
}

#[derive(Default)]
struct Recorder(Mutex<Vec<Event>>);

impl ProgressObserver for Recorder {
    fn step_started(&self, step: Step) {
        self.0.lock().unwrap().push(Event::Started(step));
    }
    fn step_log(&self, step: Step, message: &str) {
        self.0
            .lock()
            .unwrap()
            .push(Event::Log(step, message.into()));
    }
    fn step_finished(&self, step: Step, _elapsed: Duration, error: Option<&str>) {
        self.0
            .lock()
            .unwrap()
            .push(Event::Finished(step, error.map(String::from)));
    }
}

impl Recorder {
    fn events(&self) -> Vec<Event> {
        self.0.lock().unwrap().clone()
    }

    /// Started/finished events only, without logs
    fn outline(&self) -> Vec<Event> {
        self.events()
            .into_iter()
            .filter(|e| !matches!(e, Event::Log(..)))
            .collect()
    }

    fn logs(&self, step: Step) -> Vec<String> {
        self.events()
            .into_iter()
            .filter_map(|e| match e {
                Event::Log(s, message) if s == step => Some(message),
                _ => None,
            })
            .collect()
    }
}

fn test_ipa(dir: &std::path::Path) -> PathBuf {
    let ipa = dir.join("Test.ipa");
    package_ipa(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/Test.app"),
        &ipa,
    )
    .unwrap();
    ipa
}

#[test]
fn inspect_reports_each_step_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let recorder = Recorder::default();

    let app = inspect_ipa(&test_ipa(dir.path()), &dir.path().join("work"), &recorder).unwrap();

    let expected: Vec<Event> = INSPECT_STEPS
        .iter()
        .flat_map(|&step| [Event::Started(step), Event::Finished(step, None)])
        .collect();
    assert_eq!(recorder.outline(), expected);

    assert_eq!(app.main().bundle_id, "com.example.test");
    assert_eq!(app.components.len(), 3);
    assert_eq!(
        recorder.logs(Step::InspectComponents),
        [
            "App: com.example.test (0 entitlements)",
            "Framework Frameworks/Foo.framework: Foo (0 entitlements)",
            "Extension PlugIns/Widget.appex: com.example.test.widget (0 entitlements)",
        ]
    );
    assert_eq!(
        recorder.logs(Step::ReadTeamIds),
        ["No team IDs found: the app looks unsigned"]
    );
}

#[test]
fn a_failing_step_reports_its_error_and_stops() {
    let dir = tempfile::tempdir().unwrap();
    let bogus = dir.path().join("bogus.ipa");
    std::fs::write(&bogus, b"not a zip").unwrap();
    let recorder = Recorder::default();

    let result = inspect_ipa(&bogus, &dir.path().join("work"), &recorder);

    assert!(result.is_err());
    let outline = recorder.outline();
    assert_eq!(outline.len(), 2, "{outline:?}");
    assert_eq!(outline[0], Event::Started(Step::Extract));
    match &outline[1] {
        Event::Finished(Step::Extract, Some(error)) => assert!(!error.is_empty()),
        other => panic!("expected Extract to fail, got {other:?}"),
    }
}
