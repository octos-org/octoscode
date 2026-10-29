//! `@` composer file picker (#363, v1: path insert only).
//!
//! Scans the ACTIVE workspace root on the machine octoscode runs on and feeds
//! the searchable `file-picker` menu (see `menu::providers::file_picker_menu`).
//! Selecting a row inserts the file's RELATIVE path at the composer cursor —
//! no file contents are read or embedded, and no protocol traffic is involved.
//!
//! The walk is breadth-first (shallow paths surface first, which is what a
//! picker wants), sorted per directory for deterministic ordering, and bounded
//! by [`FILE_PICKER_MAX_FILES`] / [`FILE_PICKER_MAX_DEPTH`] plus directory and
//! entry budgets. The scan remains synchronous, so individual filesystem I/O
//! may still be slow, but a wide directory cannot cause unbounded enumeration,
//! sorting, or queued work. When the entry budget truncates a directory, the
//! scan stops after processing its retained entries; its subdirectories are
//! not visited. `.git` and the ubiquitous dependency/build trees (`target`,
//! `node_modules`) are skipped — with a ~2000-entry cap they would otherwise
//! flood out the source files the picker exists to find. Symlinks are skipped
//! entirely (no cycle chasing, no walking out of the workspace).

use std::collections::VecDeque;
use std::path::Path;

/// Maximum number of files the picker lists (the "~2000 entries" cap).
pub const FILE_PICKER_MAX_FILES: usize = 2000;
/// Maximum directory depth below the workspace root that is walked.
pub const FILE_PICKER_MAX_DEPTH: usize = 16;
/// Budget of directories visited per scan, so a wide tree of empty/filtered
/// directories still terminates promptly even before the file cap is hit.
const FILE_PICKER_MAX_DIRS: usize = 2000;
/// Includes filtered, unreadable and symlink entries: bounding only accepted
/// files would still enumerate an arbitrarily large excluded directory.
const FILE_PICKER_MAX_ENTRIES: usize = 16_000;

/// Directory names that are never descended into. `.git` per the picker spec;
/// `target` / `node_modules` because either one routinely holds tens of
/// thousands of generated files that would exhaust the entry cap before the
/// project's real sources are reached.
const SKIPPED_DIRS: [&str; 3] = [".git", "target", "node_modules"];

/// Snapshot of one `@` picker opening: the root that was scanned (display
/// form) and the relative paths found under it. Rebuilt on every open — the
/// picker never serves a stale tree from an earlier `@`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePickerState {
    /// Display form of the scanned workspace root.
    pub root: String,
    /// Relative paths (always `/`-separated) in breadth-first, per-directory
    /// alphabetical order.
    pub files: Vec<String>,
    /// Whether any cap (files / depth / directory / entry budget) truncated it.
    pub truncated: bool,
}

impl FilePickerState {
    /// Scan `root` and build the picker snapshot.
    pub fn scan(root: &Path) -> Self {
        let (files, truncated) = scan_workspace_files(root);
        Self {
            root: root.display().to_string(),
            files,
            truncated,
        }
    }
}

/// Walk `root` breadth-first and return `(relative_paths, truncated)`.
///
/// Paths are `/`-joined relative to `root` regardless of platform so the
/// inserted prompt text is stable. Entries within each directory are sorted by
/// name; directories in [`SKIPPED_DIRS`] and all symlinks are skipped.
pub fn scan_workspace_files(root: &Path) -> (Vec<String>, bool) {
    scan_workspace_files_with_budget(root, FILE_PICKER_MAX_ENTRIES, FILE_PICKER_MAX_DIRS)
}

/// Collect at most the remaining entry budget, plus one lookahead to tell an
/// exactly-full directory from a truncated one. Counting before filtering also
/// bounds directory-entry errors and metadata reads.
fn bounded_entries<T>(entries: impl Iterator<Item = T>, remaining: &mut usize) -> (Vec<T>, bool) {
    let mut entries: Vec<T> = entries.take(remaining.saturating_add(1)).collect();
    let truncated = entries.len() > *remaining;
    entries.truncate(*remaining);
    *remaining -= entries.len();
    (entries, truncated)
}

fn scan_workspace_files_with_budget(
    root: &Path,
    entry_budget: usize,
    directory_budget: usize,
) -> (Vec<String>, bool) {
    let mut files = Vec::new();
    let mut truncated = false;
    let mut dirs_visited = 0usize;
    let mut entries_remaining = entry_budget;
    let mut dirs_admitted = 1usize;
    // Queue of (absolute dir, relative prefix, depth). Depth 0 = root itself.
    let mut queue = VecDeque::from([(root.to_path_buf(), String::new(), 0usize)]);

    'walk: while let Some((dir, rel_prefix, depth)) = queue.pop_front() {
        if dirs_visited >= directory_budget {
            truncated = true;
            break;
        }
        dirs_visited += 1;

        let Ok(entries) = std::fs::read_dir(&dir) else {
            // Unreadable directory (permissions, raced deletion): skip quietly.
            continue;
        };
        let (entries, entries_truncated) = bounded_entries(entries, &mut entries_remaining);
        truncated |= entries_truncated;
        let mut children: Vec<(String, bool)> = entries
            .into_iter()
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let file_type = entry.file_type().ok()?;
                // `DirEntry::file_type` does not follow symlinks, so a symlink
                // reports `is_symlink` and is dropped here — never followed.
                if file_type.is_symlink() {
                    return None;
                }
                let name = entry.file_name().into_string().ok()?;
                Some((name, file_type.is_dir()))
            })
            .collect();
        children.sort();

        for (name, is_dir) in children {
            let rel = if rel_prefix.is_empty() {
                name.clone()
            } else {
                format!("{rel_prefix}/{name}")
            };
            if is_dir {
                if SKIPPED_DIRS.contains(&name.as_str()) {
                    continue;
                }
                if depth + 1 > FILE_PICKER_MAX_DEPTH {
                    truncated = true;
                    continue;
                }
                // Reserve a visit when admitting work, rather than allowing a
                // huge root directory to fill the queue before visits begin.
                if dirs_admitted >= directory_budget {
                    truncated = true;
                    continue;
                }
                dirs_admitted += 1;
                queue.push_back((dir.join(&name), rel, depth + 1));
            } else {
                if files.len() >= FILE_PICKER_MAX_FILES {
                    truncated = true;
                    break 'walk;
                }
                files.push(rel);
            }
        }
        if entries_truncated {
            break;
        }
    }

    (files, truncated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    /// Minimal self-cleaning temp dir (repo convention — no tempfile dep).
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let mut dir = std::env::temp_dir();
            dir.push(format!(
                "octoscode-file-picker-{tag}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("clock after epoch")
                    .as_nanos()
            ));
            fs::create_dir_all(&dir).expect("create temp dir");
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn touch(root: &Path, rel: &str) {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent dirs");
        }
        fs::write(&path, b"x").expect("write file");
    }

    #[test]
    fn scan_lists_relative_paths_breadth_first_and_sorted() {
        let dir = TempDir::new("order");
        touch(dir.path(), "b.rs");
        touch(dir.path(), "a.rs");
        touch(dir.path(), "sub/inner.rs");

        let (files, truncated) = scan_workspace_files(dir.path());

        assert_eq!(files, vec!["a.rs", "b.rs", "sub/inner.rs"]);
        assert!(!truncated);
    }

    #[test]
    fn scan_skips_git_target_and_node_modules() {
        let dir = TempDir::new("skips");
        touch(dir.path(), "keep.rs");
        touch(dir.path(), ".git/config");
        touch(dir.path(), "target/debug/artifact");
        touch(dir.path(), "node_modules/pkg/index.js");
        // A nested `.git` (submodule/worktree layout) is skipped too.
        touch(dir.path(), "sub/.git/HEAD");
        touch(dir.path(), "sub/kept.txt");

        let (files, _) = scan_workspace_files(dir.path());

        assert_eq!(files, vec!["keep.rs", "sub/kept.txt"]);
    }

    #[test]
    fn scan_includes_dotfiles_but_not_symlinks() {
        let dir = TempDir::new("dotfiles");
        touch(dir.path(), ".gitignore");
        #[cfg(unix)]
        {
            touch(dir.path(), "real.txt");
            std::os::unix::fs::symlink(dir.path().join("real.txt"), dir.path().join("link.txt"))
                .expect("create symlink");
        }

        let (files, _) = scan_workspace_files(dir.path());

        assert!(files.contains(&".gitignore".to_string()));
        #[cfg(unix)]
        {
            assert!(files.contains(&"real.txt".to_string()));
            assert!(!files.contains(&"link.txt".to_string()));
        }
    }

    #[test]
    fn scan_truncates_at_file_cap() {
        let dir = TempDir::new("cap");
        for i in 0..(FILE_PICKER_MAX_FILES + 5) {
            touch(dir.path(), &format!("f{i:05}.txt"));
        }

        let (files, truncated) = scan_workspace_files(dir.path());

        assert_eq!(files.len(), FILE_PICKER_MAX_FILES);
        assert!(truncated);
    }

    #[test]
    fn directory_entry_budget_stops_before_enumerating_the_remainder() {
        let examined = std::cell::Cell::new(0);
        let entries = (0..).inspect(|_| examined.set(examined.get() + 1));
        let mut remaining = 7;
        let (entries, truncated) = bounded_entries(entries, &mut remaining);
        assert_eq!(entries, (0..7).collect::<Vec<_>>());
        assert!(truncated);
        assert_eq!(remaining, 0);
        assert_eq!(examined.get(), 8, "only one lookahead beyond the budget");
    }

    #[test]
    fn directory_entry_budget_is_shared_across_directories() {
        let dir = TempDir::new("entry-budget");
        for child in ["a", "b"] {
            for index in 0..5 {
                touch(dir.path(), &format!("{child}/f{index}.txt"));
            }
        }
        let (files, truncated) = scan_workspace_files_with_budget(dir.path(), 7, 20);
        assert_eq!(files.len(), 5, "two root entries leave five file entries");
        assert!(files.iter().all(|file| file.starts_with("a/")));
        assert!(truncated);
    }

    #[test]
    fn wide_directory_tree_bounds_admitted_work() {
        let dir = TempDir::new("directory-budget");
        for index in 0..12 {
            touch(dir.path(), &format!("d{index:02}/file.txt"));
        }
        let (files, truncated) = scan_workspace_files_with_budget(dir.path(), 100, 3);
        assert_eq!(files, vec!["d00/file.txt", "d01/file.txt"]);
        assert!(truncated);
    }

    #[test]
    fn entry_budget_exactly_filled_does_not_claim_truncation() {
        let dir = TempDir::new("exact-budget");
        touch(dir.path(), "a.txt");
        touch(dir.path(), "b.txt");
        let (files, truncated) = scan_workspace_files_with_budget(dir.path(), 2, 3);
        assert_eq!(files, vec!["a.txt", "b.txt"]);
        assert!(!truncated);
    }

    #[test]
    fn scan_missing_root_is_empty_not_error() {
        let dir = TempDir::new("gone");
        let missing = dir.path().join("does-not-exist");

        let (files, truncated) = scan_workspace_files(&missing);

        assert!(files.is_empty());
        assert!(!truncated);
    }

    #[test]
    fn state_scan_records_display_root() {
        let dir = TempDir::new("state");
        touch(dir.path(), "x.txt");

        let state = FilePickerState::scan(dir.path());

        assert_eq!(state.root, dir.path().display().to_string());
        assert_eq!(state.files, vec!["x.txt"]);
        assert!(!state.truncated);
    }
}
