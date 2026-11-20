//! Parallel directory scanner behind the `tr` CLI, usable as a library.
//!
//! ```no_run
//! let opts = tr::ScanOptions { depth: 2, dir_sizes: true, ..Default::default() };
//! let tree = tr::scan(std::path::Path::new("."), &opts).unwrap();
//! for entry in &tree.entries {
//!     println!("{} {}", entry.name, entry.total_size);
//! }
//! ```

use rayon::prelude::*;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// Live progress of a running [`scan`]; share it with another thread via
/// [`ScanOptions::progress`] to show what is being scanned.
#[derive(Debug, Default)]
pub struct ScanProgress {
    current_dir: Mutex<PathBuf>,
    dirs_read: AtomicUsize,
}

impl ScanProgress {
    /// The folder most recently opened by any scan thread.
    pub fn current_dir(&self) -> PathBuf {
        self.current_dir
            .lock()
            .map(|p| p.clone())
            .unwrap_or_default()
    }

    /// Number of folders read so far.
    pub fn dirs_read(&self) -> usize {
        self.dirs_read.load(Ordering::Relaxed)
    }

    fn enter(&self, dir: &Path) {
        self.dirs_read.fetch_add(1, Ordering::Relaxed);
        // Skip the update if another thread holds the lock; it's only a display hint.
        if let Ok(mut current) = self.current_dir.try_lock() {
            current.clear();
            current.push(dir);
        }
    }
}

/// What to scan and which entries to keep.
#[derive(Debug, Clone)]
pub struct ScanOptions {
    /// Maximum depth; 1 lists only the root's direct children.
    pub depth: usize,
    /// Include dot-files.
    pub all: bool,
    /// Hide entries with these exact names.
    pub hide_names: Vec<String>,
    /// Keep only folders.
    pub folders_only: bool,
    /// Keep only entries whose name contains one of these substrings (plus the
    /// folders leading to them). Empty means no filter.
    pub filter: Vec<String>,
    /// Sort entries by name instead of keeping filesystem order.
    pub sort: bool,
    /// Compute cumulative folder sizes (scans the full subtree).
    pub dir_sizes: bool,
    /// Count each folder's children (reads folders beyond `depth`).
    pub count_children: bool,
    /// Optional live progress, updated as folders are read.
    pub progress: Option<Arc<ScanProgress>>,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            depth: 1,
            all: false,
            hide_names: Vec::new(),
            folders_only: false,
            filter: Vec::new(),
            sort: true,
            dir_sizes: false,
            count_children: false,
            progress: None,
        }
    }
}

impl ScanOptions {
    fn has_filter(&self) -> bool {
        !self.filter.is_empty()
    }
}

/// Result of [`scan`].
#[derive(Debug, Clone)]
pub struct Tree {
    /// Canonical root path, without the `\\?\` prefix on Windows.
    pub root: PathBuf,
    /// Sum of the top-level entries' sizes (folder sizes need `dir_sizes`).
    pub total_size: u64,
    /// Number of the root's children, counting entries hidden by `hide_names` or `filter`.
    pub child_count: usize,
    pub entries: Vec<TreeEntry>,
}

#[derive(Debug, Clone)]
pub struct TreeEntry {
    pub name: String,
    pub path: PathBuf,
    pub extension: Option<String>,
    pub is_dir: bool,
    pub is_exec: bool,
    pub is_hidden: bool,
    pub is_symlink: bool,
    pub symlink_target: Option<String>,
    pub modified: Option<SystemTime>,
    /// File size in bytes; 0 for folders.
    pub file_size: u64,
    /// File size, or for folders the cumulative size when `dir_sizes` is set.
    pub total_size: u64,
    /// Number of children, when `count_children` (or a deeper scan) read the folder.
    pub child_count: usize,
    pub children: Vec<TreeEntry>,
    should_print: bool,
}

/// Scans `root` and returns the entries the options select, nested by folder.
pub fn scan(root: &Path, opts: &ScanOptions) -> io::Result<Tree> {
    let metadata = fs::metadata(root)?;
    if !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not a directory", root.display()),
        ));
    }

    let root = strip_verbatim(root.canonicalize()?);
    let mut entries = build_tree(&root, 0, opts)?;

    // Totals include everything scanned, before hidden entries are pruned.
    let child_count = entries.len();
    let total_size = entries.iter().map(|e| e.total_size).sum();
    prune(&mut entries);

    Ok(Tree {
        root,
        total_size,
        child_count,
        entries,
    })
}

/// Removes `\\?\` that `canonicalize()` adds on Windows.
fn strip_verbatim(path: PathBuf) -> PathBuf {
    match path.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(stripped) => PathBuf::from(stripped),
        None => path,
    }
}

fn prune(entries: &mut Vec<TreeEntry>) {
    entries.retain(|e| e.should_print);
    for entry in entries {
        prune(&mut entry.children);
    }
}

fn build_tree(path: &Path, depth: usize, opts: &ScanOptions) -> io::Result<Vec<TreeEntry>> {
    if depth >= opts.depth {
        return Ok(Vec::new());
    }

    let mut entries = read_visible_entries(path, opts)?;

    if opts.sort {
        entries.sort_by_cached_key(|(e, _, _)| e.file_name());
    }

    // Build entries in parallel; `collect` keeps the original (sorted) order.
    // Entries we can't access are skipped instead of aborting the whole scan.
    let tree = entries
        .into_par_iter()
        .filter_map(|(entry, should_print, hidden_by_name)| {
            build_entry(entry, should_print, hidden_by_name, depth, opts).ok()
        })
        .collect();

    Ok(tree)
}

fn build_entry(
    entry: fs::DirEntry,
    should_print: bool,
    hidden_by_name: bool,
    depth: usize,
    opts: &ScanOptions,
) -> io::Result<TreeEntry> {
    let path = entry.path();
    let name = entry.file_name().to_string_lossy().to_string();
    let metadata = entry.metadata()?;
    let is_dir = metadata.is_dir();
    let is_symlink = entry.file_type().map(|t| t.is_symlink()).unwrap_or(false);
    let is_hidden = name.starts_with('.');
    let is_exec = is_executable(&path, &metadata);
    let modified = metadata.modified().ok();
    let extension = if is_dir {
        None
    } else {
        path.extension().map(|e| e.to_string_lossy().to_string())
    };
    let symlink_target = if is_symlink {
        Some(
            fs::read_link(&path)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| "unknown".to_string()),
        )
    } else {
        None
    };

    let file_size = if metadata.is_file() {
        metadata.len()
    } else {
        0
    };
    let mut total_size = file_size;
    let mut child_count = 0;
    let mut children = Vec::new();

    if is_dir {
        // Only read into a directory's entries when needed. We avoid scanning
        // hidden parent directories unless the user explicitly requested folder
        // sizes or file counts. This prevents hidden parents from leaking their
        // children into the output and avoids unnecessary IO when not required.
        let should_read_children = if hidden_by_name {
            opts.has_filter() || opts.dir_sizes || opts.count_children
        } else {
            opts.count_children
                || opts.dir_sizes
                || (should_print && depth + 1 < opts.depth)
                || opts.has_filter()
        };

        if should_read_children {
            // Unreadable directories (e.g. access denied) are treated as empty
            // instead of aborting the whole scan.
            let mut child_entries = read_visible_entries(&path, opts).unwrap_or_default();

            child_count = child_entries.len();

            if opts.sort {
                child_entries.sort_by_cached_key(|(e, _, _)| e.file_name());
            }

            // Recurse when we need to compute sizes or when the requested depth
            // allows storing children. If a filter is active we should recurse
            // so matching descendants can be found.
            let should_recurse = opts.dir_sizes
                || (should_print && depth + 1 < opts.depth)
                || opts.has_filter();
            if should_recurse {
                // Only store children when the parent is visible or when a
                // filter is active (we need the path to matching items).
                let should_store_children =
                    depth + 1 < opts.depth && (should_print || opts.has_filter());
                // Scan children in parallel (rayon work-stealing handles the
                // nested recursion). Order is preserved; inaccessible entries
                // are skipped.
                let nodes: Vec<TreeEntry> = child_entries
                    .into_par_iter()
                    .filter_map(|(child, child_should_print, child_hidden_by_name)| {
                        build_entry(
                            child,
                            child_should_print,
                            child_hidden_by_name,
                            depth + 1,
                            opts,
                        )
                        .ok()
                    })
                    .collect();
                total_size += nodes.iter().map(|n| n.total_size).sum::<u64>();
                if should_store_children {
                    children = nodes;
                }
            }
        }
    }

    // A hidden directory hides everything under it. When a filter is active, a
    // directory hidden by name is still shown if a descendant matches.
    let final_should_print = if is_dir {
        if !should_print && !opts.has_filter() {
            false
        } else {
            should_print || children.iter().any(|c| c.should_print)
        }
    } else {
        should_print
    };

    Ok(TreeEntry {
        name,
        path,
        extension,
        is_dir,
        is_exec,
        is_hidden,
        is_symlink,
        symlink_target,
        modified,
        file_size,
        total_size,
        child_count,
        children,
        should_print: final_should_print,
    })
}

/// Returns `(entry, should_print, hidden_by_name)` for each entry in `path`.
fn read_visible_entries(
    path: &Path,
    opts: &ScanOptions,
) -> io::Result<Vec<(fs::DirEntry, bool, bool)>> {
    let mut entries = Vec::new();

    if let Some(progress) = &opts.progress {
        progress.enter(path);
    }

    for entry in fs::read_dir(path)?.filter_map(Result::ok) {
        let name = entry.file_name().to_string_lossy().to_string();
        // Dot-based hidden files are normally hidden; `all` disables that
        // but does NOT disable explicit name-based hiding.
        let hidden_by_dot = !opts.all && name.starts_with('.');

        let hidden_by_name = opts.hide_names.iter().any(|n| n == &name);
        let file_type = entry.file_type().ok();
        let is_symlink = file_type.is_some_and(|t| t.is_symlink());
        let is_dir = file_type.is_some_and(|t| t.is_dir());

        // Base visibility (ignoring hide-by-name for filter handling)
        let base_visible = !hidden_by_dot && !is_symlink && (!opts.folders_only || is_dir);

        // With a filter, only matching entries are printed (but we still scan
        // into hidden-by-name dirs to find matches). Without one, hide_names
        // always suppresses the entry.
        let should_print = if opts.has_filter() {
            base_visible && opts.filter.iter().any(|pat| name.contains(pat.as_str()))
        } else {
            base_visible && !hidden_by_name
        };

        entries.push((entry, should_print, hidden_by_name));
    }

    Ok(entries)
}

fn is_executable(path: &Path, metadata: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = path;

        metadata.is_file() && (metadata.permissions().mode() & 0o111 != 0)
    }

    #[cfg(windows)]
    {
        let _ = metadata;

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());

        matches!(
            ext.as_deref(),
            Some("exe") | Some("com") | Some("bat") | Some("cmd") | Some("ps1")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Creates a fresh temp tree:
    /// root/a.txt (3B), root/b.rs (5B), root/sub/c.rs (7B), root/sub/deep/d.txt (11B), root/.hid
    fn fixture(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("tr-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("sub/deep")).unwrap();
        fs::write(root.join("a.txt"), "aaa").unwrap();
        fs::write(root.join("b.rs"), "bbbbb").unwrap();
        fs::write(root.join("sub/c.rs"), "ccccccc").unwrap();
        fs::write(root.join("sub/deep/d.txt"), "ddddddddddd").unwrap();
        fs::write(root.join(".hid"), "h").unwrap();
        root
    }

    fn names(entries: &[TreeEntry]) -> Vec<&str> {
        entries.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn depth_limits_children() {
        let root = fixture("depth");
        let tree = scan(&root, &ScanOptions::default()).unwrap();
        assert_eq!(names(&tree.entries), ["a.txt", "b.rs", "sub"]);
        assert!(tree.entries[2].children.is_empty());

        let opts = ScanOptions { depth: 2, ..Default::default() };
        let tree = scan(&root, &opts).unwrap();
        assert_eq!(names(&tree.entries[2].children), ["c.rs", "deep"]);
        assert!(tree.entries[2].children[1].children.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn hidden_and_all() {
        let root = fixture("hidden");
        let opts = ScanOptions { all: true, ..Default::default() };
        let tree = scan(&root, &opts).unwrap();
        assert_eq!(names(&tree.entries), [".hid", "a.txt", "b.rs", "sub"]);

        let opts = ScanOptions { hide_names: vec!["sub".into()], ..Default::default() };
        let tree = scan(&root, &opts).unwrap();
        assert_eq!(names(&tree.entries), ["a.txt", "b.rs"]);
        assert_eq!(tree.child_count, 4);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn filter_prunes_to_matches_and_their_parents() {
        let root = fixture("filter");
        let opts = ScanOptions { depth: 5, filter: vec![".rs".into()], ..Default::default() };
        let tree = scan(&root, &opts).unwrap();
        assert_eq!(names(&tree.entries), ["b.rs", "sub"]);
        assert_eq!(names(&tree.entries[1].children), ["c.rs"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn progress_tracks_folders_read() {
        let root = fixture("progress");
        let progress = Arc::new(ScanProgress::default());
        let opts = ScanOptions {
            depth: 5,
            progress: Some(progress.clone()),
            ..Default::default()
        };
        scan(&root, &opts).unwrap();
        assert_eq!(progress.dirs_read(), 3); // root, sub, sub/deep
        assert!(progress.current_dir().starts_with(scan(&root, &ScanOptions::default()).unwrap().root));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn dir_sizes_are_cumulative() {
        let root = fixture("sizes");
        let opts = ScanOptions { dir_sizes: true, all: true, ..Default::default() };
        let tree = scan(&root, &opts).unwrap();
        let sub = tree.entries.iter().find(|e| e.name == "sub").unwrap();
        assert_eq!(sub.total_size, 18);
        assert_eq!(tree.total_size, 1 + 3 + 5 + 18);
        fs::remove_dir_all(root).unwrap();
    }
}
