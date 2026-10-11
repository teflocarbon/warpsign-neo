//! Signs an IPA end to end against a small fake Developer Portal.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use plist::{Dictionary, Value};
use serde_json::{Value as Json, json};
use warpsign_core::identity::{Identity, import_p12};
use warpsign_core::ipa::{extract_ipa, package_ipa};
use warpsign_core::pipeline::{
    ChangeKind, ChangeReason, EntitlementChange, PipelineError, SIGN_STEPS, SignOptions, sign_ipa,
};
use warpsign_core::portal::Portal;
use warpsign_core::portal::http::{
    HttpClient, Method, Request, Response, SessionState, Transport, TransportError,
};
use warpsign_core::progress::{ProgressObserver, Step};
use warpsign_core::signature::read_signature;
use warpsign_core::signing::{BundleEntitlements, sign_app_bundle};

const TEAM: &str = "ZZZZZ99999";
const ORIGINAL_TEAM: &str = "ABCDE12345";

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data")
}

fn identity() -> Identity {
    import_p12(
        &std::fs::read(data_dir().join("test-cert.p12")).unwrap(),
        "secret",
    )
    .unwrap()
    .remove(0)
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

fn dict(pairs: Vec<(&str, Value)>) -> Dictionary {
    pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

fn strings(items: &[&str]) -> Value {
    Value::Array(items.iter().map(|s| Value::String(s.to_string())).collect())
}

/// The test app as if from another team: signed for ABCDE12345, with app
/// groups, push and keychain entitlements
/// The test app as the App Store would ship it, after `change`
fn store_ipa_with(dir: &Path, change: impl FnOnce(&Path)) -> PathBuf {
    let app = dir.join("store/Payload/Test.app");
    copy_dir(&data_dir().join("Test.app"), &app);
    change(&app);
    let main = dict(vec![
        (
            "application-identifier",
            format!("{ORIGINAL_TEAM}.com.example.test").into(),
        ),
        ("com.apple.developer.team-identifier", ORIGINAL_TEAM.into()),
        (
            "com.apple.security.application-groups",
            strings(&["group.com.example.test"]),
        ),
        ("aps-environment", "production".into()),
        (
            "keychain-access-groups",
            strings(&[&format!("{ORIGINAL_TEAM}.com.example.test")]),
        ),
        ("get-task-allow", false.into()),
    ]);
    let widget = dict(vec![
        (
            "application-identifier",
            format!("{ORIGINAL_TEAM}.com.example.test.widget").into(),
        ),
        ("com.apple.developer.team-identifier", ORIGINAL_TEAM.into()),
        (
            "com.apple.security.application-groups",
            strings(&["group.com.example.test"]),
        ),
    ]);
    let signer = identity().signing_identity().unwrap();
    sign_app_bundle(
        &app,
        &signer,
        ORIGINAL_TEAM,
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
    let ipa = dir.join("Store.ipa");
    package_ipa(&app, &ipa).unwrap();
    ipa
}

/// Just enough of the Developer Portal, keeping what was registered
struct FakePortal {
    profile: Vec<u8>,
    devices: Json,
    certificates: Json,
    profiles: Json,
    /// Make profiles as the portal would (for the App ID, with the
    /// certificate, devices and app groups it has) instead of `profile`
    realistic_profiles: bool,
    /// Instead of the usual capabilities response
    capability_list: Option<Json>,
    state: Mutex<FakeState>,
}

#[derive(Default)]
struct FakeState {
    requests: Vec<Request>,
    app_groups: Vec<String>,
    bundle_ids: Vec<String>,
    capability_updates: Vec<(String, Json)>,
    profile_forms: Vec<String>,
    /// Profiles made: ID, name, state and contents
    made_profiles: Vec<(String, String, String, Vec<u8>)>,
    /// Devices registered after the fake was made
    extra_devices: Vec<String>,
}

/// The expiry of profiles the fake makes
const PROFILE_EXPIRY: &str = "2030-01-01T00:00:00.000+00:00";

/// A provisioning profile as the portal makes them, signed with the test
/// certificate
fn make_profile(
    name: &str,
    app_identifier: &str,
    groups: &[String],
    devices: &[String],
) -> Vec<u8> {
    let identity = identity();
    let mut entitlements = Dictionary::new();
    entitlements.insert("application-identifier".into(), app_identifier.into());
    entitlements.insert("com.apple.developer.team-identifier".into(), TEAM.into());
    entitlements.insert(
        "keychain-access-groups".into(),
        strings(&[&format!("{TEAM}.*")]),
    );
    entitlements.insert("get-task-allow".into(), true.into());
    entitlements.insert("aps-environment".into(), "development".into());
    entitlements.insert(
        "com.apple.security.application-groups".into(),
        Value::Array(groups.iter().map(|group| group.clone().into()).collect()),
    );
    let mut profile = Dictionary::new();
    profile.insert("Name".into(), name.into());
    profile.insert("TeamIdentifier".into(), strings(&[TEAM]));
    profile.insert("Entitlements".into(), Value::Dictionary(entitlements));
    profile.insert(
        "ProvisionedDevices".into(),
        Value::Array(devices.iter().map(|udid| udid.clone().into()).collect()),
    );
    profile.insert(
        "DeveloperCertificates".into(),
        Value::Array(vec![Value::Data(identity.certificate_der().to_vec())]),
    );
    let expiry = warpsign_core::profile_reuse::portal_date(PROFILE_EXPIRY).unwrap();
    profile.insert("ExpirationDate".into(), Value::Date(expiry.into()));
    let mut xml = Vec::new();
    Value::Dictionary(profile).to_writer_xml(&mut xml).unwrap();

    let key = x509_certificate::InMemorySigningKeyPair::from_pkcs8_der(identity.private_key_der())
        .unwrap();
    let certificate =
        x509_certificate::CapturedX509Certificate::from_der(identity.certificate_der().to_vec())
            .unwrap();
    cryptographic_message_syntax::SignedDataBuilder::default()
        .content_inline(xml)
        .signer(cryptographic_message_syntax::SignerBuilder::new(
            &key,
            certificate,
        ))
        .build_der()
        .unwrap()
}

impl FakePortal {
    fn new(profile: &str) -> Arc<Self> {
        let serial = identity().serial_number();
        Arc::new(Self {
            profile: std::fs::read(data_dir().join("profiles").join(profile)).unwrap(),
            devices: json!({"data": [{"id": "DEV1", "attributes": {"name": "iPhone", "udid": "00008110-0001", "status": "ENABLED", "deviceClass": "IPHONE", "platform": "IOS"}}]}),
            certificates: json!({"data": [{"id": "CERT1", "attributes": {"serialNumber": serial, "certificateType": "DEVELOPMENT", "name": "Apple Development: Test User"}}]}),
            profiles: json!({"data": []}),
            realistic_profiles: false,
            capability_list: None,
            state: Mutex::default(),
        })
    }

    fn with(mut self: Arc<Self>, update: impl FnOnce(&mut Self)) -> Arc<Self> {
        update(Arc::get_mut(&mut self).unwrap());
        self
    }

    fn capabilities() -> Json {
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/portal_requests.json");
        let fixture: Json = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        fixture["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["name"] == "capabilities")
            .unwrap()["result"]
            .clone()
    }
}

fn ok(body: Json) -> Response {
    Response {
        status: 200,
        headers: Vec::new(),
        body: serde_json::to_vec(&body).unwrap(),
    }
}

fn form_value(form: &str, key: &str) -> String {
    form.split('&')
        .find_map(|pair| pair.strip_prefix(&format!("{key}=")))
        .unwrap_or_default()
        .replace("%2C", ",")
        .replace('+', " ")
}

impl Transport for FakePortal {
    fn send(&self, request: &Request) -> Result<Response, TransportError> {
        let mut state = self.state.lock().unwrap();
        state.requests.push(request.clone());
        let url = request
            .url
            .strip_prefix("https://developer.apple.com/services-account")
            .unwrap();
        let reading = request.header_value("X-HTTP-Method-Override") == Some("GET");
        let body = String::from_utf8(request.body.clone()).unwrap();
        Ok(match (request.method, url) {
            (Method::Post, url) if url.starts_with("/v1/capabilities") => ok(self
                .capability_list
                .clone()
                .unwrap_or_else(FakePortal::capabilities)),
            (Method::Post, "/v1/devices") => {
                let mut devices = self.devices.clone();
                for (index, udid) in state.extra_devices.iter().enumerate() {
                    devices["data"].as_array_mut().unwrap().push(json!({"id": format!("NEWDEV{index}"), "attributes": {"name": "iPad", "udid": udid, "status": "ENABLED", "deviceClass": "IPAD", "platform": "IOS"}}));
                }
                ok(devices)
            }
            (Method::Post, "/v1/certificates") if reading => ok(self.certificates.clone()),
            (Method::Post, "/v1/profiles") => {
                let mut profiles = self.profiles.clone();
                for (id, name, profile_state, _) in &state.made_profiles {
                    profiles["data"].as_array_mut().unwrap().push(json!({"id": id, "attributes": {"name": name, "profileState": profile_state, "expirationDate": PROFILE_EXPIRY, "platform": "IOS", "profileTypeLabel": "iOS App Development"}}));
                }
                ok(profiles)
            }
            (Method::Post, "/v1/bundleIds") if reading => ok(json!({
                "data": state.bundle_ids.iter().enumerate().map(|(index, identifier)| {
                    json!({"id": format!("BUNDLE{}", index + 1), "attributes": {"identifier": identifier, "name": "x"}})
                }).collect::<Vec<_>>()
            })),
            (Method::Post, "/QH65B2/account/ios/identifiers/listApplicationGroups.action") => {
                ok(json!({
                    "resultCode": 0,
                    "applicationGroupList": state.app_groups.iter().enumerate().map(|(index, identifier)| {
                        json!({"applicationGroup": format!("GROUP{}", index + 1), "identifier": identifier, "name": "x"})
                    }).collect::<Vec<_>>()
                }))
            }
            (Method::Post, "/QH65B2/account/ios/identifiers/addApplicationGroup.action") => {
                let identifier = form_value(&body, "identifier");
                state.app_groups.push(identifier.clone());
                ok(
                    json!({"resultCode": 0, "applicationGroup": {"applicationGroup": format!("GROUP{}", state.app_groups.len()), "identifier": identifier, "name": "x"}}),
                )
            }
            (Method::Post, "/v1/bundleIds") if !reading => {
                let identifier = request.json_body().unwrap()["data"]["attributes"]["identifier"]
                    .as_str()
                    .unwrap()
                    .to_string();
                state.bundle_ids.push(identifier.clone());
                let id = format!("BUNDLE{}", state.bundle_ids.len());
                Response {
                    status: 201,
                    headers: Vec::new(),
                    body: serde_json::to_vec(&json!({"data": {"id": id, "attributes": {"identifier": identifier, "name": "x"}}})).unwrap(),
                }
            }
            (Method::Patch, url) if url.starts_with("/v1/bundleIds/") => {
                let id = url.rsplit('/').next().unwrap().to_string();
                state
                    .capability_updates
                    .push((id, request.json_body().unwrap()));
                ok(json!({"data": {}}))
            }
            (Method::Post, "/QH65B2/account/ios/profile/regenProvisioningProfile.action") => {
                state.profile_forms.push(body.clone());
                let existing = form_value(&body, "provisioningProfileId");
                let id = if existing.is_empty() {
                    format!("PROFILE{}", state.profile_forms.len())
                } else {
                    existing
                };
                let name = form_value(&body, "provisioningProfileName");
                let content = if self.realistic_profiles {
                    let bundle = form_value(&body, "appIdId");
                    let index: usize = bundle.trim_start_matches("BUNDLE").parse().unwrap();
                    let identifier = state.bundle_ids[index - 1].clone();
                    let mut devices = vec!["00008110-0001".to_string()];
                    devices.extend(state.extra_devices.iter().cloned());
                    make_profile(
                        &name,
                        &format!("{TEAM}.{identifier}"),
                        &state.app_groups,
                        &devices,
                    )
                } else {
                    self.profile.clone()
                };
                state.made_profiles.retain(|(made, ..)| made != &id);
                state
                    .made_profiles
                    .push((id.clone(), name, "ACTIVE".into(), content));
                ok(json!({"resultCode": 0, "provisioningProfile": {"provisioningProfileId": id}}))
            }
            (Method::Get, url)
                if url.starts_with("/QH65B2/account/ios/profile/downloadProfileContent") =>
            {
                let id = url.rsplit("provisioningProfileId=").next().unwrap();
                let made = state
                    .made_profiles
                    .iter()
                    .find(|(made, ..)| made == id)
                    .map(|(.., content)| content.clone());
                Response {
                    status: 200,
                    headers: Vec::new(),
                    body: made.unwrap_or_else(|| self.profile.clone()),
                }
            }
            _ => panic!(
                "unexpected request: {} {}",
                request.method.as_str(),
                request.url
            ),
        })
    }
}

/// The work directory, what signing returned, and the steps reported
type Outcome = (
    tempfile::TempDir,
    Result<warpsign_core::pipeline::SignedApp, PipelineError>,
    Vec<(Step, Option<String>)>,
);

/// Records step events, like the app's step list
#[derive(Default)]
struct Steps(Mutex<Vec<(Step, Option<String>)>>);

impl ProgressObserver for Steps {
    fn step_started(&self, _step: Step) {}
    fn step_log(&self, _step: Step, _message: &str) {}
    fn step_finished(&self, step: Step, _elapsed: Duration, error: Option<&str>) {
        self.0.lock().unwrap().push((step, error.map(String::from)));
    }
}

fn sign(fake: &Arc<FakePortal>, options: &SignOptions, team: &str) -> Outcome {
    sign_with(fake, options, team, |_| {})
}

/// As [`sign`], changing the app first
fn sign_with(
    fake: &Arc<FakePortal>,
    options: &SignOptions,
    team: &str,
    change: impl FnOnce(&Path),
) -> Outcome {
    let dir = tempfile::tempdir().unwrap();
    let ipa = store_ipa_with(dir.path(), change);
    let session = SessionState {
        csrf: Some("t".into()),
        csrf_ts: Some("1".into()),
        ..Default::default()
    };
    let portal = Portal::new(
        Arc::new(HttpClient::with_state(fake.clone(), session)),
        team,
    );
    let steps = Steps::default();
    let result = sign_ipa(
        &portal,
        &identity(),
        &ipa,
        &dir.path().join("Signed.ipa"),
        &dir.path().join("work"),
        options,
        &steps,
    );
    let steps = steps.0.into_inner().unwrap();
    (dir, result, steps)
}

fn string_array(value: &Value) -> Vec<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_string().unwrap().to_string())
        .collect()
}

#[test]
fn signs_for_the_signed_in_team() {
    let fake = FakePortal::new("no-ppq.mobileprovision");
    let (dir, result, steps) = sign(&fake, &SignOptions::default(), TEAM);
    let signed = result.unwrap();

    // Every step ran, in order, without errors
    assert_eq!(
        steps.iter().map(|(step, _)| *step).collect::<Vec<_>>(),
        SIGN_STEPS
    );
    assert!(steps.iter().all(|(_, error)| error.is_none()), "{steps:?}");

    let state = fake.state.lock().unwrap();
    // A new identity for everything: nothing keeps the original IDs
    let new_id = &signed.bundle_id;
    assert_ne!(new_id, "com.example.test");
    assert_eq!(state.bundle_ids.len(), 2);
    assert_eq!(&state.bundle_ids[0], new_id);
    assert!(
        state.bundle_ids[1].starts_with(&format!("{new_id}.")),
        "{:?}",
        state.bundle_ids
    );
    assert_eq!(state.app_groups.len(), 1);
    let group = &state.app_groups[0];
    assert!(group.starts_with("group.") && group != "group.com.example.test");

    // App groups and push enabled, with the registered group linked
    let (bundle, update) = &state.capability_updates[0];
    assert_eq!(bundle, "BUNDLE1");
    let capabilities = update["data"]["relationships"]["bundleIdCapabilities"]["data"]
        .as_array()
        .unwrap();
    let app_groups = capabilities
        .iter()
        .find(|c| c["relationships"]["capability"]["data"]["id"] == "APP_GROUPS")
        .unwrap();
    assert_eq!(app_groups["attributes"]["enabled"], true);
    assert_eq!(
        app_groups["relationships"]["appGroups"]["data"][0]["id"],
        "GROUP1"
    );
    let push = capabilities
        .iter()
        .find(|c| c["relationships"]["capability"]["data"]["id"] == "PUSH_NOTIFICATIONS")
        .unwrap();
    assert_eq!(push["attributes"]["enabled"], true);

    // Development profiles with the certificate and device, named like
    // Python warpsign's
    assert_eq!(state.profile_forms.len(), 2);
    assert!(state.profile_forms[0].contains("distributionType=limited"));
    assert_eq!(
        form_value(&state.profile_forms[0], "certificateIds"),
        "CERT1"
    );
    assert_eq!(form_value(&state.profile_forms[0], "deviceIds"), "DEV1");
    assert_eq!(
        form_value(&state.profile_forms[0], "provisioningProfileName"),
        format!("TS {new_id} Development")
    );
    assert_eq!(signed.profiles.len(), 2);
    assert!(!signed.profiles[0].ppq_check);

    // The signed IPA
    let app = extract_ipa(&signed.output, &dir.path().join("check")).unwrap();
    let info = Value::from_file(app.join("Info.plist")).unwrap();
    assert_eq!(
        info.as_dictionary().unwrap()["CFBundleIdentifier"].as_string(),
        Some(new_id.as_str())
    );
    let profile = std::fs::read(data_dir().join("profiles/no-ppq.mobileprovision")).unwrap();
    assert_eq!(
        std::fs::read(app.join("embedded.mobileprovision")).unwrap(),
        profile
    );
    assert_eq!(
        std::fs::read(app.join("PlugIns/Widget.appex/embedded.mobileprovision")).unwrap(),
        profile
    );

    let main = read_signature(&app.join("Test")).unwrap().unwrap();
    assert_eq!(main.team_id.as_deref(), Some(TEAM));
    let entitlements = main.entitlements;
    assert_eq!(
        entitlements["application-identifier"].as_string(),
        Some(format!("{TEAM}.{new_id}").as_str())
    );
    assert_eq!(
        string_array(&entitlements["com.apple.security.application-groups"]),
        std::slice::from_ref(group)
    );
    assert_eq!(
        entitlements["aps-environment"].as_string(),
        Some("development")
    );
    assert!(!entitlements.contains_key("get-task-allow"));

    let widget = read_signature(&app.join("PlugIns/Widget.appex/Widget"))
        .unwrap()
        .unwrap();
    assert_eq!(widget.team_id.as_deref(), Some(TEAM));
    let framework = read_signature(&app.join("Frameworks/Foo.framework/Foo"))
        .unwrap()
        .unwrap();
    assert_eq!(framework.team_id.as_deref(), Some(TEAM));

    // What each component was signed with, as front-ends show it
    let paths: Vec<_> = signed.components.iter().map(|c| c.path.as_str()).collect();
    assert_eq!(
        paths,
        ["", "Frameworks/Foo.framework", "PlugIns/Widget.appex"]
    );
    let main = &signed.components[0];
    assert!(main.is_primary);
    assert_eq!(main.original_bundle_id, "com.example.test");
    assert_eq!(&main.bundle_id, new_id);
    // Exactly what's in the signature, before and after
    assert_eq!(main.entitlements, entitlements);
    assert_eq!(
        main.original_entitlements["application-identifier"].as_string(),
        Some(format!("{ORIGINAL_TEAM}.com.example.test").as_str())
    );
    assert_eq!(main.capabilities, ["APP_GROUPS", "PUSH_NOTIFICATIONS"]);
    let change = |key: &str, kind, reason| EntitlementChange {
        key: key.into(),
        kind,
        reason: Some(reason),
    };
    assert_eq!(
        main.changes,
        [
            change(
                "application-identifier",
                ChangeKind::Changed,
                ChangeReason::Remapped
            ),
            change(
                "aps-environment",
                ChangeKind::Changed,
                ChangeReason::PushEnvironment
            ),
            change(
                "com.apple.developer.team-identifier",
                ChangeKind::Changed,
                ChangeReason::Remapped
            ),
            change(
                "com.apple.security.application-groups",
                ChangeKind::Changed,
                ChangeReason::Remapped
            ),
            change(
                "get-task-allow",
                ChangeKind::Removed,
                ChangeReason::DebuggingOff
            ),
            change(
                "keychain-access-groups",
                ChangeKind::Changed,
                ChangeReason::Remapped
            ),
        ]
    );
    // Names from the portal's capabilities, and built-in ones for the rest
    let push = &main.names["aps-environment"];
    assert_eq!(push.name, "Push Notifications");
    assert_eq!(push.capability.as_deref(), Some("PUSH_NOTIFICATIONS"));
    assert_eq!(main.names["get-task-allow"].name, "Debugging");
    assert_eq!(main.names["keychain-access-groups"].capability, None);
    // The response that named them, for front-ends to cache
    assert!(signed.capabilities_response["data"].is_array());

    let framework = &signed.components[1];
    assert!(!framework.is_primary);
    assert_eq!(framework.bundle_id, "Foo");
    assert!(framework.entitlements.is_empty() && framework.changes.is_empty());

    let widget = &signed.components[2];
    assert_eq!(widget.bundle_id, state.bundle_ids[1]);
    assert_eq!(widget.entitlements, widget_signature(&app));
    assert_eq!(widget.capabilities, ["APP_GROUPS"]);
}

fn widget_signature(app: &Path) -> Dictionary {
    read_signature(&app.join("PlugIns/Widget.appex/Widget"))
        .unwrap()
        .unwrap()
        .entitlements
}

#[test]
fn refuses_original_ids_with_a_ppq_profile() {
    let fake = FakePortal::new("ppq.mobileprovision");
    let options = SignOptions {
        patching: warpsign_core::info_plist::PatchingOptions {
            force_original_id: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let (dir, result, steps) = sign(&fake, &options, TEAM);
    let error = result.unwrap_err();
    assert!(matches!(error, PipelineError::PpqRisk(_)), "{error}");
    // Stopped at the profiles, before anything was signed
    let (last, last_error) = steps.last().unwrap();
    assert_eq!(*last, Step::CreateProfiles);
    assert!(
        last_error.as_deref().unwrap().contains("PPQ"),
        "{last_error:?}"
    );
    assert!(!dir.path().join("Signed.ipa").exists());

    // Without keeping the original ID, a PPQ profile is fine
    let fake = FakePortal::new("ppq.mobileprovision");
    let (_dir, result, _) = sign(&fake, &SignOptions::default(), TEAM);
    assert!(result.unwrap().profiles[0].ppq_check);
}

#[test]
fn keeps_the_original_bundle_id_when_asked() {
    let fake = FakePortal::new("no-ppq.mobileprovision");
    let options = SignOptions {
        patching: warpsign_core::info_plist::PatchingOptions {
            force_original_id: true,
            ..Default::default()
        },
        patch_debug: true,
        ..Default::default()
    };
    let (dir, result, _) = sign(&fake, &options, TEAM);
    let signed = result.unwrap();
    assert_eq!(signed.bundle_id, "com.example.test");
    let app = extract_ipa(&signed.output, &dir.path().join("check")).unwrap();
    let entitlements = read_signature(&app.join("Test"))
        .unwrap()
        .unwrap()
        .entitlements;
    assert_eq!(entitlements["get-task-allow"].as_boolean(), Some(true));

    let main = &signed.components[0];
    assert_eq!(main.bundle_id, "com.example.test");
    assert_eq!(main.entitlements, entitlements);
    let debug = main
        .changes
        .iter()
        .find(|change| change.key == "get-task-allow")
        .unwrap();
    assert_eq!(debug.kind, ChangeKind::Changed);
    assert_eq!(debug.reason, Some(ChangeReason::DebuggingOn));
    // The widget had no get-task-allow: it's added
    let widget = &signed.components[2];
    assert!(widget.changes.contains(&EntitlementChange {
        key: "get-task-allow".into(),
        kind: ChangeKind::Added,
        reason: Some(ChangeReason::DebuggingOn),
    }));
}

#[test]
fn reuses_profiles_by_name() {
    // The App ID is stable, so a first run tells us the profile name
    let fake = FakePortal::new("no-ppq.mobileprovision");
    let (_dir, result, _) = sign(&fake, &SignOptions::default(), TEAM);
    let name = result.unwrap().profiles[0].name.clone();

    let fake = FakePortal::new("no-ppq.mobileprovision").with(|fake| {
        fake.profiles = json!({"data": [{"id": "OLD1", "attributes": {"name": name, "profileState": "INVALID", "platform": "IOS", "profileTypeLabel": "iOS App Development"}}]});
    });
    let (_dir, result, _) = sign(&fake, &SignOptions::default(), TEAM);
    result.unwrap();
    let state = fake.state.lock().unwrap();
    assert_eq!(
        form_value(&state.profile_forms[0], "provisioningProfileId"),
        "OLD1"
    );
    assert_eq!(
        form_value(&state.profile_forms[1], "provisioningProfileId"),
        ""
    );
}

#[test]
fn explains_account_problems() {
    // Signed in to another team than the certificate's
    let fake = FakePortal::new("no-ppq.mobileprovision");
    let (_dir, result, steps) = sign(&fake, &SignOptions::default(), "OTHER12345");
    let error = result.unwrap_err();
    assert!(
        error.to_string().contains("belongs to team ZZZZZ99999"),
        "{error}"
    );
    assert!(steps.is_empty());
    assert!(fake.state.lock().unwrap().requests.is_empty());

    let fake =
        FakePortal::new("no-ppq.mobileprovision").with(|fake| fake.devices = json!({"data": []}));
    let error = sign(&fake, &SignOptions::default(), TEAM).1.unwrap_err();
    assert!(error.to_string().contains("no iPhones or iPads"), "{error}");

    let fake = FakePortal::new("no-ppq.mobileprovision")
        .with(|fake| fake.certificates = json!({"data": []}));
    let error = sign(&fake, &SignOptions::default(), TEAM).1.unwrap_err();
    assert!(
        error.to_string().contains("may have been revoked"),
        "{error}"
    );
}

// Signing again: only what changed reaches the portal (not in Python
// warpsign, which re-registers and regenerates everything; see
// warpsign_core::profile_reuse)

/// Requests that change something on the portal
fn writes(requests: &[Request]) -> Vec<String> {
    requests
        .iter()
        .filter(|request| {
            let reading = request.header_value("X-HTTP-Method-Override") == Some("GET");
            let url = request.url.as_str();
            request.method == Method::Patch
                || (url.ends_with("/v1/bundleIds") && !reading)
                || url.contains("addApplicationGroup")
                || url.contains("addCloudContainer")
                || url.contains("regenProvisioningProfile")
        })
        .map(|request| format!("{} {}", request.method.as_str(), request.url))
        .collect()
}

fn downloads(requests: &[Request]) -> usize {
    requests
        .iter()
        .filter(|request| request.url.contains("downloadProfileContent"))
        .count()
}

fn realistic_portal() -> Arc<FakePortal> {
    FakePortal::new("no-ppq.mobileprovision").with(|fake| fake.realistic_profiles = true)
}

/// Sign once, then return how many requests that took
fn signed_once(
    fake: &Arc<FakePortal>,
    options: &SignOptions,
) -> (usize, warpsign_core::pipeline::SignedApp) {
    let (_dir, result, _) = sign(fake, options, TEAM);
    let signed = result.unwrap();
    let state = fake.state.lock().unwrap();
    assert!(
        !writes(&state.requests).is_empty(),
        "the first sign registers things"
    );
    (state.requests.len(), signed)
}

#[test]
fn signing_an_unchanged_app_again_changes_nothing() {
    let fake = realistic_portal();
    let cache = tempfile::tempdir().unwrap();
    let options = SignOptions {
        profile_cache: Some(cache.path().to_path_buf()),
        ..Default::default()
    };
    let (before, _) = signed_once(&fake, &options);

    let (_dir, result, _) = sign(&fake, &options, TEAM);
    let signed = result.unwrap();
    let state = fake.state.lock().unwrap();
    let again = &state.requests[before..];
    assert_eq!(writes(again), Vec::<String>::new());
    // Both profiles came from the cache
    assert_eq!(downloads(again), 0);
    assert_eq!(signed.profiles.len(), 2);
}

#[test]
fn without_a_cache_profiles_are_downloaded_but_nothing_changes() {
    let fake = realistic_portal();
    let options = SignOptions::default();
    let (before, _) = signed_once(&fake, &options);

    let (_dir, result, _) = sign(&fake, &options, TEAM);
    result.unwrap();
    let state = fake.state.lock().unwrap();
    let again = &state.requests[before..];
    assert_eq!(writes(again), Vec::<String>::new());
    assert_eq!(downloads(again), 2);
}

#[test]
fn a_new_device_regenerates_profiles_without_touching_app_ids() {
    let fake = realistic_portal();
    let cache = tempfile::tempdir().unwrap();
    let options = SignOptions {
        profile_cache: Some(cache.path().to_path_buf()),
        ..Default::default()
    };
    let (before, _) = signed_once(&fake, &options);
    fake.state
        .lock()
        .unwrap()
        .extra_devices
        .push("00008030-0002".into());

    let (_dir, result, _) = sign(&fake, &options, TEAM);
    result.unwrap();
    let state = fake.state.lock().unwrap();
    let again = writes(&state.requests[before..]);
    assert_eq!(again.len(), 2, "{again:?}");
    assert!(
        again
            .iter()
            .all(|write| write.contains("regenProvisioningProfile")),
        "{again:?}"
    );
}

#[test]
fn an_invalid_profile_sets_capabilities_and_regenerates() {
    let fake = realistic_portal();
    let cache = tempfile::tempdir().unwrap();
    let options = SignOptions {
        profile_cache: Some(cache.path().to_path_buf()),
        ..Default::default()
    };
    let (before, _) = signed_once(&fake, &options);
    // As the portal marks profiles after their App ID changes
    for profile in &mut fake.state.lock().unwrap().made_profiles {
        profile.2 = "INVALID".into();
    }

    let (_dir, result, _) = sign(&fake, &options, TEAM);
    result.unwrap();
    let state = fake.state.lock().unwrap();
    let again = writes(&state.requests[before..]);
    let patches = again
        .iter()
        .filter(|write| write.starts_with("PATCH"))
        .count();
    let regenerated = again
        .iter()
        .filter(|write| write.contains("regenProvisioningProfile"))
        .count();
    assert_eq!((patches, regenerated), (2, 2), "{again:?}");
    // App IDs and the app group were found, not registered again
    assert!(
        !again
            .iter()
            .any(|write| write.ends_with("/v1/bundleIds") || write.contains("addApplicationGroup"))
    );
}

#[test]
fn a_profile_missing_an_entitlement_updates_only_its_app_id() {
    let fake = realistic_portal();
    let options = SignOptions::default();
    let (before, signed) = signed_once(&fake, &options);
    // The main app's profile no longer grants its app group, as if the
    // group were unlinked on the portal
    let main = signed
        .profiles
        .iter()
        .find(|profile| profile.bundle_path.is_empty())
        .unwrap();
    {
        let mut state = fake.state.lock().unwrap();
        let profile = state
            .made_profiles
            .iter_mut()
            .find(|(_, name, ..)| name == &main.name)
            .unwrap();
        profile.3 = make_profile(
            &main.name,
            &format!("{TEAM}.{}", main.bundle_id),
            &[],
            &["00008110-0001".to_string()],
        );
    }

    let (_dir, result, _) = sign(&fake, &options, TEAM);
    result.unwrap();
    let state = fake.state.lock().unwrap();
    let again = writes(&state.requests[before..]);
    let patches = again
        .iter()
        .filter(|write| write.starts_with("PATCH"))
        .count();
    let regenerated: Vec<&String> = state.profile_forms[2..].iter().collect();
    assert_eq!(patches, 1, "{again:?}");
    assert_eq!(regenerated.len(), 1);
    assert!(
        regenerated[0].contains(&main.name.replace(' ', "+")),
        "{regenerated:?}"
    );
}

#[test]
fn names_bundles_that_got_no_profile() {
    // An app somewhere unusual: not a component Python warpsign finds, so
    // it gets no profile of its own
    let fake = realistic_portal();
    let (_dir, result, _) = sign_with(&fake, &SignOptions::default(), TEAM, |app| {
        copy_dir(
            &app.join("PlugIns/Widget.appex"),
            &app.join("Helpers/Helper.app"),
        );
    });
    let signed = result.unwrap();
    assert!(
        signed
            .warnings
            .iter()
            .any(|warning| warning.starts_with("Helpers/Helper.app has no provisioning profile")),
        "{:?}",
        signed.warnings
    );

    // A normal app has nothing to warn about
    let (_dir, result, _) = sign(&realistic_portal(), &SignOptions::default(), TEAM);
    let warnings = result.unwrap().warnings;
    assert!(!warnings.iter().any(|w| w.contains("iOS")), "{warnings:?}");
}

#[test]
fn removes_app_clips_and_watch_apps() {
    let fake = realistic_portal();
    let (dir, result, _) = sign_with(&fake, &SignOptions::default(), TEAM, |app| {
        copy_dir(
            &app.join("PlugIns/Widget.appex"),
            &app.join("AppClips/Clip.app"),
        );
        copy_dir(
            &app.join("PlugIns/Widget.appex"),
            &app.join("Watch/Watch.app"),
        );
    });
    let signed = result.unwrap();
    // Nothing left to warn about, and no App IDs for them
    assert!(
        !signed.warnings.iter().any(|w| w.contains("iOS")),
        "{:?}",
        signed.warnings
    );
    assert_eq!(signed.profiles.len(), 2);

    let out = dir.path().join("out");
    let app = extract_ipa(&signed.output, &out).unwrap();
    assert!(!app.join("AppClips").exists());
    assert!(!app.join("Watch").exists());
}

#[test]
fn names_entitlements_the_profile_does_not_grant() {
    // The test profile grants none of this app's identifiers
    let fake = FakePortal::new("no-ppq.mobileprovision");
    let (_dir, result, _) = sign(&fake, &SignOptions::default(), TEAM);
    let warnings = result.unwrap().warnings;
    let expected = [
        "App: its provisioning profile doesn't grant com.apple.security.application-groups",
        "App: its provisioning profile doesn't grant aps-environment (development)",
        "PlugIns/Widget.appex: its provisioning profile doesn't grant application-identifier",
    ];
    for start in expected {
        assert!(
            warnings.iter().any(|warning| warning.starts_with(start)),
            "{start} in {warnings:?}"
        );
    }
}

#[test]
fn drops_entitlements_apple_has_not_granted() {
    // Push Notifications as if it had to be requested from Apple (as
    // Family Controls (Distribution) does), and the profile doesn't grant it
    let mut list = FakePortal::capabilities();
    for capability in list["data"].as_array_mut().unwrap() {
        if capability["id"] == "PUSH_NOTIFICATIONS" {
            capability["attributes"]["canRequestFromPortal"] = json!(true);
            capability["attributes"]["requestFormUrl"] =
                json!("https://developer.apple.com/contact/request/push/");
        }
    }
    let fake =
        FakePortal::new("no-ppq.mobileprovision").with(|fake| fake.capability_list = Some(list));
    let (_dir, result, _) = sign(&fake, &SignOptions::default(), TEAM);
    let signed = result.unwrap();

    assert!(
        signed.warnings.iter().any(|warning| warning
            == "App: removed aps-environment, as Apple hasn't granted your team Push Notifications. Request it at https://developer.apple.com/contact/request/push/"),
        "{:?}",
        signed.warnings
    );
    // Nothing else is dropped for it: an ungranted app group still only warns
    assert!(
        signed.warnings.iter().any(|warning| warning.starts_with(
            "App: its provisioning profile doesn't grant com.apple.security.application-groups"
        )),
        "{:?}",
        signed.warnings
    );
    let main = &signed.components[0];
    let main = if main.path.is_empty() {
        main
    } else {
        signed
            .components
            .iter()
            .find(|c| c.path.is_empty())
            .unwrap()
    };
    assert!(!main.entitlements.contains_key("aps-environment"));
    let change = main
        .changes
        .iter()
        .find(|change| change.key == "aps-environment")
        .unwrap();
    assert_eq!(change.kind, ChangeKind::Removed);
    assert_eq!(change.reason, Some(ChangeReason::NotGranted));
}
