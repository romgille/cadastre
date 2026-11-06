//! Regression tests for the most important property of the project: secrets
//! found in `docker inspect` output never reach what cadastre writes.
//!
//! Every sensitive value in `tests/fixtures` starts with `LEAK-`. Anything
//! derived from a fixture that contains that prefix is a leak.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use cadastre::sanitize::container_from_inspect;
use support::{fixture, fixtures, CANARY};

#[test]
fn the_fixtures_really_contain_canaries() {
    // Guards against a vacuous pass: a fixture without canaries proves nothing.
    let all = fixtures();
    assert!(all.len() >= 3, "expected at least three fixtures");
    for (name, document) in all {
        let raw = document.to_string();
        assert!(
            raw.matches(CANARY).count() >= 3,
            "{name} has too few canaries"
        );
    }
}

#[test]
fn the_sanitized_model_never_contains_a_canary() {
    for (name, document) in fixtures() {
        let container = container_from_inspect(&document).unwrap();
        let serialized = serde_json::to_string_pretty(&container).unwrap();
        assert!(!serialized.contains(CANARY), "{name} leaked:\n{serialized}");
    }
}

#[test]
fn sensitive_variables_are_documented_by_name_only() {
    let container = container_from_inspect(&fixture("web.json")).unwrap();
    for key in ["DATABASE_URL", "SECRET_KEY_BASE", "APP_ENV"] {
        assert!(container.env_keys.iter().any(|k| k == key), "{key} missing");
    }
}

#[test]
fn routing_labels_survive_but_secrets_in_labels_do_not() {
    let container = container_from_inspect(&fixture("web.json")).unwrap();
    assert!(container
        .labels
        .contains_key("traefik.http.routers.shop.rule"));
    assert!(!container.labels.keys().any(|key| key.contains("basicauth")));
    assert!(!container.labels.contains_key("com.example.api-key"));
}
