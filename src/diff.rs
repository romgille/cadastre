//! What changed between two snapshots.

use std::collections::BTreeMap;

use crate::model::Container;

/// One change to one container. Containers are identified by name: a
/// container recreated under the same name is an update, not a removal and an
/// addition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Added {
        name: String,
        image: String,
    },
    Removed {
        name: String,
    },
    /// The image reference or the image content changed.
    ImageChanged {
        name: String,
        from: String,
        to: String,
    },
    /// Anything else that is documented changed; `fields` names what.
    Reconfigured {
        name: String,
        fields: Vec<&'static str>,
    },
}

impl Change {
    pub fn name(&self) -> &str {
        match self {
            Self::Added { name, .. }
            | Self::Removed { name }
            | Self::ImageChanged { name, .. }
            | Self::Reconfigured { name, .. } => name,
        }
    }
}

/// Changes needed to go from `old` to `new`, sorted by container name.
pub fn diff(old: &[Container], new: &[Container]) -> Vec<Change> {
    let old: BTreeMap<&str, &Container> = old.iter().map(|c| (c.name.as_str(), c)).collect();
    let new: BTreeMap<&str, &Container> = new.iter().map(|c| (c.name.as_str(), c)).collect();

    let mut changes = Vec::new();
    for (name, current) in &new {
        match old.get(name) {
            None => changes.push(Change::Added {
                name: (*name).into(),
                image: current.image.clone(),
            }),
            Some(previous) => changes.extend(compare(previous, current)),
        }
    }
    for name in old.keys().filter(|name| !new.contains_key(*name)) {
        changes.push(Change::Removed {
            name: (*name).into(),
        });
    }
    changes.sort_by(|a, b| a.name().cmp(b.name()));
    changes
}

fn compare(old: &Container, new: &Container) -> Vec<Change> {
    let mut changes = Vec::new();
    if old.image != new.image || old.image_id != new.image_id || old.version != new.version {
        changes.push(Change::ImageChanged {
            name: new.name.clone(),
            from: describe_image(old),
            to: describe_image(new),
        });
    }

    let checks: [(&'static str, bool); 8] = [
        ("restart policy", old.restart_policy != new.restart_policy),
        ("network mode", old.network_mode != new.network_mode),
        ("ports", old.ports != new.ports),
        ("mounts", old.mounts != new.mounts),
        ("networks", old.networks != new.networks),
        ("environment variables", old.env_keys != new.env_keys),
        ("labels", old.labels != new.labels),
        (
            "stack",
            old.project != new.project || old.service != new.service,
        ),
    ];
    let fields: Vec<&'static str> = checks
        .into_iter()
        .filter_map(|(field, changed)| changed.then_some(field))
        .collect();
    if !fields.is_empty() {
        changes.push(Change::Reconfigured {
            name: new.name.clone(),
            fields,
        });
    }
    changes
}

/// `image` plus the version label or short ID, so that a changed digest under
/// an unchanged tag (`latest`) still reads as a change.
fn describe_image(c: &Container) -> String {
    match &c.version {
        Some(version) => format!("{} ({version})", c.image),
        None => format!("{} ({})", c.image, c.short_image_id()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn container(name: &str) -> Container {
        Container {
            name: name.into(),
            image: "nginx:1".into(),
            image_id: "sha256:aaaaaaaaaaaaaaaa".into(),
            version: None,
            project: Some("shop".into()),
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
    fn identical_states_have_no_changes() {
        let state = [container("a"), container("b")];
        assert!(diff(&state, &state).is_empty());
    }

    #[test]
    fn added_and_removed_containers_are_detected() {
        let changes = diff(&[container("old")], &[container("new")]);
        assert_eq!(
            changes,
            [
                Change::Added {
                    name: "new".into(),
                    image: "nginx:1".into()
                },
                Change::Removed { name: "old".into() },
            ]
        );
    }

    #[test]
    fn a_new_digest_under_the_same_tag_is_an_image_change() {
        let old = container("a");
        let mut new = container("a");
        new.image_id = "sha256:bbbbbbbbbbbbbbbb".into();
        let changes = diff(&[old], &[new]);
        assert_eq!(
            changes,
            [Change::ImageChanged {
                name: "a".into(),
                from: "nginx:1 (aaaaaaaaaaaa)".into(),
                to: "nginx:1 (bbbbbbbbbbbb)".into(),
            }]
        );
    }

    #[test]
    fn the_version_label_is_preferred_when_present() {
        let mut old = container("a");
        old.version = Some("1.0".into());
        let mut new = container("a");
        new.version = Some("1.1".into());
        match &diff(&[old], &[new])[0] {
            Change::ImageChanged { from, to, .. } => {
                assert_eq!(from, "nginx:1 (1.0)");
                assert_eq!(to, "nginx:1 (1.1)");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn other_differences_name_what_changed() {
        let old = container("a");
        let mut new = container("a");
        new.restart_policy = "always".into();
        new.env_keys = vec!["TOKEN".into()];
        assert_eq!(
            diff(&[old], &[new]),
            [Change::Reconfigured {
                name: "a".into(),
                fields: vec!["restart policy", "environment variables"],
            }]
        );
    }

    #[test]
    fn changes_are_sorted_by_name() {
        let changes = diff(&[], &[container("z"), container("a")]);
        let names: Vec<_> = changes.iter().map(Change::name).collect();
        assert_eq!(names, ["a", "z"]);
    }
}
