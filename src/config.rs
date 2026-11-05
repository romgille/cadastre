//! Runtime configuration, read from environment variables.
//!
//! The binary has no hidden defaults for URLs or paths: a missing required
//! variable is a startup error, never a silent fallback. The container image
//! sets the usual values through `ENV`.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{anyhow, bail, Result};

/// Prefix shared by every environment variable.
pub const ENV_PREFIX: &str = "CADASTRE_";

const DEFAULT_HEARTBEAT_FILE: &str = "/tmp/cadastre.heartbeat";
const DEFAULT_DEBOUNCE_SECONDS: u64 = 30;
const DEFAULT_RECONCILE_SECONDS: u64 = 3600;
const DEFAULT_SHRINK_GUARD_PERCENT: u32 = 50;

/// Everything cadastre needs to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Base URL of the Docker API, normally a read-only socket proxy such as
    /// `http://socket-proxy:2375`. Never ends with a slash.
    pub docker_url: String,
    /// Directory that receives the generated Markdown.
    pub out_dir: PathBuf,
    /// Directory for persistent state (the snapshot of the previous run).
    pub state_dir: PathBuf,
    /// Directory read by node_exporter's textfile collector. Metrics are
    /// disabled when unset.
    pub metrics_dir: Option<PathBuf>,
    /// File touched by the main loop and read by the `healthcheck` command.
    pub heartbeat_file: PathBuf,
    /// Quiet period after the last Docker event before documentation is
    /// regenerated.
    pub debounce: Duration,
    /// Interval of the unconditional regeneration. This bounds how stale the
    /// documentation can get when the event stream silently dies.
    pub reconcile: Duration,
    /// Refuse to publish when the container count falls below this percentage
    /// of the previous snapshot. `0` disables the guard.
    pub shrink_guard_percent: u32,
    /// Emit Obsidian-flavoured Markdown (front matter and wikilinks).
    pub obsidian: bool,
}

impl Config {
    /// Reads the configuration from the process environment.
    pub fn from_env() -> Result<Self> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// Reads the configuration through `lookup`, which receives the full
    /// variable name (prefix included). Tests use this instead of mutating the
    /// process environment.
    pub fn from_lookup<F>(lookup: F) -> Result<Self>
    where
        F: Fn(&str) -> Option<String>,
    {
        let get =
            |name: &str| lookup(&format!("{ENV_PREFIX}{name}")).filter(|v| !v.trim().is_empty());
        let required =
            |name: &str| get(name).ok_or_else(|| anyhow!("{ENV_PREFIX}{name} is required"));

        let shrink_guard_percent = match get("SHRINK_GUARD_PERCENT") {
            None => DEFAULT_SHRINK_GUARD_PERCENT,
            Some(raw) => {
                let value: u32 = raw.trim().parse().map_err(|_| {
                    anyhow!("{ENV_PREFIX}SHRINK_GUARD_PERCENT must be a whole number, got {raw:?}")
                })?;
                if value > 100 {
                    bail!(
                        "{ENV_PREFIX}SHRINK_GUARD_PERCENT must be between 0 and 100, got {value}"
                    );
                }
                value
            }
        };

        Ok(Self {
            docker_url: normalize_url(&required("DOCKER_URL")?)?,
            out_dir: PathBuf::from(required("OUT_DIR")?),
            state_dir: PathBuf::from(required("STATE_DIR")?),
            metrics_dir: get("METRICS_DIR").map(PathBuf::from),
            heartbeat_file: heartbeat_file_from(&lookup),
            debounce: parse_seconds(
                "DEBOUNCE_SECONDS",
                get("DEBOUNCE_SECONDS"),
                DEFAULT_DEBOUNCE_SECONDS,
                0,
            )?,
            reconcile: parse_seconds(
                "RECONCILE_SECONDS",
                get("RECONCILE_SECONDS"),
                DEFAULT_RECONCILE_SECONDS,
                1,
            )?,
            shrink_guard_percent,
            obsidian: match get("OBSIDIAN") {
                None => false,
                Some(raw) => parse_bool("OBSIDIAN", &raw)?,
            },
        })
    }
}

/// Location of the heartbeat file. The `healthcheck` command only needs this
/// one setting, so it is readable without the rest of the configuration.
pub fn heartbeat_file_from_env() -> PathBuf {
    heartbeat_file_from(&|key: &str| std::env::var(key).ok())
}

fn heartbeat_file_from<F>(lookup: &F) -> PathBuf
where
    F: Fn(&str) -> Option<String>,
{
    lookup(&format!("{ENV_PREFIX}HEARTBEAT_FILE"))
        .filter(|v| !v.trim().is_empty())
        .map_or_else(|| PathBuf::from(DEFAULT_HEARTBEAT_FILE), PathBuf::from)
}

fn normalize_url(raw: &str) -> Result<String> {
    let url = raw.trim();
    if url.starts_with("https://") {
        bail!(
            "{ENV_PREFIX}DOCKER_URL uses https, which is not supported: point cadastre at a plain-HTTP \
             socket proxy on a private network"
        );
    }
    if !url.starts_with("http://") {
        bail!("{ENV_PREFIX}DOCKER_URL must start with http://, got {raw:?}");
    }
    Ok(url.trim_end_matches('/').to_string())
}

fn parse_seconds(name: &str, raw: Option<String>, default: u64, min: u64) -> Result<Duration> {
    let seconds = match raw {
        None => default,
        Some(raw) => raw.trim().parse::<u64>().map_err(|_| {
            anyhow!("{ENV_PREFIX}{name} must be a whole number of seconds, got {raw:?}")
        })?,
    };
    if seconds < min {
        bail!("{ENV_PREFIX}{name} must be at least {min}, got {seconds}");
    }
    Ok(Duration::from_secs(seconds))
}

fn parse_bool(name: &str, raw: &str) -> Result<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => bail!("{ENV_PREFIX}{name} must be true or false, got {raw:?}"),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(key, value)| (format!("{ENV_PREFIX}{key}"), (*value).to_string()))
            .collect();
        move |key| map.get(key).cloned()
    }

    const REQUIRED: [(&str, &str); 3] = [
        ("DOCKER_URL", "http://socket-proxy:2375"),
        ("OUT_DIR", "/out"),
        ("STATE_DIR", "/state"),
    ];

    #[test]
    fn minimal_configuration_gets_documented_defaults() {
        let config = Config::from_lookup(env(&REQUIRED)).unwrap();
        assert_eq!(config.docker_url, "http://socket-proxy:2375");
        assert_eq!(config.out_dir, PathBuf::from("/out"));
        assert_eq!(config.state_dir, PathBuf::from("/state"));
        assert_eq!(config.metrics_dir, None);
        assert_eq!(
            config.heartbeat_file,
            PathBuf::from("/tmp/cadastre.heartbeat")
        );
        assert_eq!(config.debounce, Duration::from_secs(30));
        assert_eq!(config.reconcile, Duration::from_secs(3600));
        assert_eq!(config.shrink_guard_percent, 50);
        assert!(!config.obsidian);
    }

    #[test]
    fn every_setting_can_be_overridden() {
        let mut pairs = REQUIRED.to_vec();
        pairs.extend([
            ("METRICS_DIR", "/metrics"),
            ("HEARTBEAT_FILE", "/run/hb"),
            ("DEBOUNCE_SECONDS", "5"),
            ("RECONCILE_SECONDS", "600"),
            ("SHRINK_GUARD_PERCENT", "0"),
            ("OBSIDIAN", "yes"),
        ]);
        let config = Config::from_lookup(env(&pairs)).unwrap();
        assert_eq!(config.metrics_dir, Some(PathBuf::from("/metrics")));
        assert_eq!(config.heartbeat_file, PathBuf::from("/run/hb"));
        assert_eq!(config.debounce, Duration::from_secs(5));
        assert_eq!(config.reconcile, Duration::from_secs(600));
        assert_eq!(config.shrink_guard_percent, 0);
        assert!(config.obsidian);
    }

    #[test]
    fn a_missing_required_variable_is_named_in_the_error() {
        for missing in ["DOCKER_URL", "OUT_DIR", "STATE_DIR"] {
            let pairs: Vec<_> = REQUIRED
                .iter()
                .filter(|(k, _)| *k != missing)
                .copied()
                .collect();
            let error = Config::from_lookup(env(&pairs)).unwrap_err().to_string();
            assert!(error.contains(&format!("{ENV_PREFIX}{missing}")), "{error}");
        }
    }

    #[test]
    fn empty_values_count_as_unset() {
        let mut pairs = REQUIRED.to_vec();
        pairs.push(("METRICS_DIR", "  "));
        let config = Config::from_lookup(env(&pairs)).unwrap();
        assert_eq!(config.metrics_dir, None);
    }

    #[test]
    fn trailing_slashes_are_removed_from_the_docker_url() {
        let pairs = [
            ("DOCKER_URL", "http://proxy:2375//"),
            ("OUT_DIR", "/o"),
            ("STATE_DIR", "/s"),
        ];
        let config = Config::from_lookup(env(&pairs)).unwrap();
        assert_eq!(config.docker_url, "http://proxy:2375");
    }

    #[test]
    fn https_is_rejected_with_an_explanation() {
        let pairs = [
            ("DOCKER_URL", "https://proxy:2376"),
            ("OUT_DIR", "/o"),
            ("STATE_DIR", "/s"),
        ];
        let error = Config::from_lookup(env(&pairs)).unwrap_err().to_string();
        assert!(error.contains("not supported"), "{error}");
    }

    #[test]
    fn unix_sockets_are_rejected() {
        let pairs = [
            ("DOCKER_URL", "unix:///var/run/docker.sock"),
            ("OUT_DIR", "/o"),
            ("STATE_DIR", "/s"),
        ];
        assert!(Config::from_lookup(env(&pairs)).is_err());
    }

    #[test]
    fn invalid_numbers_are_rejected_and_name_the_variable() {
        for (name, value) in [
            ("DEBOUNCE_SECONDS", "soon"),
            ("RECONCILE_SECONDS", "-1"),
            ("RECONCILE_SECONDS", "0"),
            ("SHRINK_GUARD_PERCENT", "101"),
            ("SHRINK_GUARD_PERCENT", "half"),
        ] {
            let mut pairs = REQUIRED.to_vec();
            pairs.push((name, value));
            let error = Config::from_lookup(env(&pairs)).unwrap_err().to_string();
            assert!(error.contains(name), "{name}={value}: {error}");
        }
    }

    #[test]
    fn booleans_accept_common_spellings_and_reject_typos() {
        for value in ["1", "true", "TRUE", "On", "yes"] {
            assert!(parse_bool("OBSIDIAN", value).unwrap(), "{value}");
        }
        for value in ["0", "false", "Off", "no"] {
            assert!(!parse_bool("OBSIDIAN", value).unwrap(), "{value}");
        }
        assert!(parse_bool("OBSIDIAN", "ture").is_err());
    }

    #[test]
    fn heartbeat_file_is_readable_without_the_rest_of_the_configuration() {
        let lookup = env(&[("HEARTBEAT_FILE", "/run/hb")]);
        assert_eq!(heartbeat_file_from(&lookup), PathBuf::from("/run/hb"));
        assert_eq!(
            heartbeat_file_from(&env(&[])),
            PathBuf::from("/tmp/cadastre.heartbeat")
        );
    }
}
