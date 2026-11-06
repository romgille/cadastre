//! Small file-system helpers.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Replaces `dest` with `contents` atomically.
///
/// The data goes to a temporary file in `tmp_dir` and is then renamed over
/// `dest`, so a reader (a sync tool, node_exporter, a person with an editor)
/// never sees a half-written file. `tmp_dir` must be on the same file system
/// as `dest`, otherwise the rename fails loudly. Missing directories are
/// created.
pub fn atomic_write(tmp_dir: &Path, dest: &Path, contents: &[u8]) -> Result<()> {
    fs::create_dir_all(tmp_dir).with_context(|| format!("cannot create {}", tmp_dir.display()))?;
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }

    let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = tmp_dir.join(format!(".{}-{sequence}.tmp", std::process::id()));
    let result = write_then_rename(&tmp, dest, contents);
    if result.is_err() {
        // Best effort: the original error is the one worth reporting.
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn write_then_rename(tmp: &Path, dest: &Path, contents: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(tmp)
        .with_context(|| format!("cannot create {}", tmp.display()))?;
    file.write_all(contents)
        .with_context(|| format!("cannot write {}", tmp.display()))?;
    // Make sure the bytes are on disk before the name points at them.
    file.sync_all()
        .with_context(|| format!("cannot sync {}", tmp.display()))?;
    fs::rename(tmp, dest)
        .with_context(|| format!("cannot move {} to {}", tmp.display(), dest.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_the_contents_and_creates_missing_directories() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("a/b/file.txt");
        atomic_write(&dir.path().join(".tmp"), &dest, b"hello").unwrap();
        assert_eq!(fs::read_to_string(&dest).unwrap(), "hello");
    }

    #[test]
    fn replaces_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("file.txt");
        fs::write(&dest, "old").unwrap();
        atomic_write(&dir.path().join(".tmp"), &dest, b"new").unwrap();
        assert_eq!(fs::read_to_string(&dest).unwrap(), "new");
    }

    #[test]
    fn leaves_no_temporary_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        let tmp = dir.path().join(".tmp");
        atomic_write(&tmp, &dir.path().join("file.txt"), b"x").unwrap();
        assert_eq!(fs::read_dir(&tmp).unwrap().count(), 0);
    }

    #[test]
    fn a_failed_rename_reports_an_error_and_cleans_up() {
        let dir = tempfile::tempdir().unwrap();
        let tmp = dir.path().join(".tmp");
        // A directory cannot be replaced by a file.
        let dest = dir.path().join("occupied");
        fs::create_dir(&dest).unwrap();
        let error = atomic_write(&tmp, &dest, b"x").unwrap_err();
        assert!(format!("{error:#}").contains("cannot move"), "{error:#}");
        assert_eq!(fs::read_dir(&tmp).unwrap().count(), 0);
    }
}
