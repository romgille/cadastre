//! Helpers shared by the integration tests.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::PathBuf;

use serde_json::Value;

/// Prefix of every sensitive value in the fixtures. If it shows up in
/// anything cadastre produces, a secret leaked.
pub const CANARY: &str = "LEAK-";

/// Every `docker inspect` fixture as `(file name, parsed document)`, sorted
/// by file name.
pub fn fixtures() -> Vec<(String, Value)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let document = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            (name, document)
        })
        .collect()
}

/// One fixture by file name, e.g. `web.json`.
pub fn fixture(name: &str) -> Value {
    fixtures()
        .into_iter()
        .find(|(candidate, _)| candidate == name)
        .map(|(_, document)| document)
        .unwrap_or_else(|| panic!("no fixture named {name}"))
}
