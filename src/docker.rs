//! A minimal read-only client for the Docker Engine API.
//!
//! It speaks plain HTTP to a TCP endpoint, normally a socket proxy that only
//! allows `GET` on containers. It never needs, and never uses, anything that
//! changes state.

use std::io::{BufRead, BufReader};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;

use crate::model::Container;
use crate::sanitize::container_from_inspect;

const CALL_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// `{"type":["container","network"]}`, percent-encoded for a query string.
const EVENT_FILTERS: &str = "%7B%22type%22%3A%5B%22container%22%2C%22network%22%5D%7D";

#[derive(Debug, Clone)]
pub struct Docker {
    base: String,
    calls: ureq::Agent,
    stream: ureq::Agent,
}

impl Docker {
    pub fn new(base_url: &str) -> Self {
        let agent = |timeout: Option<Duration>| {
            ureq::Agent::config_builder()
                .http_status_as_error(false)
                .timeout_connect(Some(CONNECT_TIMEOUT))
                .timeout_global(timeout)
                .build()
                .new_agent()
        };
        Self {
            base: base_url.trim_end_matches('/').to_string(),
            calls: agent(Some(CALL_TIMEOUT)),
            // The event stream is idle for as long as nothing happens, so it
            // has no deadline. A dead connection is caught by the periodic
            // reconcile, not by this client.
            stream: agent(None),
        }
    }

    /// Fetches, inspects and sanitizes every container, running or not.
    ///
    /// A container that disappears between the listing and its inspection is
    /// skipped: that is a normal race, not an error.
    pub fn containers(&self) -> Result<Vec<Container>> {
        let mut containers = Vec::new();
        for id in self.container_ids()? {
            let Some(document) = self.inspect(&id)? else {
                continue;
            };
            let container = container_from_inspect(&document)
                .with_context(|| format!("cannot document container {}", short(&id)))?;
            containers.push(container);
        }
        Ok(containers)
    }

    pub fn container_ids(&self) -> Result<Vec<String>> {
        let mut response = self
            .calls
            .get(&self.url("/containers/json?all=true"))
            .call()?;
        check(response.status().as_u16(), "/containers/json")?;
        let list = read_json(&mut response, "the container list")?;
        let items = list
            .as_array()
            .ok_or_else(|| anyhow!("the container list is not an array"))?;
        Ok(items
            .iter()
            .filter_map(|item| item.get("Id").and_then(Value::as_str))
            .map(String::from)
            .collect())
    }

    /// The raw inspect document, or `None` if the container no longer exists.
    pub fn inspect(&self, id: &str) -> Result<Option<Value>> {
        let path = format!("/containers/{id}/json");
        let mut response = self.calls.get(&self.url(&path)).call()?;
        let status = response.status().as_u16();
        if status == 404 {
            return Ok(None);
        }
        check(status, &path)?;
        let what = format!("the inspect output of {}", short(id));
        Ok(Some(read_json(&mut response, &what)?))
    }

    /// Opens the event stream. Only container and network events are asked
    /// for; of those, [`Event::is_relevant`] tells which matter.
    pub fn events(&self) -> Result<Events> {
        let url = self.url(&format!("/events?filters={EVENT_FILTERS}"));
        let response = self.stream.get(&url).call()?;
        check(response.status().as_u16(), "/events")?;
        let reader = response.into_body().into_reader();
        Ok(Events {
            lines: Box::new(BufReader::new(reader)),
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }
}

fn read_json(response: &mut ureq::http::Response<ureq::Body>, what: &str) -> Result<Value> {
    let text = response
        .body_mut()
        .read_to_string()
        .with_context(|| format!("cannot read {what}"))?;
    serde_json::from_str(&text).with_context(|| format!("{what} is not valid JSON"))
}

fn short(id: &str) -> &str {
    id.get(..12).unwrap_or(id)
}

fn check(status: u16, path: &str) -> Result<()> {
    match status {
        200..=299 => Ok(()),
        403 => bail!(
            "GET {path} was refused (403). If a socket proxy is in front of Docker, \
             it must allow the sections cadastre reads: CONTAINERS=1 for the \
             container list and inspection, EVENTS=1 for the event stream"
        ),
        other => bail!("GET {path} failed with HTTP status {other}"),
    }
}

/// A stream of Docker events. Iteration ends when the daemon closes the
/// connection, which callers must treat as a reason to reconnect.
pub struct Events {
    lines: Box<dyn BufRead + Send>,
}

impl Iterator for Events {
    type Item = Result<Event>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let mut line = String::new();
            match self.lines.read_line(&mut line) {
                Ok(0) => return None,
                Err(error) => return Some(Err(error).context("the event stream broke")),
                Ok(_) => {}
            }
            if line.trim().is_empty() {
                continue;
            }
            return Some(
                serde_json::from_str::<Value>(&line)
                    .context("an event is not valid JSON")
                    .map(|value| Event::from_json(&value)),
            );
        }
    }
}

/// The part of a Docker event that cadastre looks at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub kind: String,
    pub action: String,
}

impl Event {
    fn from_json(value: &Value) -> Self {
        let text = |key| value.get(key).and_then(Value::as_str).unwrap_or("");
        Self {
            kind: text("Type").to_string(),
            action: text("Action").to_string(),
        }
    }

    /// Whether the event can change what is documented.
    ///
    /// Health checks and `exec` sessions fire constantly and change nothing
    /// that is written down. Network events matter only when a container
    /// joins or leaves a network.
    pub fn is_relevant(&self) -> bool {
        match self.kind.as_str() {
            "container" => {
                !(self.action.starts_with("exec_")
                    || self.action.starts_with("health_status")
                    || self.action.starts_with("attach")
                    || self.action == "resize")
            }
            "network" => matches!(self.action.as_str(), "connect" | "disconnect"),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(kind: &str, action: &str) -> Event {
        Event {
            kind: kind.into(),
            action: action.into(),
        }
    }

    #[test]
    fn lifecycle_events_are_relevant() {
        for action in ["create", "start", "die", "destroy", "rename", "update"] {
            assert!(event("container", action).is_relevant(), "{action}");
        }
    }

    #[test]
    fn noisy_container_events_are_not() {
        for action in [
            "exec_create: sh",
            "exec_start: sh",
            "exec_die",
            "health_status: healthy",
            "attach",
            "resize",
        ] {
            assert!(!event("container", action).is_relevant(), "{action}");
        }
    }

    #[test]
    fn only_network_membership_changes_are_relevant() {
        assert!(event("network", "connect").is_relevant());
        assert!(event("network", "disconnect").is_relevant());
        assert!(!event("network", "create").is_relevant());
        assert!(!event("network", "destroy").is_relevant());
    }

    #[test]
    fn other_event_types_are_ignored() {
        assert!(!event("image", "pull").is_relevant());
        assert!(!event("volume", "create").is_relevant());
    }

    #[test]
    fn the_filter_constant_is_the_encoding_of_the_documented_json() {
        assert_eq!(
            EVENT_FILTERS,
            "%7B%22type%22%3A%5B%22container%22%2C%22network%22%5D%7D"
        );
    }
}
