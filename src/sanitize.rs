//! Turns the raw output of `docker inspect` into a [`Container`].
//!
//! The rule is an allowlist. Each field of the model is copied explicitly from
//! a known place in the inspect document and nothing is forwarded by default,
//! so a field added to the Docker API tomorrow cannot leak into the
//! documentation. Text coming from Docker is also cleaned (control characters
//! removed, length capped) because it ends up in files that other tools
//! render.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{anyhow, Context, Result};
use serde_json::{Map, Value};

use crate::model::{Container, Mount, Port};

const LABEL_PROJECT: &str = "com.docker.compose.project";
const LABEL_SERVICE: &str = "com.docker.compose.service";
const LABEL_VERSION: &str = "org.opencontainers.image.version";

/// Longest text value kept, in characters.
const MAX_TEXT_CHARS: usize = 512;

/// Extracts the documentation facts from one `docker inspect` document.
///
/// The raw document is not kept: the caller is expected to drop it right
/// after this call.
pub fn container_from_inspect(inspect: &Value) -> Result<Container> {
    let name = text(inspect, "/Name")
        .map(|name| clean(name.trim_start_matches('/')))
        .filter(|name| !name.is_empty())
        .ok_or_else(|| anyhow!("inspect document has no container name"))?;
    build(inspect, &name).with_context(|| format!("container {name:?}"))
}

fn build(inspect: &Value, name: &str) -> Result<Container> {
    let labels = inspect.pointer("/Config/Labels").and_then(Value::as_object);
    let label = |key: &str| {
        labels
            .and_then(|all| all.get(key))
            .and_then(Value::as_str)
            .map(clean)
            .filter(|value| !value.is_empty())
    };

    Ok(Container {
        name: name.to_string(),
        image: clean(text(inspect, "/Config/Image").unwrap_or_default()),
        image_id: clean(text(inspect, "/Image").unwrap_or_default()),
        version: label(LABEL_VERSION),
        project: label(LABEL_PROJECT),
        service: label(LABEL_SERVICE),
        restart_policy: match text(inspect, "/HostConfig/RestartPolicy/Name") {
            None | Some("") => "no".to_string(),
            Some(policy) => clean(policy),
        },
        network_mode: clean(text(inspect, "/HostConfig/NetworkMode").unwrap_or("default")),
        ports: ports(inspect)?,
        mounts: mounts(inspect),
        networks: networks(inspect),
        env_keys: env_keys(inspect),
        labels: allowed_labels(labels),
    })
}

/// Whether a label may appear in the documentation.
///
/// Labels are free-form and routinely carry credentials (basic-auth hashes,
/// API keys), so the default is to drop them. Only keys that are known to be
/// descriptive are allowed.
pub fn label_allowed(key: &str) -> bool {
    key.starts_with("org.opencontainers.image.")
}

fn allowed_labels(labels: Option<&Map<String, Value>>) -> BTreeMap<String, String> {
    let mut kept = BTreeMap::new();
    for (key, value) in labels.into_iter().flatten() {
        // Labels promoted to dedicated fields are not repeated here.
        let promoted = [LABEL_PROJECT, LABEL_SERVICE, LABEL_VERSION].contains(&key.as_str());
        if promoted || !label_allowed(key) {
            continue;
        }
        if let Some(value) = value.as_str() {
            kept.insert(clean(key), clean(value));
        }
    }
    kept
}

fn ports(inspect: &Value) -> Result<Vec<Port>> {
    let mut ports = Vec::new();
    let mut published = BTreeSet::new();

    // The configured bindings are used rather than `NetworkSettings.Ports`,
    // which is empty while the container is stopped: documentation describes
    // configuration, not runtime state.
    if let Some(bindings) = inspect
        .pointer("/HostConfig/PortBindings")
        .and_then(Value::as_object)
    {
        for (key, entries) in bindings {
            let (container_port, protocol) = parse_port_key(key)?;
            for entry in entries.as_array().into_iter().flatten() {
                // An empty host port means "pick any": nothing stable to document.
                let Some(host_port) = text(entry, "/HostPort").filter(|port| !port.is_empty())
                else {
                    continue;
                };
                let host_port = host_port
                    .parse::<u16>()
                    .map_err(|_| anyhow!("invalid host port {host_port:?} for {key}"))?;
                // An empty host IP means every interface.
                let host_ip = match text(entry, "/HostIp") {
                    None | Some("") => "0.0.0.0",
                    Some(ip) => ip,
                };
                ports.push(Port {
                    container_port,
                    protocol: protocol.clone(),
                    host_ip: Some(clean(host_ip)),
                    host_port: Some(host_port),
                });
                published.insert((container_port, protocol.clone()));
            }
        }
    }

    if let Some(exposed) = inspect
        .pointer("/Config/ExposedPorts")
        .and_then(Value::as_object)
    {
        for key in exposed.keys() {
            let (container_port, protocol) = parse_port_key(key)?;
            if !published.contains(&(container_port, protocol.clone())) {
                ports.push(Port {
                    container_port,
                    protocol,
                    host_ip: None,
                    host_port: None,
                });
            }
        }
    }

    ports.sort();
    ports.dedup();
    Ok(ports)
}

fn parse_port_key(key: &str) -> Result<(u16, String)> {
    let (port, protocol) = key.split_once('/').unwrap_or((key, "tcp"));
    let port = port
        .parse::<u16>()
        .map_err(|_| anyhow!("invalid port {key:?}"))?;
    Ok((port, clean(protocol)))
}

fn mounts(inspect: &Value) -> Vec<Mount> {
    let mut mounts: Vec<Mount> = inspect
        .pointer("/Mounts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|mount| {
            let kind = text(mount, "/Type").unwrap_or("bind");
            // The source of a volume is a path inside Docker's own storage
            // area, which says nothing useful: the volume name does.
            let source = if kind == "volume" {
                text(mount, "/Name")
            } else {
                text(mount, "/Source")
            };
            Mount {
                kind: clean(kind),
                source: clean(source.unwrap_or_default()),
                destination: clean(text(mount, "/Destination").unwrap_or_default()),
                read_write: mount
                    .pointer("/RW")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
            }
        })
        .collect();
    mounts.sort();
    mounts
}

fn networks(inspect: &Value) -> Vec<String> {
    let names: BTreeSet<String> = inspect
        .pointer("/NetworkSettings/Networks")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(Map::keys)
        .map(|name| clean(name))
        .collect();
    names.into_iter().collect()
}

/// Only the names of environment variables, never their values.
fn env_keys(inspect: &Value) -> Vec<String> {
    let keys: BTreeSet<String> = inspect
        .pointer("/Config/Env")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|entry| clean(entry.split('=').next().unwrap_or_default()))
        .filter(|key| !key.is_empty())
        .collect();
    keys.into_iter().collect()
}

fn text<'a>(value: &'a Value, pointer: &str) -> Option<&'a str> {
    value.pointer(pointer)?.as_str()
}

/// Replaces control characters (line breaks included), trims and caps the
/// length of a text coming from Docker.
fn clean(raw: &str) -> String {
    let flattened: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let trimmed = flattened.trim();
    if trimmed.chars().count() > MAX_TEXT_CHARS {
        let kept: String = trimmed.chars().take(MAX_TEXT_CHARS - 1).collect();
        format!("{kept}…")
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn sample() -> Value {
        json!({
            "Name": "/shop-web-1",
            "Image": "sha256:0123456789abcdef",
            "Config": {
                "Image": "ghcr.io/example/web:2.4.1",
                "Env": ["PATH=/usr/bin", "DB_PASSWORD=hunter2", "FLAG", "URL=http://a?b=c"],
                "Labels": {
                    "com.docker.compose.project": "shop",
                    "com.docker.compose.service": "web",
                    "org.opencontainers.image.version": "2.4.1",
                    "org.opencontainers.image.source": "https://example.com/web",
                    "com.example.api-key": "hunter2"
                },
                "ExposedPorts": {"80/tcp": {}, "5432/tcp": {}}
            },
            "HostConfig": {
                "RestartPolicy": {"Name": "unless-stopped"},
                "NetworkMode": "shop_default",
                "PortBindings": {
                    "80/tcp": [{"HostIp": "", "HostPort": "8080"}],
                    "53/udp": [{"HostIp": "127.0.0.1", "HostPort": "5353"}],
                    "9000/tcp": null
                }
            },
            "Mounts": [
                {"Type": "volume", "Name": "shop_cache", "Source": "/var/lib/docker/volumes/shop_cache/_data", "Destination": "/cache", "RW": true},
                {"Type": "bind", "Source": "/srv/uploads", "Destination": "/uploads", "RW": false}
            ],
            "NetworkSettings": {"Networks": {"shop_default": {}, "proxy": {}}}
        })
    }

    #[test]
    fn basic_facts_are_extracted() {
        let c = container_from_inspect(&sample()).unwrap();
        assert_eq!(c.name, "shop-web-1");
        assert_eq!(c.image, "ghcr.io/example/web:2.4.1");
        assert_eq!(c.image_id, "sha256:0123456789abcdef");
        assert_eq!(c.version.as_deref(), Some("2.4.1"));
        assert_eq!(c.project.as_deref(), Some("shop"));
        assert_eq!(c.service.as_deref(), Some("web"));
        assert_eq!(c.restart_policy, "unless-stopped");
        assert_eq!(c.network_mode, "shop_default");
        assert_eq!(c.networks, ["proxy", "shop_default"]);
    }

    #[test]
    fn only_environment_names_are_kept() {
        let c = container_from_inspect(&sample()).unwrap();
        assert_eq!(c.env_keys, ["DB_PASSWORD", "FLAG", "PATH", "URL"]);
        let serialized = serde_json::to_string(&c).unwrap();
        assert!(!serialized.contains("hunter2"), "{serialized}");
    }

    #[test]
    fn published_and_exposed_ports_are_told_apart() {
        let c = container_from_inspect(&sample()).unwrap();
        let ports: Vec<_> = c
            .ports
            .iter()
            .map(|p| {
                (
                    p.container_port,
                    p.protocol.as_str(),
                    p.host_ip.as_deref(),
                    p.host_port,
                )
            })
            .collect();
        assert_eq!(
            ports,
            [
                (53, "udp", Some("127.0.0.1"), Some(5353)),
                (80, "tcp", Some("0.0.0.0"), Some(8080)),
                (5432, "tcp", None, None),
            ]
        );
    }

    #[test]
    fn volumes_are_named_and_bind_mounts_keep_their_host_path() {
        let c = container_from_inspect(&sample()).unwrap();
        let mounts: Vec<_> = c
            .mounts
            .iter()
            .map(|m| {
                (
                    m.kind.as_str(),
                    m.source.as_str(),
                    m.destination.as_str(),
                    m.read_write,
                )
            })
            .collect();
        assert_eq!(
            mounts,
            [
                ("bind", "/srv/uploads", "/uploads", false),
                ("volume", "shop_cache", "/cache", true)
            ]
        );
    }

    #[test]
    fn only_allowlisted_labels_survive() {
        let c = container_from_inspect(&sample()).unwrap();
        let keys: Vec<_> = c.labels.keys().map(String::as_str).collect();
        assert_eq!(keys, ["org.opencontainers.image.source"]);
    }

    #[test]
    fn a_missing_restart_policy_means_no() {
        let mut inspect = sample();
        inspect["HostConfig"]["RestartPolicy"]["Name"] = json!("");
        assert_eq!(
            container_from_inspect(&inspect).unwrap().restart_policy,
            "no"
        );
    }

    #[test]
    fn text_is_flattened_and_capped() {
        let mut inspect = sample();
        inspect["Config"]["Image"] = json!(format!("line one\nline two {}", "x".repeat(600)));
        let c = container_from_inspect(&inspect).unwrap();
        assert!(!c.image.contains('\n'));
        assert_eq!(c.image.chars().count(), MAX_TEXT_CHARS);
        assert!(c.image.ends_with('…'));
    }

    #[test]
    fn a_document_without_a_name_is_rejected() {
        let error = container_from_inspect(&json!({"Config": {}})).unwrap_err();
        assert!(error.to_string().contains("no container name"));
    }

    #[test]
    fn errors_name_the_offending_container() {
        let mut inspect = sample();
        inspect["HostConfig"]["PortBindings"] = json!({"http/tcp": [{"HostPort": "80"}]});
        let error = format!("{:#}", container_from_inspect(&inspect).unwrap_err());
        assert!(
            error.contains("shop-web-1") && error.contains("http/tcp"),
            "{error}"
        );
    }
}
