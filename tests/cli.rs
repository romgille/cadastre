//! The binary, as a user or a container runtime would invoke it.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::fs;
use std::process::{Command, Output};

use cadastre::heartbeat;
use cadastre::timefmt::unix_now;
use support::fixtures;
use support::mock_docker::MockDocker;

fn binary() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cadastre"));
    command.env_clear();
    command
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn version_and_help_succeed() {
    let version = binary().arg("--version").output().unwrap();
    assert!(version.status.success());
    assert!(String::from_utf8_lossy(&version.stdout).starts_with("cadastre "));
    let help = binary().arg("--help").output().unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("healthcheck"));
}

#[test]
fn unknown_arguments_print_usage_and_fail_with_code_2() {
    for args in [vec![], vec!["bogus"], vec!["once", "--nope"]] {
        let output = binary().args(&args).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(stderr(&output).contains("Usage:"));
    }
}

#[test]
fn missing_configuration_is_a_clear_error_not_a_default() {
    let output = binary().arg("once").output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("CADASTRE_"), "{}", stderr(&output));
}

#[test]
fn healthcheck_without_a_heartbeat_fails() {
    let dir = tempfile::tempdir().unwrap();
    let output = binary()
        .arg("healthcheck")
        .env("CADASTRE_HEARTBEAT_FILE", dir.path().join("none"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("not running"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn healthcheck_follows_the_heartbeat() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hb");
    let check = || {
        binary()
            .arg("healthcheck")
            .env("CADASTRE_HEARTBEAT_FILE", &path)
            .output()
            .unwrap()
    };
    heartbeat::write(&path, unix_now(), true).unwrap();
    assert!(check().status.success());
    heartbeat::write(&path, unix_now(), false).unwrap();
    assert_eq!(check().status.code(), Some(1));
    heartbeat::write(&path, unix_now() - 10_000, true).unwrap();
    assert_eq!(check().status.code(), Some(1));
}

#[test]
fn once_documents_the_containers_and_reports_a_summary() {
    let dir = tempfile::tempdir().unwrap();
    for sub in ["out", "state"] {
        fs::create_dir(dir.path().join(sub)).unwrap();
    }
    let documents: Vec<_> = fixtures().into_iter().map(|(_, d)| d).collect();
    let mock = MockDocker::with_containers(&documents);
    let output = binary()
        .arg("once")
        .env("CADASTRE_DOCKER_URL", &mock.url)
        .env("CADASTRE_OUT_DIR", dir.path().join("out"))
        .env("CADASTRE_STATE_DIR", dir.path().join("state"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("4 container(s)"), "{stdout}");
    assert!(dir.path().join("out/index.md").exists());
}

#[test]
fn once_fails_loudly_when_docker_is_unreachable() {
    let dir = tempfile::tempdir().unwrap();
    for sub in ["out", "state"] {
        fs::create_dir(dir.path().join(sub)).unwrap();
    }
    let output = binary()
        .arg("once")
        .env("CADASTRE_DOCKER_URL", "http://127.0.0.1:1")
        .env("CADASTRE_OUT_DIR", dir.path().join("out"))
        .env("CADASTRE_STATE_DIR", dir.path().join("state"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(!dir.path().join("out/index.md").exists());
}

#[test]
fn run_publishes_then_stops_cleanly_on_sigterm() {
    let dir = tempfile::tempdir().unwrap();
    for sub in ["out", "state"] {
        fs::create_dir(dir.path().join(sub)).unwrap();
    }
    let documents: Vec<_> = fixtures().into_iter().map(|(_, d)| d).collect();
    let mock = MockDocker::with_containers(&documents);
    let mut child = binary()
        .arg("run")
        .env("CADASTRE_DOCKER_URL", &mock.url)
        .env("CADASTRE_OUT_DIR", dir.path().join("out"))
        .env("CADASTRE_STATE_DIR", dir.path().join("state"))
        .env("CADASTRE_HEARTBEAT_FILE", dir.path().join("hb"))
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !dir.path().join("out/index.md").exists() {
        assert!(std::time::Instant::now() < deadline, "no output in time");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    let signalled = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(signalled.success());
    let started = std::time::Instant::now();
    let status = child.wait().unwrap();
    assert!(status.success(), "exit status {status:?}");
    assert!(started.elapsed() < std::time::Duration::from_secs(3));
}
