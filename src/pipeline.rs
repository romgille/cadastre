//! One complete run: turn the current containers into published documentation.

use std::fs;
use std::io::ErrorKind;

use anyhow::{bail, Context, Result};

use crate::changelog;
use crate::config::Config;
use crate::docker::Docker;
use crate::model::Container;
use crate::output::{OutDir, Report};
use crate::render::{render, Options};
use crate::snapshot::Snapshot;
use crate::timefmt::rfc3339_utc;

/// What a run did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub containers: usize,
    pub report: Report,
    pub changelog_entry: bool,
}

/// Reads the containers from Docker and publishes them.
pub fn run_once(
    config: &Config,
    docker: &Docker,
    now: u64,
    accept_shrink: bool,
) -> Result<Outcome> {
    let containers = docker.containers()?;
    publish(config, &containers, now, accept_shrink)
}

/// Publishes `containers`: pages, then changelog, then snapshot.
///
/// The order matters. Everything that can be refused (a changelog written by
/// a person, a corrupt snapshot) is checked before the first byte is written.
/// The snapshot is saved last: a crash in between repeats a changelog entry
/// on the next run, which is better than losing one.
pub fn publish(
    config: &Config,
    containers: &[Container],
    now: u64,
    accept_shrink: bool,
) -> Result<Outcome> {
    let previous = Snapshot::load(&config.state_dir)?;
    if !accept_shrink {
        check_shrink(config, previous.as_ref(), containers.len())?;
    }
    let out = OutDir::new(&config.out_dir);

    let changelog_text = changelog_update(&out, previous.as_ref(), containers, now)?;

    let pages = render(
        containers,
        &Options {
            obsidian: config.obsidian,
        },
    );
    let report = out.sync(&pages)?;

    let changelog_entry = if let Some(text) = changelog_text {
        out.write_if_changed(changelog::PATH, text.as_bytes())?;
        true
    } else {
        false
    };

    Snapshot::new(containers.to_vec()).save(&config.state_dir)?;

    Ok(Outcome {
        containers: containers.len(),
        report,
        changelog_entry,
    })
}

/// The new content of the changelog, or `None` if nothing changed.
fn changelog_update(
    out: &OutDir,
    previous: Option<&Snapshot>,
    containers: &[Container],
    now: u64,
) -> Result<Option<String>> {
    let before = previous.map(|snapshot| snapshot.containers.as_slice());
    let Some(entry) = changelog::entry(&rfc3339_utc(now), before, containers) else {
        return Ok(None);
    };
    let path = out.root().join(changelog::PATH);
    let existing = match fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => return Err(error).with_context(|| format!("cannot read {}", path.display())),
    };
    changelog::prepend(existing.as_deref(), &entry).map(Some)
}

/// Refuses a result that is implausibly small compared with the last
/// snapshot.
///
/// A socket proxy that lost its permissions, a wrong endpoint or a restarting
/// daemon can all answer with an empty list and no error. Publishing that
/// would delete every page and record a history of "everything was removed"
/// that never happened. Failing instead is loud: the run is marked failed and
/// the success timestamp stops advancing.
fn check_shrink(config: &Config, previous: Option<&Snapshot>, now_count: usize) -> Result<()> {
    let percent = config.shrink_guard_percent as usize;
    let Some(previous) = previous else {
        return Ok(());
    };
    let before = previous.containers.len();
    if percent == 0 || before == 0 || now_count * 100 >= before * percent {
        return Ok(());
    }
    bail!(
        "refusing to publish: Docker reports {now_count} container(s), down from {before} \
         (the guard allows a drop to {percent}%). This usually means a broken socket proxy or \
         a wrong endpoint. If the drop is real, run `cadastre once --accept-shrink`"
    )
}
