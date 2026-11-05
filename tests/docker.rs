//! The Docker client against a mock Engine API.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use cadastre::docker::Docker;
use support::mock_docker::MockDocker;
use support::{fixture, fixtures, CANARY};

fn mock_with_all_fixtures() -> MockDocker {
    let documents: Vec<_> = fixtures().into_iter().map(|(_, d)| d).collect();
    MockDocker::with_containers(&documents)
}

#[test]
fn lists_and_sanitizes_every_container() {
    let mock = mock_with_all_fixtures();
    let containers = Docker::new(&mock.url).containers().unwrap();
    let mut names: Vec<_> = containers.iter().map(|c| c.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        ["node-agent", "shop-db-1", "shop-web-1", "uptime-probe"]
    );
    let serialized = serde_json::to_string(&containers).unwrap();
    assert!(!serialized.contains(CANARY), "{serialized}");
}

#[test]
fn a_trailing_slash_in_the_url_is_tolerated() {
    let mock = mock_with_all_fixtures();
    let containers = Docker::new(&format!("{}/", mock.url)).containers().unwrap();
    assert_eq!(containers.len(), 4);
}

#[test]
fn a_container_removed_between_listing_and_inspection_is_skipped() {
    let mock = mock_with_all_fixtures();
    mock.state.lock().unwrap().ghosts.push("deadbeef".into());
    let containers = Docker::new(&mock.url).containers().unwrap();
    assert_eq!(containers.len(), 4);
}

#[test]
fn inspect_reports_a_missing_container_as_none() {
    let mock = mock_with_all_fixtures();
    assert!(Docker::new(&mock.url).inspect("nope").unwrap().is_none());
}

#[test]
fn a_forbidden_response_explains_how_to_configure_the_proxy() {
    let mock = mock_with_all_fixtures();
    mock.set_status(Some(403));
    let error = Docker::new(&mock.url).containers().unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("CONTAINERS=1"), "{message}");
}

#[test]
fn a_server_error_is_an_error() {
    let mock = mock_with_all_fixtures();
    mock.set_status(Some(500));
    let error = Docker::new(&mock.url).containers().unwrap_err();
    assert!(format!("{error:#}").contains("500"), "{error:#}");
}

#[test]
fn an_unreachable_daemon_is_an_error() {
    let error = Docker::new("http://127.0.0.1:1").containers().unwrap_err();
    assert!(!format!("{error:#}").is_empty());
}

#[test]
fn an_inspect_document_that_cannot_be_documented_names_the_container() {
    let mock = MockDocker::start();
    let mut broken = fixture("web.json");
    broken["Config"] = serde_json::Value::Null;
    broken["Name"] = serde_json::Value::Null;
    mock.add(broken);
    let error = Docker::new(&mock.url).containers().unwrap_err();
    assert!(format!("{error:#}").contains("3f8c1a2b9d4e"), "{error:#}");
}

#[test]
fn events_arrive_as_they_are_sent() {
    let mock = MockDocker::start();
    mock.push_event("container", "start");
    mock.push_event("container", "exec_create: sh");
    mock.push_event("network", "connect");
    let mut events = Docker::new(&mock.url).events().unwrap();
    let actions: Vec<_> = (0..3)
        .map(|_| events.next().unwrap().unwrap())
        .map(|e| (e.kind.clone(), e.action.clone(), e.is_relevant()))
        .collect();
    assert_eq!(
        actions,
        [
            ("container".into(), "start".into(), true),
            ("container".into(), "exec_create: sh".into(), false),
            ("network".into(), "connect".into(), true),
        ]
    );
}

#[test]
fn events_sent_after_the_connection_opened_are_received() {
    let mock = MockDocker::start();
    let mut events = Docker::new(&mock.url).events().unwrap();
    mock.push_event("container", "die");
    let event = events.next().unwrap().unwrap();
    assert_eq!(event.action, "die");
}

#[test]
fn the_event_request_asks_only_for_container_and_network_events() {
    let mock = MockDocker::start();
    let _events = Docker::new(&mock.url).events().unwrap();
    let requests = mock.state.lock().unwrap().requests.clone();
    let request = requests.iter().find(|r| r.starts_with("/events")).unwrap();
    assert!(
        request.contains("container") && request.contains("network"),
        "{request}"
    );
}
