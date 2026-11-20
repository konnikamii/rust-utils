//! Parallel directory scanner behind the `tr` CLI, usable as a library.
//!
//! ```no_run
//! use std::path::Path;
//!
//! // Explorer-style listing of one folder.
//! let opts = tr::ScanOptions { dirs_first: true, ..Default::default() };
//! let tree = tr::scan(Path::new("C:/some/dir"), &opts).unwrap();
//!
//! // Search: every .jpg/.png over 1 MB, at most 500 results.
//! let opts = tr::ScanOptions {
//!     depth: usize::MAX,
//!     extensions: vec!["jpg".into(), "png".into()],
//!     min_size: Some(1024 * 1024),
//!     limit: Some(500),
//!     ..Default::default()
//! };
//! let tree = tr::scan(Path::new("C:/some/dir"), &opts).unwrap();
//! for hit in tree.matches() {
//!     println!("{}", hit.path.display());
//! }
//! ```
//!
//! # How entries are selected
//!
//! * **Hidden** entries are never shown or searched into: dot-files (unless
//!   [`ScanOptions::all`]), names in [`ScanOptions::exclude`], and symlinks /
//!   junctions (never followed). They still count toward folder sizes and
//!   [`TreeEntry::child_count`].
//! * Without search criteria every visible entry within `depth` is listed.
//! * With search criteria (name, extension, size, date, or `kind: Files`) only
//!   entries that **match** are listed, plus the folders leading to them
//!   ([`TreeEntry::matched`] tells the two apart).

use rayon::prelude::*;
use std::borrow::Cow;
use std::fs;
use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// Which entry types can match.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EntryKind {
    #[default]
    Any,
    /// Only files match; folders are shown only as parents of matches.
    Files,
    /// Only folders are listed.
    Folders,
}

/// Shared handle to a running [`scan`]: read progress or cancel it from
/// another thread. Pass it in via [`ScanOptions::handle`].
#[derive(Debug, Default)]
pub struct ScanHandle {
    current_dir: Mutex<PathBuf>,
    dirs_read: AtomicUsize,
    cancelled: AtomicBool,
}

impl ScanHandle {
    pub fn new() -> Arc<Self> {
        Arc::default()
    }

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

    /// Stops the scan as soon as possible; [`scan`] then returns an
    /// [`io::ErrorKind::Interrupted`] error.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
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
    /// Maximum depth; 1 lists only the root's direct children. Use
    /// `usize::MAX` to search the whole subtree.
    pub depth: usize,
    /// Include dot-files.
    pub all: bool,
    /// Never show or search into entries with these exact names
    /// (e.g. `node_modules`, `$Recycle.Bin`).
    pub exclude: Vec<String>,
    /// Sort entries by name (case-insensitive) instead of filesystem order.
    pub sort: bool,
    /// List folders before files (only when `sort` is on).
    pub dirs_first: bool,
    /// Make name, extension and exclude matching case-sensitive.
    pub case_sensitive: bool,

    // Search criteria: an entry must pass all that are set.
    /// Name contains any of these substrings.
    pub name_contains: Vec<String>,
    /// File extension is one of these (with or without the leading dot).
    pub extensions: Vec<String>,
    pub kind: EntryKind,
    /// File size in bytes, inclusive (files only).
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
    /// Last-modified time, inclusive (files only).
    pub modified_after: Option<SystemTime>,
    pub modified_before: Option<SystemTime>,

    /// Stop after this many matching entries and set [`Tree::truncated`].
    /// Which matches are found first depends on thread timing, but the
    /// returned ones stay in sorted order. Sizes and counts are partial then.
    pub limit: Option<usize>,
    /// Compute cumulative folder sizes (reads every subfolder).
    pub dir_sizes: bool,
    /// Count each listed folder's children.
    pub count_children: bool,
    /// Progress and cancellation.
    pub handle: Option<Arc<ScanHandle>>,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            depth: 1,
            all: false,
            exclude: Vec::new(),
            sort: true,
            dirs_first: false,
            case_sensitive: false,
            name_contains: Vec::new(),
            extensions: Vec::new(),
            kind: EntryKind::Any,
            min_size: None,
            max_size: None,
            modified_after: None,
            modified_before: None,
            limit: None,
            dir_sizes: false,
            count_children: false,
            handle: None,
        }
    }
}

/// Result of [`scan`].
#[derive(Debug, Clone)]
pub struct Tree {
    /// Canonical root path, without the `\\?\` prefix on Windows.
    pub root: PathBuf,
    /// Sum of the top-level entries' sizes (folders count only with `dir_sizes`).
    pub total_size: u64,
    /// Number of the root's children, hidden ones included.
    pub child_count: usize,
    /// `limit` was reached; more matches exist.
    pub truncated: bool,
    pub entries: Vec<TreeEntry>,
}

impl Tree {
    /// All listed entries, depth-first, parents before children.
    pub fn iter(&self) -> Iter<'_> {
        Iter {
            stack: vec![self.entries.iter()],
        }
    }

    /// Only the entries that matched (skips folders listed just as parents).
    pub fn matches(&self) -> impl Iterator<Item = &TreeEntry> {
        self.iter().filter(|e| e.matched)
    }
}

/// Depth-first iterator over a [`Tree`].
pub struct Iter<'a> {
    stack: Vec<std::slice::Iter<'a, TreeEntry>>,
}

impl<'a> Iterator for Iter<'a> {
    type Item = &'a TreeEntry;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let top = self.stack.last_mut()?;
            match top.next() {
                Some(entry) => {
                    self.stack.push(entry.children.iter());
                    return Some(entry);
                }
                None => {
                    self.stack.pop();
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct TreeEntry {
    pub name: String,
    pub path: PathBuf,
    /// Without the dot, as on disk; `None` for folders.
    pub extension: Option<String>,
    pub is_dir: bool,
    pub is_exec: bool,
    /// Name starts with a dot.
    pub is_hidden: bool,
    pub modified: Option<SystemTime>,
    /// File size in bytes; 0 for folders.
    pub file_size: u64,
    /// File size, or for folders the cumulative size when `dir_sizes` is set.
    pub total_size: u64,
    /// Number of children (hidden included), when the folder was read.
    pub child_count: usize,
    /// The entry passed the search criteria itself (always true without
    /// criteria); false for folders listed only because a descendant matched.
    pub matched: bool,
    pub children: Vec<TreeEntry>,
    shown: bool,
}

/// Finds the first occurrence of `needle` in `haystack`, returning its byte
/// range in `haystack`. This is the matching `name_contains` uses.
pub fn find_pattern(haystack: &str, needle: &str, case_sensitive: bool) -> Option<Range<usize>> {
    if needle.is_empty() {
        return None;
    }
    if case_sensitive {
        return haystack.find(needle).map(|i| i..i + needle.len());
    }
    if needle.is_ascii() {
        // ASCII bytes only equal ASCII bytes, so the window lands on char boundaries.
        let (h, n) = (haystack.as_bytes(), needle.as_bytes());
        return h
            .windows(n.len())
            .position(|w| w.eq_ignore_ascii_case(n))
            .map(|i| i..i + n.len());
    }
    for (start, _) in haystack.char_indices() {
        let mut hay = haystack[start..].char_indices();
        let mut end = Some(start);
        for nc in needle.chars() {
            match hay.next() {
                Some((off, hc)) if hc == nc || hc.to_lowercase().eq(nc.to_lowercase()) => {
                    end = Some(start + off + hc.len_utf8());
                }
                _ => {
                    end = None;
                    break;
                }
            }
        }
        if let Some(end) = end {
            return Some(start..end);
        }
    }
    None
}

/// Scans `root` and returns the entries the options select, nested by folder.
///
/// Folders that can't be read (e.g. access denied) are treated as empty.
/// Returns an [`io::ErrorKind::Interrupted`] error if cancelled.
pub fn scan(root: &Path, opts: &ScanOptions) -> io::Result<Tree> {
    if !fs::metadata(root)?.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not a directory", root.display()),
        ));
    }

    let root = strip_verbatim(root.canonicalize()?);
    let ctx = Ctx::new(opts);

    let list = ctx.read_dir(&root, opts.sort)?;
    let child_count = list.len();
    let (mut entries, total_size) = if opts.depth > 0 {
        let entries = ctx.build_children(list, 0, true);
        let total = entries.iter().map(|e| e.total_size).sum();
        (entries, total)
    } else if opts.dir_sizes {
        (Vec::new(), ctx.size_of(list))
    } else {
        (Vec::new(), 0)
    };

    if ctx.cancelled() {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "scan cancelled"));
    }

    let mut budget = opts.limit.unwrap_or(usize::MAX);
    prune(&mut entries, &mut budget);

    Ok(Tree {
        root,
        total_size,
        child_count,
        truncated: ctx.limit_hit.load(Ordering::Relaxed),
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

/// Drops entries that aren't shown, and matches beyond `budget` (in tree order).
fn prune(entries: &mut Vec<TreeEntry>, budget: &mut usize) {
    entries.retain_mut(|e| {
        if !e.shown {
            return false;
        }
        let keep_self = e.matched && *budget > 0;
        if keep_self {
            *budget -= 1;
        }
        prune(&mut e.children, budget);
        keep_self || !e.children.is_empty()
    });
}

/// Per-scan state: normalized options plus match counting.
struct Ctx<'a> {
    opts: &'a ScanOptions,
    /// Any search criteria set: list only matches and their parents.
    search: bool,
    patterns: Vec<String>,
    extensions: Vec<String>,
    exclude: Vec<String>,
    found: AtomicUsize,
    limit_hit: AtomicBool,
}

impl<'a> Ctx<'a> {
    fn new(opts: &'a ScanOptions) -> Self {
        let fold = |s: &str| {
            if opts.case_sensitive {
                s.to_string()
            } else {
                s.to_lowercase()
            }
        };
        let patterns: Vec<String> = opts
            .name_contains
            .iter()
            .filter(|p| !p.is_empty())
            .cloned()
            .collect();
        let extensions: Vec<String> = opts
            .extensions
            .iter()
            .map(|e| e.trim().trim_start_matches('.'))
            .filter(|e| !e.is_empty())
            .map(fold)
            .collect();
        let exclude = opts.exclude.iter().map(|x| fold(x)).collect();
        let search = !patterns.is_empty()
            || !extensions.is_empty()
            || opts.kind == EntryKind::Files
            || opts.min_size.is_some()
            || opts.max_size.is_some()
            || opts.modified_after.is_some()
            || opts.modified_before.is_some();
        Self {
            opts,
            search,
            patterns,
            extensions,
            exclude,
            found: AtomicUsize::new(0),
            limit_hit: AtomicBool::new(false),
        }
    }

    fn fold<'s>(&self, s: &'s str) -> Cow<'s, str> {
        if self.opts.case_sensitive {
            Cow::Borrowed(s)
        } else {
            Cow::Owned(s.to_lowercase())
        }
    }

    fn cancelled(&self) -> bool {
        self.opts.handle.as_ref().is_some_and(|h| h.is_cancelled())
    }

    fn stopped(&self) -> bool {
        self.limit_hit.load(Ordering::Relaxed) || self.cancelled()
    }

    fn record_match(&self) {
        let found = self.found.fetch_add(1, Ordering::Relaxed) + 1;
        // Stop only once there is one more than the limit, so `truncated`
        // means more matches really exist.
        if self.opts.limit.is_some_and(|limit| found > limit) {
            self.limit_hit.store(true, Ordering::Relaxed);
        }
    }

    fn is_excluded(&self, name: &str) -> bool {
        !self.exclude.is_empty() && {
            let name = self.fold(name);
            self.exclude.iter().any(|x| *x == *name)
        }
    }

    fn matches(
        &self,
        name: &str,
        is_dir: bool,
        size: u64,
        modified: Option<SystemTime>,
        extension: Option<&str>,
    ) -> bool {
        let o = self.opts;
        match o.kind {
            EntryKind::Files if is_dir => return false,
            EntryKind::Folders if !is_dir => return false,
            _ => {}
        }
        if !self.patterns.is_empty()
            && !self
                .patterns
                .iter()
                .any(|p| find_pattern(name, p, o.case_sensitive).is_some())
        {
            return false;
        }
        if !self.extensions.is_empty() {
            let Some(ext) = extension.filter(|_| !is_dir) else {
                return false;
            };
            let ext = self.fold(ext);
            if !self.extensions.iter().any(|e| *e == *ext) {
                return false;
            }
        }
        if (o.min_size.is_some() || o.max_size.is_some())
            && (is_dir
                || o.min_size.is_some_and(|min| size < min)
                || o.max_size.is_some_and(|max| size > max))
        {
            return false;
        }
        if o.modified_after.is_some() || o.modified_before.is_some() {
            let Some(time) = modified.filter(|_| !is_dir) else {
                return false;
            };
            if o.modified_after.is_some_and(|after| time < after)
                || o.modified_before.is_some_and(|before| time > before)
            {
                return false;
            }
        }
        true
    }

    fn read_dir(&self, dir: &Path, sort: bool) -> io::Result<Vec<fs::DirEntry>> {
        if let Some(handle) = &self.opts.handle {
            handle.enter(dir);
        }
        let mut entries: Vec<fs::DirEntry> = fs::read_dir(dir)?.filter_map(Result::ok).collect();
        if sort {
            let dirs_first = self.opts.dirs_first;
            entries.sort_by_cached_key(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
                (dirs_first && !is_dir, name.to_lowercase(), name)
            });
        }
        Ok(entries)
    }

    /// Builds entries in parallel; `collect` keeps the (sorted) order.
    /// Entries that can't be read are skipped.
    fn build_children(
        &self,
        list: Vec<fs::DirEntry>,
        depth: usize,
        parent_visible: bool,
    ) -> Vec<TreeEntry> {
        list.into_par_iter()
            .filter_map(|entry| {
                if self.stopped() {
                    return None;
                }
                self.build_entry(entry, depth, parent_visible).ok()
            })
            .collect()
    }

    /// Total size of `list` and everything below it, without building entries.
    fn size_of(&self, list: Vec<fs::DirEntry>) -> u64 {
        list.into_par_iter()
            .map(|entry| {
                if self.stopped() {
                    return 0;
                }
                match entry.metadata() {
                    Ok(m) if m.is_file() => m.len(),
                    // DirEntry metadata doesn't follow links, so links are never descended.
                    Ok(m) if m.is_dir() => self
                        .read_dir(&entry.path(), false)
                        .map(|list| self.size_of(list))
                        .unwrap_or(0),
                    _ => 0,
                }
            })
            .sum()
    }

    fn build_entry(
        &self,
        entry: fs::DirEntry,
        depth: usize,
        parent_visible: bool,
    ) -> io::Result<TreeEntry> {
        let opts = self.opts;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        // DirEntry::metadata doesn't follow symlinks/junctions.
        let metadata = entry.metadata()?;
        let is_symlink = metadata.file_type().is_symlink();
        let is_dir = metadata.is_dir();
        let is_hidden = name.starts_with('.');
        let modified = metadata.modified().ok();
        let extension = if is_dir {
            None
        } else {
            path.extension().map(|e| e.to_string_lossy().into_owned())
        };
        let file_size = if metadata.is_file() {
            metadata.len()
        } else {
            0
        };

        let visible =
            parent_visible && !is_symlink && (opts.all || !is_hidden) && !self.is_excluded(&name);
        let matched =
            visible && self.matches(&name, is_dir, file_size, modified, extension.as_deref());
        if matched {
            self.record_match();
        }

        let mut total_size = file_size;
        let mut child_count = 0;
        let mut children = Vec::new();

        if is_dir {
            // Search looks inside every visible folder; a plain listing only
            // opens folders that are themselves listed.
            let show_children = visible && depth + 1 < opts.depth && (self.search || matched);
            let read = show_children || opts.dir_sizes || (opts.count_children && matched);

            if read && !self.stopped() {
                // Unreadable folders (e.g. access denied) count as empty.
                let list = self
                    .read_dir(&path, opts.sort && show_children)
                    .unwrap_or_default();
                child_count = list.len();
                if show_children {
                    children = self.build_children(list, depth + 1, visible);
                    total_size += children.iter().map(|c| c.total_size).sum::<u64>();
                } else if opts.dir_sizes {
                    total_size += self.size_of(list);
                }
            }
        }

        let shown = matched || (self.search && children.iter().any(|c| c.shown));

        Ok(TreeEntry {
            is_exec: is_executable(&path, &metadata),
            name,
            path,
            extension,
            is_dir,
            is_hidden,
            modified,
            file_size,
            total_size,
            child_count,
            matched,
            children,
            shown,
        })
    }
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
    use std::time::Duration;

    /// Creates a fresh temp tree:
    /// root/a.txt (3B), root/B.RS (5B), root/sub/c.rs (7B),
    /// root/sub/deep/d.txt (11B), root/.hid (1B), root/skip/e.rs (2B)
    fn fixture(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("tr-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("sub/deep")).unwrap();
        fs::create_dir_all(root.join("skip")).unwrap();
        fs::write(root.join("a.txt"), "aaa").unwrap();
        fs::write(root.join("B.RS"), "bbbbb").unwrap();
        fs::write(root.join("sub/c.rs"), "ccccccc").unwrap();
        fs::write(root.join("sub/deep/d.txt"), "ddddddddddd").unwrap();
        fs::write(root.join(".hid"), "h").unwrap();
        fs::write(root.join("skip/e.rs"), "ee").unwrap();
        root
    }

    fn names(entries: &[TreeEntry]) -> Vec<&str> {
        entries.iter().map(|e| e.name.as_str()).collect()
    }

    fn matched(tree: &Tree) -> Vec<&str> {
        tree.matches().map(|e| e.name.as_str()).collect()
    }

    fn deep() -> ScanOptions {
        ScanOptions {
            depth: usize::MAX,
            ..Default::default()
        }
    }

    #[test]
    fn depth_limits_children() {
        let root = fixture("depth");
        let tree = scan(&root, &ScanOptions::default()).unwrap();
        assert_eq!(names(&tree.entries), ["a.txt", "B.RS", "skip", "sub"]);
        assert!(tree.entries[3].children.is_empty());

        let opts = ScanOptions {
            depth: 2,
            ..Default::default()
        };
        let tree = scan(&root, &opts).unwrap();
        assert_eq!(names(&tree.entries[3].children), ["c.rs", "deep"]);
        assert!(tree.entries[3].children[1].children.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn dirs_first_sorting() {
        let root = fixture("dirsfirst");
        let opts = ScanOptions {
            dirs_first: true,
            ..Default::default()
        };
        let tree = scan(&root, &opts).unwrap();
        assert_eq!(names(&tree.entries), ["skip", "sub", "a.txt", "B.RS"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn hidden_and_exclude() {
        let root = fixture("hidden");
        let opts = ScanOptions {
            all: true,
            ..Default::default()
        };
        let tree = scan(&root, &opts).unwrap();
        assert_eq!(
            names(&tree.entries),
            [".hid", "a.txt", "B.RS", "skip", "sub"]
        );

        let opts = ScanOptions {
            exclude: vec!["SUB".into()],
            ..Default::default()
        };
        let tree = scan(&root, &opts).unwrap();
        assert_eq!(names(&tree.entries), ["a.txt", "B.RS", "skip"]);
        assert_eq!(tree.child_count, 5);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn name_filter_case_insensitive_by_default() {
        let root = fixture("case");
        let opts = ScanOptions {
            name_contains: vec![".rs".into()],
            ..deep()
        };
        let tree = scan(&root, &opts).unwrap();
        assert_eq!(matched(&tree), ["B.RS", "e.rs", "c.rs"]);
        // Parent folders are listed but not matched.
        assert_eq!(names(&tree.entries), ["B.RS", "skip", "sub"]);

        let opts = ScanOptions {
            case_sensitive: true,
            ..opts
        };
        let tree = scan(&root, &opts).unwrap();
        assert_eq!(matched(&tree), ["e.rs", "c.rs"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn exclude_applies_during_search() {
        let root = fixture("exclude");
        let opts = ScanOptions {
            name_contains: vec![".rs".into()],
            exclude: vec!["skip".into()],
            ..deep()
        };
        let tree = scan(&root, &opts).unwrap();
        assert_eq!(matched(&tree), ["B.RS", "c.rs"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn extension_kind_and_size_filters() {
        let root = fixture("filters");
        let opts = ScanOptions {
            extensions: vec![".TXT".into()],
            ..deep()
        };
        assert_eq!(matched(&scan(&root, &opts).unwrap()), ["a.txt", "d.txt"]);

        let opts = ScanOptions {
            min_size: Some(5),
            max_size: Some(7),
            ..deep()
        };
        assert_eq!(matched(&scan(&root, &opts).unwrap()), ["B.RS", "c.rs"]);

        let opts = ScanOptions {
            kind: EntryKind::Folders,
            ..deep()
        };
        assert_eq!(
            matched(&scan(&root, &opts).unwrap()),
            ["skip", "sub", "deep"]
        );

        let opts = ScanOptions {
            kind: EntryKind::Files,
            ..deep()
        };
        assert_eq!(scan(&root, &opts).unwrap().matches().count(), 5);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn modified_filter() {
        let root = fixture("modified");
        let hour = Duration::from_secs(3600);
        let opts = ScanOptions {
            modified_after: Some(SystemTime::now() - hour),
            ..deep()
        };
        assert_eq!(scan(&root, &opts).unwrap().matches().count(), 5);

        let opts = ScanOptions {
            modified_before: Some(SystemTime::now() - hour),
            ..deep()
        };
        assert_eq!(scan(&root, &opts).unwrap().matches().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn limit_truncates() {
        let root = fixture("limit");
        let opts = ScanOptions {
            kind: EntryKind::Files,
            limit: Some(2),
            ..deep()
        };
        let tree = scan(&root, &opts).unwrap();
        assert!(tree.truncated);
        assert_eq!(tree.matches().count(), 2);

        // Exactly as many matches as the limit is not truncated.
        let opts = ScanOptions {
            limit: Some(5),
            ..opts
        };
        let tree = scan(&root, &opts).unwrap();
        assert!(!tree.truncated);
        assert_eq!(tree.matches().count(), 5);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cancel_returns_interrupted() {
        let root = fixture("cancel");
        let handle = ScanHandle::new();
        handle.cancel();
        let opts = ScanOptions {
            handle: Some(handle),
            ..deep()
        };
        let err = scan(&root, &opts).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Interrupted);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn handle_tracks_folders_read() {
        let root = fixture("progress");
        let handle = ScanHandle::new();
        let opts = ScanOptions {
            handle: Some(handle.clone()),
            ..deep()
        };
        let tree = scan(&root, &opts).unwrap();
        assert_eq!(handle.dirs_read(), 4); // root, skip, sub, sub/deep
        assert!(handle.current_dir().starts_with(&tree.root));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn dir_sizes_are_cumulative() {
        let root = fixture("sizes");
        let opts = ScanOptions {
            dir_sizes: true,
            ..Default::default()
        };
        let tree = scan(&root, &opts).unwrap();
        let sub = tree.entries.iter().find(|e| e.name == "sub").unwrap();
        assert_eq!(sub.total_size, 18);
        // Hidden .hid still counts toward the total.
        assert_eq!(tree.total_size, 1 + 3 + 5 + 2 + 18);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn find_pattern_ranges() {
        assert_eq!(find_pattern("Main.RS", ".rs", false), Some(4..7));
        assert_eq!(find_pattern("Main.RS", ".rs", true), None);
        assert_eq!(find_pattern("ÄBC", "äb", false), Some(0..3));
        assert_eq!(find_pattern("abc", "", false), None);
    }
}
