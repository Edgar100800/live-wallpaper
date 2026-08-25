//! Contract tests: the crate must accept exactly what the neutral fixtures
//! in shared/contracts/fixtures declare, and nothing else.

use particlewall_contracts::fixture_dir;
use particlewall_contracts::manifest::ManifestV1;
use particlewall_contracts::navigation::{scheme_decision, SchemeDecision};
use particlewall_contracts::paths::is_within_root;
use particlewall_contracts::playback::{resolve, PolicyInputs};
use std::path::Path;

fn load_fixture(rel: &str) -> serde_json::Value {
    let path = fixture_dir().join(rel);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("missing fixture {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("bad fixture JSON {rel}: {e}"))
}

#[test]
fn playback_policy_matches_all_fixture_cases() {
    let fixture = load_fixture("playback/policy.json");
    assert_eq!(fixture["contractVersion"], 1);
    for case in fixture["cases"].as_array().expect("cases array") {
        let input = &case["input"];
        let policy = resolve(PolicyInputs {
            user_paused: input["userPaused"].as_bool().unwrap(),
            power_save: input["powerSave"].as_bool().unwrap(),
            system_unavailable: input["systemUnavailable"].as_bool().unwrap(),
            battery_pause: input["batteryPause"].as_bool().unwrap(),
        });
        let expected = &case["expected"];
        assert_eq!(
            policy.paused,
            expected["paused"].as_bool().unwrap(),
            "case {} paused",
            case["name"].as_str().unwrap_or("?")
        );
        assert_eq!(policy.deep_sleep, expected["deepSleep"].as_bool().unwrap());
        assert_eq!(
            policy.preserving_frame,
            expected["preservingFrame"].as_bool().unwrap()
        );
        assert_eq!(
            policy.persist_snapshot,
            expected["persistSnapshot"].as_bool().unwrap()
        );
    }
}

#[test]
fn path_containment_matches_all_fixture_cases() {
    let fixture = load_fixture("navigation/path-containment.json");
    assert_eq!(fixture["contractVersion"], 1);
    for case in fixture["cases"].as_array().expect("cases array") {
        let root = Path::new(case["root"].as_str().unwrap());
        let target = Path::new(case["target"].as_str().unwrap());
        let allowed = is_within_root(root, target);
        assert_eq!(
            allowed,
            case["allowed"].as_bool().unwrap(),
            "case {}",
            case["name"].as_str().unwrap_or("?")
        );
    }
}

#[test]
fn scheme_policy_matches_all_fixture_cases() {
    let fixture = load_fixture("navigation/schemes.json");
    assert_eq!(fixture["contractVersion"], 1);
    for case in fixture["cases"].as_array().expect("cases array") {
        let decision = scheme_decision(case["scheme"].as_str().unwrap());
        let expected = match case["allowed"].as_str() {
            Some("path-check") => SchemeDecision::File,
            _ => {
                if case["allowed"].as_bool().unwrap() {
                    SchemeDecision::Allow
                } else {
                    SchemeDecision::Deny
                }
            }
        };
        assert_eq!(decision, expected, "scheme {}", case["scheme"]);
    }
}

#[test]
fn manifest_migration_matches_all_fixture_cases() {
    let fixture = load_fixture("manifest/migration-cases.json");
    assert_eq!(fixture["contractVersion"], 1);
    for case in fixture["cases"].as_array().expect("cases array") {
        let json = serde_json::to_string(&case["manifest"]).unwrap();
        let name = case["name"].as_str().unwrap_or("?");
        // Invalid means either a parse failure or a parsed manifest whose
        // renderer cannot be resolved to a shared module.
        let module = ManifestV1::parse(&json).ok().and_then(|m| m.effective_module());
        match case["expectedModule"].as_str() {
            Some(expected) => assert_eq!(
                module.as_deref(),
                Some(expected),
                "{name}: wrong module"
            ),
            None => assert!(module.is_none(), "{name}: expected invalid manifest"),
        }
    }
}
