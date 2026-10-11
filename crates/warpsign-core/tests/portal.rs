//! Signing in and the portal behaviour Python warpsign didn't have, against
//! a scripted stand-in for Apple. Request shapes Python did have are checked
//! in portal_parity.rs.

mod support;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::{Value, json};
use sha1::{Digest, Sha1};
use support::{MockApple, json_response, response};
use warpsign_core::identity::CertificateKind;
use warpsign_core::portal::api::BundleId;
use warpsign_core::portal::http::{HttpClient, Method, SessionState};
use warpsign_core::portal::{AuthError, Authenticator, Portal, SignInOutcome};

const AUTH: &str = "https://idmsa.apple.com/appleauth/auth";
const SIGN_OUT_URL: &str = "https://appstoreconnect.apple.com/logout";
const WIDGET_KEY_URL: &str = "https://appstoreconnect.apple.com/olympus/v1/app/config";
const ACCOUNT_URL: &str = "https://developer.apple.com/account/resources";
const TEAMS_URL: &str = "https://developer.apple.com/services-account/QH65B2/account/getTeams";

fn fixture(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

/// Python's srp library values for an s2k sign-in
fn srp_case() -> Value {
    fixture("srp.json")["cases"][0].clone()
}

fn authenticator(mock: &Arc<MockApple>) -> Authenticator {
    Authenticator::new(mock.clone()).with_srp_secret(&unhex(srp_case()["a"].as_str().unwrap()))
}

fn header<'a>(request: &'a warpsign_core::portal::Request, name: &str) -> &'a str {
    request
        .header_value(name)
        .unwrap_or_else(|| panic!("no {name} header on {}", request.url))
}

/// The sign-out redirect that carries the widget key, as Apple answers now
fn script_widget_key(mock: &MockApple) {
    mock.expect(SIGN_OUT_URL, |request| {
        assert_eq!(request.method, Method::Head);
        assert_eq!(request.header_value("Cookie"), None);
        response(
            302,
            &[(
                "Location",
                "https://appstoreconnect.apple.com/login?targetUrl=%2F&widgetKey=widget-key",
            )],
            "",
        )
    });
}

/// Script the sign-in up to `signin/complete`, which `complete` answers
fn script_sign_in(
    mock: &MockApple,
    complete: impl Fn(&warpsign_core::portal::Request) -> warpsign_core::portal::Response
    + Send
    + 'static,
) {
    let case = srp_case();
    script_widget_key(mock);
    mock.expect(&format!("{AUTH}/signin?widgetKey=widget-key"), |request| {
        assert_eq!(request.method, Method::Get);
        response(
            200,
            &[
                ("X-Apple-HC-Bits", "10"),
                ("X-Apple-HC-Challenge", "4d74fb15eb23f465f1f6fcbf534e5877"),
                ("Set-Cookie", "aasp=first-visit; Path=/; Secure"),
            ],
            "<html></html>",
        )
    });
    let init_case = case.clone();
    mock.expect(&format!("{AUTH}/signin/init"), move |request| {
        assert_eq!(request.method, Method::Post);
        assert_eq!(header(request, "X-Apple-Widget-Key"), "widget-key");
        assert_eq!(header(request, "Content-Type"), "application/json");
        assert_eq!(header(request, "Cookie"), "aasp=first-visit");
        let body = request.json_body().unwrap();
        let a = BASE64.decode(body["a"].as_str().unwrap()).unwrap();
        assert_eq!(a, unhex(init_case["expected"]["A"].as_str().unwrap()));
        assert_eq!(body["accountName"], init_case["email"]);
        assert_eq!(body["protocols"], json!(["s2k", "s2k_fo"]));
        json_response(
            200,
            json!({
                "iteration": init_case["iterations"],
                "salt": BASE64.encode(unhex(init_case["salt"].as_str().unwrap())),
                "protocol": init_case["protocol"],
                "b": BASE64.encode(unhex(init_case["B"].as_str().unwrap())),
                "c": "challenge-c",
            }),
        )
    });
    mock.expect(
        &format!("{AUTH}/signin/complete?isRememberMeEnabled=true"),
        move |request| {
            let body = request.json_body().unwrap();
            let decode = |key: &str| BASE64.decode(body[key].as_str().unwrap()).unwrap();
            assert_eq!(
                decode("m1"),
                unhex(case["expected"]["M1"].as_str().unwrap())
            );
            assert_eq!(
                decode("m2"),
                unhex(case["expected"]["M2"].as_str().unwrap())
            );
            assert_eq!(body["c"], "challenge-c");
            assert_eq!(body["accountName"], case["email"]);
            assert_eq!(body["rememberMe"], true);

            // A solved hashcash for the challenge
            let hashcash = header(request, "X-Apple-HC");
            let fields: Vec<&str> = hashcash.split(':').collect();
            assert_eq!(fields[..2], ["1", "10"]);
            assert_eq!(fields[3], "4d74fb15eb23f465f1f6fcbf534e5877");
            let digest = Sha1::digest(hashcash.as_bytes());
            assert_eq!(digest[0], 0);
            assert!(digest[1] < 0x40, "{hashcash} doesn't have 10 zero bits");
            complete(request)
        },
    );
}

/// Script opening the portal after signing in
fn script_portal(mock: &MockApple, teams: Value) {
    mock.expect(ACCOUNT_URL, |request| {
        assert!(header(request, "Cookie").contains("myacinfo=signed-in"));
        response(
            302,
            &[
                ("Location", "/account/resources/certificates/list"),
                ("Set-Cookie", "csrf=csrf-token; Path=/"),
                ("Set-Cookie", "csrf_ts=1700000000000; Path=/"),
            ],
            "",
        )
    });
    mock.respond(
        "https://developer.apple.com/account/resources/certificates/list",
        response(200, &[], "<html>portal</html>"),
    );
    mock.expect(TEAMS_URL, move |request| {
        assert_eq!(
            request.json_body().unwrap(),
            json!({"includeInMigrationTeams": 1})
        );
        assert!(header(request, "Cookie").contains("myacinfo=signed-in"));
        json_response(200, json!({"resultCode": 0, "teams": teams.clone()}))
    });
}

fn one_team() -> Value {
    json!([{"teamId": "ZZZZZ99999", "name": "Test Team", "status": "active", "entityType": "INDIVIDUAL", "userRoles": ["TEAM_ADMIN"]}])
}

fn signed_in() -> warpsign_core::portal::Response {
    response(
        200,
        &[
            (
                "Set-Cookie",
                "myacinfo=signed-in; Domain=.apple.com; Path=/; Secure; HttpOnly",
            ),
            ("X-Apple-ID-Session-Id", "session-1"),
            ("scnt", "scnt-1"),
        ],
        "{}",
    )
}

#[test]
fn signs_in_without_two_factor() {
    let mock = MockApple::new();
    script_sign_in(&mock, |_| signed_in());
    script_portal(&mock, one_team());

    let auth = authenticator(&mock);
    let outcome = auth
        .sign_in("someone@example.com", "correct horse battery staple")
        .unwrap();
    mock.assert_done();
    let SignInOutcome::SignedIn { teams } = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(teams[0].team_id, "ZZZZZ99999");

    let session = auth.session();
    assert_eq!(session.csrf.as_deref(), Some("csrf-token"));
    assert_eq!(session.csrf_ts.as_deref(), Some("1700000000000"));
    assert_eq!(session.cookie("myacinfo"), Some("signed-in"));
    // The session survives being saved
    let saved = serde_json::to_string(&session).unwrap();
    assert_eq!(
        serde_json::from_str::<SessionState>(&saved).unwrap(),
        session
    );
}

#[test]
fn signs_in_with_a_trusted_device_code() {
    let mock = MockApple::new();
    script_sign_in(&mock, |_| {
        response(
            409,
            &[("X-Apple-ID-Session-Id", "session-1"), ("scnt", "scnt-1")],
            r#"{"authType":"hsa2"}"#,
        )
    });
    mock.expect(AUTH, |request| {
        assert_eq!(request.url, AUTH);
        assert_eq!(header(request, "X-Apple-ID-Session-Id"), "session-1");
        assert_eq!(header(request, "scnt"), "scnt-1");
        json_response(
            200,
            json!({
                "trustedPhoneNumbers": [{"id": 1, "numberWithDialCode": "+44 ••••• ••••12", "pushMode": "sms"}],
                "securityCode": {"length": 6},
                "noTrustedDevices": false,
                "authenticationType": "hsa2",
            }),
        )
    });

    let auth = authenticator(&mock);
    let outcome = auth
        .sign_in("someone@example.com", "correct horse battery staple")
        .unwrap();
    let SignInOutcome::NeedsCode(challenge) = outcome else {
        panic!("{outcome:?}");
    };
    assert!(challenge.trusted_devices);
    assert_eq!(challenge.code_length, 6);
    assert_eq!(challenge.phone_numbers[0].id, 1);
    assert_eq!(challenge.phone_numbers[0].number, "+44 ••••• ••••12");
    assert_eq!(challenge.sms_sent_to, None);
    mock.assert_done();

    // A wrong code, then the right one
    mock.respond(
        &format!("{AUTH}/verify/trusteddevice/securitycode"),
        json_response(
            400,
            json!({"serviceErrors": [{"code": "-21669", "message": "Incorrect verification code."}]}),
        ),
    );
    let error = auth.verify_device_code("000000").unwrap_err();
    assert_eq!(
        error,
        AuthError::IncorrectCode("Incorrect verification code. (-21669)".into())
    );

    mock.expect(
        &format!("{AUTH}/verify/trusteddevice/securitycode"),
        |request| {
            assert_eq!(
                request.json_body().unwrap(),
                json!({"securityCode": {"code": "123456"}})
            );
            assert_eq!(header(request, "X-Apple-ID-Session-Id"), "session-1");
            assert_eq!(header(request, "scnt"), "scnt-1");
            response(204, &[("scnt", "scnt-2")], "")
        },
    );
    mock.expect(&format!("{AUTH}/2sv/trust"), |request| {
        // The latest scnt Apple handed out
        assert_eq!(header(request, "scnt"), "scnt-2");
        response(
            204,
            &[(
                "Set-Cookie",
                "myacinfo=signed-in; Domain=.apple.com; Path=/",
            )],
            "",
        )
    });
    // CSRF tokens from the page's script this time
    mock.respond(
        ACCOUNT_URL,
        response(
            200,
            &[],
            r#"<script>var config = {"csrf": "page-token", "csrf_ts": "1700000000001"};</script>"#,
        ),
    );
    mock.respond(
        TEAMS_URL,
        json_response(200, json!({"resultCode": 0, "teams": one_team()})),
    );

    let teams = auth.verify_device_code(" 123456 ").unwrap();
    assert_eq!(teams.len(), 1);
    assert_eq!(auth.session().csrf.as_deref(), Some("page-token"));
    mock.assert_done();
}

#[test]
fn signs_in_with_a_text_message_code() {
    let mock = MockApple::new();
    script_sign_in(&mock, |_| {
        response(
            409,
            &[("X-Apple-ID-Session-Id", "session-1"), ("scnt", "scnt-1")],
            "{}",
        )
    });
    mock.respond(
        AUTH,
        json_response(
            200,
            json!({
                "trustedPhoneNumbers": [
                    {"id": 1, "numberWithDialCode": "+44 ••••• ••••12"},
                    {"id": 2, "numberWithDialCode": "+1 (•••) •••-••34"},
                ],
                "trustedPhoneNumber": {"id": 1, "numberWithDialCode": "+44 ••••• ••••12"},
                "securityCode": {"length": 6},
                "noTrustedDevices": true,
            }),
        ),
    );
    let auth = authenticator(&mock);
    let SignInOutcome::NeedsCode(challenge) = auth
        .sign_in("someone@example.com", "correct horse battery staple")
        .unwrap()
    else {
        panic!();
    };
    assert!(!challenge.trusted_devices);
    assert_eq!(challenge.sms_sent_to.as_ref().map(|p| p.id), Some(1));
    assert_eq!(challenge.phone_numbers.len(), 2);

    mock.expect(&format!("{AUTH}/verify/phone"), |request| {
        assert_eq!(request.method, Method::Put);
        assert_eq!(
            request.json_body().unwrap(),
            json!({"phoneNumber": {"id": 2}, "mode": "sms"})
        );
        json_response(200, json!({}))
    });
    auth.request_sms(2).unwrap();

    mock.expect(&format!("{AUTH}/verify/phone/securitycode"), |request| {
        assert_eq!(
            request.json_body().unwrap(),
            json!({"securityCode": {"code": "654321"}, "phoneNumber": {"id": 2}, "mode": "sms"})
        );
        json_response(200, json!({}))
    });
    mock.respond(&format!("{AUTH}/2sv/trust"), response(204, &[], ""));
    mock.respond(
        ACCOUNT_URL,
        response(200, &[("csrf", "header-token"), ("csrf_ts", "1")], "<html>"),
    );
    mock.respond(
        TEAMS_URL,
        json_response(200, json!({"resultCode": 0, "teams": one_team()})),
    );
    auth.verify_sms_code(2, "654321").unwrap();
    assert_eq!(auth.session().csrf.as_deref(), Some("header-token"));
    mock.assert_done();
}

#[test]
fn falls_back_to_the_olympus_config_for_the_widget_key() {
    let mock = MockApple::new();
    mock.respond(SIGN_OUT_URL, response(404, &[], ""));
    mock.respond(
        WIDGET_KEY_URL,
        json_response(200, json!({"authServiceKey": "olympus-key"})),
    );
    mock.respond(
        &format!("{AUTH}/signin?widgetKey=olympus-key"),
        response(200, &[], "<html></html>"),
    );
    mock.expect(&format!("{AUTH}/signin/init"), |request| {
        assert_eq!(header(request, "X-Apple-Widget-Key"), "olympus-key");
        json_response(403, json!({"serviceErrors": [{"message": "Stop here."}]}))
    });
    let error = authenticator(&mock)
        .sign_in("someone@example.com", "correct horse battery staple")
        .unwrap_err();
    assert_eq!(error, AuthError::Refused("Stop here.".into()));
    mock.assert_done();
}

#[test]
fn reports_a_missing_widget_key() {
    let mock = MockApple::new();
    // Apple's sign-out redirect without a key, and the retired endpoint
    mock.respond(
        SIGN_OUT_URL,
        response(
            302,
            &[("Location", "https://appstoreconnect.apple.com/login")],
            "",
        ),
    );
    mock.respond(WIDGET_KEY_URL, response(404, &[], ""));
    let error = authenticator(&mock)
        .sign_in("someone@example.com", "correct horse battery staple")
        .unwrap_err();
    assert_eq!(
        error,
        AuthError::Unexpected("no sign-in service key from App Store Connect (HTTP 404)".into())
    );
    mock.assert_done();
}

#[test]
fn a_refused_sign_in_is_final() {
    let mock = MockApple::new();
    script_sign_in(&mock, |_| {
        json_response(
            403,
            json!({"serviceErrors": [{"code": "-20209", "message": "This Apple ID has been locked for security reasons."}]}),
        )
    });
    let error = authenticator(&mock)
        .sign_in("someone@example.com", "correct horse battery staple")
        .unwrap_err();
    assert_eq!(
        error,
        AuthError::Refused("This Apple ID has been locked for security reasons. (-20209)".into())
    );
    // Nothing after the refusal: no retry
    mock.assert_done();
    assert_eq!(mock.sent().len(), 4);
}

#[test]
fn reports_wrong_passwords_and_pending_actions() {
    for (status, check) in [
        (
            401,
            (|e: &AuthError| matches!(e, AuthError::InvalidCredentials(m) if m.contains("incorrect")))
                as fn(&AuthError) -> bool,
        ),
        (412, |e: &AuthError| {
            matches!(e, AuthError::ActionRequired(_))
        }),
    ] {
        let mock = MockApple::new();
        script_sign_in(&mock, move |_| {
            json_response(
                status,
                json!({"serviceErrors": [{"code": "-20101", "message": "Your Apple ID or password was incorrect."}]}),
            )
        });
        let error = authenticator(&mock)
            .sign_in("someone@example.com", "correct horse battery staple")
            .unwrap_err();
        if status == 401 {
            assert_eq!(
                error.to_string(),
                "Your Apple ID or password was incorrect. (-20101)"
            );
        } else {
            assert!(check(&error), "{error:?}");
        }
        mock.assert_done();
    }
}

#[test]
fn rejects_accounts_without_a_team() {
    let mock = MockApple::new();
    script_sign_in(&mock, |_| signed_in());
    script_portal(&mock, json!([]));
    let error = authenticator(&mock)
        .sign_in("someone@example.com", "correct horse battery staple")
        .unwrap_err();
    assert_eq!(error, AuthError::NoTeams);
    assert!(
        error
            .to_string()
            .contains("free Apple accounts aren't supported")
    );
}

#[test]
fn rejects_older_two_step_verification() {
    let mock = MockApple::new();
    script_sign_in(&mock, |_| response(409, &[], "{}"));
    mock.respond(
        AUTH,
        json_response(
            200,
            json!({"authenticationType": "hsa", "trustedDevices": []}),
        ),
    );
    let error = authenticator(&mock)
        .sign_in("someone@example.com", "correct horse battery staple")
        .unwrap_err();
    assert_eq!(error, AuthError::TwoStepVerification);
}

#[test]
fn restores_or_expires_saved_sessions() {
    let saved = SessionState {
        cookies: vec![warpsign_core::portal::http::Cookie {
            name: "myacinfo".into(),
            value: "signed-in".into(),
            domain: "apple.com".into(),
            host_only: false,
            path: "/".into(),
            expires: None,
        }],
        ..Default::default()
    };

    let mock = MockApple::new();
    script_portal(&mock, one_team());
    let teams = Authenticator::with_session(mock.clone(), saved.clone())
        .restore()
        .unwrap();
    assert_eq!(teams[0].name, "Test Team");
    mock.assert_done();

    let mock = MockApple::new();
    mock.respond(
        ACCOUNT_URL,
        response(302, &[("Location", "https://idmsa.apple.com/signin")], ""),
    );
    mock.respond(
        "https://idmsa.apple.com/signin",
        response(200, &[], "<html>sign in</html>"),
    );
    mock.respond(TEAMS_URL, response(403, &[], ""));
    let error = Authenticator::with_session(mock.clone(), saved)
        .restore()
        .unwrap_err();
    assert_eq!(error, AuthError::SessionExpired);
}

// Portal behaviour beyond Python warpsign

fn portal(mock: &Arc<MockApple>, csrf: bool) -> Portal {
    let state = SessionState {
        csrf: csrf.then(|| "csrf-token".into()),
        csrf_ts: csrf.then(|| "1700000000000".into()),
        ..Default::default()
    };
    Portal::new(
        Arc::new(HttpClient::with_state(mock.clone(), state)),
        "ZZZZZ99999",
    )
}

fn capabilities() -> Value {
    fixture("portal_requests.json")["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == "capabilities")
        .unwrap()["result"]
        .clone()
}

fn enabled(request: &warpsign_core::portal::Request) -> Vec<String> {
    request.json_body().unwrap()["data"]["relationships"]["bundleIdCapabilities"]["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["attributes"]["enabled"] == true)
        .map(|c| {
            c["relationships"]["capability"]["data"]["id"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect()
}

#[test]
fn retries_capabilities_without_unknown_settings() {
    let mock = MockApple::new();
    mock.respond(
        "https://developer.apple.com/services-account/v1/capabilities",
        json_response(200, capabilities()),
    );
    let rejection = json!({"errors": [{"status": "409", "detail": "Settings are required for Something New."}]});
    let first = rejection.clone();
    mock.expect(
        "https://developer.apple.com/services-account/v1/bundleIds/BUNDLE1",
        move |request| {
            // As Python warpsign would send it
            assert_eq!(
                enabled(request),
                ["PUSH_NOTIFICATIONS", "IN_APP_PURCHASE", "NEW_THING"]
            );
            json_response(409, first.clone())
        },
    );
    mock.expect(
        "https://developer.apple.com/services-account/v1/bundleIds/BUNDLE1",
        |request| {
            assert_eq!(enabled(request), ["PUSH_NOTIFICATIONS", "IN_APP_PURCHASE"]);
            json_response(200, json!({"data": {}}))
        },
    );
    let bundle = BundleId {
        id: "BUNDLE1".into(),
        identifier: "com.example.app".into(),
        name: "x".into(),
    };
    let update = portal(&mock, true)
        .update_capabilities(
            &bundle,
            &["PUSH_NOTIFICATIONS".into(), "NEW_THING".into()],
            &BTreeMap::new(),
        )
        .unwrap();
    assert_eq!(update.skipped, ["NEW_THING"]);
    assert_eq!(update.required, ["IN_APP_PURCHASE"]);
    mock.assert_done();

    // Nothing to leave out: the first rejection is the answer
    let mock = MockApple::new();
    mock.respond(
        "https://developer.apple.com/services-account/v1/capabilities",
        json_response(200, capabilities()),
    );
    mock.respond(
        "https://developer.apple.com/services-account/v1/bundleIds/BUNDLE1",
        json_response(409, rejection),
    );
    let error = portal(&mock, true)
        .update_capabilities(&bundle, &["PUSH_NOTIFICATIONS".into()], &BTreeMap::new())
        .unwrap_err();
    assert_eq!(error.status, Some(409));
    assert_eq!(
        error.to_string(),
        "setting capabilities of com.example.app: Settings are required for Something New."
    );
    mock.assert_done();
}

fn test_certificate_der() -> Vec<u8> {
    let p12 =
        std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/test-cert.p12"))
            .unwrap();
    warpsign_core::identity::import_p12(&p12, "secret").unwrap()[0]
        .certificate_der()
        .to_vec()
}

#[test]
fn creates_certificates_from_a_csr() {
    let der = test_certificate_der();
    let mock = MockApple::new();
    let content = BASE64.encode(&der);
    mock.expect("https://developer.apple.com/services-account/v1/certificates", move |request| {
        assert_eq!(request.method, Method::Post);
        assert_eq!(header(request, "csrf"), "csrf-token");
        assert_eq!(header(request, "Content-Type"), "application/vnd.api+json");
        assert_eq!(
            request.json_body().unwrap(),
            json!({"data": {"type": "certificates", "attributes": {
                "certificateType": "DISTRIBUTION",
                "csrContent": "-----BEGIN CERTIFICATE REQUEST-----\n...",
                "teamId": "ZZZZZ99999",
            }}})
        );
        json_response(201, json!({"data": {"type": "certificates", "id": "CERT9", "attributes": {
            "serialNumber": "1A2B", "certificateType": "DISTRIBUTION", "name": "Apple Distribution: Test User",
            "certificateContent": content,
        }}}))
    });
    let (certificate, content) = portal(&mock, true)
        .create_certificate(
            CertificateKind::Distribution,
            "-----BEGIN CERTIFICATE REQUEST-----\n...",
        )
        .unwrap();
    assert_eq!(certificate.id, "CERT9");
    assert_eq!(certificate.kind(), Some(CertificateKind::Distribution));
    assert_eq!(content, der);

    // Without the content in the response, it's fetched separately
    let der = test_certificate_der();
    let mock = MockApple::new();
    mock.respond(
        "https://developer.apple.com/services-account/v1/certificates",
        json_response(201, json!({"data": {"type": "certificates", "id": "CERT9", "attributes": {"serialNumber": "1A2B", "certificateType": "DEVELOPMENT"}}})),
    );
    let content = BASE64.encode(&der);
    mock.expect("https://developer.apple.com/services-account/v1/certificates/CERT9", move |request| {
        assert_eq!(header(request, "X-HTTP-Method-Override"), "GET");
        json_response(200, json!({"data": {"id": "CERT9", "attributes": {"serialNumber": "1A2B", "certificateContent": content}}}))
    });
    let (_, content) = portal(&mock, true)
        .create_certificate(CertificateKind::Development, "csr")
        .unwrap();
    assert_eq!(content, der);
    mock.assert_done();
}

#[test]
fn surfaces_portal_errors() {
    let mock = MockApple::new();
    mock.respond(
        "https://developer.apple.com/services-account/v1/certificates",
        json_response(403, json!({"errors": [{"status": "403", "code": "FORBIDDEN_ERROR", "title": "Forbidden", "detail": "You need to accept the latest Program License Agreement."}]})),
    );
    let error = portal(&mock, true).certificates().unwrap_err();
    assert_eq!(error.status, Some(403));
    assert_eq!(
        error.to_string(),
        "listing certificates: You need to accept the latest Program License Agreement."
    );

    let mock = MockApple::new();
    mock.respond(
        "https://developer.apple.com/services-account/QH65B2/account/ios/profile/regenProvisioningProfile.action",
        json_response(200, json!({"resultCode": 35, "userString": "Multiple profiles found with the name 'x'."})),
    );
    let bundle = BundleId {
        id: "B".into(),
        identifier: "com.example".into(),
        name: "x".into(),
    };
    let error = portal(&mock, true)
        .create_or_regenerate_profile(
            None,
            &bundle,
            "x",
            &[],
            &[],
            warpsign_core::ProfileType::Development,
        )
        .unwrap_err();
    assert_eq!(error.message, "Multiple profiles found with the name 'x'.");

    // A signed-out session gets the sign-in page, not JSON
    let mock = MockApple::new();
    mock.respond(
        "https://developer.apple.com/services-account/v1/devices",
        response(200, &[], "<html>"),
    );
    assert!(
        portal(&mock, true)
            .devices()
            .unwrap_err()
            .message
            .contains("signed out")
    );
}

#[test]
fn sends_csrf_tokens_only_when_known() {
    let mock = MockApple::new();
    mock.expect("https://developer.apple.com/services-account/v1/bundleIds", |request| {
        assert!(request.header_value("csrf").is_none());
        assert!(request.header_value("csrf_ts").is_none());
        json_response(201, json!({"data": {"id": "B", "attributes": {"identifier": "com.example", "name": "x"}}}))
    });
    portal(&mock, false)
        .register_bundle_id("com.example", "x")
        .unwrap();
}

#[test]
fn registers_devices() {
    let mock = MockApple::new();
    mock.expect("https://developer.apple.com/services-account/v1/devices", |request| {
        assert_eq!(request.method, Method::Post);
        assert!(request.header_value("X-HTTP-Method-Override").is_none());
        assert_eq!(header(request, "csrf"), "csrf-token");
        assert_eq!(
            request.json_body().unwrap(),
            json!({"data": {"type": "devices", "attributes": {
                "name": "My iPhone", "udid": "00008110-000A", "platform": "IOS", "teamId": "ZZZZZ99999",
            }}})
        );
        json_response(201, json!({"data": {"id": "DEV9", "attributes": {
            "name": "My iPhone", "udid": "00008110-000A", "status": "ENABLED", "deviceClass": "IPHONE", "platform": "IOS",
        }}}))
    });
    let device = portal(&mock, true)
        .register_device("My iPhone", "00008110-000A")
        .unwrap();
    assert_eq!(device.id, "DEV9");
    assert_eq!(device.device_class, "IPHONE");
    mock.assert_done();
}
