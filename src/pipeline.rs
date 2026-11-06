//! One complete run: turn the current containers into published documentation.

use std::fs;
use std::io::ErrorKind;

use anyhow::{bail, Context, Result};

use crate::changelog;
use crate::config::Config;
use crate::docker::Docker;
use crate::log;
use crate::model::Container;
use crate::output::{OutDir, Report};
use crate::render::{render_with_hosts, HostStatus, Options};
use crate::snapshot::Snapshot;
use crate::timefmt::rfc3339_utc;

/// What a run did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub containers: usize,
    pub report: Report,
    pub changelog_entry: bool,
}

/// Reads the containers from every Docker endpoint and publishes them.
///
/// An endpoint that cannot be read does not stop the others: its containers
/// keep their entries from the previous snapshot and the index marks the
/// endpoint as unreachable. The run fails only when no endpoint answers.
pub fn run_once(
    config: &Config,
    dockers: &[Docker],
    now: u64,
    accept_shrink: bool,
) -> Result<Outcome> {
    if dockers.is_empty() {
        bail!("no Docker endpoint to read");
    }
    let multi = dockers.len() > 1;
    if multi {
        let mut seen = std::collections::HashSet::new();
        for docker in dockers {
            let Some(name) = docker.host() else {
                bail!("several Docker endpoints require every endpoint to have a host name");
            };
            if !seen.insert(name) {
                bail!("duplicate Docker host name `{name}`");
            }
        }
    }
    let previous = if multi {
        Snapshot::load(&config.state_dir)?
    } else {
        None
    };
    let percent = config.shrink_guard_percent as usize;

    let mut containers = Vec::new();
    let mut hosts = Vec::new();
    let mut failed = Vec::new();
    for docker in dockers {
        let result = docker.containers().and_then(|found| {
            if let (true, false, Some(previous), Some(name)) =
                (multi, accept_shrink, &previous, docker.host())
            {
                let before = previous
                    .containers
                    .iter()
                    .filter(|c| c.host.as_deref() == Some(name))
                    .count();
                if percent != 0 && before != 0 && found.len() * 100 < before * percent {
                    bail!(
                        "Docker reports {} container(s), down from {before} (the guard allows a \
                         drop to {percent}%); run `cadastre once --accept-shrink` if the drop is real",
                        found.len()
                    );
                }
            }
            Ok(found)
        });
        match result {
            Ok(found) => {
                containers.extend(found);
                if let Some(name) = docker.host() {
                    hosts.push(HostStatus {
                        name: name.to_string(),
                        reachable: true,
                    });
                }
            }
            Err(error) => failed.push((docker, error)),
        }
    }

    if !failed.is_empty() && failed.len() == dockers.len() {
        if failed.len() == 1 {
            return Err(failed.remove(0).1);
        }
        let messages: Vec<String> = failed
            .iter()
            .map(|(docker, error)| format!("{}: {error:#}", docker.host().unwrap_or("docker")))
            .collect();
        bail!("no Docker endpoint could be read: {}", messages.join("; "));
    }

    if !failed.is_empty() {
        if previous
            .as_ref()
            .is_some_and(|p| p.containers.iter().any(|c| c.host.is_none()))
        {
            let messages: Vec<String> = failed
                .iter()
                .map(|(docker, error)| format!("{}: {error:#}", docker.host().unwrap_or("docker")))
                .collect();
            bail!(
                "cannot attribute the previous containers to a Docker host while some endpoints \
                 are unavailable, keeping the current publication: {}",
                messages.join("; ")
            );
        }
        for (docker, error) in failed {
            let host = docker.host();
            log::warn(format!(
                "cannot read {}, keeping its last known containers: {error:#}",
                host.unwrap_or("the Docker endpoint")
            ));
            if let Some(previous) = &previous {
                containers.extend(
                    previous
                        .containers
                        .iter()
                        .filter(|c| c.host.as_deref() == host)
                        .cloned(),
                );
            }
            if let Some(name) = host {
                hosts.push(HostStatus {
                    name: name.to_string(),
                    reachable: false,
                });
            }
        }
    }
    hosts.sort_by(|a, b| a.name.cmp(&b.name));
    publish_with_hosts(config, &containers, &hosts, now, accept_shrink)
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
    publish_with_hosts(config, containers, &[], now, accept_shrink)
}

/// Like [`publish`], and lists `hosts` with their status in the index.
pub fn publish_with_hosts(
    config: &Config,
    containers: &[Container],
    hosts: &[HostStatus],
    now: u64,
    accept_shrink: bool,
) -> Result<Outcome> {
    let previous = Snapshot::load(&config.state_dir)?;
    if !accept_shrink {
        check_shrink(config, previous.as_ref(), containers.len())?;
    }
    let out = OutDir::new(&config.out_dir);

    let changelog_text = changelog_update(&out, previous.as_ref(), containers, now)?;

    let pages = render_with_hosts(
        containers,
        hosts,
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
