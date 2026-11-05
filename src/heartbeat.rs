//! A file that proves the main loop is alive, and says how its last run went.
//!
//! The container's health check reads it. Without it, a process that is
//! alive but stuck, or one that fails every run, would look healthy.

use std::fs;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};

use crate::fsutil::atomic_write;

/// A heartbeat older than this means the loop is stuck. It is generous
/// because a run is not interrupted: it can take a while on a slow daemon.
pub const MAX_AGE_SECONDS: u64 = 300;

/// Records that the loop is alive at `now`, and whether the last run worked.
pub fn write(path: &Path, now: u64, last_run_ok: bool) -> Result<()> {
    let state = if last_run_ok { "ok" } else { "failing" };
    let dir = path
        .parent()
        .ok_or_else(|| anyhow!("{} has no parent directory", path.display()))?;
    atomic_write(dir, path, format!("{now} {state}\n").as_bytes())
}

/// Succeeds only if the heartbeat is fresh and the last run worked.
pub fn check(path: &Path, now: u64, max_age: u64) -> Result<()> {
    let text = fs::read_to_string(path).with_context(|| {
        format!(
            "no heartbeat at {}: the daemon is not running",
            path.display()
        )
    })?;
    let mut parts = text.split_whitespace();
    let written: u64 = parts
        .next()
        .and_then(|part| part.parse().ok())
        .ok_or_else(|| anyhow!("{} is malformed", path.display()))?;
    let age = now.saturating_sub(written);
    if age > max_age {
        bail!("the last heartbeat is {age}s old (limit {max_age}s): the main loop is stuck");
    }
    match parts.next() {
        Some("ok") => Ok(()),
        Some("failing") => bail!("the last run failed; see the container logs"),
        _ => bail!("{} is malformed", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heartbeat(ok: bool, at: u64) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hb");
        write(&path, at, ok).unwrap();
        (dir, path)
    }

    #[test]
    fn a_fresh_ok_heartbeat_is_healthy() {
        let (_dir, path) = heartbeat(true, 1000);
        check(&path, 1010, 300).unwrap();
    }

    #[test]
    fn a_stale_heartbeat_is_unhealthy() {
        let (_dir, path) = heartbeat(true, 1000);
        let error = check(&path, 1301, 300).unwrap_err();
        assert!(format!("{error}").contains("stuck"), "{error}");
    }

    #[test]
    fn a_failing_run_is_unhealthy_even_though_the_loop_is_alive() {
        let (_dir, path) = heartbeat(false, 1000);
        let error = check(&path, 1001, 300).unwrap_err();
        assert!(format!("{error}").contains("failed"), "{error}");
    }

    #[test]
    fn a_missing_heartbeat_is_unhealthy() {
        let dir = tempfile::tempdir().unwrap();
        assert!(check(&dir.path().join("nope"), 1, 300).is_err());
    }

    #[test]
    fn garbage_is_unhealthy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hb");
        fs::write(&path, "garbage").unwrap();
        assert!(check(&path, 1, 300).is_err());
        fs::write(&path, "5 maybe").unwrap();
        assert!(check(&path, 6, 300).is_err());
    }
}
