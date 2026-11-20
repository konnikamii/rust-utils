# tr — directory tree viewer

Small, fast directory tree printer inspired by the Unix `tree` utility.

## Usage

Runs as a small CLI. Example:

```
cargo run -p tr -- [DIRECTORY]
```

If no directory is provided, the current directory is used.

## Options (short)

- `-a`, `--all`: Show hidden files (files starting with `.`).
- `-x`, `--hide-name`: Comma-separated exact names to exclude; never shown or searched into.
- `-d`, `--depth`: Maximum recursion depth (default: `1`).
- `-n`, `--file-count`: Show number of files in each directory.
- `-s`, `--file-sizes`: Show file sizes for files.
- `-S`, `--folder-sizes`: Show cumulative sizes for directories.
- `--no-sort`: Preserve filesystem order instead of sorting entries by name.
- `--dirs-first`: List folders before files.
- `-o`, `--folders-only`: Show only folders (hide files).
- `-c`, `--clean`: Disable colored output.

Search (only matches and the folders leading to them are shown; combine freely):

- `-F`, `--filter`: Name contains any of these comma-separated substrings.
- `-e`, `--ext`: File extension is one of these (e.g. `jpg,png`).
- `--files-only`: Match only files.
- `--min-size` / `--max-size`: File size bounds, e.g. `500K`, `10M`, `1.5G`.
- `--newer` / `--older`: Modified within / longer ago than, e.g. `30m`, `12h`, `7d`, `2w`.
- `-C`, `--case-sensitive`: Make `-F`, `-e` and `-x` case-sensitive (default: insensitive).
- `-l`, `--limit`: Stop after this many matches.

## Behavior

- The tool scans the specified directory and prints a tree of entries.
- By default it hides dotfiles and symlinks (never followed) and sorts entries by name, case-insensitively.
- Size and file-count options may require reading child entries and therefore increase scan time.
- Search only looks as deep as `-d`; use a large value (e.g. `-d 99`) to search everything.

## Examples

Show a colored tree with file sizes and two levels of depth:

```
cargo run -p tr -- -sS -d 2 path/to/dir
```

Find videos over 100 MB changed in the last week, skipping `node_modules`:

```
cargo run -p tr -- -e mp4,mkv --min-size 100M --newer 7d -x node_modules -d 99 path/to/dir
```

## Library

The scanner is also a library, so other Rust apps can call it directly instead of
spawning the CLI. Turn off default features to skip the CLI-only dependencies (clap, colored):

```toml
[dependencies]
tr = { path = "../rust-utils/crates/tr", default-features = false }
# or: tr = { git = "https://github.com/konnikamii/rust-utils", default-features = false }
```

```rust
use tr::{EntryKind, ScanHandle, ScanOptions};

// Explorer-style listing of one folder.
let opts = ScanOptions { dirs_first: true, ..Default::default() };
let tree = tr::scan(Path::new("C:/some/dir"), &opts)?;
for entry in &tree.entries {
    println!("{} {} {:?}", entry.path.display(), entry.total_size, entry.modified);
}

// Cancellable search with a result cap.
let handle = ScanHandle::new();
let opts = ScanOptions {
    depth: usize::MAX,
    name_contains: vec!["report".into()],
    extensions: vec!["pdf".into(), "docx".into()],
    kind: EntryKind::Files,
    exclude: vec!["node_modules".into(), "$Recycle.Bin".into()],
    limit: Some(300),
    handle: Some(handle.clone()),
    ..Default::default()
};
let tree = tr::scan(Path::new("C:/"), &opts)?; // handle.cancel() from another thread stops it
for hit in tree.matches() { /* flat list of matches */ }
```

- `scan` returns only the entries the options select (the same ones the CLI prints), nested via
  `children`. `Tree::iter()` walks them depth-first, and `Tree::matches()` skips folders that are
  listed only because something inside matched (`TreeEntry::matched == false`).
- Matching is case-insensitive unless `case_sensitive` is set; `tr::find_pattern` exposes the same
  matching, e.g. for highlighting.
- `limit` stops the scan once more matches exist and sets `Tree::truncated`. Which matches are found
  first depends on thread timing; sizes and counts are partial then.
- `ScanHandle` gives live `current_dir()` / `dirs_read()` (the CLI spinner uses it) and `cancel()`,
  which makes `scan` return an `io::ErrorKind::Interrupted` error.
- `Tree::total_size` and `child_count` also count hidden entries.
- `scan` blocks and uses rayon's global thread pool, so in an async app (Tauri) run it inside
  `spawn_blocking`.

## License / Notes

This is a small utility intended for local development and inspection.
