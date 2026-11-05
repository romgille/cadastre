//! Regression tests for the most important property of the project: secrets
//! found in `docker inspect` output never reach what cadastre writes.
//!
//! Every sensitive value in `tests/fixtures` starts with `LEAK-`. Anything
//! derived from a fixture that contains that prefix is a leak.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use cadastre::model::Container;
use cadastre::render::{render, Options, Page};
use cadastre::sanitize::container_from_inspect;
use support::{fixture, fixtures, CANARY};

fn all_containers() -> Vec<Container> {
    fixtures()
        .iter()
        .map(|(_, document)| container_from_inspect(document).unwrap())
        .collect()
}

fn page<'a>(pages: &'a [Page], path: &str) -> &'a str {
    &pages.iter().find(|p| p.path == path).unwrap().content
}

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

#[test]
fn rendered_pages_never_contain_a_canary() {
    let containers = all_containers();
    for options in [Options::default(), Options { obsidian: true }] {
        for page in render(&containers, &options) {
            assert!(
                !page.content.contains(CANARY),
                "{} leaked with {options:?}:\n{}",
                page.path,
                page.content
            );
        }
    }
}

#[test]
fn rendered_pages_still_carry_the_useful_facts() {
    // Guards against a pass caused by empty output.
    let pages = render(&all_containers(), &Options::default());
    assert!(page(&pages, "index.md").contains("4 containers in 2 stacks."));
    let shop = page(&pages, "stacks/shop.md");
    assert!(shop.contains("`ghcr.io/example/web:2.4.1`"), "{shop}");
    assert!(shop.contains("`SECRET_KEY_BASE`"), "{shop}");
    assert!(shop.contains("`traefik.http.routers.shop.rule`"), "{shop}");
    assert!(shop.contains("`shop_pgdata`"), "{shop}");
}

#[test]
fn the_index_warns_about_the_exposed_fixtures() {
    let pages = render(&all_containers(), &Options::default());
    let index = page(&pages, "index.md");
    assert!(
        index.contains("`uptime-probe` publishes `9115/tcp` on all interfaces"),
        "{index}"
    );
    assert!(
        index.contains("`node-agent` uses host networking"),
        "{index}"
    );
    assert!(
        !index.contains("`shop-web-1` publishes"),
        "loopback bindings are not a warning"
    );
}

#[test]
fn a_hostile_container_name_cannot_inject_markdown_or_html() {
    let mut document = fixture("web.json");
    document["Name"] = serde_json::json!("/evil`\n# injected <img src=x onerror=alert(1)>");
    let container = container_from_inspect(&document).unwrap();
    for options in [Options::default(), Options { obsidian: true }] {
        for page in render(std::slice::from_ref(&container), &options) {
            assert!(
                !page.content.contains("\n# injected"),
                "{}:\n{}",
                page.path,
                page.content
            );
            // The tag only ever appears inside a code span.
            for line in page.content.lines().filter(|line| line.contains("<img")) {
                assert!(line.contains('`'), "unquoted HTML in {}: {line}", page.path);
            }
        }
    }
}
