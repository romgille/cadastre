//! The sanitized view of a container.
//!
//! This is the only shape in which container data leaves the sanitizer. It is
//! deliberately small: every field is something that is safe to write into a
//! shared Markdown folder. Anything that is not listed here (environment
//! values, command lines, health-check commands, hostnames) cannot reach the
//! documentation, because there is nowhere to put it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Group name for containers that were not started by Docker Compose.
/// Compose project names cannot start with an underscore, so this cannot
/// collide with a real project.
pub const STANDALONE: &str = "_standalone";

/// What cadastre knows about one container.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Container {
    /// Container name without the leading slash.
    pub name: String,
    /// Friendly name of the Docker endpoint the container runs on. `None`
    /// when a single, unnamed endpoint is configured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// Image reference the container was created from, e.g. `nginx:1.27`.
    pub image: String,
    /// Content-addressed image ID (`sha256:...`). Unlike the tag, it changes
    /// whenever the image is updated.
    pub image_id: String,
    /// Value of the `org.opencontainers.image.version` label, when present.
    pub version: Option<String>,
    /// Docker Compose project, when the container belongs to one.
    pub project: Option<String>,
    /// Docker Compose service, when the container belongs to one.
    pub service: Option<String>,
    /// Restart policy name (`no`, `always`, `unless-stopped`, `on-failure`).
    pub restart_policy: String,
    /// Network mode (`default`, `host`, `bridge`, a network name, ...).
    pub network_mode: String,
    pub ports: Vec<Port>,
    pub mounts: Vec<Mount>,
    /// Names of the networks the container is attached to, sorted.
    pub networks: Vec<String>,
    /// Names (never values) of the environment variables, sorted.
    pub env_keys: Vec<String>,
    /// Allowlisted labels only, see `sanitize::label_allowed`.
    pub labels: BTreeMap<String, String>,
}

impl Container {
    /// The name that identifies the container among all endpoints:
    /// `name@host`, or just `name` without a host.
    pub fn display_name(&self) -> String {
        match &self.host {
            Some(host) => format!("{}@{host}", self.name),
            None => self.name.clone(),
        }
    }

    /// The group this container is documented under.
    pub fn stack(&self) -> &str {
        self.project.as_deref().unwrap_or(STANDALONE)
    }

    /// Whether the container shares the host's network stack, which exposes
    /// every port it listens on.
    pub fn uses_host_network(&self) -> bool {
        self.network_mode == "host"
    }

    /// The first twelve characters of the image ID, without the `sha256:`
    /// prefix: enough to tell two images apart at a glance.
    pub fn short_image_id(&self) -> &str {
        let id = self
            .image_id
            .strip_prefix("sha256:")
            .unwrap_or(&self.image_id);
        id.get(..12).unwrap_or(id)
    }
}

/// A port of a container, published on the host or merely exposed.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Port {
    pub container_port: u16,
    /// `tcp`, `udp` or `sctp`.
    pub protocol: String,
    /// Host address the port is bound to. `None` when not published.
    pub host_ip: Option<String>,
    /// Host port. `None` when not published.
    pub host_port: Option<u16>,
}

impl Port {
    pub fn is_published(&self) -> bool {
        self.host_port.is_some()
    }

    /// Published on every interface of the host, rather than on a specific
    /// address such as `127.0.0.1` or a private network address.
    pub fn is_on_all_interfaces(&self) -> bool {
        self.is_published() && matches!(self.host_ip.as_deref(), None | Some("0.0.0.0" | "::"))
    }
}

/// A mount of a container.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Mount {
    /// `bind`, `volume` or `tmpfs`.
    pub kind: String,
    /// Host path for bind mounts, volume name for volumes, empty for tmpfs.
    pub source: String,
    /// Path inside the container.
    pub destination: String,
    pub read_write: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn container(project: Option<&str>) -> Container {
        Container {
            name: "web-1".into(),
            host: None,
            image: "nginx:1.27".into(),
            image_id: "sha256:abc".into(),
            version: None,
            project: project.map(String::from),
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
    fn compose_containers_are_grouped_by_project() {
        assert_eq!(container(Some("shop")).stack(), "shop");
    }

    #[test]
    fn other_containers_fall_into_the_standalone_group() {
        assert_eq!(container(None).stack(), STANDALONE);
    }

    #[test]
    fn the_short_image_id_drops_the_algorithm_and_is_twelve_characters_long() {
        let mut c = container(None);
        c.image_id = "sha256:0123456789abcdef0123".into();
        assert_eq!(c.short_image_id(), "0123456789ab");
        c.image_id = "abc".into();
        assert_eq!(c.short_image_id(), "abc");
    }

    #[test]
    fn only_published_wildcard_bindings_count_as_all_interfaces() {
        let port = |ip: Option<&str>, host_port: Option<u16>| Port {
            container_port: 80,
            protocol: "tcp".into(),
            host_ip: ip.map(String::from),
            host_port,
        };
        assert!(port(Some("0.0.0.0"), Some(80)).is_on_all_interfaces());
        assert!(port(Some("::"), Some(80)).is_on_all_interfaces());
        assert!(!port(Some("127.0.0.1"), Some(80)).is_on_all_interfaces());
        assert!(!port(Some("10.0.0.5"), Some(80)).is_on_all_interfaces());
        assert!(
            !port(None, None).is_on_all_interfaces(),
            "unpublished ports are not exposed"
        );
    }

    #[test]
    fn host_networking_is_detected() {
        let mut c = container(None);
        assert!(!c.uses_host_network());
        c.network_mode = "host".into();
        assert!(c.uses_host_network());
    }

    #[test]
    fn ports_sort_by_container_port_then_protocol() {
        let port = |p, proto: &str| Port {
            container_port: p,
            protocol: proto.into(),
            host_ip: None,
            host_port: None,
        };
        let mut ports = [port(443, "tcp"), port(53, "udp"), port(53, "tcp")];
        ports.sort();
        let order: Vec<_> = ports
            .iter()
            .map(|p| (p.container_port, p.protocol.as_str()))
            .collect();
        assert_eq!(order, [(53, "tcp"), (53, "udp"), (443, "tcp")]);
    }
}
