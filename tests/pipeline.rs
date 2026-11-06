//! A full run against a temporary output and state folder.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::fs;
use std::path::Path;

use cadastre::config::Config;
use cadastre::model::Container;
use cadastre::pipeline::publish;
use cadastre::sanitize::container_from_inspect;
use support::{fixtures, CANARY};
use tempfile::TempDir;

struct Env {
    _dir: TempDir,
    config: Config,
}

impl Env {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        for sub in ["out", "state"] {
            fs::create_dir(dir.path().join(sub)).unwrap();
        }
        let lookup = |key: &str| match key {
            "CADASTRE_DOCKER_URL" => Some("http://unused:2375".to_string()),
            "CADASTRE_OUT_DIR" => Some(dir.path().join("out").display().to_string()),
            "CADASTRE_STATE_DIR" => Some(dir.path().join("state").display().to_string()),
            _ => None,
        };
        let config = Config::from_lookup(lookup).unwrap();
        Self { _dir: dir, config }
    }

    fn out(&self, relative: &str) -> String {
        fs::read_to_string(self.config.out_dir.join(relative)).unwrap()
    }

    fn exists(&self, relative: &str) -> bool {
        Path::new(&self.config.out_dir.join(relative)).exists()
    }
}

fn all() -> Vec<Container> {
    fixtures()
        .iter()
        .map(|(_, document)| container_from_inspect(document).unwrap())
        .collect()
}

#[test]
fn the_first_run_writes_pages_a_changelog_and_a_snapshot() {
    let env = Env::new();
    let outcome = publish(&env.config, &all(), 1_700_000_000, false).unwrap();
    assert_eq!(outcome.containers, 4);
    assert!(outcome.changelog_entry);
    assert!(env.exists("index.md"));
    assert!(env.exists("docker/shop.md"));
    assert!(env.config.state_dir.join("snapshot.json").exists());
    let changelog = env.out("changelog.md");
    assert!(
        changelog.contains("Initial snapshot: 4 containers in 2 stacks."),
        "{changelog}"
    );
    assert!(changelog.contains("2023-11-14T22:13:20Z"), "{changelog}");
}

#[test]
fn nothing_written_anywhere_contains_a_canary() {
    let env = Env::new();
    publish(&env.config, &all(), 1, false).unwrap();
    for root in [&env.config.out_dir, &env.config.state_dir] {
        for entry in walk(root) {
            let text = fs::read_to_string(&entry).unwrap();
            assert!(!text.contains(CANARY), "{} leaked", entry.display());
        }
    }
}

#[test]
fn an_unchanged_second_run_touches_nothing() {
    let env = Env::new();
    publish(&env.config, &all(), 1_700_000_000, false).unwrap();
    let changelog = env.out("changelog.md");
    let outcome = publish(&env.config, &all(), 1_700_003_600, false).unwrap();
    assert!(!outcome.report.has_changes());
    assert!(!outcome.changelog_entry);
    assert_eq!(env.out("changelog.md"), changelog);
}

#[test]
fn a_change_updates_the_page_and_prepends_a_changelog_entry() {
    let env = Env::new();
    publish(&env.config, &all(), 1_700_000_000, false).unwrap();
    let mut changed = all();
    changed.retain(|c| c.name != "uptime-probe");
    changed[0].image_id = "sha256:ffffffffffffffffffff".into();
    let outcome = publish(&env.config, &changed, 1_700_003_600, false).unwrap();
    assert!(outcome.changelog_entry);
    let changelog = env.out("changelog.md");
    let newest = changelog.find("2023-11-14T23:13:20Z").unwrap();
    let oldest = changelog.find("2023-11-14T22:13:20Z").unwrap();
    assert!(newest < oldest, "{changelog}");
    assert!(
        changelog.contains("- Removed `uptime-probe`"),
        "{changelog}"
    );
    assert!(!env.out("index.md").contains("uptime-probe"));
}

#[test]
fn a_stack_that_disappears_loses_its_page() {
    let env = Env::new();
    publish(&env.config, &all(), 1, false).unwrap();
    let mut standalone_only = all();
    standalone_only.retain(|c| c.project.is_none());
    publish(&env.config, &standalone_only, 2, false).unwrap();
    assert!(!env.exists("docker/shop.md"));
    assert!(env.exists("docker/_standalone.md") || env.exists("docker/standalone.md"));
}

#[test]
fn a_changelog_written_by_a_person_stops_the_run_before_anything_is_written() {
    let env = Env::new();
    fs::write(env.config.out_dir.join("changelog.md"), "# mine\n").unwrap();
    let error = publish(&env.config, &all(), 1, false).unwrap_err();
    assert!(format!("{error:#}").contains("not generated"), "{error:#}");
    assert!(
        !env.exists("index.md"),
        "pages were written before the refusal"
    );
    assert!(!env.config.state_dir.join("snapshot.json").exists());
}

#[test]
fn a_corrupt_snapshot_stops_the_run_before_anything_is_written() {
    let env = Env::new();
    fs::write(env.config.state_dir.join("snapshot.json"), "garbage").unwrap();
    assert!(publish(&env.config, &all(), 1, false).is_err());
    assert!(!env.exists("index.md"));
}

#[test]
fn a_missing_output_folder_fails_and_leaves_no_snapshot() {
    let env = Env::new();
    fs::remove_dir_all(&env.config.out_dir).unwrap();
    assert!(publish(&env.config, &all(), 1, false).is_err());
    assert!(
        !env.config.state_dir.join("snapshot.json").exists(),
        "a snapshot for output that was never written would hide the change"
    );
}

fn walk(root: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(root).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(walk(&path));
        } else {
            files.push(path);
        }
    }
    files
}

fn drop_to(count: usize) -> Vec<Container> {
    all().into_iter().take(count).collect()
}

#[test]
fn an_empty_answer_after_a_populated_run_is_refused_and_changes_nothing() {
    let env = Env::new();
    publish(&env.config, &all(), 1, false).unwrap();
    let index = env.out("index.md");
    let changelog = env.out("changelog.md");
    let error = publish(&env.config, &[], 2, false).unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("refusing to publish"), "{message}");
    assert!(message.contains("--accept-shrink"), "{message}");
    assert_eq!(env.out("index.md"), index);
    assert_eq!(env.out("changelog.md"), changelog);
}

#[test]
fn a_drop_below_the_threshold_is_refused() {
    let env = Env::new();
    publish(&env.config, &all(), 1, false).unwrap();
    assert!(publish(&env.config, &drop_to(1), 2, false).is_err());
}

#[test]
fn a_drop_to_exactly_the_threshold_is_allowed() {
    let env = Env::new();
    publish(&env.config, &all(), 1, false).unwrap();
    publish(&env.config, &drop_to(2), 2, false).unwrap();
}

#[test]
fn accepting_the_shrink_publishes_it() {
    let env = Env::new();
    publish(&env.config, &all(), 1, false).unwrap();
    publish(&env.config, &[], 2, true).unwrap();
    assert!(env.out("changelog.md").contains("- Removed"));
}

#[test]
fn the_guard_can_be_disabled() {
    let mut env = Env::new();
    publish(&env.config, &all(), 1, false).unwrap();
    env.config.shrink_guard_percent = 0;
    publish(&env.config, &[], 2, false).unwrap();
}

#[test]
fn the_first_run_is_never_blocked_by_the_guard() {
    let env = Env::new();
    publish(&env.config, &[], 1, false).unwrap();
}

#[test]
fn a_run_against_the_mock_docker_api_documents_what_it_serves() {
    use cadastre::docker::Docker;
    use cadastre::pipeline::run_once;
    use support::mock_docker::MockDocker;

    let env = Env::new();
    let documents: Vec<_> = fixtures().into_iter().map(|(_, d)| d).collect();
    let mock = MockDocker::with_containers(&documents);
    let outcome = run_once(&env.config, &Docker::new(&mock.url), 1, false).unwrap();
    assert_eq!(outcome.containers, 4);
    assert!(env.out("docker/shop.md").contains("shop-web-1"));
}

#[test]
fn a_proxy_that_suddenly_answers_with_nothing_does_not_wipe_the_documentation() {
    use cadastre::docker::Docker;
    use cadastre::pipeline::run_once;
    use support::mock_docker::MockDocker;

    let env = Env::new();
    let documents: Vec<_> = fixtures().into_iter().map(|(_, d)| d).collect();
    let mock = MockDocker::with_containers(&documents);
    let docker = Docker::new(&mock.url);
    run_once(&env.config, &docker, 1, false).unwrap();
    for document in &documents {
        mock.remove(document["Id"].as_str().unwrap());
    }
    assert!(run_once(&env.config, &docker, 2, false).is_err());
    assert!(env.exists("docker/shop.md"));
}
