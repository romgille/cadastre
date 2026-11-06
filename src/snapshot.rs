//! The state remembered between runs, used to work out what changed.

use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::fsutil::atomic_write;
use crate::model::Container;

/// Bumped when the stored shape changes incompatibly.
const FORMAT: u32 = 1;

/// File name inside the state folder.
pub const FILE_NAME: &str = "snapshot.json";

/// The sanitized containers as of the last successful publication.
///
/// Only sanitized data is stored, so the state folder holds nothing that the
/// output folder does not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub format: u32,
    pub containers: Vec<Container>,
}

impl Snapshot {
    pub fn new(mut containers: Vec<Container>) -> Self {
        containers.sort_by(|a, b| a.name.cmp(&b.name));
        Self {
            format: FORMAT,
            containers,
        }
    }

    /// Loads the snapshot. `Ok(None)` means there is none yet (first run).
    /// A snapshot that exists but cannot be read is an error: treating it as
    /// absent would silently reset the changelog.
    pub fn load(state_dir: &Path) -> Result<Option<Self>> {
        let path = state_dir.join(FILE_NAME);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error).with_context(|| format!("cannot read {}", path.display()))
            }
        };
        let snapshot: Self = serde_json::from_slice(&bytes).with_context(|| {
            format!(
                "{} is corrupt; fix or delete it to start a new history",
                path.display()
            )
        })?;
        if snapshot.format != FORMAT {
            bail!(
                "{} has format {}, this build understands {FORMAT}",
                path.display(),
                snapshot.format
            );
        }
        Ok(Some(snapshot))
    }

    pub fn save(&self, state_dir: &Path) -> Result<()> {
        let json = serde_json::to_vec_pretty(self).context("cannot serialize the snapshot")?;
        atomic_write(&state_dir.join(".tmp"), &state_dir.join(FILE_NAME), &json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn container(name: &str) -> Container {
        Container {
            name: name.into(),
            image: "nginx:1".into(),
            image_id: "sha256:abc".into(),
            version: None,
            project: None,
            service: None,
            restart_policy: "no".into(),
            network_mode: "default".into(),
            ports: vec![],
            mounts: vec![],
            networks: vec![],
            env_keys: vec![],
            labels: BTreeMap::new(),
        }
    }

    #[test]
    fn there_is_no_snapshot_on_the_first_run() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Snapshot::load(dir.path()).unwrap(), None);
    }

    #[test]
    fn a_saved_snapshot_loads_back_identically() {
        let dir = tempfile::tempdir().unwrap();
        let snapshot = Snapshot::new(vec![container("b"), container("a")]);
        snapshot.save(dir.path()).unwrap();
        let loaded = Snapshot::load(dir.path()).unwrap().unwrap();
        assert_eq!(loaded, snapshot);
        assert_eq!(loaded.containers[0].name, "a", "sorted by name");
    }

    #[test]
    fn a_corrupt_snapshot_is_an_error_not_an_empty_history() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(FILE_NAME), "{ not json").unwrap();
        let error = Snapshot::load(dir.path()).unwrap_err();
        assert!(format!("{error:#}").contains("corrupt"), "{error:#}");
    }

    #[test]
    fn an_unknown_format_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(FILE_NAME),
            r#"{"format": 99, "containers": []}"#,
        )
        .unwrap();
        let error = Snapshot::load(dir.path()).unwrap_err();
        assert!(format!("{error:#}").contains("format 99"), "{error:#}");
    }
}
