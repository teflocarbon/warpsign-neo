use std::time::{Duration, UNIX_EPOCH};

use warpsign_core::auth::hashcash;
use warpsign_core::auth::srp::{Protocol, SrpClient, SrpError, derive_password};

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn hashcash_matches_fastlane() {
    // From fastlane's Spaceship::Hashcash tests
    assert_eq!(
        hashcash::make_at(11, "4d74fb15eb23f465f1f6fcbf534e5877", "20230223170600"),
        "1:11:20230223170600:4d74fb15eb23f465f1f6fcbf534e5877::6373"
    );
}

#[test]
fn hashcash_timestamp_is_utc() {
    let t = UNIX_EPOCH + Duration::from_secs(1_677_171_960); // 2023-02-23T17:06:00Z
    assert_eq!(hashcash::timestamp(t), "20230223170600");
    assert_eq!(hashcash::timestamp(UNIX_EPOCH), "19700101000000");
    let leap = UNIX_EPOCH + Duration::from_secs(1_709_210_096); // 2024-02-29T12:34:56Z
    assert_eq!(hashcash::timestamp(leap), "20240229123456");
}

#[test]
fn srp_matches_python_warpsign() {
    let fixture: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/srp.json"),
        )
        .unwrap(),
    )
    .unwrap();

    for case in fixture["cases"].as_array().unwrap() {
        let s = |key: &str| case[key].as_str().unwrap();
        let expected = |key: &str| unhex(case["expected"][key].as_str().unwrap());
        let protocol = Protocol::from_name(s("protocol")).unwrap();
        let salt = unhex(s("salt"));

        let derived = derive_password(
            s("password"),
            &salt,
            case["iterations"].as_u64().unwrap() as u32,
            protocol,
        );
        assert_eq!(derived, expected("derived_password"), "{}", s("email"));

        let client = SrpClient::with_secret(s("email"), &unhex(s("a")));
        assert_eq!(client.public_key(), expected("A"), "{}", s("email"));

        let proof = client
            .process_challenge(&salt, &unhex(s("B")), &derived)
            .unwrap();
        assert_eq!(proof.client_proof, expected("M1"), "{}", s("email"));
        assert_eq!(
            proof.expected_server_proof,
            expected("M2"),
            "{}",
            s("email")
        );
        assert!(proof.verify_server(&expected("M2")).is_ok());
        assert_eq!(
            proof.verify_server(&[0u8; 32]),
            Err(SrpError::ServerProofMismatch)
        );
    }
}

#[test]
fn srp_rejects_a_zero_challenge() {
    let client = SrpClient::new("someone@example.com");
    assert!(matches!(
        client.process_challenge(b"salt", &[0u8; 256], &[1u8; 32]),
        Err(SrpError::InvalidChallenge)
    ));
}
