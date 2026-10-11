//! Parses the synthetic profiles built by tools/build_test_profiles.sh

use std::path::PathBuf;

use plist::Value;
use warpsign_core::profile::{ProfileError, ProvisioningProfile, check_ppq_risk};

fn profile(name: &str) -> ProvisioningProfile {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/profiles")
        .join(format!("{name}.mobileprovision"));
    ProvisioningProfile::parse(&std::fs::read(path).unwrap()).unwrap()
}

#[test]
fn parses_profile_fields() {
    let profile = profile("ppq");

    assert_eq!(profile.name.as_deref(), Some("WS ppq Development"));
    assert_eq!(
        profile.uuid.as_deref(),
        Some("11111111-1111-1111-1111-111111111111")
    );
    assert_eq!(profile.team_ids, ["ZZZZZ99999"]);
    assert_eq!(profile.provisioned_devices.len(), 2);
    assert_eq!(
        profile.entitlements.get("application-identifier"),
        Some(&Value::String("ZZZZZ99999.com.example.test".into()))
    );
    assert!(profile.ppq_check);
}

#[test]
fn ppq_check_is_off_when_false_or_missing() {
    assert!(!profile("no-ppq").ppq_check);
    // Profiles from teams created before June 2021 have no PPQCheck key
    let legacy = profile("legacy");
    assert!(!legacy.ppq_check);
    assert!(!legacy.plist.contains_key("PPQCheck"));
}

#[test]
fn garbage_is_not_a_profile() {
    assert!(matches!(
        ProvisioningProfile::parse(b"not a profile"),
        Err(ProfileError::Cms(_))
    ));
}

#[test]
fn keeping_original_ids_is_refused_with_ppq() {
    let risk = check_ppq_risk(&[profile("no-ppq"), profile("ppq")], true).unwrap_err();
    assert_eq!(risk.profile, "WS ppq Development");
    assert!(risk.to_string().contains("PPQ"));
}

#[test]
fn encoded_ids_are_fine_with_ppq() {
    assert!(check_ppq_risk(&[profile("ppq")], false).is_ok());
}

#[test]
fn keeping_original_ids_is_fine_without_ppq() {
    assert!(check_ppq_risk(&[profile("no-ppq"), profile("legacy")], true).is_ok());
}

#[test]
fn detects_wildcard_profiles() {
    assert!(profile("wildcard").is_wildcard());
    assert!(!profile("ppq").is_wildcard());
}

#[test]
fn reads_expiration_date() {
    let expires = profile("ppq").expiration_date.unwrap();
    let seconds = expires
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    // 2030-01-01T00:00:00Z
    assert_eq!(seconds, 1_893_456_000);
}
