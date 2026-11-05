//! Writing the rendered pages into the output folder.
//!
//! The folder is shared with a sync tool and read by humans, so three rules
//! apply: files are replaced atomically, a file whose content did not change
//! is not touched (no spurious sync traffic, no moving modification times),
//! and cadastre only ever deletes files it can prove it generated.

use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::fsutil::atomic_write;
use crate::render::{Page, MARKER};

/// Name of the scratch folder inside the output folder. It has to live on
/// the same file system as the pages for the final rename to be atomic.
/// Add it to the sync tool's ignore list.
pub const TMP_DIR: &str = ".tmp";

/// Folder, relative to the output root, that holds one page per stack.
pub const STACKS_DIR: &str = "stacks";

/// What a synchronization did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Report {
    pub written: Vec<String>,
    pub unchanged: usize,
    pub removed: Vec<String>,
}

impl Report {
    pub fn has_changes(&self) -> bool {
        !self.written.is_empty() || !self.removed.is_empty()
    }
}

/// The output folder.
#[derive(Debug, Clone)]
pub struct OutDir {
    root: PathBuf,
}

impl OutDir {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Makes the folder match `pages`: writes new and changed pages, removes
    /// generated pages that are no longer rendered.
    ///
    /// Everything is validated before anything is touched, so a bad page
    /// path cannot leave the folder half updated.
    pub fn sync(&self, pages: &[Page]) -> Result<Report> {
        for page in pages {
            checked_relative(&page.path)?;
        }
        if !self.root.is_dir() {
            bail!(
                "output folder {} does not exist; refusing to create it (is the volume mounted?)",
                self.root.display()
            );
        }
        self.clean_stale_temporaries();

        let mut report = Report::default();
        for page in pages {
            if self.write_if_changed(&page.path, page.content.as_bytes())? {
                report.written.push(page.path.clone());
            } else {
                report.unchanged += 1;
            }
        }
        let keep: HashSet<&str> = pages.iter().map(|p| p.path.as_str()).collect();
        report.removed = self.prune_generated(&keep)?;
        Ok(report)
    }

    /// Writes `contents` to `relative` unless the file already holds exactly
    /// that. Returns whether the file was written.
    pub fn write_if_changed(&self, relative: &str, contents: &[u8]) -> Result<bool> {
        let dest = self.root.join(checked_relative(relative)?);
        if let Ok(existing) = fs::read(&dest) {
            if existing == contents {
                return Ok(false);
            }
        }
        atomic_write(&self.root.join(TMP_DIR), &dest, contents)?;
        Ok(true)
    }

    /// Removes pages under `stacks/` that carry the generated marker and are
    /// not in `keep`. Files without the marker are never touched: they were
    /// written by a person.
    fn prune_generated(&self, keep: &HashSet<&str>) -> Result<Vec<String>> {
        let dir = self.root.join(STACKS_DIR);
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(error).with_context(|| format!("cannot list {}", dir.display()))
            }
        };

        let mut removed = Vec::new();
        for entry in entries {
            let entry = entry.with_context(|| format!("cannot list {}", dir.display()))?;
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let relative = format!("{STACKS_DIR}/{name}");
            let is_page = name.ends_with(".md") && entry.file_type()?.is_file();
            if !is_page || keep.contains(relative.as_str()) {
                continue;
            }
            let text = fs::read_to_string(&path).unwrap_or_default();
            if text.contains(MARKER) {
                fs::remove_file(&path)
                    .with_context(|| format!("cannot remove {}", path.display()))?;
                removed.push(relative);
            }
        }
        removed.sort();
        Ok(removed)
    }

    /// Deletes leftovers of an interrupted write. Failure to do so is not
    /// worth failing a run for.
    fn clean_stale_temporaries(&self) {
        let Ok(entries) = fs::read_dir(self.root.join(TMP_DIR)) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "tmp") {
                let _ = fs::remove_file(path);
            }
        }
    }
}

/// Accepts only plain relative paths made of normal components. Rejects
/// absolute paths and `..`, so a hostile or buggy page path can never escape
/// the output folder.
fn checked_relative(path: &str) -> Result<PathBuf> {
    let candidate = Path::new(path);
    let mut count = 0;
    for component in candidate.components() {
        match component {
            Component::Normal(_) => count += 1,
            _ => bail!("refusing to write outside the output folder: {path:?}"),
        }
    }
    if count == 0 {
        bail!("empty page path");
    }
    Ok(candidate.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(path: &str, content: &str) -> Page {
        Page {
            path: path.into(),
            content: content.into(),
        }
    }

    fn generated(text: &str) -> String {
        format!("{MARKER}\n{text}\n")
    }

    fn setup() -> (tempfile::TempDir, OutDir) {
        let dir = tempfile::tempdir().unwrap();
        let out = OutDir::new(dir.path());
        (dir, out)
    }

    #[test]
    fn writes_pages_and_reports_them() {
        let (dir, out) = setup();
        let report = out
            .sync(&[page("index.md", "a"), page("stacks/x.md", "b")])
            .unwrap();
        assert_eq!(report.written, ["index.md", "stacks/x.md"]);
        assert_eq!(
            fs::read_to_string(dir.path().join("stacks/x.md")).unwrap(),
            "b"
        );
    }

    #[test]
    fn an_unchanged_page_is_not_rewritten() {
        let (dir, out) = setup();
        let pages = [page("index.md", "a")];
        out.sync(&pages).unwrap();
        let before = fs::metadata(dir.path().join("index.md"))
            .unwrap()
            .modified()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        let report = out.sync(&pages).unwrap();
        assert!(!report.has_changes());
        assert_eq!(report.unchanged, 1);
        let after = fs::metadata(dir.path().join("index.md"))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn a_changed_page_is_rewritten() {
        let (dir, out) = setup();
        out.sync(&[page("index.md", "a")]).unwrap();
        let report = out.sync(&[page("index.md", "b")]).unwrap();
        assert_eq!(report.written, ["index.md"]);
        assert_eq!(
            fs::read_to_string(dir.path().join("index.md")).unwrap(),
            "b"
        );
    }

    #[test]
    fn stale_generated_pages_are_removed() {
        let (dir, out) = setup();
        out.sync(&[
            page("stacks/old.md", &generated("old")),
            page("stacks/new.md", &generated("new")),
        ])
        .unwrap();
        let report = out
            .sync(&[page("stacks/new.md", &generated("new"))])
            .unwrap();
        assert_eq!(report.removed, ["stacks/old.md"]);
        assert!(!dir.path().join("stacks/old.md").exists());
        assert!(dir.path().join("stacks/new.md").exists());
    }

    #[test]
    fn pages_written_by_a_person_are_never_removed() {
        let (dir, out) = setup();
        fs::create_dir(dir.path().join("stacks")).unwrap();
        fs::write(dir.path().join("stacks/mine.md"), "my own notes").unwrap();
        fs::write(dir.path().join("stacks/data.txt"), "not a page").unwrap();
        let report = out.sync(&[page("index.md", "a")]).unwrap();
        assert!(report.removed.is_empty());
        assert!(dir.path().join("stacks/mine.md").exists());
        assert!(dir.path().join("stacks/data.txt").exists());
    }

    #[test]
    fn files_outside_the_stacks_folder_are_never_removed() {
        let (dir, out) = setup();
        fs::write(dir.path().join("notes.md"), generated("looks generated")).unwrap();
        out.sync(&[page("index.md", "a")]).unwrap();
        assert!(dir.path().join("notes.md").exists());
    }

    #[test]
    fn paths_that_escape_the_folder_are_refused_before_anything_is_written() {
        let (dir, out) = setup();
        for bad in ["../evil.md", "/etc/evil.md", "stacks/../../evil.md", ""] {
            let error = out
                .sync(&[page("index.md", "a"), page(bad, "x")])
                .unwrap_err();
            assert!(format!("{error:#}").contains("refusing") || bad.is_empty());
            assert!(
                !dir.path().join("index.md").exists(),
                "{bad:?} left a partial update"
            );
        }
    }

    #[test]
    fn a_missing_output_folder_is_an_error_not_something_to_create() {
        // A missing mount point must fail loudly rather than quietly fill the
        // container's own file system.
        let dir = tempfile::tempdir().unwrap();
        let out = OutDir::new(dir.path().join("not-mounted"));
        let error = out.sync(&[page("index.md", "a")]).unwrap_err();
        assert!(format!("{error:#}").contains("does not exist"), "{error:#}");
        assert!(!dir.path().join("not-mounted").exists());
    }

    #[test]
    fn leftovers_of_an_interrupted_write_are_cleaned_up() {
        let (dir, out) = setup();
        fs::create_dir(dir.path().join(TMP_DIR)).unwrap();
        fs::write(dir.path().join(TMP_DIR).join(".1-0.tmp"), "partial").unwrap();
        out.sync(&[page("index.md", "a")]).unwrap();
        assert_eq!(fs::read_dir(dir.path().join(TMP_DIR)).unwrap().count(), 0);
    }
}
