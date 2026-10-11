//! Replays Python warpsign's Developer Portal calls (recorded by
//! `tools/gen_fixtures.py` into fixtures/portal_requests.json) through the
//! port, checking it sends the same requests and reads the same results.

mod support;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use base64::Engine;
use serde_json::{Value, json};
use support::{MockApple, response};
use warpsign_core::ProfileType;
use warpsign_core::portal::Portal;
use warpsign_core::portal::api::{BundleId, fetch_teams};
use warpsign_core::portal::http::{HttpClient, Request, SessionState};

const TEAM: &str = "ZZZZZ99999";

fn fixture() -> Value {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/portal_requests.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn case(name: &str) -> Value {
    fixture()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == name)
        .unwrap_or_else(|| panic!("no case {name}"))
        .clone()
}

/// A portal session replaying the case's responses
fn replay(case: &Value) -> (Arc<MockApple>, Arc<HttpClient>) {
    let mock = MockApple::new();
    for canned in case["responses"].as_array().unwrap() {
        let body = match canned.get("json") {
            Some(json) => serde_json::to_vec(json).unwrap(),
            None => canned["text"].as_str().unwrap_or("").as_bytes().to_vec(),
        };
        let headers: Vec<(String, String)> = canned
            .get("headers")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string()))
            .collect();
        let headers: Vec<(&str, &str)> = headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let canned = response(canned["status"].as_u64().unwrap() as u16, &headers, body);
        // URLs are compared in full below, so match anything here
        mock.respond("https://", canned);
    }
    let session = SessionState {
        csrf: Some("csrf-token".into()),
        csrf_ts: Some("1700000000000".into()),
        ..Default::default()
    };
    let http = Arc::new(HttpClient::with_state(mock.clone(), session));
    (mock, http)
}

/// The request as Python recorded it: transport-owned headers dropped, and
/// JSON bodies parsed
fn recorded(request: &Request) -> Value {
    let headers: serde_json::Map<String, Value> = request
        .headers
        .iter()
        .filter(|(name, _)| {
            ![
                "user-agent",
                "accept-encoding",
                "connection",
                "host",
                "content-length",
                "cookie",
            ]
            .contains(&name.to_ascii_lowercase().as_str())
        })
        .map(|(k, v)| (k.clone(), Value::String(v.clone())))
        .collect();
    let mut value = json!({
        "method": request.method.as_str(),
        "url": request.url,
        "headers": headers,
    });
    let is_json = request
        .header_value("Content-Type")
        .is_some_and(|t| t.contains("json"));
    if is_json && !request.body.is_empty() {
        value["json"] = request.json_body().unwrap();
    } else {
        value["body"] = Value::String(String::from_utf8(request.body.clone()).unwrap());
    }
    value
}

fn assert_same_requests(case: &Value, mock: &MockApple) {
    mock.assert_done();
    let sent: Vec<Value> = mock.sent().iter().map(recorded).collect();
    let expected = case["requests"].as_array().unwrap();
    assert_eq!(
        sent.len(),
        expected.len(),
        "{}: request count",
        case["name"]
    );
    for (index, (sent, expected)) in sent.iter().zip(expected).enumerate() {
        assert_eq!(sent, expected, "{}: request {index}", case["name"]);
    }
}

fn bundle() -> BundleId {
    BundleId {
        id: "BUNDLE1".into(),
        identifier: "com.example.app".into(),
        name: "WS App app".into(),
    }
}

#[test]
fn teams() {
    let case = case("teams");
    let (mock, http) = replay(&case);
    let teams = fetch_teams(&http).unwrap();
    assert_same_requests(&case, &mock);
    let expected = &case["result"][0];
    assert_eq!(teams[0].team_id, expected["team_id"]);
    assert_eq!(teams[0].kind, expected["type"]);
    assert_eq!(teams[0].roles, ["TEAM_ADMIN"]);
}

#[test]
fn lists() {
    for name in [
        "certificates",
        "bundle_ids",
        "app_groups",
        "cloud_containers",
        "devices",
        "profiles",
    ] {
        let case = case(name);
        let (mock, http) = replay(&case);
        let portal = Portal::new(http, TEAM);
        let ids: Vec<String> = match name {
            "certificates" => {
                let certificates = portal.certificates().unwrap();
                assert_eq!(
                    certificates[0].serial_number,
                    case["result"][0]["serial_number"]
                );
                certificates.into_iter().map(|c| c.id).collect()
            }
            "bundle_ids" => portal
                .bundle_ids()
                .unwrap()
                .into_iter()
                .map(|b| b.id)
                .collect(),
            "app_groups" => portal
                .app_groups()
                .unwrap()
                .into_iter()
                .map(|g| g.id)
                .collect(),
            "cloud_containers" => portal
                .cloud_containers()
                .unwrap()
                .into_iter()
                .map(|c| c.id)
                .collect(),
            "devices" => portal
                .devices()
                .unwrap()
                .into_iter()
                .map(|d| d.id)
                .collect(),
            "profiles" => portal
                .profiles()
                .unwrap()
                .into_iter()
                .map(|p| p.id)
                .collect(),
            _ => unreachable!(),
        };
        let expected: Vec<&str> = case["result"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, expected, "{name}");
        assert_same_requests(&case, &mock);
    }
}

#[test]
fn capabilities() {
    let case = case("capabilities");
    let (mock, http) = replay(&case);
    let capabilities = Portal::new(http, TEAM).capabilities().unwrap();
    assert_eq!(capabilities, case["result"]);
    assert_same_requests(&case, &mock);
}

#[test]
fn set_capabilities() {
    let case = case("set_capabilities");
    let (mock, http) = replay(&case);
    let args = &case["args"];
    let enable: Vec<String> =
        serde_json::from_value(args["capabilities_to_enable"].clone()).unwrap();
    let relationships: BTreeMap<String, Vec<String>> =
        serde_json::from_value(args["group_ids"].clone()).unwrap();
    let update = Portal::new(http, TEAM)
        .update_capabilities(&bundle(), &enable, &relationships)
        .unwrap();
    assert_eq!(update.required, ["IN_APP_PURCHASE"]);
    assert!(update.skipped.is_empty());
    assert_same_requests(&case, &mock);
}

#[test]
fn registrations() {
    for name in [
        "register_bundle_id",
        "register_existing_bundle_id",
        "register_app_group",
        "register_taken_app_group",
        "register_cloud_container",
        "register_taken_cloud_container",
    ] {
        let case = case(name);
        let (mock, http) = replay(&case);
        let portal = Portal::new(http, TEAM);
        let identifier = case["args"]["identifier"].as_str().unwrap();
        let display = case["args"]["name"].as_str().unwrap();
        let (id, registered) = match case["call"].as_str().unwrap() {
            "register_bundle_id" => {
                let bundle = portal.register_bundle_id(identifier, display).unwrap();
                (bundle.id, bundle.identifier)
            }
            "register_app_group" => {
                let group = portal.register_app_group(identifier, display).unwrap();
                (group.id, group.identifier)
            }
            "register_icloud_container" => {
                let container = portal
                    .register_cloud_container(identifier, display)
                    .unwrap();
                (container.id, container.identifier)
            }
            other => panic!("{other}"),
        };
        assert_eq!(id, case["result"]["id"], "{name}");
        assert_eq!(registered, identifier, "{name}");
        assert_same_requests(&case, &mock);
    }
}

#[test]
fn profiles() {
    for name in ["create_profile", "regenerate_adhoc_profile"] {
        let case = case(name);
        let (mock, http) = replay(&case);
        let args = &case["args"];
        let existing = args["profile_id"].as_str().filter(|id| !id.is_empty());
        let profile_type: ProfileType =
            args["distribution_type"].as_str().unwrap().parse().unwrap();
        let strings =
            |key: &str| -> Vec<String> { serde_json::from_value(args[key].clone()).unwrap() };
        let (_, content) = Portal::new(http, TEAM)
            .create_or_regenerate_profile(
                existing,
                &bundle(),
                args["profile_name"].as_str().unwrap(),
                &strings("certificate_ids"),
                &strings("device_ids"),
                profile_type,
            )
            .unwrap();
        let expected = base64::engine::general_purpose::STANDARD
            .decode(case["result"]["base64"].as_str().unwrap())
            .unwrap();
        assert_eq!(content, expected, "{name}");
        assert_same_requests(&case, &mock);
    }
}
