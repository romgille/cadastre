//! The daemon loop against a mock Docker API and temporary folders.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use cadastre::config::Config;
use cadastre::daemon::{run, Timing};
use cadastre::docker::Docker;
use cadastre::heartbeat;
use cadastre::timefmt::unix_now;
use serde_json::Value;
use support::mock_docker::MockDocker;
use support::{fixture, fixtures};
use tempfile::TempDir;

struct Running {
    dir: TempDir,
    config: Config,
    mock: MockDocker,
    shutdown: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Running {
    fn start(documents: &[Value], reconcile_seconds: u64) -> Self {
        let dir = tempfile::tempdir().unwrap();
        for sub in ["out", "state", "metrics"] {
            fs::create_dir(dir.path().join(sub)).unwrap();
        }
        let mock = MockDocker::with_containers(documents);
        let root = dir.path().to_path_buf();
        let lookup = move |key: &str| match key {
            "CADASTRE_DOCKER_URL" => None,
            "CADASTRE_OUT_DIR" => Some(root.join("out").display().to_string()),
            "CADASTRE_STATE_DIR" => Some(root.join("state").display().to_string()),
            "CADASTRE_METRICS_DIR" => Some(root.join("metrics").display().to_string()),
            "CADASTRE_HEARTBEAT_FILE" => Some(root.join("heartbeat").display().to_string()),
            "CADASTRE_RECONCILE_SECONDS" => Some(reconcile_seconds.to_string()),
            _ => None,
        };
        let url = mock.url.clone();
        let mut config = Config::from_lookup(|key| {
            lookup(key).or_else(|| (key == "CADASTRE_DOCKER_URL").then(|| url.clone()))
        })
        .unwrap();
        config.debounce = Duration::from_millis(100);

        let shutdown = Arc::new(AtomicBool::new(false));
        let handle = {
            let config = config.clone();
            let shutdown = Arc::clone(&shutdown);
            let docker = [Docker::new(&mock.url)];
            let timing = Timing {
                retry: Duration::from_millis(200),
                reconnect: Duration::from_millis(100),
                tick: Duration::from_millis(50),
            };
            thread::spawn(move || run(&config, &docker, &timing, &shutdown).unwrap())
        };
        Self {
            dir,
            config,
            mock,
            shutdown,
            handle: Some(handle),
        }
    }

    fn out(&self, relative: &str) -> Option<String> {
        fs::read_to_string(self.config.out_dir.join(relative)).ok()
    }

    fn metrics(&self) -> String {
        fs::read_to_string(self.dir.path().join("metrics/cadastre.prom")).unwrap_or_default()
    }

    fn heartbeat(&self) -> Result<(), String> {
        let path: PathBuf = self.dir.path().join("heartbeat");
        heartbeat::check(&path, unix_now(), 300).map_err(|e| format!("{e:#}"))
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn wait_for(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if condition() {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!("timed out waiting for {what}");
}

fn two_containers() -> Vec<Value> {
    vec![fixture("web.json"), fixture("db.json")]
}

#[test]
fn the_first_run_happens_at_startup() {
    let daemon = Running::start(&two_containers(), 3600);
    wait_for("the first publication", || daemon.out("index.md").is_some());
    wait_for("a healthy heartbeat", || daemon.heartbeat().is_ok());
    wait_for("metrics", || {
        daemon.metrics().contains("cadastre_containers 2")
    });
}

#[test]
fn a_new_container_shows_up_after_an_event() {
    let daemon = Running::start(&two_containers(), 3600);
    wait_for("the first publication", || daemon.out("index.md").is_some());
    let extra = fixture("standalone.json");
    daemon.mock.add(extra);
    daemon.mock.push_event("container", "start");
    wait_for("the new container", || {
        daemon
            .out("index.md")
            .is_some_and(|text| text.contains("uptime-probe"))
    });
    wait_for("the changelog entry", || {
        daemon
            .out("changelog.md")
            .is_some_and(|text| text.contains("- Added `uptime-probe`"))
    });
}

#[test]
fn irrelevant_events_do_not_cause_a_run() {
    let daemon = Running::start(&two_containers(), 3600);
    wait_for("the first publication", || daemon.out("index.md").is_some());
    wait_for("the event stream", || daemon.mock.event_connections() >= 1);
    let before = daemon.mock.list_requests();
    daemon.mock.push_event("container", "exec_create: sh");
    daemon
        .mock
        .push_event("container", "health_status: healthy");
    thread::sleep(Duration::from_millis(600));
    assert_eq!(daemon.mock.list_requests(), before);
}

#[test]
fn the_reconcile_catches_changes_even_without_events() {
    let daemon = Running::start(&two_containers(), 1);
    wait_for("the first publication", || daemon.out("index.md").is_some());
    daemon.mock.add(fixture("hostnet.json"));
    wait_for("the reconcile to notice", || {
        daemon
            .out("index.md")
            .is_some_and(|text| text.contains("node-agent"))
    });
}

#[test]
fn a_failing_daemon_api_is_visible_in_the_heartbeat_and_the_metrics_and_recovers() {
    let daemon = Running::start(&two_containers(), 3600);
    wait_for("a healthy start", || daemon.heartbeat().is_ok());

    daemon.mock.set_status(Some(500));
    daemon.mock.push_event("container", "die");
    wait_for("the heartbeat to report the failure", || {
        daemon.heartbeat().is_err_and(|e| e.contains("failed"))
    });
    wait_for("the metrics to report the failure", || {
        daemon.metrics().contains("cadastre_last_attempt_success 0")
    });
    assert!(
        daemon
            .metrics()
            .contains("cadastre_last_success_timestamp_seconds"),
        "the last success stays visible so its age can alert"
    );

    daemon.mock.set_status(None);
    wait_for("recovery by retry", || {
        daemon.heartbeat().is_ok() && daemon.metrics().contains("cadastre_last_attempt_success 1")
    });
}

#[test]
fn a_daemon_that_loses_everything_keeps_the_old_pages_and_reports_failure() {
    let documents: Vec<Value> = fixtures().into_iter().map(|(_, d)| d).collect();
    let daemon = Running::start(&documents, 3600);
    wait_for("the first publication", || {
        daemon.out("docker/shop.md").is_some()
    });
    for document in &documents {
        daemon.mock.remove(document["Id"].as_str().unwrap());
    }
    daemon.mock.push_event("container", "destroy");
    wait_for("the guard to trip", || daemon.heartbeat().is_err());
    assert!(daemon.out("docker/shop.md").is_some());
}

#[test]
fn shutdown_is_prompt() {
    let daemon = Running::start(&two_containers(), 3600);
    wait_for("the first publication", || daemon.out("index.md").is_some());
    let started = Instant::now();
    drop(daemon);
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn an_endpoint_that_was_down_at_startup_is_picked_up_once_it_recovers() {
    let dir = tempfile::tempdir().unwrap();
    for sub in ["out", "state"] {
        fs::create_dir(dir.path().join(sub)).unwrap();
    }
    let root = dir.path().to_path_buf();
    let nas = MockDocker::with_containers(&[fixture("web.json")]);
    let vps = MockDocker::with_containers(&[fixture("standalone.json")]);
    vps.set_status(Some(500));
    let mut config = Config::from_lookup(|key| match key {
        "CADASTRE_DOCKER_URL" => Some(nas.url.clone()),
        "CADASTRE_OUT_DIR" => Some(root.join("out").display().to_string()),
        "CADASTRE_STATE_DIR" => Some(root.join("state").display().to_string()),
        "CADASTRE_HEARTBEAT_FILE" => Some(root.join("heartbeat").display().to_string()),
        "CADASTRE_RECONCILE_SECONDS" => Some("3600".to_string()),
        _ => None,
    })
    .unwrap();
    config.debounce = Duration::from_millis(100);
    let shutdown = Arc::new(AtomicBool::new(false));
    let handle = {
        let config = config.clone();
        let shutdown = Arc::clone(&shutdown);
        let dockers = [
            Docker::named(&nas.url, Some("nas")),
            Docker::named(&vps.url, Some("vps")),
        ];
        let timing = Timing {
            retry: Duration::from_millis(200),
            reconnect: Duration::from_millis(100),
            tick: Duration::from_millis(50),
        };
        thread::spawn(move || run(&config, &dockers, &timing, &shutdown).unwrap())
    };
    let index = config.out_dir.join("index.md");
    wait_for("the partial publication", || {
        fs::read_to_string(&index).is_ok_and(|text| text.contains("unreachable"))
    });
    wait_for("the failing endpoint to be listened to", || {
        vps.state
            .lock()
            .unwrap()
            .requests
            .iter()
            .any(|path| path.starts_with("/events"))
    });
    vps.set_status(None);
    wait_for("the recovered endpoint", || {
        fs::read_to_string(&index).is_ok_and(|text| text.contains("uptime-probe"))
    });
    shutdown.store(true, Ordering::Relaxed);
    handle.join().unwrap();
}
