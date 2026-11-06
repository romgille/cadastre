//! Prometheus metrics, written for node_exporter's textfile collector.
//!
//! Every run, successful or not, rewrites the file. The timestamps are what
//! make a dead cadastre visible: an alert on `time() - last_success` fires
//! when runs stop succeeding, and an `absent()` guard fires when the file or
//! the whole exporter is gone.

use std::fmt::Write as _;
use std::path::Path;

use anyhow::Result;

use crate::fsutil::atomic_write;

/// File name inside the metrics folder. node_exporter only reads `*.prom`.
pub const FILE_NAME: &str = "cadastre.prom";

/// What the last runs did. Kept in memory: after a restart there is no
/// success until the first run succeeds, and the metric is then absent, which
/// the alert rules treat as a failure.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Status {
    pub last_attempt: Option<u64>,
    pub last_attempt_succeeded: bool,
    pub last_success: Option<u64>,
    pub containers: Option<usize>,
}

impl Status {
    pub fn record_success(&mut self, now: u64, containers: usize) {
        self.last_attempt = Some(now);
        self.last_attempt_succeeded = true;
        self.last_success = Some(now);
        self.containers = Some(containers);
    }

    pub fn record_failure(&mut self, now: u64) {
        self.last_attempt = Some(now);
        self.last_attempt_succeeded = false;
    }

    /// The exposition text.
    pub fn render(&self, version: &str) -> String {
        let mut out = String::new();
        gauge(
            &mut out,
            "cadastre_last_success_timestamp_seconds",
            "Unix time of the last run that published successfully.",
            self.last_success,
        );
        gauge(
            &mut out,
            "cadastre_last_attempt_timestamp_seconds",
            "Unix time of the last run, successful or not.",
            self.last_attempt,
        );
        gauge(
            &mut out,
            "cadastre_last_attempt_success",
            "1 if the last run succeeded, 0 if it failed.",
            self.last_attempt
                .map(|_| u64::from(self.last_attempt_succeeded)),
        );
        gauge(
            &mut out,
            "cadastre_containers",
            "Containers in the last successful snapshot.",
            self.containers.map(|n| n as u64),
        );
        let _ = write!(
            out,
            "# HELP cadastre_build_info Version of the running build.\n\
             # TYPE cadastre_build_info gauge\n\
             cadastre_build_info{{version=\"{version}\"}} 1\n"
        );
        out
    }
}

/// Appends a gauge. A value of `None` leaves the series out entirely rather
/// than reporting a made-up number.
fn gauge(out: &mut String, name: &str, help: &str, value: Option<u64>) {
    let Some(value) = value else { return };
    let _ = write!(
        out,
        "# HELP {name} {help}\n# TYPE {name} gauge\n{name} {value}\n"
    );
}

/// Writes the metrics file atomically, so the collector never reads half of
/// it.
pub fn write(dir: &Path, status: &Status, version: &str) -> Result<()> {
    atomic_write(
        &dir.join(".tmp"),
        &dir.join(FILE_NAME),
        status.render(version).as_bytes(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_status_reports_no_timestamps_at_all() {
        let text = Status::default().render("1.2.3");
        assert!(!text.contains("last_success"), "{text}");
        assert!(!text.contains("last_attempt"), "{text}");
        assert!(text.contains(r#"cadastre_build_info{version="1.2.3"} 1"#));
    }

    #[test]
    fn a_success_sets_every_series() {
        let mut status = Status::default();
        status.record_success(1000, 7);
        let text = status.render("1");
        assert!(
            text.contains("\ncadastre_last_success_timestamp_seconds 1000\n"),
            "{text}"
        );
        assert!(
            text.contains("\ncadastre_last_attempt_timestamp_seconds 1000\n"),
            "{text}"
        );
        assert!(
            text.contains("\ncadastre_last_attempt_success 1\n"),
            "{text}"
        );
        assert!(text.contains("\ncadastre_containers 7\n"), "{text}");
    }

    #[test]
    fn a_failure_keeps_the_last_success_and_flags_the_attempt() {
        let mut status = Status::default();
        status.record_success(1000, 7);
        status.record_failure(2000);
        let text = status.render("1");
        assert!(
            text.contains("\ncadastre_last_success_timestamp_seconds 1000\n"),
            "{text}"
        );
        assert!(
            text.contains("\ncadastre_last_attempt_timestamp_seconds 2000\n"),
            "{text}"
        );
        assert!(
            text.contains("\ncadastre_last_attempt_success 0\n"),
            "{text}"
        );
    }

    #[test]
    fn a_failure_before_any_success_leaves_the_success_series_absent() {
        let mut status = Status::default();
        status.record_failure(2000);
        let text = status.render("1");
        assert!(!text.contains("last_success"), "{text}");
        assert!(
            text.contains("\ncadastre_last_attempt_success 0\n"),
            "{text}"
        );
    }

    #[test]
    fn no_series_carries_an_instance_label() {
        // node_exporter adds the instance; a second one would break joins.
        let mut status = Status::default();
        status.record_success(1, 1);
        assert!(!status.render("1").contains("instance"));
    }

    #[test]
    fn the_file_is_written_atomically_and_replaced_on_every_write() {
        let dir = tempfile::tempdir().unwrap();
        let mut status = Status::default();
        status.record_success(1, 1);
        write(dir.path(), &status, "1").unwrap();
        status.record_success(2, 2);
        write(dir.path(), &status, "1").unwrap();
        let text = std::fs::read_to_string(dir.path().join(FILE_NAME)).unwrap();
        assert!(text.contains("cadastre_containers 2"), "{text}");
        assert_eq!(
            std::fs::read_dir(dir.path().join(".tmp")).unwrap().count(),
            0
        );
    }
}
